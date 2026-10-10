// Copyright (c) 2026 nikcodex (KizunaLink)
// Licensed under the MIT License

use std::io::ErrorKind;
use std::panic::{AssertUnwindSafe, catch_unwind};

use flume::Receiver;
use symphonia::core::{
    audio::SampleBuffer,
    codecs::Decoder,
    errors::Error,
    formats::{FormatReader, SeekMode, SeekTo},
    io::MediaSource,
    units::Time,
};
use tracing::{Level, debug, span, warn};

use crate::{
    common::types::AudioFormat,
    config::player::{PlayerConfig, ResamplingQuality},
    engine::{
        AudioFrame,
        constants::{MIXER_CHANNELS, TARGET_SAMPLE_RATE},
        demux::{DemuxResult, open_format},
        engine::{BoxedEngine, StandardEngine},
        resample::Resampler,
    },
};

#[derive(Debug, Clone, PartialEq)]
pub enum DecoderCommand {
    Seek(u64),
    Stop,
}

#[derive(Debug, PartialEq)]
pub enum CommandOutcome {
    Stop,
    Seeked,
    SeekFailed,
    None,
}

pub struct AudioProcessor {
    format: Box<dyn FormatReader>,
    decoder: Box<dyn Decoder>,
    resampler: Resampler,
    track_id: u32,
    engine: BoxedEngine,
    cmd_rx: Receiver<DecoderCommand>,
    error_tx: Option<flume::Sender<String>>,
    sample_buf: Option<SampleBuffer<i16>>,
    source_rate: u32,
    channels: usize,
    config: PlayerConfig,
    recoverable_errors: u32,
    downmix_buf: Vec<i16>,
}

impl AudioProcessor {
    /// Builds the resampler for a given source rate, honoring the configured
    /// quality. Passthrough (linear) is used when no resampling is needed.
    fn build_resampler(source_rate: u32, quality: ResamplingQuality) -> Resampler {
        if source_rate == TARGET_SAMPLE_RATE {
            return Resampler::linear(source_rate, TARGET_SAMPLE_RATE, MIXER_CHANNELS);
        }
        match quality {
            ResamplingQuality::Low => {
                Resampler::linear(source_rate, TARGET_SAMPLE_RATE, MIXER_CHANNELS)
            }
            ResamplingQuality::Medium => {
                Resampler::hermite(source_rate, TARGET_SAMPLE_RATE, MIXER_CHANNELS)
            }
            ResamplingQuality::High => {
                Resampler::sinc(source_rate, TARGET_SAMPLE_RATE, MIXER_CHANNELS)
            }
        }
    }

    pub fn new(
        source: Box<dyn MediaSource>,
        kind: Option<AudioFormat>,
        frame_tx: flume::Sender<AudioFrame>,
        cmd_rx: Receiver<DecoderCommand>,
        error_tx: Option<flume::Sender<String>>,
        config: PlayerConfig,
    ) -> Result<Self, Error> {
        Self::with_engine(
            source,
            kind,
            Box::new(StandardEngine::new(frame_tx)),
            cmd_rx,
            error_tx,
            config,
        )
    }

    /// Opens an audio source and creates a processor that writes decoded stereo
    /// PCM to `engine` at the mixer sample rate.
    ///
    /// Returns the format-probing, track-selection, or decoder error encountered
    /// while opening the source.
    pub fn with_engine(
        source: Box<dyn MediaSource>,
        kind: Option<AudioFormat>,
        engine: BoxedEngine,
        cmd_rx: Receiver<DecoderCommand>,
        error_tx: Option<flume::Sender<String>>,
        config: PlayerConfig,
    ) -> Result<Self, Error> {
        let DemuxResult::Transcode {
            format,
            track_id,
            decoder,
            sample_rate,
            channels,
        } = open_format(source, kind)?;

        debug!(
            "AudioProcessor: opened format — {}Hz {}ch",
            sample_rate, channels
        );

        let resampler = Self::build_resampler(sample_rate, config.resampling_quality);

        Ok(Self {
            format,
            decoder,
            resampler,
            track_id,
            engine,
            cmd_rx,
            error_tx,
            sample_buf: None,
            source_rate: sample_rate,
            channels,
            config,
            recoverable_errors: 0,
            downmix_buf: Vec::with_capacity(1920),
        })
    }

