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
use std::collections::{HashSet, VecDeque};
use std::sync::LazyLock;
use std::time::Duration;
use tokio::sync::mpsc;
use tracing::{error, info, warn};

use crate::models::protocol::PlayerUpdatePayload;
use crate::ratelimit::extract_ip;
use crate::util::constant_time_eq;
use crate::AppState;

const MAX_WS_MESSAGE_SIZE: usize = 65536;

static SESSION_STORE: LazyLock<DashMap<String, std::time::Instant>> = LazyLock::new(DashMap::new);

pub static SESSION_STATE: LazyLock<DashMap<String, SessionState>> = LazyLock::new(DashMap::new);

static DISPATCHER_STARTED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

pub struct SessionState {
    pub resuming: bool,
    pub timeout: u64,
    pub guild_ids: HashSet<String>,
    pub event_buffer: VecDeque<String>,
    pub connected: bool,
    pub user_id: String,
}

impl Default for SessionState {
    fn default() -> Self {
        Self {
            resuming: false,
            timeout: 60,
            guild_ids: HashSet::new(),
            event_buffer: VecDeque::new(),
            connected: true,
            user_id: String::new(),
        }
    }
}

pub fn add_guild_to_session(session_id: &str, guild_id: &str) {
    let mut entry = SESSION_STATE.entry(session_id.to_string()).or_default();
    entry.guild_ids.insert(guild_id.to_string());
}

pub fn remove_guild_from_session(session_id: &str, guild_id: &str) {
    if let Some(mut entry) = SESSION_STATE.get_mut(session_id) {
        entry.guild_ids.remove(guild_id);
    }
}

