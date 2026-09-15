// Copyright (c) 2026 nikcodex (KizunaLink)
// Licensed under the MIT License

//! Process-wide rustls crypto provider selection.
//!
//! Feature unification leaves **both** `ring` and `aws-lc-rs` enabled in this
//! dependency graph: our workspace pins `ring`, while `reqwest`/`hyper-rustls`
//! opt into `aws-lc-rs` through their defaults. When rustls cannot resolve
//! exactly one provider from crate features it refuses to guess, and *every*
//! TLS handshake panics with:
//!
//! ```text
//! Could not automatically determine the process-level CryptoProvider from
//! Rustls crate features.
//! ```
//!
//! That failure is easy to miss because it happens on a spawned task: the voice
//! gateway's WebSocket connect dies while the player keeps reporting progress,
//! so the node looks alive but never sends a single audio packet.
//!
//! Installing the provider once, explicitly, makes handshakes deterministic.
//! `main()` calls this before building the runtime; the voice gateway also calls
//! it lazily so the library stays correct when embedded in another binary.

use std::sync::OnceLock;

static INSTALLED: OnceLock<()> = OnceLock::new();

/// Install the `ring` provider as the process-wide rustls default.
///
/// Idempotent and safe to call from anywhere, at any time: the first caller
/// wins and later calls are no-ops. `ring` matches the workspace's pinned
/// `rustls` features, so it is always compiled in.
pub fn install_crypto_provider() {
    INSTALLED.get_or_init(|| {
        // A provider may already be installed by another component; that is a
        // success, not an error, so the result is deliberately discarded.
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_is_idempotent() {
        install_crypto_provider();
        install_crypto_provider();
        assert!(
            rustls::crypto::CryptoProvider::get_default().is_some(),
            "a provider must be installed after the first call"
        );
    }
}
