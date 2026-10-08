// Copyright (c) 2026 nikcodex (KizunaLink)
// Licensed under the MIT License

pub mod reader;
use std::{
    net::{IpAddr, ToSocketAddrs},
    sync::{Arc, OnceLock},
};

use async_trait::async_trait;
use regex::Regex;
use symphonia::core::{
    codecs::CODEC_TYPE_NULL,
    formats::FormatOptions,
    io::MediaSourceStream,
    meta::{MetadataOptions, StandardTagKey},
    probe::Hint,
};
use tracing::{debug, error, warn};

use crate::{
    common::types::AnyResult,
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

fn url_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| Regex::new(r"^(?:https?|icy)://").expect("valid regex"))
}

pub struct HttpSource;

impl Default for HttpSource {
    fn default() -> Self {
        Self::new()
    }
}

impl HttpSource {
    pub fn new() -> Self {
        Self
    }

    fn probe_metadata(url: String, local_addr: Option<std::net::IpAddr>) -> AnyResult<TrackInfo> {
        let source = reader::HttpReader::new(&url, local_addr, None)?;
        let mut hint = Hint::new();

        if let Some(content_type) = source.content_type() {
            hint.mime_type(content_type.as_str());
        }

        let mss = MediaSourceStream::new(Box::new(source), Default::default());

        if let Some(ext) = std::path::Path::new(&url)
            .extension()
            .and_then(|s| s.to_str())
        {
            hint.with_extension(ext);
        }

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

        let duration = if let Some(n_frames) = track.codec_params.n_frames {
            if let Some(rate) = track.codec_params.sample_rate {
                (n_frames as f64 / rate as f64 * 1000.0) as u64
            } else {
                0
            }
        } else {
            0
        };

        let mut title = String::new();
        let mut author = String::new();

        if let Some(metadata) = format.metadata().current() {
            if let Some(tag) = metadata
                .tags()
                .iter()
                .find(|t| t.std_key == Some(StandardTagKey::TrackTitle))
            {
                title = tag.value.to_string();
            }
            if let Some(tag) = metadata
                .tags()
                .iter()
                .find(|t| t.std_key == Some(StandardTagKey::Artist))
            {
                author = tag.value.to_string();
            }
        }

        if title.is_empty() {
            title = url
                .split('/')
                .next_back()
                .and_then(|s| s.split('?').next())
                .unwrap_or("Unknown Title")
                .to_owned();
        }

        if author.is_empty() {
            author = "Unknown Artist".to_owned();
        }

        Ok(TrackInfo {
            identifier: url.clone(),
            author,
            length: duration,
            is_seekable: true,
            is_stream: false,
            position: 0,
            title,
            uri: Some(url),
            source_name: "http".to_owned(),
            artwork_url: None,
            isrc: None,
        })
    }
}

/// Validate that a URL is a valid HTTP/HTTPS URL with host and port.
/// Does NOT perform DNS resolution (safe to call from synchronous contexts).
pub(crate) fn validate_http_url(raw: &str) -> AnyResult<(String, u16)> {
    let parsed = reqwest::Url::parse(raw).map_err(|e| format!("invalid HTTP URL: {e}"))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(format!("unsupported HTTP URL scheme: {}", parsed.scheme()).into());
    }
    let host = parsed.host_str().ok_or("HTTP URL has no host")?.to_owned();
    let port = parsed
        .port_or_known_default()
        .ok_or("HTTP URL has no port")?;
    Ok((host, port))
}

/// Check whether an IP address is in a blocked range (loopback, private,
/// link-local, multicast, unspecified, IPv4-mapped IPv6 loopback, ULA).
fn is_blocked_ip(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            ip.is_loopback()
                || ip.is_private()
                || ip.is_link_local()
                || ip.is_multicast()
                || ip.is_unspecified()
        }
        IpAddr::V6(ip) => {
            // Check for IPv4-mapped IPv6 addresses (e.g. ::ffff:127.0.0.1)
            if let Some(mapped_v4) = ip.to_ipv4_mapped() {
                return is_blocked_ip(&IpAddr::V4(mapped_v4));
            }
            let segments = ip.segments();
            let first = segments[0];
            ip.is_loopback()
                || ip.is_multicast()
                || ip.is_unspecified()
                || (first & 0xfe00) == 0xfc00 // unique-local fc00::/7
                || (first & 0xffc0) == 0xfe80 // link-local fe80::/10
        }
    }
}

/// Reject server-side requests to loopback, private, link-local, multicast, and
/// unspecified addresses before opening the user-supplied URL.
/// Returns (host, port, addresses) for IP pinning in the client.
pub(crate) fn validate_public_url(
    raw: &str,
) -> AnyResult<(String, u16, Vec<std::net::SocketAddr>)> {
    let (host, port) = validate_http_url(raw)?;
    let addresses: Vec<_> = (host.as_str(), port)
        .to_socket_addrs()
        .map_err(|e| format!("could not resolve HTTP host {host}: {e}"))?
        .collect();
    if addresses.is_empty() {
        return Err(format!("HTTP host {host} resolved to no addresses").into());
    }
    if addresses.iter().any(|address| {
        let ip = address.ip();
        // Check for IPv4-mapped IPv6 addresses (e.g. ::ffff:127.0.0.1)
        if let IpAddr::V6(ipv6) = ip
            && let Some(mapped_v4) = ipv6.to_ipv4_mapped()
        {
            return is_blocked_ip(&IpAddr::V4(mapped_v4));
        }
        is_blocked_ip(&ip)
    }) {
        return Err(format!("HTTP host {host} resolves to a private or local address").into());
    }
    Ok((host, port, addresses))
}

