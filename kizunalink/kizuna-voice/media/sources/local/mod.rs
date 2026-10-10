// Copyright (c) 2026 nikcodex (KizunaLink)
// Licensed under the MIT License

use std::{
    io::{Read, Seek, SeekFrom},
    path::Path,
    sync::Arc,
};

use async_trait::async_trait;
use symphonia::core::{
    codecs::CODEC_TYPE_NULL,
    formats::FormatOptions,
    io::MediaSourceStream,
    meta::{MetadataOptions, StandardTagKey},
    probe::Hint,
};
use tracing::{debug, error, warn};

use crate::{
    common::Severity,
    engine::{
        AudioFrame,
        processor::{AudioProcessor, DecoderCommand},
    },
    lavalink::protocol::tracks::{LoadError, LoadResult, Track, TrackInfo},
    media::sources::{
        SourcePlugin,
        plugin::{DecoderOutput, PlayableTrack},
    },
};

pub struct LocalSource {
    media_dir: Option<std::path::PathBuf>,
}

impl Default for LocalSource {
    fn default() -> Self {
        Self::new(None)
    }
}

impl LocalSource {
    pub fn new(media_dir: Option<String>) -> Self {
        Self {
            media_dir: media_dir.and_then(|path| std::fs::canonicalize(path).ok()),
        }
    }

    fn allowed_path(&self, path: &str) -> Option<std::path::PathBuf> {
        let root = self.media_dir.as_ref()?;
        let candidate = std::fs::canonicalize(path).ok()?;
        candidate.starts_with(root).then_some(candidate)
    }

    fn probe_file(path: &str) -> Result<TrackInfo, Box<dyn std::error::Error + Send + Sync>> {
        let file = std::fs::File::open(path)?;
        let path_obj = Path::new(path);
        let ext = path_obj
            .extension()
            .and_then(|e| e.to_str())
            .map(|s| s.to_lowercase());

        let mut hint = Hint::new();
        if let Some(ref e) = ext {
            hint.with_extension(e);
        }

        let mss = MediaSourceStream::new(Box::new(file), Default::default());
        let probed = symphonia::default::get_probe().format(
            &hint,
            mss,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )?;

        let mut format = probed.format;
        let track = format
            .tracks()
            .iter()
            .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
            .ok_or("no audio track found")?;

        // Duration calculation
        let duration = track
            .codec_params
            .n_frames
            .and_then(|n| {
                track
                    .codec_params
                    .sample_rate
                    .map(|r| (n as f64 / r as f64 * 1000.0) as u64)
            })
            .unwrap_or(0);

        // Metadata extraction
        let mut title = String::new();
        let mut author = String::new();

        if let Some(meta) = format.metadata().current() {
            for tag in meta.tags() {
                match tag.std_key {
                    Some(StandardTagKey::TrackTitle) => title = tag.value.to_string(),
                    Some(StandardTagKey::Artist) | Some(StandardTagKey::AlbumArtist)
                        if author.is_empty() =>
                    {
                        author = tag.value.to_string();
                    }
                    _ => {}
                }
            }
        }

        // Fallback: use filename if metadata is missing
        if title.is_empty() {
            title = path_obj
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("Unknown")
                .to_owned();
        }
        if author.is_empty() {
            author = "Unknown Artist".to_owned();
        }

        Ok(TrackInfo {
            identifier: path.to_owned(),
            is_seekable: true,
            author,
            length: duration,
            is_stream: false,
            position: 0,
            title,
            uri: Some(format!("file://{path}")),
            source_name: "local".to_owned(),
            artwork_url: None,
            isrc: None,
        })
    }
}

#[async_trait]
impl SourcePlugin for LocalSource {
    fn name(&self) -> &str {
        "local"
    }

    fn can_handle(&self, identifier: &str) -> bool {
        let path = identifier.strip_prefix("file://").unwrap_or(identifier);
        self.allowed_path(path).is_some_and(|path| path.is_file())
    }

