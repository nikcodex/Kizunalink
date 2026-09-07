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

/// Fill `buf` from the shared pipeline core.
///
/// Shared between the synchronous [`std::io::Read`] impl and the blocking-pool
/// wrapper so both paths behave identically.
fn read_pcm_core(core: &Arc<Mutex<PipelineCore>>, buf: &mut [u8]) -> std::io::Result<usize> {
    const BYTES_PER_FRAME: usize = std::mem::size_of::<f32>() * 2;

    loop {
        {
            let mut core = core.lock().unwrap();
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

        // Need more data; produce it. This may block on the network-backed
        // decoder, so it must never run on an async worker thread.
        {
            let mut core = core.lock().unwrap();
            if !core.eof {
                core.fill();
                continue;
            }
        }
    }
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

    /// Fill `buf` with filtered f32 PCM.
    ///
    /// This can block on the network-backed decoder (`ChannelByteSource` parks
    /// the thread until the HTTP feeder task delivers more bytes), so it must
    /// only ever run on a thread that is allowed to block. Async callers must go
    /// through [`FilteredAudioReader::read_pcm_blocking`] instead.
    fn read_pcm(&self, buf: &mut [u8]) -> std::io::Result<usize> {
        read_pcm_core(&self.core, buf)
    }

    /// Read up to `len` bytes of filtered PCM, running the (potentially
    /// blocking) decoder on the runtime's blocking thread pool.
    ///
    /// Returns fewer bytes only at end of stream, and an empty `Vec` once the
    /// source is exhausted.
    pub async fn read_pcm_blocking(&self, len: usize) -> std::io::Result<Vec<u8>> {
        let core = self.core.clone();
        let join = tokio::task::spawn_blocking(move || {
            let mut buf = vec![0u8; len];
            let mut total_read = 0;
            while total_read < buf.len() {
                match read_pcm_core(&core, &mut buf[total_read..]) {
                    Ok(0) => break,
                    Ok(n) => total_read += n,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => continue,
                    Err(e) => return Err(e),
                }
            }
            buf.truncate(total_read);
            Ok(buf)
        });

        match join.await {
            Ok(result) => result,
            Err(e) => {
                let msg = format!("audio decode task failed: {}", e);
                Err(std::io::Error::other(msg))
            }
        }
    }

    #[cfg(test)]
    pub fn total_frames_out(&self) -> u64 {
        self.core.lock().unwrap().total_frames_out
    }
}

impl Read for FilteredAudioReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.read_pcm(buf)
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

/// Build the playback source for `stream_url`.
///
/// Every playback fetch goes through this function, which makes it the chokepoint
/// for the stream policy. `/v4/loadtracks` validates identifiers, but the stream
/// URL of a playing track can also come straight from an *encoded track* — and
/// Lavalink's encoding is unsigned base64 + big-endian binary, so any client that
/// knows the password can forge one. The URL is therefore re-validated here
/// (scheme, private/loopback/link-local/metadata ranges, blocked hosts,
/// `sources.local`), and remote hosts are resolved and **pinned** to a verified
/// public address so a hostname cannot be re-resolved to an internal address
/// between validation and connect.
pub async fn create_kizuna_source(
    stream_url: String,
    extension_hint: Option<String>,
    shared_chain: SharedChain,
    skip_frames: u64,
) -> Result<KizunaFilteredSource, String> {
    let local_sources = crate::config::local_sources_enabled();
    let target = crate::security::resolve_stream_target(&stream_url, local_sources).await?;

    let (tx, rx) = tokio::sync::mpsc::channel::<Vec<u8>>(128);
    match target {
        crate::security::StreamTarget::LocalFile(path) => {
            tokio::spawn(async move {
                use tokio::io::AsyncReadExt;
                match tokio::fs::File::open(&path).await {
                    Ok(mut file) => {
                        let mut buf = [0u8; 8192];
                        while let Ok(n) = file.read(&mut buf).await {
                            if n == 0 {
                                break;
                            }
                            if tx.send(buf[..n].to_vec()).await.is_err() {
                                break;
                            }
                        }
                    }
                    Err(e) => {
                        let safe_path = crate::security::sanitize_for_log(&path);
                        tracing::warn!("Cannot open local audio file '{}': {}", safe_path, e);
                    }
                }
            });
        }
        crate::security::StreamTarget::Remote { url, host, pin } => {
            // `resolve` pins the validated IP for this host; the port in the
            // address is ignored by reqwest (the URL's port is used) and TLS SNI
            // plus certificate validation still run against the hostname.
            let pinned = std::net::SocketAddr::new(pin.ip(), pin.port());
            let client = crate::config::global_proxy()
                .apply_to_builder(reqwest::Client::builder())
                .timeout(crate::config::global_request_timeout())
                .resolve(&host, pinned)
                .build()
                .map_err(|e| format!("Failed to build HTTP client: {}", e))?;

            tokio::spawn(async move {
                // Stream URLs carry signed query parameters for some sources, so
                // only the host is logged.
                let resp = match client.get(&url).send().await {
                    Ok(r) => r,
                    Err(e) => {
                        tracing::warn!("Stream request to '{}' failed: {}", host, e);
                        return;
                    }
                };
                if !resp.status().is_success() {
                    tracing::warn!("Stream '{}' returned HTTP {}", host, resp.status());
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
        }
    }

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
        // The decode + filter chain blocks on the network-backed byte source
        // (`ChannelByteSource` uses `blocking_recv`), so it has to run on the
        // runtime's blocking pool. Calling it inline would panic the async
        // worker thread, which aborts the whole process in release builds.
        let read = self.reader.read_pcm_blocking(7680).await;
        let buf = match read {
            Ok(buf) => buf,
            Err(e) => return Err(kizuna_voice::error::Error::Connection(e.to_string())),
        };

        if buf.is_empty() {
            return Ok(None);
        }

        let usable = buf.len() - (buf.len() % 4);
        let num_samples = usable / 4;
        let mut samples = Vec::with_capacity(num_samples);
        for chunk in buf[..usable].as_chunks::<4>().0 {
            let f = f32::from_le_bytes(*chunk);
            let s = (f * i16::MAX as f32).clamp(i16::MIN as f32, i16::MAX as f32) as i16;
            samples.push(s);
        }

        Ok(Some(AudioFrame::Pcm(samples)))
    }
}

#[cfg(test)]
mod tests {
    use super::{create_kizuna_source, new_shared_chain};

    /// A playing track's stream URL can come from a forged encoded track, so the
    /// playback path has to enforce `sources.local` itself — the shipped
    /// configuration disables it.
    #[tokio::test]
    async fn test_local_stream_rejected_when_local_source_disabled() {
        crate::config::init_local_sources(false);

        let chain = new_shared_chain(48_000.0);
        let path = "/etc/hostname".to_string();
        let result = create_kizuna_source(path, None, chain, 0).await;
        // KizunaFilteredSource has no Debug impl, so match instead of unwrap_err.
        let err = match result {
            Ok(_) => panic!("playback of a blocked address must be refused"),
            Err(err) => err,
        };
        assert!(err.contains("sources.local"), "unexpected error: {}", err);
    }

    /// Loopback and cloud-metadata targets must never be fetched for playback,
    /// even when the URL arrives through a track rather than an identifier.
    #[tokio::test]
    async fn test_private_stream_url_rejected() {
        let chain = new_shared_chain(48_000.0);

        let metadata = "http://169.254.169.254/latest/meta-data/".to_string();
        let first = chain.clone();
        let result = create_kizuna_source(metadata, None, first, 0).await;
        assert!(result.is_err());

        let localhost = "http://localhost:8080/stream".to_string();
        let result = create_kizuna_source(localhost, None, chain, 0).await;
        assert!(result.is_err());
    }
}
