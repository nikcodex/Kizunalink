// Copyright (c) 2026 nikcodex (KizunaLink)
// Licensed under the MIT License

// N05: server library code logs via `tracing`, it never prints (the binary may: `main.rs`
// delegates to the banner module which is allow-listed inside the `kizunalink` crate).
#![cfg_attr(
    not(test),
    deny(clippy::print_stdout, clippy::print_stderr, clippy::dbg_macro)
)]

pub mod api;
pub mod lavalink;
pub mod monitoring;
pub mod server;

// Re-export kizunalink submodules so server code can use `crate::` paths.
pub use kizunalink::common;
pub use kizunalink::discord;
pub use kizunalink::engine as audio;
pub use kizunalink::media::sources;

// Convenience re-exports matching how the server code references these.
pub use kizunalink::discord::gateway;
pub use kizunalink::discord::player;
pub use kizunalink::lavalink::protocol;
pub use kizunalink::lavalink::routeplanner;