    /// Processes audio until the source ends, a stop command arrives, or the
    /// output engine stops accepting frames.
    ///
    /// Packet-read and nonrecoverable decode errors are forwarded to the optional
    /// error channel and returned to the caller. Recoverable decode errors are
    /// skipped.
    pub fn run(&mut self) -> Result<(), Error> {
        let _span = span!(Level::DEBUG, "audio_processor").entered();

        debug!(
            "Starting transcode loop: {}Hz {}ch -> {}Hz",
            self.source_rate, self.channels, TARGET_SAMPLE_RATE
        );

        let mut packet_count = 0u64;
        loop {
            packet_count += 1;
            if self.check_commands() == CommandOutcome::Stop {
                break;
            }

            let packet = match self.format.next_packet() {
                Ok(p) => p,
                Err(Error::IoError(e)) if e.kind() == ErrorKind::UnexpectedEof => break,
                Err(e) => {
                    self.send_error(format!("Packet read error: {e}"));
                    return Err(e);
                }
            };

            if packet.track_id() != self.track_id {
                continue;
            }

            match self.decoder.decode(&packet) {
                Ok(decoded) => {
                    self.recoverable_errors = 0;
                    let spec = *decoded.spec();
                    let mut buf = self.sample_buf.take().unwrap_or_else(|| {
                        SampleBuffer::<i16>::new(decoded.capacity() as u64, spec)
                    });

                    buf.copy_interleaved_ref(decoded);
                    let samples = buf.samples();

                    if !samples.is_empty() {
                        let frame_channels = spec.channels.count();
                        let frame_rate = spec.rate;

                        if frame_rate != self.source_rate {
                            debug!(
                                "AudioProcessor: frame rate mismatch ({}Hz vs {}Hz) — re-initializing resampler",
                                frame_rate, self.source_rate
                            );
                            self.source_rate = frame_rate;
                            self.resampler =
                                Self::build_resampler(frame_rate, self.config.resampling_quality);
                        }

                        let pcm_data = if frame_channels == MIXER_CHANNELS {
                            samples
                        } else {
                            if packet_count.is_multiple_of(100) {
                                debug!(
                                    "AudioProcessor: Downmixing {}ch -> {}ch (samples: {})",
                                    frame_channels,
                                    MIXER_CHANNELS,
                                    samples.len()
                                );
                            }
                            let num_frames = samples.len() / frame_channels;
                            self.downmix_buf.clear();
                            self.downmix_buf.reserve(num_frames * MIXER_CHANNELS);

                            for i in 0..num_frames {
                                let frame = &samples[i * frame_channels..(i + 1) * frame_channels];
                                let mut l = 0i32;
                                let mut r = 0i32;

                                for (ch, &sample) in frame.iter().enumerate() {
                                    if ch % 2 == 0 {
                                        l += sample as i32;
                                    } else {
                                        r += sample as i32;
                                    }
                                }

                                let left_count = frame_channels.div_ceil(2);
                                let right_count = frame_channels / 2;

                                self.downmix_buf.push((l / left_count as i32) as i16);
                                if right_count > 0 {
                                    self.downmix_buf.push((r / right_count as i32) as i16);
                                } else {
                                    // Upmix mono to stereo
                                    self.downmix_buf.push((l / left_count as i32) as i16);
                                }
                            }
                            &self.downmix_buf[..]
                        };

                        let capacity = (pcm_data.len() as f64 * TARGET_SAMPLE_RATE as f64
                            / self.source_rate as f64)
                            .ceil() as usize
                            + 32;
                        let mut resampled = crate::engine::buffer::acquire_buffer(capacity);
                        if self.resampler.is_passthrough() {
                            resampled.extend_from_slice(pcm_data);
                        } else {
                            self.resampler.process(pcm_data, &mut resampled);
                        }

                        if !resampled.is_empty() && !self.engine.push(AudioFrame::Pcm(resampled)) {
                            return Ok(());
                        }
                    }

                    self.sample_buf = Some(buf);
                }
                Err(Error::IoError(e)) if e.kind() == ErrorKind::UnexpectedEof => break,
                Err(Error::DecodeError(e)) => {
                    self.recoverable_errors += 1;
                    if self.recoverable_errors == 1 {
                        warn!("Decode error (recoverable): {e}");
                    } else if self.recoverable_errors.is_multiple_of(100) {
                        warn!(
                            "Decode error (recoverable, x{}): {e}",
                            self.recoverable_errors
                        );
                    }
                }
                Err(e) => {
                    self.send_error(format!("Decode error: {e}"));
                    return Err(e);
                }
            }
        }

        debug!("Transcode loop finished");
        Ok(())
    }

    fn check_commands(&mut self) -> CommandOutcome {
        match self.cmd_rx.try_recv() {
            Ok(DecoderCommand::Seek(ms)) => {
                let time = Time::from(ms as f64 / 1000.0);
                if self
                    .format
                    .seek(
                        SeekMode::Coarse,
                        SeekTo::Time {
                            time,
                            track_id: Some(self.track_id),
                        },
                    )
                    .is_ok()
                {
                    self.resampler.reset();
                    self.decoder.reset();
                    self.sample_buf = None;
                    let _ = self.engine.push(AudioFrame::Pcm(Vec::new()));
                    CommandOutcome::Seeked
                } else {
                    warn!("AudioProcessor: seek to {}ms failed", ms);
                    CommandOutcome::SeekFailed
                }
            }
            Ok(DecoderCommand::Stop) | Err(flume::TryRecvError::Disconnected) => {
                CommandOutcome::Stop
            }
            _ => CommandOutcome::None,
        }
    }

    fn send_error(&self, msg: String) {
        if let Some(tx) = &self.error_tx {
            let _ = tx.send(msg);
        }
    }

