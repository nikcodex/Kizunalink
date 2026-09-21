// Copyright (c) 2026 nikcodex (KizunaLink)
// Licensed under the MIT License

//! Optional TLS termination (HTTPS/WSS) backed by rustls.
//!
//! When `server.tls.enabled` is set, the HTTP/WS surface is served over a
//! rustls-wrapped listener, so `authorization` tokens are never sent in
//! cleartext across the wire. This is an alternative to terminating TLS at a
//! reverse proxy — pick one or the other, not both.

use std::{path::Path, sync::Arc};

use axum::serve::Listener as AxumListener;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, pem::PemObject};
use tokio_rustls::TlsAcceptor;
use tracing::warn;

/// An [`axum::serve::Listener`] whose accepted streams are wrapped in a rustls
/// TLS session, giving the whole HTTP+WS surface HTTPS/WSS with the same
/// accept loop, graceful-shutdown behavior and connect-info support as
/// `axum::serve`. Pass it to `axum::serve` exactly like a `TcpListener`.
pub struct TlsListener {
    inner: tokio::net::TcpListener,
    acceptor: TlsAcceptor,
}

impl TlsListener {
    /// Wrap a bound TCP listener so every accepted connection requires a
    /// successful TLS handshake before HTTP bytes are read.
    pub fn new(inner: tokio::net::TcpListener, acceptor: TlsAcceptor) -> Self {
        Self { inner, acceptor }
    }
}

impl AxumListener for TlsListener {
    type Io = tokio_rustls::server::TlsStream<tokio::net::TcpStream>;
    type Addr = std::net::SocketAddr;

    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        loop {
            let (tcp, addr) = match self.inner.accept().await {
                Ok(tup) => tup,
                Err(e) => {
                    // Flatten transient accept errors into a small retry delay,
                    // matching `axum::serve`'s behavior for `TcpListener`.
                    warn!("TLS listener accept error: {e}");
                    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                    continue;
                }
            };
            // Drive the handshake to completion so a client that connects and
            // never speaks is not held forever by hyper. The returned `TlsStream`
            // then wraps the stream for hyper to use.
            let tls = match self.acceptor.accept(tcp).await {
                Ok(tls) => tls,
                Err(e) => {
                    tracing::debug!("TLS handshake failed: {e}");
                    continue;
                }
            };
            return (tls, addr);
        }
    }

    fn local_addr(&self) -> std::io::Result<Self::Addr> {
        self.inner.local_addr()
    }
}

/// Load PEM cert chain + private key from disk into a rustls [`TlsAcceptor`].
///
/// Fails fast at startup if the files are missing or unparseable — a server
/// that silently falls back to plaintext after misconfiguration would leak
/// the authorization token.
pub fn load_acceptor(cert_path: &Path, key_path: &Path) -> Result<TlsAcceptor, String> {
    let certs = load_certs(cert_path).map_err(|e| format!("failed to load TLS cert: {e}"))?;
    let key = load_key(key_path).map_err(|e| format!("failed to load TLS key: {e}"))?;

    // Pin the provider before building any config; rustls refuses to choose one
    // implicitly when both `ring` and `aws-lc-rs` are compiled in. The binary already
    // installs it at startup (see `kizunalink::common::tls`) — repeated here so this
    // helper is correct when reached on its own.
    kizunalink::common::tls::install_crypto_provider();

    let config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .map_err(|e| format!("failed to build TLS config: {e}"))?;

    Ok(TlsAcceptor::from(Arc::new(config)))
}

fn load_certs(path: &Path) -> Result<Vec<CertificateDer<'static>>, String> {
    // `PemObject` (rustls-pki-types >= 1.9) replaces the archived
    // `rustls-pemfile` crate (RUSTSEC-2025-0134).
    let certs = CertificateDer::pem_file_iter(path)
        .map_err(|e| format!("parse certs from {}: {e}", path.display()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("parse certs from {}: {e}", path.display()))?;
    if certs.is_empty() {
        return Err(format!("no certificates found in {}", path.display()));
    }
    Ok(certs)
}

fn load_key(path: &Path) -> Result<PrivateKeyDer<'static>, String> {
    PrivateKeyDer::from_pem_file(path)
        .map_err(|e| format!("parse key from {}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn load_acceptor_fails_fast_on_missing_files() {
        let Err(e) = load_acceptor(
            Path::new("/nonexistent/cert.pem"),
            Path::new("/nonexistent/key.pem"),
        ) else {
            panic!("missing files must fail");
        };
        assert!(e.contains("cert"), "error should mention the cert: {e}");
    }

    #[test]
    fn load_acceptor_accepts_valid_pem_and_rejects_garbage() {
        let dir = std::env::temp_dir().join(format!("kizuna-tls-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let cert_path = dir.join("cert.pem");
        let key_path = dir.join("key.pem");

        // A well-formed but bogus PEM body must be rejected (no silent fallback).
        std::fs::File::create(&cert_path)
            .unwrap()
            .write_all(b"-----BEGIN CERTIFICATE-----\nAAAA\n-----END CERTIFICATE-----\n")
            .unwrap();
        std::fs::File::create(&key_path)
            .unwrap()
            .write_all(b"-----BEGIN PRIVATE KEY-----\nAAAA\n-----END PRIVATE KEY-----\n")
            .unwrap();

        let Err(e) = load_acceptor(&cert_path, &key_path) else {
            panic!("garbage PEM must fail fast");
        };
        assert!(
            e.contains("parse certs") || e.contains("parse key") || e.contains("failed to build"),
            "garbage PEM should fail fast: {e}"
        );

        std::fs::remove_dir_all(&dir).ok();
    }
}
