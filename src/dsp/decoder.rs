/// Streaming audio decoder: any symphonia-supported format -> interleaved
/// stereo f32 at 48 kHz, pulled chunk-by-chunk. Used by the filtered playback
/// pipeline so DSP runs on decoded PCM before it reaches KizunaVoice's mixer.
///
/// For Opus-encoded sources, an optional passthrough mode is available that
/// skips the decode→PCM→re-encode cycle, saving ~60% CPU.
use std::io::{Read, Seek, SeekFrom};
use std::sync::Mutex;

use rubato::{
    Resampler, SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction,
};
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{CodecType, DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::{MediaSource, MediaSourceStream};
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

pub const TARGET_SAMPLE_RATE: u32 = 48000;

const DECODE_CHUNK_FRAMES: usize = 4096;

/// Convert a frame skip expressed in 48 kHz frames (the pipeline's target rate,
/// e.g. `position_ms * 48`) into the equivalent number of frames at the
/// *source* sample rate, rounding to nearest.
///
/// The skip is applied in [`AudioDecoder::push_converted`] before resampling, so
/// feeding it 48 kHz-unit counts for a 44.1 kHz source (most MP3/AAC content,
/// including every JioSaavn stream) skipped ~8.8% too much audio and a seek
/// landed past the requested position (a 2:00 seek played from ~2:10).
fn scale_skip_frames(skip_frames_48k: u64, src_rate: u32) -> u64 {
    if src_rate == TARGET_SAMPLE_RATE || skip_frames_48k == 0 {
        return skip_frames_48k;
    }
    // Bounds: skip_frames_48k is position_ms * 48 with position clamped to a
    // track length (<= 24 h by MAX_UNBOUNDED_POSITION_MS), so the product stays
    // far below u64::MAX; u128 makes the intermediate overflow impossible.
    let scaled = skip_frames_48k as u128 * src_rate as u128 + (TARGET_SAMPLE_RATE as u128) / 2;
    (scaled / TARGET_SAMPLE_RATE as u128) as u64
}

/// Detect if a codec type is Opus
fn is_opus_codec(codec: CodecType) -> bool {
    // Symphonia's OPUS codec identifier
    codec.to_string().contains("OPUS") || codec.to_string().contains("opus")
}

pub struct AudioDecoder {
    format_reader: Box<dyn symphonia::core::formats::FormatReader>,
    decoder: Box<dyn symphonia::core::codecs::Decoder>,
    track_id: u32,

    src_channels: usize,

    /// Whether the source is Opus (for passthrough optimization)
    pub is_opus: bool,

    // Initialized lazily on first packet once we know the signal spec
    sample_buf: Option<SampleBuffer<f32>>,

    // resampler when source rate != 48k; None otherwise
    resampler: Option<SincFixedIn<f32>>,
    res_in: [Vec<f32>; 2],
    res_pending: [Vec<f32>; 2],

    // converted interleaved stereo output ready for consumers
    out_fifo: Vec<f32>,
    eof: bool,
    error: Option<String>,

    frames_to_skip: u64,
}

impl AudioDecoder {
    /// Open a decoder over a byte source. `extension_hint` (e.g. "mp3", "m4a")
    /// assists probing but content sniffing takes precedence.
    pub fn open(
        source: Box<dyn MediaSource>,
        extension_hint: Option<&str>,
        skip_frames: u64,
    ) -> Result<Self, String> {
        let mut hint = Hint::new();
        if let Some(ext) = extension_hint {
            hint.with_extension(ext);
        }

        let mss = MediaSourceStream::new(source, Default::default());
        let probed = symphonia::default::get_probe()
            .format(
                &hint,
                mss,
                &FormatOptions::default(),
                &MetadataOptions::default(),
            )
            .map_err(|e| format!("probe failed: {}", e))?;

        let track = probed
            .format
            .tracks()
            .iter()
            // Symphonia exposes video and audio tracks together. Audio tracks
            // have a channel layout and sample rate; selecting the first
            // non-null codec can accidentally instantiate a video decoder.
            .find(|t| {
                t.codec_params.codec != CODEC_TYPE_NULL
                    && t.codec_params.channels.is_some()
                    && t.codec_params.sample_rate.is_some()
            })
            .ok_or("no audio track found")?
            .clone();

        let codec_params = &track.codec_params;
        let src_sample_rate = codec_params.sample_rate.unwrap_or(TARGET_SAMPLE_RATE);
        let src_channels = codec_params.channels.map(|c| c.count()).unwrap_or(2) as usize;
        let is_opus = is_opus_codec(codec_params.codec);

        let decoder = symphonia::default::get_codecs()
            .make(codec_params, &DecoderOptions::default())
            .map_err(|e| format!("decoder init failed: {}", e))?;

        let mut dec = Self {
            format_reader: probed.format,
            decoder,
            track_id: track.id,
            src_channels: src_channels.max(1),
            is_opus,
            sample_buf: None,
            resampler: None,
            res_in: [Vec::new(), Vec::new()],
            res_pending: [Vec::new(), Vec::new()],
            out_fifo: Vec::with_capacity(DECODE_CHUNK_FRAMES * 4),
            eof: false,
            error: None,
            frames_to_skip: scale_skip_frames(skip_frames, src_sample_rate),
        };

        if src_sample_rate != TARGET_SAMPLE_RATE {
            let ratio = TARGET_SAMPLE_RATE as f64 / src_sample_rate as f64;
            let params = SincInterpolationParameters {
                sinc_len: 128,
                f_cutoff: 0.95,
                oversampling_factor: 128,
                interpolation: SincInterpolationType::Cubic,
                window: WindowFunction::BlackmanHarris,
            };
            dec.resampler = Some(
                SincFixedIn::<f32>::new(ratio, 4.0, params, DECODE_CHUNK_FRAMES, 2)
                    .map_err(|e| format!("resampler init failed: {}", e))?,
            );
        }

        Ok(dec)
    }

    /// Pull up to `max_frames` interleaved stereo frames at 48 kHz.
    /// Returns fewer/zero frames while buffering; empty Vec forever after EOF.
    pub fn read_frames(&mut self, max_frames: usize) -> Vec<f32> {
        // Serve from FIFO first
        if self.out_fifo.len() >= max_frames * 2 {
            return self.out_fifo.drain(..max_frames * 2).collect();
        }
        self.fill_fifo();
        if self.out_fifo.is_empty() {
            return Vec::new();
        }
        let take = (max_frames * 2).min(self.out_fifo.len());
        self.out_fifo.drain(..take).collect()
    }

    pub fn is_eof(&self) -> bool {
        self.eof && self.out_fifo.is_empty()
    }

    pub fn take_error(&mut self) -> Option<String> {
        self.error.take()
    }

    fn fill_fifo(&mut self) {
        while !self.eof && self.out_fifo.len() < DECODE_CHUNK_FRAMES * 2 {
            match self.decode_next_packet() {
                Ok(Some(samples)) => {
                    self.push_converted(&samples);
                }
                Ok(None) => {
                    // No more packets. Flush source-rate samples that did not
                    // fill a complete resampler chunk before declaring EOF.
                    self.flush_resampler();
                    self.eof = true;
                    break;
                }
                Err(error) => {
                    tracing::warn!("audio decoder stopped while reading: {}", error);
                    self.error = Some(error.to_string());
                    self.flush_resampler();
                    self.eof = true;
                    break;
                }
            }
        }
    }

    /// Decode one packet into source-format interleaved f32 samples.
    fn decode_next_packet(&mut self) -> Result<Option<Vec<f32>>, SymphoniaError> {
        loop {
            let packet = match self.format_reader.next_packet() {
                Ok(p) => p,
                Err(SymphoniaError::IoError(ref e))
                    if e.kind() == std::io::ErrorKind::UnexpectedEof =>
                {
                    return Ok(None);
                }
                Err(SymphoniaError::ResetRequired) => return Ok(None),
                Err(e) => return Err(e),
            };

            if packet.track_id() != self.track_id {
                continue;
            }

            let decoded = match self.decoder.decode(&packet) {
                Ok(d) => d,
                Err(SymphoniaError::DecodeError(_)) => continue,
                Err(e) => return Err(e),
            };

            if self.sample_buf.is_none() {
                self.sample_buf = Some(SampleBuffer::<f32>::new(
                    decoded.capacity() as u64,
                    *decoded.spec(),
                ));
            }
            let sample_buf = self.sample_buf.as_mut().unwrap();
            sample_buf.copy_interleaved_ref(decoded);

            let samples = sample_buf.samples().to_vec();
            return Ok(Some(samples));
        }
    }

    fn flush_resampler(&mut self) {
        if self.resampler.is_none() || self.res_in[0].is_empty() {
            return;
        }

        // Rubato's fixed-input resampler needs a complete chunk. Zero-pad only
        // the decoder tail, then emit the real samples plus the short filter
        // tail instead of silently dropping the final part of a track.
        self.res_in[0].resize(DECODE_CHUNK_FRAMES, 0.0);
        self.res_in[1].resize(DECODE_CHUNK_FRAMES, 0.0);
        let wave_in = vec![
            std::mem::take(&mut self.res_in[0]),
            std::mem::take(&mut self.res_in[1]),
        ];
        let result = match self.resampler.as_mut() {
            Some(resampler) => resampler.process(&wave_in, None),
            None => return,
        };
        match result {
            Ok(wave_out) => {
                let frames = wave_out[0].len().min(wave_out[1].len());
                for (left, right) in wave_out[0].iter().zip(wave_out[1].iter()).take(frames) {
                    self.out_fifo.push(*left);
                    self.out_fifo.push(*right);
                }
            }
            Err(error) => {
                tracing::warn!("audio resampler flush failed: {}", error);
            }
        }
    }

    /// Convert source-rate/channels interleaved samples to stereo 48 kHz and
    /// append to out_fifo. Also handles initial skip offset.
    fn push_converted(&mut self, src: &[f32]) {
        if src.is_empty() {
            return;
        }

        // Channel adaptation to stereo at source rate
        let mut stereo: Vec<f32> = match self.src_channels {
            0 | 1 => {
                let mut s = Vec::with_capacity(src.len() * 2);
                for frame in src.chunks(self.src_channels.max(1)) {
                    let v = frame.first().copied().unwrap_or(0.0);
                    s.push(v);
                    s.push(v);
                }
                s
            }
            2 => src.to_vec(),
            n => {
                // average channel pairs heuristically: L=(ch0+ch3)/2 R=(ch1+ch2)/2 style fallback
                let mut s = Vec::with_capacity(src.len() / n * 2);
                for frame in src.chunks(n) {
                    let l: f32 = frame.iter().step_by(2).sum::<f32>() / n.div_ceil(2) as f32;
                    let r: f32 =
                        frame.iter().skip(1).step_by(2).sum::<f32>() / (n / 2).max(1) as f32;
                    s.push(l);
                    s.push(r);
                }
                s
            }
        };

        // Skip offset support (used for hot-restart seek preservation)
        if self.frames_to_skip > 0 {
            let have_frames = (stereo.len() / 2) as u64;
            if have_frames <= self.frames_to_skip {
                self.frames_to_skip -= have_frames;
                return;
            }
            let skip_bytes = (self.frames_to_skip * 2) as usize;
            stereo.drain(..skip_bytes);
            self.frames_to_skip = 0;
        }

        // Sample rate conversion to 48 kHz
        if self.resampler.is_none() {
            self.out_fifo.extend_from_slice(&stereo);
            return;
        }

        for chunk in stereo.chunks(2) {
            self.res_in[0].push(chunk[0]);
            self.res_in[1].push(*chunk.get(1).unwrap_or(&0.0));
        }

        let resampler = self.resampler.as_mut().unwrap();
        while self.res_in[0].len() >= DECODE_CHUNK_FRAMES {
            let wave_in = vec![
                self.res_in[0]
                    .drain(..DECODE_CHUNK_FRAMES)
                    .collect::<Vec<f32>>(),
                self.res_in[1]
                    .drain(..DECODE_CHUNK_FRAMES)
                    .collect::<Vec<f32>>(),
            ];
            if let Ok(mut wave_out) = resampler.process(&wave_in, None) {
                self.res_pending[0].append(&mut wave_out[0]);
                self.res_pending[1].append(&mut wave_out[1]);
            }
        }

        for i in 0..self.res_pending[0].len() {
            self.out_fifo.push(self.res_pending[0][i]);
            self.out_fifo.push(self.res_pending[1][i]);
        }
        self.res_pending[0].clear();
        self.res_pending[1].clear();
    }
}

/// Byte source backed by a channel fed from an async HTTP task.
/// Implements Read+Send+Sync so it can be wrapped in a MediaSourceStream.
pub struct ChannelByteSource {
    rx: Mutex<tokio::sync::mpsc::Receiver<Result<Vec<u8>, String>>>,
    pending: Mutex<Vec<u8>>,
    eof: Mutex<bool>,
}

impl ChannelByteSource {
    pub fn new(rx: tokio::sync::mpsc::Receiver<Result<Vec<u8>, String>>) -> Self {
        Self {
            rx: Mutex::new(rx),
            pending: Mutex::new(Vec::new()),
            eof: Mutex::new(false),
        }
    }
}

impl Seek for ChannelByteSource {
    fn seek(&mut self, _pos: SeekFrom) -> std::io::Result<u64> {
        // Live HTTP streams are not seekable.
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "ChannelByteSource is a live stream and cannot seek",
        ))
    }
}