    async fn load(
        &self,
        identifier: &str,
        _routeplanner: Option<Arc<dyn crate::lavalink::routeplanner::RoutePlanner>>,
    ) -> LoadResult {
        let path = identifier
            .strip_prefix("file://")
            .unwrap_or(identifier)
            .to_owned();
        let Some(allowed_path) = self.allowed_path(&path) else {
            return LoadResult::Empty {};
        };
        let path = allowed_path.to_string_lossy().into_owned();
        debug!("Local source probing file: {path}");

        let path_clone = path.clone();
        let result =
            tokio::task::spawn_blocking(move || LocalSource::probe_file(&path_clone)).await;

        match result {
            Ok(Ok(info)) => LoadResult::Track(Track::new(info)),
            Ok(Err(e)) => {
                warn!("Local source: failed to probe '{path}': {e}");
                LoadResult::Error(LoadError {
                    message: Some(format!("Failed to load local file: {e}")),
                    severity: Severity::Suspicious,
                    cause: e.to_string(),
                    cause_stack_trace: None,
                })
            }
            Err(e) => {
                error!("Local source: task join error: {e}");
                LoadResult::Error(LoadError {
                    message: Some("Internal error reading local file".to_owned()),
                    severity: Severity::Fault,
                    cause: e.to_string(),
                    cause_stack_trace: None,
                })
            }
        }
    }

    async fn get_track(
        &self,
        identifier: &str,
        _routeplanner: Option<Arc<dyn crate::lavalink::routeplanner::RoutePlanner>>,
    ) -> Option<Box<dyn PlayableTrack>> {
        let path = identifier
            .strip_prefix("file://")
            .unwrap_or(identifier)
            .to_owned();
        self.allowed_path(&path)
            .filter(|path| path.is_file())
            .map(|path| {
                Box::new(LocalTrack {
                    path: path.to_string_lossy().into_owned(),
                }) as Box<dyn PlayableTrack>
            })
    }
}

pub struct LocalTrack {
    pub path: String,
}

struct LocalFileSource {
    file: std::fs::File,
    len: u64,
}

impl LocalFileSource {
    fn open(path: &str) -> std::io::Result<Self> {
        let file = std::fs::File::open(path)?;
        let len = file.metadata()?.len();
        Ok(Self { file, len })
    }
}

impl Read for LocalFileSource {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.file.read(buf)
    }
}

impl Seek for LocalFileSource {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        self.file.seek(pos)
    }
}

impl symphonia::core::io::MediaSource for LocalFileSource {
    fn is_seekable(&self) -> bool {
        true
    }
    fn byte_len(&self) -> Option<u64> {
        Some(self.len)
    }
}

impl PlayableTrack for LocalTrack {
    fn start_decoding(&self, config: crate::config::player::PlayerConfig) -> DecoderOutput {
        let (tx, rx) = flume::bounded::<AudioFrame>((config.buffer_duration_ms / 20) as usize);
        let (cmd_tx, cmd_rx) = flume::unbounded::<DecoderCommand>();
        let (err_tx, err_rx) = flume::bounded::<String>(1);

        let path = self.path.clone();

        let handle = tokio::runtime::Handle::current();
        tokio::task::spawn_blocking(move || {
            let _guard = handle.enter();
            let source = match LocalFileSource::open(&path) {
                Ok(s) => Box::new(s) as Box<dyn symphonia::core::io::MediaSource>,
                Err(e) => {
                    error!("LocalTrack: failed to open '{path}': {e}");
                    let _ = err_tx.send(format!("Failed to open file: {e}"));
                    return;
                }
            };

            let kind = Path::new(&path)
                .extension()
                .and_then(|e| e.to_str())
                .map(crate::common::types::AudioFormat::from_ext);

            match AudioProcessor::new(source, kind, tx, cmd_rx, Some(err_tx.clone()), config) {
                Ok(mut processor) => {
                    let spawn_result = std::thread::Builder::new()
                        .name(format!("local-decoder-{}", path))
                        .spawn(move || {
                            if let Err(e) = processor.run_guarded() {
                                error!("LocalTrack audio processor error: {e}");
                            }
                        });
                    if let Err(e) = spawn_result {
                        error!("Failed to spawn local decoder thread: {e}");
                        let _ = err_tx.send(format!("failed to spawn decoder thread: {e}"));
                    }
                }
                Err(e) => {
                    error!("LocalTrack failed to initialize processor: {e}");
                    let _ = err_tx.send(format!("Failed to initialize processor: {e}"));
                }
            }
        });

        (rx, cmd_tx, err_rx)
    }
}

#[cfg(test)]
mod pipeline_test {
    use super::*;
    use crate::{
        config::player::PlayerConfig,
        discord::gateway::udp_link::{RtpState, UDPVoiceTransport},
        engine::{engine::Encoder, mix::mixer::Mixer, playback::handle::PlaybackState},
    };
    use std::{
        path::PathBuf,
        sync::atomic::{AtomicBool, AtomicU8, AtomicU32, AtomicU64},
        time::Duration,
    };

