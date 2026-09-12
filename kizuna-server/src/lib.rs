// Copyright (c) 2026 nikcodex (KizunaLink)
// Licensed under the MIT License

pub mod api;
pub mod lavalink;
pub mod monitoring;
pub mod server;

// Re-export kizunalink submodules so server code can use `crate::` paths.
pub use kizunalink::common;
pub use kizunalink::discord;
pub use kizunalink::engine as audio;
pub use kizunalink::media::sources as sources;

// Convenience re-exports matching how the server code references these.
pub use kizunalink::discord::gateway as gateway;
pub use kizunalink::discord::player as player;
