/// Filtered playback pipeline.
///
/// Architecture:
///   HTTP stream (tokio task) -> ChannelByteSource -> AudioDecoder (symphonia)
///     -> FilterChain (DSP) -> f32 LE PCM bytes
///
/// The mixer reads from [`FilteredAudioReader`] synchronously, so every sample
/// it receives has already been through the DSP chain - true pre-mixer filtering.
///
/// The same chain code path is exercised by offline verification tests.
use std::io::{Read, Seek, SeekFrom};
use std::sync::{Arc, Mutex};

use super::decoder::{AudioDecoder, ChannelByteSource};
use super::filters::FilterChain;
use symphonia::core::io::MediaSource;


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

pub async fn create_kizuna_source(
    http: reqwest::Client,
    stream_url: String,
    extension_hint: Option<String>,
    shared_chain: SharedChain,
    skip_frames: u64,
) -> Result<KizunaFilteredSource, String> {
    let (tx, rx) = tokio::sync::mpsc::channel::<Vec<u8>>(128);
    let url = stream_url.clone();
    tokio::spawn(async move {
        if let Some(file_path) = url.strip_prefix("file://").or_else(|| if url.starts_with('/') { Some(url.as_str()) } else { None }) {
            // Local file streaming
            use tokio::io::AsyncReadExt;
            if let Ok(mut file) = tokio::fs::File::open(file_path).await {
                let mut buf = [0u8; 8192];
                while let Ok(n) = file.read(&mut buf).await {
                    if n == 0 { break; }
                    if tx.send(buf[..n].to_vec()).await.is_err() { break; }
                }
            }
            return;
        }

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
                    if tx.send(bytes.to_vec()).await.is_err() {
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

use async_trait::async_trait;
use kizuna_voice::audio::{AudioFrame, AudioSource};

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
        for chunk in buf[..total_read].as_chunks::<4>().0 {
            let f = f32::from_le_bytes(*chunk);
            let s = (f * i16::MAX as f32).clamp(i16::MIN as f32, i16::MAX as f32) as i16;
            samples.push(s);
        }

        Ok(Some(AudioFrame::Pcm(samples)))
    }
}
