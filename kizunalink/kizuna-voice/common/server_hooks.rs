// Copyright (c) 2026 nikcodex (KizunaLink)
// Licensed under the MIT License

use std::sync::atomic::AtomicU64;

/// Marker trait for the server's application state.
///
/// Implemented by the concrete `AppState` in `kizuna-server`.
/// The player stores this to access server-level resources if needed.
pub trait ServerContext: Send + Sync + 'static {}

/// Abstraction over a client session (WebSocket connection).
///
/// Implemented by the concrete `Session` in `kizuna-server`.
/// The player manager uses this to push events back to the client
/// and register spawned tasks for lifecycle management.
pub trait SessionContext: Send + Sync + 'static {
    /// Send a protocol-level message to the connected client.
    fn send_message(&self, msg: &crate::lavalink::protocol::OutgoingMessage);

    /// Register an abort handle so the session can cancel spawned tasks on shutdown.
    fn register_task(&self, handle: tokio::task::AbortHandle);

    /// Access the cumulative frames-sent counter (for stats).
    fn total_sent_historical(&self) -> &AtomicU64;

    /// Access the cumulative frames-nulled counter (for stats).
    fn total_nulled_historical(&self) -> &AtomicU64;

    /// Look up a player context lock by guild id, if the session has one.
    fn get_player(
        &self,
        guild_id: &crate::common::types::GuildId,
    ) -> Option<std::sync::Arc<tokio::sync::RwLock<crate::discord::player::PlayerContext>>>;
}