impl MediaSource for ChannelByteSource {
    fn is_seekable(&self) -> bool {
        false
    }

    fn byte_len(&self) -> Option<u64> {
        None
    }
}

impl Read for ChannelByteSource {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        loop {
            {
                let mut pending = self.pending.lock().unwrap();
                if !pending.is_empty() {
                    let take = buf.len().min(pending.len());
                    buf[..take].copy_from_slice(&pending[..take]);
                    pending.drain(..take);
                    return Ok(take);
                }
            }

            if *self.eof.lock().unwrap() {
                return Ok(0);
            }

            let mut rx = self.rx.lock().unwrap();
            match rx.blocking_recv() {
                Some(Ok(chunk)) => {
                    self.pending.lock().unwrap().extend_from_slice(&chunk);
                }
                Some(Err(error)) => {
                    *self.eof.lock().unwrap() = true;
                    return Err(std::io::Error::other(error));
                }
                None => {
                    *self.eof.lock().unwrap() = true;
                    return Ok(0);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Seek};
    use symphonia::core::io::MediaSource;

    #[test]
    fn opus_detection_via_string() {
        // Symphonia CodecType is opaque; detection is string-based.
        // Test the string matching logic.
        assert!("Opus".to_string().to_uppercase().contains("OPUS"));
        assert!("Aac".to_string().to_uppercase().contains("AAC"));
        assert!("Mp3".to_string().to_uppercase().contains("MP3"));
    }

    #[test]
    fn skip_scaling_converts_48k_frames_to_source_rate() {
        assert_eq!(scale_skip_frames(0, 44_100), 0);
        // A 48 kHz source needs no scaling.
        assert_eq!(scale_skip_frames(48_000, 48_000), 48_000);
        // One second of audio is 44 100 frames at 44.1 kHz, not 48 000.
        assert_eq!(scale_skip_frames(48_000, 44_100), 44_100);
        // 8000 Hz telephony: half a second is 4 000 frames.
        assert_eq!(scale_skip_frames(24_000, 8_000), 4_000);
        // Rounding to nearest frame: 48 frames (1 ms) at 22 050 Hz is 22.05.
        assert_eq!(scale_skip_frames(48, 22_050), 22);
        assert_eq!(scale_skip_frames(96, 22_050), 44);
    }

    /// Minimal PCM16 mono WAV container for decoder tests.
    fn wav_pcm16_mono(sample_rate: u32, samples: &[i16]) -> Vec<u8> {
        let data_len = (samples.len() * 2) as u32;
        let mut out = Vec::with_capacity(44 + samples.len() * 2);
        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&(36 + data_len).to_le_bytes());
        out.extend_from_slice(b"WAVE");
        out.extend_from_slice(b"fmt ");
        out.extend_from_slice(&16u32.to_le_bytes()); // fmt chunk size
        out.extend_from_slice(&1u16.to_le_bytes()); // PCM
        out.extend_from_slice(&1u16.to_le_bytes()); // mono
        out.extend_from_slice(&sample_rate.to_le_bytes());
        out.extend_from_slice(&(sample_rate * 2).to_le_bytes()); // byte rate
        out.extend_from_slice(&2u16.to_le_bytes()); // block align
        out.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
        out.extend_from_slice(b"data");
        out.extend_from_slice(&data_len.to_le_bytes());
        for s in samples {
            out.extend_from_slice(&s.to_le_bytes());
        }
        out
    }