#[async_trait]
impl SourcePlugin for HttpSource {
    fn name(&self) -> &str {
        "http"
    }

    fn can_handle(&self, identifier: &str) -> bool {
        url_regex().is_match(identifier) && validate_http_url(identifier).is_ok()
    }

    async fn load(
        &self,
        identifier: &str,
        routeplanner: Option<Arc<dyn crate::lavalink::routeplanner::RoutePlanner>>,
    ) -> LoadResult {
        debug!("Probing HTTP source: {identifier}");

        let identifier = identifier.to_owned();
        let local_addr = routeplanner.as_ref().and_then(|rp| rp.get_address());

        let identifier_clone = identifier.clone();
        let probe_result = tokio::task::spawn_blocking(move || {
            HttpSource::probe_metadata(identifier_clone, local_addr)
        })
        .await;

        match probe_result {
            Ok(Ok(info)) => LoadResult::Track(Track::new(info)),
            Ok(Err(e)) => {
                warn!("Probing failed for {identifier}: {e}");
                // This mimics Lavaplayer's behavior where unknown formats return null.
                LoadResult::Empty {}
            }
            Err(e) => {
                error!("Task join error: {e}");
                LoadResult::Error(LoadError {
                    message: Some("Internal error during probing".to_owned()),
                    severity: crate::common::Severity::Suspicious,
                    cause: e.to_string(),
                    cause_stack_trace: None,
                })
            }
        }
    }

    async fn get_track(
        &self,
        identifier: &str,
        routeplanner: Option<Arc<dyn crate::lavalink::routeplanner::RoutePlanner>>,
    ) -> Option<Box<dyn PlayableTrack>> {
        let clean = identifier
            .trim()
            .trim_start_matches('<')
            .trim_end_matches('>');

        if self.can_handle(clean) {
            Some(Box::new(HttpTrack {
                url: clean.to_owned(),
                local_addr: routeplanner.and_then(|rp| rp.get_address()),
                proxy: None,
            }))
        } else {
            None
        }
    }
}

pub struct HttpTrack {
    pub url: String,
    pub local_addr: Option<std::net::IpAddr>,
    pub proxy: Option<crate::config::sources::HttpProxyConfig>,
}

impl PlayableTrack for HttpTrack {
    fn start_decoding(&self, config: crate::config::player::PlayerConfig) -> DecoderOutput {
        let (tx, rx) = flume::bounded::<AudioFrame>((config.buffer_duration_ms / 20) as usize);
        let (cmd_tx, cmd_rx) = flume::unbounded::<DecoderCommand>();
        let (err_tx, err_rx) = flume::bounded::<String>(1);

        let url = self.url.clone();
        let local_addr = self.local_addr;
        let proxy = self.proxy.clone();

        let handle = tokio::runtime::Handle::current();
        tokio::task::spawn_blocking(move || {
            let _guard = handle.enter();
            let reader = match reader::HttpReader::new(&url, local_addr, proxy) {
                Ok(r) => Box::new(r) as Box<dyn symphonia::core::io::MediaSource>,
                Err(e) => {
                    error!("Failed to create HttpReader for HTTP: {e}");
                    let _ = err_tx.send(format!("Failed to open stream: {e}"));
                    return;
                }
            };

            let kind = std::path::Path::new(&url)
                .extension()
                .and_then(|s| s.to_str())
                .map(crate::common::types::AudioFormat::from_ext);

            match AudioProcessor::new(reader, kind, tx, cmd_rx, Some(err_tx.clone()), config) {
                Ok(mut processor) => {
                    let spawn_result = std::thread::Builder::new()
                        .name(format!("http-decoder-{}", url))
                        .spawn(move || {
                            if let Err(e) = processor.run() {
                                error!("HTTP track audio processor error: {e}");
                            }
                        });
                    if let Err(e) = spawn_result {
                        error!("Failed to spawn HTTP decoder thread: {e}");
                        let _ = err_tx.send(format!("failed to spawn decoder thread: {e}"));
                    }
                }
                Err(e) => {
                    error!("HTTP track failed to initialize processor: {e}");
                    let _ = err_tx.send(format!("Failed to initialize processor: {e}"));
                }
            }
        });

        (rx, cmd_tx, err_rx)
    }
}

#[cfg(test)]
mod tests {
    use super::validate_public_url;

    #[test]
    fn rejects_loopback_http_targets() {
        assert!(validate_public_url("http://127.0.0.1:8080/audio.mp3").is_err());
        assert!(validate_public_url("http://[::1]:8080/audio.mp3").is_err());
    }

    #[test]
    fn rejects_non_http_schemes() {
        assert!(validate_public_url("file:///etc/passwd").is_err());
        assert!(validate_public_url("ftp://example.com/audio.mp3").is_err());
    }

    #[test]
    fn rejects_empty_public_url() {
        assert!(validate_public_url("").is_err());
    }

    #[test]
    fn accepts_valid_public_url() {
        let result = validate_public_url("http://example.com:8080/audio.mp3");
        assert!(result.is_ok());
        let (host, port, addrs) = result.unwrap();
        assert_eq!(host, "example.com");
        assert_eq!(port, 8080);
        assert!(!addrs.is_empty());
    }
}
