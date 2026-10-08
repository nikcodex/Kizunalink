// Copyright (c) 2026 nikcodex (KizunaLink)
// Licensed under the MIT License

use std::io::{Read, Seek, SeekFrom};

use symphonia::core::io::MediaSource;

use crate::{
    common::types::AnyResult,
    engine::source::{AudioSource, HttpSource, create_client},
};

/// Check whether an IP address is in a blocked range (loopback, private,
/// link-local, multicast, unspecified, IPv4-mapped IPv6 loopback, ULA, metadata).
fn is_blocked_ip(ip: &std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(ip) => {
            ip.is_loopback()
                || ip.is_private()
                || ip.is_link_local()
                || ip.is_multicast()
                || ip.is_unspecified()
        }
        std::net::IpAddr::V6(ip) => {
            // Check for IPv4-mapped IPv6 loopback (::ffff:127.0.0.1)
            if let Some(mapped_v4) = ip.to_ipv4_mapped() {
                return is_blocked_ip(&std::net::IpAddr::V4(mapped_v4));
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

pub struct HttpReader {
    inner: HttpSource,
}

impl HttpReader {
    pub fn new(
        url: &str,
        local_addr: Option<std::net::IpAddr>,
        proxy: Option<crate::config::sources::HttpProxyConfig>,
    ) -> AnyResult<Self> {
        // Validate URL and resolve DNS to prevent SSRF (SEC-001) and avoid blocking workers (PERF-001)
        // This runs inside spawn_blocking, so blocking DNS resolution is acceptable here.
        let parsed = reqwest::Url::parse(url).map_err(|e| format!("invalid HTTP URL: {e}"))?;
        if !matches!(parsed.scheme(), "http" | "https") {
            return Err(format!("unsupported HTTP URL scheme: {}", parsed.scheme()).into());
        }
        let host = parsed.host_str().ok_or("HTTP URL has no host")?.to_string();
        let port = parsed
            .port_or_known_default()
            .ok_or("HTTP URL has no port")?;
        let addresses: Vec<_> = (host.as_str(), port)
            .to_socket_addrs()
            .map_err(|e| format!("could not resolve HTTP host {host}: {e}"))?
            .collect();
        if addresses.is_empty() {
            return Err(format!("HTTP host {host} resolved to no addresses").into());
        }
        if addresses.iter().any(|addr| is_blocked_ip(&addr.ip())) {
            return Err(format!("HTTP host {host} resolves to a blocked IP address").into());
        }

        let user_agent = crate::common::utils::default_user_agent();
        let client = crate::engine::source::create_client(
            user_agent,
            local_addr,
            proxy,
            None,
            Some(host),
            Some(addresses),
        )?;
        let inner = HttpSource::new(client, url)?;
        Ok(Self { inner })
    }
}

impl Read for HttpReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.inner.read(buf)
    }
}

impl Seek for HttpReader {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        self.inner.seek(pos)
    }
}

impl MediaSource for HttpReader {
    fn is_seekable(&self) -> bool {
        self.inner.is_seekable()
    }

    fn byte_len(&self) -> Option<u64> {
        self.inner.byte_len()
    }

    // Explicitly delegate content_type if needed for probing
}

impl HttpReader {
    pub fn content_type(&self) -> Option<String> {
        self.inner.content_type()
    }
}