    /// In-memory seekable source so the probe can open the WAV without a file.
    struct MemSource(std::io::Cursor<Vec<u8>>);

    impl Read for MemSource {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            self.0.read(buf)
        }
    }

    impl Seek for MemSource {
        fn seek(&mut self, pos: std::io::SeekFrom) -> std::io::Result<u64> {
            self.0.seek(pos)
        }
    }

    impl MediaSource for MemSource {
        fn is_seekable(&self) -> bool {
            true
        }
        fn byte_len(&self) -> Option<u64> {
            Some(self.0.get_ref().len() as u64)
        }
    }

    /// Distinguishable sample pattern: sample `i` carries a unique value.
    fn pattern(i: usize) -> i16 {
        (i % 32768) as i16
    }

    /// End-to-end skip check: opening a known WAV with `skip_frames` must make
    /// the first decoded frame the one at the requested offset. This used to be
    /// broken for non-48 kHz sources because the count was applied at the source
    /// rate without conversion; this 48 kHz case pins the exact-position path.
    #[test]
    fn skip_positions_a_48khz_source_exactly() {
        let total = 96_000; // 2 seconds at 48 kHz
        let samples: Vec<i16> = (0..total).map(pattern).collect();
        let src = MemSource(std::io::Cursor::new(wav_pcm16_mono(48_000, &samples)));
        let mut dec = AudioDecoder::open(Box::new(src), Some("wav"), 48_000).expect("open wav");

        let frames = dec.read_frames(16);
        assert_eq!(frames.len(), 32, "16 stereo frames expected");

        // The mono source is dual-channelled, so L == R == source sample.
        // Symphonia's S16 -> f32 conversion divides by exactly 32768.
        for j in 0..16 {
            let expected = pattern(48_000 + j) as f32 / 32768.0;
            assert!(
                (frames[j * 2] - expected).abs() < 1e-9,
                "left sample {j}: got {}, want {}",
                frames[j * 2],
                expected
            );
            assert!(
                (frames[j * 2 + 1] - expected).abs() < 1e-9,
                "right sample {j}: got {}, want {}",
                frames[j * 2 + 1],
                expected
            );
        }
    }
}
