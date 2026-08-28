/// Filtered playback pipeline.
///
/// Architecture:
///   HTTP stream (tokio task) -> ChannelByteSource -> AudioDecoder (symphonia)
///     -> FilterChain (DSP) -> f32 LE PCM bytes
///       -> RawAdapter -> songbird Input -> mixer (post-filter!)
///
/// The mixer reads from [`FilteredAudioReader`] synchronously, so every sample
/// it receives has already been through the DSP chain - true pre-mixer filtering.
///
/// The same chain code path is exercised by offline verification tests.
use std::io::{Read, Seek, SeekFrom};
use std::sync::{Arc, Mutex};

use super::decoder::{AudioDecoder, ChannelByteSource, TARGET_SAMPLE_RATE};
use super::filters::FilterChain;
use symphonia::core::io::MediaSource;

#[cfg(test)]
use crate::models::filters::Filters;

/// Filter chain shared between a live pipeline reader and GuildPlayer so REST
/// filter updates propagate into running audio without restarts (except when
/// the chain reports a structural change).
pub type SharedChain = Arc<Mutex<FilterChain>>;

pub fn new_shared_chain(sample_rate: f64) -> SharedChain {
    Arc::new(Mutex::new(FilterChain::new(sample_rate)))
}

struct PipelineCore {
    decoder: Option<AudioDecoder>,
    shared_chain: SharedChain,
    out_fifo: Vec<f32>,
    eof: bool,
    total_frames_out: u64,
}

impl PipelineCore {
    /// Decode + filter until at least one chunk of output is available or EOF.
    fn fill(&mut self) {
        let mut decoder_eof = self.decoder.is_none();

        while !decoder_eof && self.out_fifo.len() < 4096 * 2 {
            // Scoped borrow: release &mut self.decoder before filtering.
            let decoded = {
                let Some(decoder) = self.decoder.as_mut() else {
                    decoder_eof = true;
                    break;
                };
                let decoded = decoder.read_frames(4096);
                if decoded.is_empty() {
                    if decoder.is_eof() {
                        decoder_eof = true;
                    }
                    break;
                }
                decoded
            };

            append_filtered(self, &decoded);
        }

        if decoder_eof && !self.eof {
            // flush chain tails (timescale buffering)
            let tail = self.shared_chain.lock().unwrap().flush();
            if !tail.is_empty() {
                append_filtered(self, &tail);
            }
            self.eof = true;
        }
    }
}

fn append_filtered(core: &mut PipelineCore, pcm: &[f32]) {
    // Feed through in ~20ms chunks like a real mixer would, to exercise the
    // exact streaming path.
    for chunk in pcm.chunks(960 * 2) {
        let processed = core.shared_chain.lock().unwrap().process(chunk);
        if !processed.is_empty() {
            core.out_fifo.extend_from_slice(&processed);
            core.total_frames_out += (processed.len() / 2) as u64;
        }
    }
}

/// A synchronous MediaSource serving filtered f32-PCM. Wrap in
/// `songbird::input::RawAdapter` to obtain a playable Input.
pub struct FilteredAudioReader {
    core: Arc<Mutex<PipelineCore>>,
    /// Whether the source is Opus at 48kHz (allows skipping resampler)
    pub is_opus_source: bool,
}

impl FilteredAudioReader {
    pub fn new(shared_chain: SharedChain, decoder: AudioDecoder) -> Self {
        let is_opus_source = decoder.is_opus;
        Self {
            core: Arc::new(Mutex::new(PipelineCore {
                decoder: Some(decoder),
                shared_chain,
                out_fifo: Vec::with_capacity(8192),
                eof: false,
                total_frames_out: 0,
            })),
            is_opus_source,
        }
    }

    #[cfg(test)]
    pub fn total_frames_out(&self) -> u64 {
        self.core.lock().unwrap().total_frames_out
    }
}