pub fn ensure_event_dispatcher(event_tx: &tokio::sync::broadcast::Sender<String>) {
    if DISPATCHER_STARTED
        .compare_exchange(
            false,
            true,
            std::sync::atomic::Ordering::SeqCst,
            std::sync::atomic::Ordering::SeqCst,
        )
        .is_ok()
    {
        let mut rx = event_tx.subscribe();
        tokio::spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(msg) => {
                        let guild_id = serde_json::from_str::<serde_json::Value>(&msg)
                            .ok()
                            .and_then(|v| {
                                v.get("guildId")
                                    .and_then(|g| g.as_str())
                                    .map(|s| s.to_string())
                            });

                        for mut entry in SESSION_STATE.iter_mut() {
                            let state = entry.value_mut();
                            if !state.connected {
                                let should_buffer = match &guild_id {
                                    Some(gid) => state.guild_ids.contains(gid),
                                    None => true,
                                };
                                if should_buffer {
                                    if state.event_buffer.len() >= 100 {
                                        state.event_buffer.pop_front();
                                    }
                                    state.event_buffer.push_back(msg.clone());
                                }
                            }
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        });
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
    let mut entry = SESSION_STATE.entry(session_id.to_string()).or_default();
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
        let mut retry_headers = HeaderMap::new();
        if let Ok(val) =
            axum::http::HeaderValue::from_str(&state.rate_limiter.window_secs().to_string())
        {
            retry_headers.insert(axum::http::header::RETRY_AFTER, val);
        }
        return (StatusCode::TOO_MANY_REQUESTS, retry_headers).into_response();
    }

    let user_id = headers
        .get("user-id")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("0")
        .to_string();

    {
        let mut id_lock = state.player_manager.bot_user_id.write().await;
        if *id_lock == "0" && user_id != "0" {
            *id_lock = user_id.clone();
        }
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

    ws.on_upgrade(move |socket| handle_socket(socket, state, session_id, user_id))
}

async fn handle_socket(
    mut socket: WebSocket,
    state: AppState,
    resume_session_id: Option<String>,
    user_id: String,
) {
    ensure_event_dispatcher(&state.event_tx);

    let (session_id, is_resumed) = if let Some(resume_id) = resume_session_id {
        if SESSION_STORE.contains_key(&resume_id) || SESSION_STATE.contains_key(&resume_id) {
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
        SESSION_STORE.remove(&session_id);
        SESSION_STATE.remove(&session_id);
        return;
    }

    if is_resumed {
        if let Some(mut entry) = SESSION_STATE.get_mut(&session_id) {
            entry.connected = true;
            if !user_id.is_empty() && user_id != "0" {
                entry.user_id = user_id;
            }
            let replay_events: Vec<String> = entry.event_buffer.drain(..).collect();
            drop(entry);
            for event in replay_events {
                if let Err(e) = socket.send(Message::Text(event)).await {
                    error!("Failed to replay event on resume: {:?}", e);
                    break;
                }
            }
        }
    } else {
        let mut entry = SESSION_STATE.entry(session_id.clone()).or_default();
        entry.connected = true;
        entry.user_id = user_id;
    }

    crate::metrics::Metrics::global().ws_connections.inc();
    crate::metrics::Metrics::global().active_sessions.inc();

    let (mut sender, mut receiver) = socket.split();
    let mut event_rx = state.event_tx.subscribe();
    let (direct_tx, mut direct_rx) = mpsc::channel::<Message>(32);
    let sess_id = session_id.clone();

    let event_task = tokio::spawn(async move {
        let mut ping_interval = tokio::time::interval(Duration::from_secs(30));
        ping_interval.reset();
        loop {
            tokio::select! {
                _ = ping_interval.tick() => {
                    if sender.send(Message::Ping(Vec::new())).await.is_err() {
                        break;
                    }
                }
                direct_msg = direct_rx.recv() => {
                    match direct_msg {
                        Some(msg) => {
                            if sender.send(msg).await.is_err() {
                                break;
                            }
                        }
                        None => break,
                    }
                }
                event_msg = event_rx.recv() => {
                    match event_msg {
                        Ok(msg) => {
                            let should_forward = if let Ok(v) = serde_json::from_str::<serde_json::Value>(&msg) {
                                if let Some(gid) = v.get("guildId").and_then(|g| g.as_str()) {
                                    SESSION_STATE
                                        .get(&sess_id)
                                        .map(|s| s.guild_ids.contains(gid))
                                        .unwrap_or(false)
                                } else {
                                    true
                                }
                            } else {
                                true
                            };

                            if should_forward && sender.send(Message::Text(msg)).await.is_err() {
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
            }
        }
    });

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
                handle_ws_message(&state, &session_id, &text, &direct_tx).await;
            }
            Ok(Message::Ping(payload)) => {
                info!("WebSocket Ping received ({} bytes)", payload.len());
            }
            Ok(Message::Pong(_)) => {}
            Ok(Message::Binary(_)) => {}
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

    crate::metrics::Metrics::global().ws_connections.dec();

    let timeout = SESSION_STATE
        .get(&session_id)
        .map(|s| s.timeout)
        .unwrap_or(60);

    if let Some(mut entry) = SESSION_STATE.get_mut(&session_id) {
        entry.connected = false;
    }

    if timeout > 0 {
        let s_id = session_id.clone();
        tokio::spawn(async move {
            tokio::time::sleep(tokio::time::Duration::from_secs(timeout)).await;
            if let Some(entry) = SESSION_STATE.get(&s_id) {
                if !entry.connected {
                    drop(entry);
                    SESSION_STORE.remove(&s_id);
                    SESSION_STATE.remove(&s_id);
                    crate::metrics::Metrics::global().active_sessions.dec();
                    info!("WebSocket session {} expired after resume timeout", s_id);
                }
            }
        });
        info!(
            "WebSocket session {} disconnected, waiting up to {}s to resume",
            session_id, timeout
        );
    } else {
        SESSION_STORE.remove(&session_id);
        SESSION_STATE.remove(&session_id);
        crate::metrics::Metrics::global().active_sessions.dec();
        info!("WebSocket session {} cleaned up", session_id);
    }
}

async fn handle_ws_message(
    state: &AppState,
    session_id: &str,
    text: &str,
    direct_tx: &mpsc::Sender<Message>,
) {
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
                add_guild_to_session(session_id, guild_id);
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
            remove_guild_from_session(session_id, guild_id);
            state.player_manager.destroy_player(guild_id);
        }
        "ping" => {
            let pong = serde_json::json!({ "op": "pong" });
            let _ = direct_tx.send(Message::Text(pong.to_string())).await;
        }
        _ => {
            warn!("Unknown WebSocket op: {}", op);
        }
    }
}
