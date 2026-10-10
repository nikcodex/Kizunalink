// Copyright (c) 2026 nikcodex (KizunaLink)
// Licensed under the MIT License

use std::io::{Read, Seek, SeekFrom};

use symphonia::core::io::MediaSource;

use crate::{
    common::types::AnyResult,
    engine::source::{AudioSource, HttpSource, create_client_with_pinning},
};

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
        let (host, _port, addresses) = super::resolve_and_validate_public_url(url)?;

        let user_agent = crate::common::utils::default_user_agent();
        let client = create_client_with_pinning(
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
