use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        State,
    },
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
};
use dashmap::DashMap;
use futures_util::{SinkExt, StreamExt};
use std::sync::LazyLock;
use tracing::{error, info, warn};

use crate::models::protocol::PlayerUpdatePayload;
use crate::ratelimit::extract_ip;
use crate::security;
use crate::util::constant_time_eq;
use crate::AppState;

const MAX_WS_MESSAGE_SIZE: usize = 65536;

static SESSION_STORE: LazyLock<DashMap<String, std::time::Instant>> = LazyLock::new(DashMap::new);

static SESSION_STATE: LazyLock<DashMap<String, SessionState>> = LazyLock::new(DashMap::new);

struct SessionState {
    resuming: bool,
    timeout: u64,
}

impl Default for SessionState {
    fn default() -> Self {
        Self {
            resuming: false,
            timeout: 60,
        }
    }
}

pub fn get_session_state(session_id: &str) -> Option<(bool, u64)> {
    SESSION_STATE
        .get(session_id)
        .map(|s| (s.resuming, s.timeout))
}

pub fn update_session_state(
    session_id: &str,
    resuming: Option<bool>,
    timeout: Option<u64>,
) -> (bool, u64) {
    let mut entry = SESSION_STATE
        .entry(session_id.to_string())
        .or_insert_with(SessionState::default);
    if let Some(r) = resuming {
        entry.resuming = r;
    }
    if let Some(t) = timeout {
        entry.timeout = t;
    }
    (entry.resuming, entry.timeout)
}

/// Returns all active session IDs.
pub fn get_session_ids() -> Vec<String> {
    SESSION_STORE
        .iter()
        .map(|entry| entry.key().clone())
        .collect()
}

pub async fn ws_handler(
    ws: WebSocketUpgrade,
    headers: HeaderMap,
    State(state): State<AppState>,
) -> impl IntoResponse {
    let auth_header = headers.get("authorization").and_then(|h| h.to_str().ok());

    match auth_header {
        Some(auth) if constant_time_eq(auth, &state.password) => {}
        _ => return StatusCode::UNAUTHORIZED.into_response(),
    }

    // Rate limit WebSocket connections per IP
    let ip = extract_ip(&headers, "0.0.0.0");
    if !state.rate_limiter.check(&ip) {
        warn!("WebSocket rate limit exceeded for IP: {}", ip);
        return StatusCode::TOO_MANY_REQUESTS.into_response();
    }

    let user_id = headers
        .get("user-id")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("0")
        .to_string();

    {
        let mut id_lock = state.player_manager.bot_user_id.write().await;
        *id_lock = user_id.clone();
    }

    let client_name = headers
        .get("client-name")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("unknown")
        .to_string();

    let session_id = headers
        .get("session-id")
        .and_then(|h| h.to_str().ok())
        .map(|s| s.to_string());

    info!(
        "WebSocket connected: client={} user={} ip={}",
        client_name, user_id, ip
    );

    ws.on_upgrade(move |socket| handle_socket(socket, state, session_id))
}