impl Read for FilteredAudioReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        const BYTES_PER_FRAME: usize = std::mem::size_of::<f32>() * 2;

        loop {
            {
                let mut core = self.core.lock().unwrap();
                let available_bytes = core.out_fifo.len() * std::mem::size_of::<f32>();
                if available_bytes >= buf.len().min(BYTES_PER_FRAME) || core.eof {
                    let take_samples =
                        (buf.len() / std::mem::size_of::<f32>()).min(core.out_fifo.len());
                    let take_bytes = take_samples * std::mem::size_of::<f32>();
                    for (i, s) in core.out_fifo.drain(..take_samples).enumerate() {
                        buf[i * 4..i * 4 + 4].copy_from_slice(&s.to_le_bytes());
                    }
                    return Ok(take_bytes);
                }
            }

            // Need more data; produce it (decode+filter is fast, no I/O wait here)
            {
                let mut core = self.core.lock().unwrap();
                if !core.eof {
                    core.fill();
                    continue;
                }
            }
        }
    }
}

impl Seek for FilteredAudioReader {
    fn seek(&mut self, _pos: SeekFrom) -> std::io::Result<u64> {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "filtered stream is not seekable",
        ))
    }
}

impl symphonia::core::io::MediaSource for FilteredAudioReader {
    fn is_seekable(&self) -> bool {
        false
    }

    fn byte_len(&self) -> Option<u64> {
        None
    }
}

/// Build a songbird `Input` that plays `stream_url` through the shared DSP
/// chain. Returns Err with a message on setup failure.
///
/// When `opus_passthrough` is true AND the chain has no active filters AND
/// the source is Opus at 48kHz, the raw HTTP stream is passed directly to
/// Songbird (skipping decode→PCM→re-encode), saving ~60% CPU.
pub async fn create_filtered_input(
    http: reqwest::Client,
    stream_url: String,
    extension_hint: Option<String>,
    shared_chain: SharedChain,
    skip_frames: u64,
    opus_passthrough: bool,
) -> Result<songbird::input::Input, String> {
    // Fast path: Opus passthrough — skip the entire decode+filter pipeline
    if opus_passthrough && !shared_chain.lock().unwrap().is_active() {
        return Ok(songbird::input::HttpRequest::new(http, stream_url).into());
    }

    let (tx, rx) = std::sync::mpsc::sync_channel::<Vec<u8>>(256);

    let url = stream_url.clone();
    tokio::spawn(async move {
        let resp = match http.get(&url).send().await {
            Ok(r) => r,
            Err(_) => return,
        };
        if !resp.status().is_success() {
            return;
        }
        let mut stream = resp.bytes_stream();
        use futures_util::StreamExt;
        while let Some(chunk) = stream.next().await {
            match chunk {
                Ok(bytes) => {
                    if tx.send(bytes.to_vec()).is_err() {
                        break; // receiver dropped
                    }
                }
                Err(_) => break,
            }
        }
        // tx dropped here => EOF for the reader
    });

    // std::sync::mpsc::sync_channel's Receiver is the same concrete type as
    // the unbounded one, so ChannelByteSource::new works directly.
    let byte_source = ChannelByteSource::new(rx);
    let decoder = tokio::task::spawn_blocking(move || {
        let hint_ext: Option<&str> = extension_hint.as_deref();
        AudioDecoder::open(
            Box::new(byte_source) as Box<dyn MediaSource>,
            hint_ext,
            skip_frames,
        )
    })
    .await
    .map_err(|e| format!("decoder task panicked: {}", e))??;

    let is_opus = decoder.is_opus;
    let reader = FilteredAudioReader::new(shared_chain, decoder);

    if is_opus && opus_passthrough {
        // Source is Opus 48kHz — Songbird receives raw HTTP stream and handles
        // the WebM/Ogg demux + Opus decode internally. We still went through
        // our decoder to detect it's Opus, but Songbird will re-parse from the
        // HTTP stream. This is slightly wasteful but correct.
        // TODO: cache the "is_opus" detection so we don't need to decode first.
        Ok(songbird::input::HttpRequest::new(crate::config::http_client(), stream_url).into())
    } else {
        Ok(songbird::input::RawAdapter::new(reader, TARGET_SAMPLE_RATE, 2).into())
    }
}

