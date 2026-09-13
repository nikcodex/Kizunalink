// Copyright (c) 2026 nikcodex (KizunaLink)
// Licensed under the MIT License

use std::{
    collections::VecDeque,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};

use axum::extract::ws::Message;
use dashmap::DashMap;
use parking_lot::Mutex;
use tokio::task::AbortHandle;

use crate::{
    common::types::{GuildId, SessionId, UserId},
    player::PlayerContext,
    protocol,
    server::AppState,
};

pub type PlayerMap = DashMap<GuildId, Arc<tokio::sync::RwLock<PlayerContext>>>;

pub struct Session {
    pub session_id: SessionId,
    pub user_id: Option<UserId>,
    pub players: PlayerMap,
    pub sender: parking_lot::RwLock<flume::Sender<Message>>,
    pub resumable: AtomicBool,
    pub resume_timeout: AtomicU64,
    /// True when WS is disconnected but session is kept for resume.
    pub paused: AtomicBool,
    pub event_queue: Mutex<VecDeque<String>>,
    pub max_queue_size: usize,

    pub last_stats_sent: AtomicU64,
    pub last_stats_nulled: AtomicU64,
    pub total_sent_historical: AtomicU64,
    pub total_nulled_historical: AtomicU64,

    /// Abort handles for all spawned player tasks (gateway + track).
    /// Stored separately so shutdown never needs to hold the player write lock.
    task_handles: Mutex<Vec<AbortHandle>>,
}

impl Session {
    pub fn new(
        session_id: SessionId,
        user_id: Option<UserId>,
        sender: flume::Sender<Message>,
        max_queue_size: usize,
    ) -> Self {
        Self {
            session_id,
            user_id,
            players: DashMap::new(),
            sender: parking_lot::RwLock::new(sender),
            resumable: AtomicBool::new(false),
            resume_timeout: AtomicU64::new(60),
            paused: AtomicBool::new(false),
            event_queue: Mutex::new(VecDeque::new()),
            max_queue_size,
            last_stats_sent: AtomicU64::new(0),
            last_stats_nulled: AtomicU64::new(0),
            total_sent_historical: AtomicU64::new(0),
            total_nulled_historical: AtomicU64::new(0),
            task_handles: Mutex::new(Vec::new()),
        }
    }

    /// Register an abort handle for a spawned player task.
    ///
    /// Called by player manager code when it spawns the gateway or track task.
    /// The handle is aborted during session shutdown without needing the player lock.
    pub fn register_task(&self, handle: AbortHandle) {
        self.task_handles.lock().push(handle);
    }

    pub fn get_or_create_player(
        &self,
        guild_id: GuildId,
        state: Arc<AppState>,
    ) -> Arc<tokio::sync::RwLock<PlayerContext>> {
        self.players
            .entry(guild_id.clone())
            .or_insert_with(|| {
                let player_config = state.config.player.clone();
                let ctx: Arc<dyn kizunalink::common::server_hooks::ServerContext> = state;
                Arc::new(tokio::sync::RwLock::new(PlayerContext::new(
                    guild_id,
                    &player_config,
                    ctx,
                )))
            })
            .value()
            .clone()
    }

    pub async fn destroy_player(&self, guild_id: &GuildId) -> bool {
        if let Some((_, player_arc)) = self.players.remove(guild_id) {
            let mut player = player_arc.write().await;
            player.destroy().await;
            true
        } else {
            false
        }
    }

    pub fn send_json(&self, json: impl Into<String>) {
        if self.paused.load(Ordering::Relaxed) {
            if self.max_queue_size == 0 {
                return;
            }

            let mut queue = self.event_queue.lock();
            if queue.len() >= self.max_queue_size {
                queue.pop_front();
            }
            queue.push_back(json.into());
        } else {
            let msg = Message::Text(json.into().into());
            // B07: don't silently drop frames — log when the WebSocket sink is
            // gone (e.g. the session disconnected before a detached task ran).
            if let Err(e) = self.sender.read().try_send(msg) {
                match e {
                    flume::TrySendError::Full(_) => tracing::warn!(
                        "WebSocket output queue full for session {}; dropping message",
                        self.session_id
                    ),
                    flume::TrySendError::Disconnected(_) => tracing::debug!(
                        "Failed to send WS message for session {}: channel disconnected",
                        self.session_id
                    ),
                }
            }
        }
    }

    pub fn send_message(&self, msg: &protocol::OutgoingMessage) {
        match serde_json::to_string(msg) {
            Ok(json) => self.send_json(json),
            Err(e) => tracing::warn!(
                "Failed to serialize outgoing message for session {}: {}",
                self.session_id,
                e
            ),
        }
    }

    pub async fn shutdown(&self) {
        tracing::info!("Shutting down session: {}", self.session_id);
        self.stop_all_players();
        let guilds: Vec<GuildId> = self.players.iter().map(|kv| kv.key().clone()).collect();
        for guild in guilds {
            self.destroy_player(&guild).await;
        }
    }

    fn stop_all_players(&self) {
        for handle in self.task_handles.lock().drain(..) {
            handle.abort();
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        tracing::info!("Dropping session: {}", self.session_id);
        self.stop_all_players();
        self.players.clear();
    }
}

// Implement the library's SessionContext trait so the player manager
// can push events back through the WebSocket without depending on kizuna-server.
impl kizunalink::common::server_hooks::SessionContext for Session {
    fn send_message(&self, msg: &kizunalink::lavalink::protocol::OutgoingMessage) {
        Session::send_message(self, msg);
    }

    fn register_task(&self, handle: tokio::task::AbortHandle) {
        Session::register_task(self, handle);
    }

    fn total_sent_historical(&self) -> &std::sync::atomic::AtomicU64 {
        &self.total_sent_historical
    }

    fn total_nulled_historical(&self) -> &std::sync::atomic::AtomicU64 {
        &self.total_nulled_historical
    }

    fn get_player(
        &self,
        guild_id: &kizunalink::common::types::GuildId,
    ) -> Option<std::sync::Arc<tokio::sync::RwLock<kizunalink::discord::player::PlayerContext>>>
    {
        self.players.get(guild_id).map(|kv| kv.value().clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_event_queue_size_drops_paused_events() {
        let (sender, _receiver) = flume::unbounded();
        let session = Session::new(
            kizunalink::common::types::SessionId("test-session".into()),
            None,
            sender,
            0,
        );
        session.paused.store(true, Ordering::Relaxed);

        session.send_json("event");

        assert!(session.event_queue.lock().is_empty());
    }
}