    /// Runs the transcode loop with panic isolation.
    ///
    /// A panic anywhere in the decoder (symphonia, a format reader, the engine)
    /// would otherwise unwind out of this thread and drop every voice session with
    /// it. Catching it here confines the failure to this one track: the error is
    /// surfaced to the same `error_tx` channel the feed loop already watches, so
    /// the player reports "load failed" and moves on. Requires the release profile
    /// to keep `panic = "unwind"`; with `panic = "abort"` this guard is dead code.
    pub fn run_guarded(&mut self) -> Result<(), Error> {
        match catch_unwind(AssertUnwindSafe(|| self.run())) {
            Ok(result) => result,
            Err(payload) => {
                let detail = panic_message(&payload);
                self.send_error(format!("decoder panicked (track isolated): {detail}"));
                Err(Error::IoError(std::io::Error::other(format!(
                    "decoder panicked: {detail}"
                ))))
            }
        }
    }
}

/// Best-effort extraction of a human-readable message from a panic payload.
fn panic_message(payload: &Box<dyn std::any::Any + Send>) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "unknown panic".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::engine::Engine;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    /// Counts pushes and reports the pipeline healthy.
    struct CountingEngine(Arc<AtomicUsize>);
    impl Engine for CountingEngine {
        fn push(&mut self, _frame: AudioFrame) -> bool {
            self.0.fetch_add(1, Ordering::Relaxed);
            true
        }
    }

    /// Delivers `ok_pushes` frames, then panics on the next one, simulating an
    /// engine/decoder fault partway through a stream.
    struct PanicAfter {
        ok_pushes: usize,
    }
    impl Engine for PanicAfter {
        fn push(&mut self, _frame: AudioFrame) -> bool {
            if self.ok_pushes == 0 {
                panic!("simulated mid-stream engine fault");
            }
            self.ok_pushes -= 1;
            true
        }
    }

    /// Minimal 100 ms stereo 48 kHz 16-bit WAV fixture (generated, not committed).
    fn wav_bytes() -> Vec<u8> {
        let samples = 4800u32;
        let data_len = samples * 4;
        let mut wav = Vec::new();
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(36 + data_len).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&2u16.to_le_bytes());
        wav.extend_from_slice(&48_000u32.to_le_bytes());
        wav.extend_from_slice(&192_000u32.to_le_bytes());
        wav.extend_from_slice(&4u16.to_le_bytes());
        wav.extend_from_slice(&16u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&data_len.to_le_bytes());
        for i in 0..samples {
            let s = ((i as f32 * 0.1).sin() * 8000.0) as i16;
            wav.extend_from_slice(&s.to_le_bytes());
            wav.extend_from_slice(&s.to_le_bytes());
        }
        wav
    }

    fn processor(engine: Box<dyn Engine>) -> (AudioProcessor, flume::Sender<DecoderCommand>) {
        let path = std::env::temp_dir().join(format!("kizuna-proc-{}.wav", uuid::Uuid::new_v4()));
        std::fs::write(&path, wav_bytes()).expect("write wav fixture");
        let file = std::fs::File::open(&path).expect("open wav fixture");
        let _ = std::fs::remove_file(&path);
        let (cmd_tx, cmd_rx) = flume::unbounded();
        let processor = AudioProcessor::with_engine(
            Box::new(file),
            Some(AudioFormat::Wav),
            engine,
            cmd_rx,
            None,
            PlayerConfig::default(),
        )
        .expect("processor builds from valid WAV");
        // The command sender must outlive `run`; a dropped sender ends the loop
        // before the first packet, exactly as it would in production.
        (processor, cmd_tx)
    }

    #[test]
    fn run_guarded_completes_cleanly_on_valid_stream() {
        let pushed = Arc::new(AtomicUsize::new(0));
        let (mut p, _cmd_tx) = processor(Box::new(CountingEngine(pushed.clone())));
        assert!(
            p.run_guarded().is_ok(),
            "valid stream must decode without error"
        );
        assert!(
            pushed.load(Ordering::Relaxed) > 0,
            "decoder produced no frames"
        );
    }

    /// The whole point of `run_guarded`: a panic while decoding must surface as an
    /// `Err` on this thread instead of unwinding out and (in release) aborting the
    /// process. The engine accepts the first frame then panics, so the transcode
    /// loop is provably mid-stream when the fault hits.
    #[test]
    fn run_guarded_confines_decoder_panic_to_an_error() {
        let (mut p, _cmd_tx) = processor(Box::new(PanicAfter { ok_pushes: 1 }));
        match p.run_guarded() {
            Err(Error::IoError(e)) => {
                assert!(
                    e.to_string().contains("decoder panicked"),
                    "panic must be reported as an IoError, got: {e}"
                );
            }
            other => panic!("expected a confined panic error, got {other:?}"),
        }
    }

    #[test]
    fn panic_message_extracts_str_and_string_payloads() {
        let s: Box<dyn std::any::Any + Send> = Box::new("boom");
        assert_eq!(panic_message(&s), "boom");
        let s2: Box<dyn std::any::Any + Send> = Box::new(String::from("kaboom"));
        assert_eq!(panic_message(&s2), "kaboom");
        let other: Box<dyn std::any::Any + Send> = Box::new(7u32);
        assert_eq!(panic_message(&other), "unknown panic");
    }
}