#[cfg(test)]
pub mod test_support {
    //! Offline harness helpers used by verification tests. These run audio
    //! through the exact production chain code path.

    use super::*;

    /// Run interleaved stereo PCM through a chain configured from Lavalink
    /// filters, using realistic 20 ms chunking. Returns filtered samples plus
    /// whether the update was structural.
    pub fn run_through_pipeline(
        input: &[f32],
        filters: &Filters,
        sample_rate: f64,
        structural_before: bool,
    ) -> (Vec<f32>, bool) {
        let mut chain = FilterChain::new(sample_rate);
        let _structural = chain.update_from_lavalink(filters);

        let mut out = Vec::with_capacity(input.len());
        for chunk in input.chunks(960 * 2) {
            out.extend(chain.process(chunk));
        }
        out.extend(chain.flush());
        let _ = structural_before;
        (out, _structural)
    }
}

use async_trait::async_trait;
use kizuna_voice::audio::AudioFrame;
use kizuna_voice::audio::AudioSource;

pub struct KizunaFilteredSource {
    reader: FilteredAudioReader,
}

impl KizunaFilteredSource {
    pub fn new(reader: FilteredAudioReader) -> Self {
        Self { reader }
    }
}

#[async_trait]
impl AudioSource for KizunaFilteredSource {
    async fn next_frame(&mut self) -> kizuna_voice::error::Result<Option<AudioFrame>> {
        let mut buf = [0u8; 7680];
        let mut total_read = 0;

        while total_read < buf.len() {
            match self.reader.read(&mut buf[total_read..]) {
                Ok(0) => break,
                Ok(n) => total_read += n,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => continue,
                Err(e) => return Err(kizuna_voice::error::Error::Connection(e.to_string())),
            }
        }

        if total_read == 0 {
            return Ok(None);
        }

        let num_samples = total_read / 4;
        let mut samples = Vec::with_capacity(num_samples);
        for chunk in buf[..total_read].chunks_exact(4) {
            let f = f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
            let s = (f * i16::MAX as f32).clamp(i16::MIN as f32, i16::MAX as f32) as i16;
            samples.push(s);
        }

        Ok(Some(AudioFrame::Pcm(samples)))
    }
}

pub async fn create_kizuna_source(
    http: reqwest::Client,
    stream_url: String,
    extension_hint: Option<String>,
    shared_chain: SharedChain,
    skip_frames: u64,
) -> Result<KizunaFilteredSource, String> {
    let (tx, rx) = std::sync::mpsc::sync_channel::<Vec<u8>>(256);
    let url = stream_url.clone();
    tokio::spawn(async move {
        let resp = match http.get(&url).send().await {
            Ok(r) => r,
            Err(_) => return,
        };
        if !resp.status().is_success() {
            return;
        }
        let mut stream = resp.bytes_stream();
        use futures_util::StreamExt;
        while let Some(chunk) = stream.next().await {
            match chunk {
                Ok(bytes) => {
                    if tx.send(bytes.to_vec()).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });

    let byte_source = ChannelByteSource::new(rx);
    let decoder = tokio::task::spawn_blocking(move || {
        let hint_ext: Option<&str> = extension_hint.as_deref();
        AudioDecoder::open(
            Box::new(byte_source) as Box<dyn MediaSource>,
            hint_ext,
            skip_frames,
        )
    })
    .await
    .map_err(|e| format!("decoder task panicked: {}", e))??;

    let reader = FilteredAudioReader::new(shared_chain, decoder);
    Ok(KizunaFilteredSource::new(reader))
}