    struct Fixture(PathBuf);
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn local_wav_resolves_decodes_mixes_encodes_and_sends_rtp_to_loopback() {
        // Real generated PCM/WAV and application decoder/encoder/transport,
        // but NOT Discord/DAVE or a remote HTTP source.
        let dir = std::env::temp_dir().join(format!("kizuna-pipeline-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&dir).expect("create fixture directory");
        let _cleanup = Fixture(dir.clone());
        let path = dir.join("tone.wav");
        let samples = 48_000u32 / 5; // 200 ms stereo, 10 Opus frame periods
        let data_len = samples * 4;
        let mut wav = Vec::with_capacity(44 + data_len as usize);
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
            let tone =
                (8000.0 * (2.0 * std::f32::consts::PI * 440.0 * i as f32 / 48_000.0).sin()) as i16;
            wav.extend_from_slice(&tone.to_le_bytes());
            wav.extend_from_slice(&tone.to_le_bytes());
        }
        std::fs::write(&path, wav).expect("write PCM fixture");
        let url = format!("file://{}", path.display());
        let source = LocalSource::new(Some(dir.to_string_lossy().into_owned()));
        assert!(source.can_handle(&url));
        assert!(matches!(
            source.load(&url, None).await,
            LoadResult::Track(_)
        ));
        let track = source
            .get_track(&url, None)
            .await
            .expect("resolved playable track");
        let (rx, cmd_tx, err_rx) = track.start_decoding(PlayerConfig::default());
        let frames = tokio::time::timeout(
            Duration::from_secs(8),
            tokio::task::spawn_blocking(move || {
                let mut frames = Vec::new();
                while let Ok(frame) = rx.recv_timeout(Duration::from_secs(2)) {
                    frames.push(frame);
                }
                frames
            }),
        )
        .await
        .expect("decoder must finish")
        .expect("collector task");
        assert!(err_rx.try_recv().is_err(), "decoder reported an error");
        assert!(!frames.is_empty(), "decoder produced no PCM");
        drop(cmd_tx);

        let (tx, rx) = flume::unbounded();
        for frame in frames {
            tx.send(frame).expect("queue decoded PCM");
        }
        drop(tx);
        let mut mixer = Mixer::new(48_000);
        mixer.add_track(
            rx,
            Arc::new(AtomicU8::new(PlaybackState::Playing as u8)),
            Arc::new(AtomicU32::new(1f32.to_bits())),
            Arc::new(AtomicU64::new(0)),
            Arc::new(AtomicBool::new(false)),
            PlayerConfig::default(),
        );
        let mut encoder = Encoder::new().expect("application Opus encoder");
        let receiver = tokio::net::UdpSocket::bind("127.0.0.1:0")
            .await
            .expect("receiver");
        let sender = Arc::new(
            tokio::net::UdpSocket::bind("127.0.0.1:0")
                .await
                .expect("sender"),
        );
        let mut transport = UDPVoiceTransport::new(
            sender,
            receiver.local_addr().expect("receiver address"),
            42,
            [7; 32],
            "aead_xchacha20_poly1305_rtpsize",
            Some(RtpState {
                sequence: 7,
                timestamp: 960,
                nonce: 3,
            }),
        )
        .expect("voice transport");
        let mut count = 0u32;
        for _ in 0..15 {
            let mut pcm = [0i16; 1920];
            if !mixer.mix(&mut pcm) {
                break;
            }
            assert!(
                pcm.iter().any(|&sample| sample != 0),
                "mixed PCM was silent"
            );
            let mut opus = [0u8; 4000];
            let len = encoder.encode(&pcm, &mut opus).expect("encode PCM");
            assert!(len > 0);
            transport
                .transmit_opus(&opus[..len])
                .await
                .expect("send packet");
            let mut packet = [0u8; 4096];
            let (n, _) =
                tokio::time::timeout(Duration::from_secs(2), receiver.recv_from(&mut packet))
                    .await
                    .expect("RTP arrived")
                    .expect("receive RTP");
            assert!(n > 12 + 16 + 4, "RTP payload missing AEAD tag");
            assert_eq!(packet[0], 0x80);
            assert_eq!(u16::from_be_bytes([packet[2], packet[3]]), 7 + count as u16);
            assert_eq!(
                u32::from_be_bytes(packet[4..8].try_into().expect("timestamp")),
                960 + 960 * count
            );
            count += 1;
        }
        assert!(count >= 2, "expected multiple 20 ms packet periods");
        mixer.stop_all();
    }
}
