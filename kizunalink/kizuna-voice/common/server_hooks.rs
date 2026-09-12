use std::sync::Arc;
use async_trait::async_trait;

/// A trait that abstracts away the server's AppState and Session,
/// so the voice library doesn't need to know about HTTP or WebSockets.
#[async_trait]
pub trait ServerContext: Send + Sync {
    // Add methods here if the player needs to fetch info from the server
}

#[async_trait]
pub trait SessionContext: Send + Sync {
    // Add methods here if the player needs to push events to the websocket
    async fn send_event(&self, event_data: &[u8]);
}