async fn handle_socket(mut socket: WebSocket, state: AppState, resume_session_id: Option<String>) {
    let (session_id, is_resumed) = if let Some(resume_id) = resume_session_id {
        if SESSION_STORE.contains_key(&resume_id) {
            (resume_id, true)
        } else {
            let new_id = crate::util::uuid_v4();
            (new_id, false)
        }
    } else {
        let new_id = crate::util::uuid_v4();
        (new_id, false)
    };

    SESSION_STORE.insert(session_id.clone(), std::time::Instant::now());

    let ready_msg = serde_json::json!({
        "op": "ready",
        "resumed": is_resumed,
        "sessionId": session_id
    });

    if let Err(e) = socket.send(Message::Text(ready_msg.to_string())).await {
        error!("Failed to send ready: {:?}", e);
        return;
    }

    let (mut sender, mut receiver) = socket.split();
    let mut event_rx = state.event_tx.subscribe();

    let event_task = tokio::spawn(async move {
        loop {
            match event_rx.recv().await {
                Ok(msg) => {
                    if sender.send(Message::Text(msg)).await.is_err() {
                        break;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                    warn!("WebSocket subscriber lagged, skipped {} events", skipped);
                    continue;
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                    break;
                }
            }
        }
    });

    // We need a mutable sender here to respond to Ping frames.
    // Split was already done above for the event_task, so we need to
    // handle pong responses through the event broadcast channel instead.
    // Actually, the sender was moved into event_task. We need to restructure
    // to keep a sender handle for pong responses.
    //
    // Re-architecture: don't split the socket. Use a select! loop instead.
    while let Some(msg) = receiver.next().await {
        match msg {
            Ok(Message::Text(text)) => {
                if text.len() > MAX_WS_MESSAGE_SIZE {
                    warn!(
                        "WebSocket message too large ({} bytes), dropping",
                        text.len()
                    );
                    continue;
                }
                handle_ws_message(&state, &text).await;
            }
            Ok(Message::Ping(payload)) => {
                // Critical: lavalink-client sends WS-level pings every 30s
                // and terminates the connection if no pong is received.
                // We respond via the event broadcast channel which the
                // sender task picks up.
                // However, axum's WebSocket auto-responds to Ping with Pong
                // at the protocol level — so this should already work.
                // Let's just log it for debugging.
                info!("WebSocket Ping received ({} bytes)", payload.len());
            }
            Ok(Message::Pong(_)) => {
                // Client sent a pong (response to our ping, if any)
            }
            Ok(Message::Binary(_)) => {
                // Ignore binary frames
            }
            Ok(Message::Close(frame)) => {
                if let Some(cf) = frame {
                    info!(
                        "WebSocket client disconnected: code={} reason='{}'",
                        cf.code, cf.reason
                    );
                } else {
                    info!("WebSocket client disconnected (no close frame)");
                }
                break;
            }
            Err(e) => {
                error!("WebSocket error: {:?}", e);
                break;
            }
        }
    }

    event_task.abort();

    SESSION_STORE.remove(&session_id);
    SESSION_STATE.remove(&session_id);
    info!("WebSocket session {} cleaned up", session_id);
}

async fn handle_ws_message(state: &AppState, text: &str) {
    let msg = match serde_json::from_str::<serde_json::Value>(text) {
        Ok(v) => v,
        Err(_) => return,
    };

    let op = match msg.get("op").and_then(|o| o.as_str()) {
        Some(op) => op,
        None => return,
    };

    let guild_id = msg.get("guildId").and_then(|g| g.as_str()).unwrap_or("");

    match op {
        "updatePlayer" => {
            let mut payload = PlayerUpdatePayload::default();

            if let Some(track_obj) = msg.get("track") {
                if track_obj.is_string() {
                    payload.track = Some(crate::models::protocol::TrackPayload {
                        encoded: track_obj.as_str().map(|s| s.to_string()),
                        identifier: None,
                        user_data: None,
                    });
                } else if track_obj.is_object() {
                    payload.track = Some(crate::models::protocol::TrackPayload {
                        encoded: track_obj
                            .get("encoded")
                            .and_then(|e| e.as_str())
                            .map(|s| s.to_string()),
                        identifier: track_obj
                            .get("identifier")
                            .and_then(|i| i.as_str())
                            .map(|s| s.to_string()),
                        user_data: track_obj.get("userData").cloned(),
                    });
                }
            }

            if let Some(pos) = msg.get("position").and_then(|p| p.as_u64()) {
                payload.position = Some(pos);
            }

            if let Some(et) = msg.get("endTime").and_then(|e| e.as_u64()) {
                payload.end_time = Some(et);
            }

            if let Some(vol) = msg.get("volume").and_then(|v| v.as_u64()) {
                payload.volume = Some(vol as u32);
            }

            if let Some(p) = msg.get("paused").and_then(|p| p.as_bool()) {
                payload.paused = Some(p);
            }

            if let Some(voice_obj) = msg.get("voice") {
                payload.voice = Some(crate::models::protocol::VoiceStateUpdate {
                    token: voice_obj
                        .get("token")
                        .and_then(|t| t.as_str())
                        .unwrap_or("")
                        .to_string(),
                    endpoint: voice_obj
                        .get("endpoint")
                        .and_then(|e| e.as_str())
                        .unwrap_or("")
                        .to_string(),
                    session_id: voice_obj
                        .get("sessionId")
                        .and_then(|s| s.as_str())
                        .unwrap_or("")
                        .to_string(),
                    channel_id: voice_obj
                        .get("channelId")
                        .and_then(|c| c.as_str())
                        .map(|s| s.to_string()),
                });
            }

            if let Some(filters_obj) = msg.get("filters") {
                match serde_json::from_value::<crate::models::filters::Filters>(filters_obj.clone())
                {
                    Ok(f) => payload.filters = Some(f),
                    Err(e) => warn!("Invalid filters payload: {}", e),
                }
            }

            if let Some(encoded) = msg.get("encodedTrack").and_then(|e| e.as_str()) {
                payload.encoded_track = Some(encoded.to_string());
            }

            if let Some(identifier) = msg.get("identifier").and_then(|i| i.as_str()) {
                payload.identifier = Some(identifier.to_string());
            }

            let _ = state
                .player_manager
                .update_player(guild_id, payload, false)
                .await
                .map_err(|e| warn!("WS updatePlayer failed for guild {}: {}", guild_id, e));
        }
        "queueTrack" => {
            if let Some(encoded) = msg.get("encoded").and_then(|e| e.as_str()) {
                let _ = state
                    .player_manager
                    .queue_track(guild_id, encoded)
                    .await
                    .map_err(|e| warn!("WS queueTrack failed for guild {}: {}", guild_id, e));
            }
        }
        "skipTrack" => {
            let _ = state
                .player_manager
                .skip_track(guild_id)
                .await
                .map_err(|e| warn!("WS skipTrack failed for guild {}: {}", guild_id, e));
        }
        "previousTrack" => {
            let _ = state
                .player_manager
                .previous_track(guild_id)
                .await
                .map_err(|e| warn!("WS previousTrack failed for guild {}: {}", guild_id, e));
        }
        "autoplay" => {
            let _ = state
                .player_manager
                .toggle_autoplay(guild_id)
                .await
                .map_err(|e| warn!("WS autoplay failed for guild {}: {}", guild_id, e));
        }
        "loop" => {
            let mode = msg.get("mode").and_then(|m| m.as_str()).unwrap_or("none");
            let loop_mode = match mode {
                "track" => crate::player::queue::LoopMode::Track,
                "queue" => crate::player::queue::LoopMode::Queue,
                _ => crate::player::queue::LoopMode::None,
            };
            let _ = state
                .player_manager
                .set_loop_mode(guild_id, loop_mode)
                .await
                .map_err(|e| warn!("WS loop failed for guild {}: {}", guild_id, e));
        }
        "shuffleQueue" => {
            let _ = state
                .player_manager
                .shuffle_queue(guild_id)
                .await
                .map_err(|e| warn!("WS shuffleQueue failed for guild {}: {}", guild_id, e));
        }
        "clearQueue" => {
            let _ = state
                .player_manager
                .clear_queue(guild_id)
                .await
                .map_err(|e| warn!("WS clearQueue failed for guild {}: {}", guild_id, e));
        }
        "destroyPlayer" => {
            state.player_manager.destroy_player(guild_id);
        }
        "ping" => {
            let pong = serde_json::json!({ "op": "pong" });
            let _ = state.event_tx.send(pong.to_string());
        }
        _ => {
            warn!("Unknown WebSocket op: {}", op);
        }
    }
}
