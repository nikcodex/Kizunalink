use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        State,
    },
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
};
use futures_util::{SinkExt, StreamExt};
use std::time::Duration;
use tokio::sync::mpsc;
use tracing::{error, info, warn};

use crate::models::protocol::PlayerUpdatePayload;
use crate::ratelimit::extract_ip;
use crate::security;
use crate::util::constant_time_eq;
use crate::AppState;

const MAX_WS_MESSAGE_SIZE: usize = 65536;

static DISPATCHER_STARTED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

pub fn ensure_event_dispatcher(state: &AppState) {
    if DISPATCHER_STARTED
        .compare_exchange(
            false,
            true,
            std::sync::atomic::Ordering::SeqCst,
            std::sync::atomic::Ordering::SeqCst,
        )
        .is_ok()
    {
        let mut rx = state.event_tx.subscribe();
        let sm = state.session_manager.clone();
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
                        sm.buffer_event(guild_id.as_deref(), &msg);
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        });
    }
}

pub async fn ws_handler(
    ws: WebSocketUpgrade,
    headers: HeaderMap,
    State(state): State<AppState>,
) -> impl IntoResponse {
    let auth_header = headers.get("authorization").and_then(|h| h.to_str().ok());

    match auth_header {
        Some(auth) if constant_time_eq(auth, &state.password) => {}
        _ => {
            crate::metrics::Metrics::global().errors_auth.inc();
            return StatusCode::UNAUTHORIZED.into_response();
        }
    }

    // Rate limit WebSocket connections per IP
    let ip = extract_ip(&headers, "0.0.0.0");
    if !state.rate_limiter.check(&ip) {
        warn!(
            "WebSocket rate limit exceeded for IP: {}",
            security::sanitize_for_log(&ip)
        );
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
        security::sanitize_for_log(&client_name),
        security::sanitize_for_log(&user_id),
        security::sanitize_for_log(&ip)
    );

    // Enforce the message/frame limits at the protocol layer. Without this the
    // 64 KiB guard in `handle_socket` only runs *after* tungstenite has already
    // buffered the whole frame (axum's defaults are 64 MiB message / 16 MiB
    // frame), so a single connection could pin that much memory per message.
    let ws = ws.max_message_size(MAX_WS_MESSAGE_SIZE);
    let ws = ws.max_frame_size(MAX_WS_MESSAGE_SIZE);
    ws.on_upgrade(move |socket| handle_socket(socket, state, session_id, user_id))
}

async fn handle_socket(
    mut socket: WebSocket,
    state: AppState,
    resume_session_id: Option<String>,
    user_id: String,
) {
    ensure_event_dispatcher(&state);

    let (session_id, is_resumed, replay_events) = match state
        .session_manager
        .handle_connection(resume_session_id, user_id)
    {
        Ok(res) => res,
        Err(e) => {
            warn!("WebSocket connection rejected: {}", e);
            let _ = socket
                .send(Message::Close(Some(axum::extract::ws::CloseFrame {
                    code: 1008,
                    reason: e.into(),
                })))
                .await;
            return;
        }
    };

    let ready_msg = serde_json::json!({
        "op": "ready",
        "resumed": is_resumed,
        "sessionId": session_id
    });

    if let Err(e) = socket.send(Message::Text(ready_msg.to_string())).await {
        error!("Failed to send ready: {:?}", e);
        state.session_manager.remove_session(&session_id);
        return;
    }

    for event in replay_events {
        if let Err(e) = socket.send(Message::Text(event)).await {
            error!("Failed to replay event on resume: {:?}", e);
            break;
        }
    }

    crate::metrics::Metrics::global().ws_connections.inc();

    let (mut sender, mut receiver) = socket.split();
    let mut event_rx = state.event_tx.subscribe();
    let (direct_tx, mut direct_rx) = mpsc::channel::<Message>(32);
    let sess_id = session_id.clone();
    let sm_for_events = state.session_manager.clone();

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
                                    sm_for_events.is_guild_subscribed(&sess_id, gid)
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
                let _ = direct_tx.send(Message::Pong(payload)).await;
            }
            Ok(Message::Pong(_)) => {}
            Ok(Message::Binary(_)) => {}
            Ok(Message::Close(frame)) => {
                if let Some(cf) = frame {
                    info!(
                        "WebSocket client disconnected: code={} reason='{}'",
                        cf.code,
                        security::sanitize_for_log(&cf.reason)
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

    if let Some(timeout) = state.session_manager.mark_disconnected(&session_id) {
        let sm = state.session_manager.clone();
        let pm = state.player_manager.clone();
        let s_id = session_id.clone();
        tokio::spawn(async move {
            tokio::time::sleep(tokio::time::Duration::from_secs(timeout)).await;
            if let Some(guild_ids) = sm.expire_if_disconnected(&s_id) {
                info!(
                    "WebSocket session {} expired after resume timeout, destroying {} players",
                    s_id,
                    guild_ids.len()
                );
                for gid in guild_ids {
                    pm.destroy_player(&gid);
                }
            }
        });
        info!(
            "WebSocket session {} disconnected, waiting up to {}s to resume",
            session_id, timeout
        );
    } else {
        if let Some(st) = state.session_manager.remove_session(&session_id) {
            for gid in st.guild_ids {
                state.player_manager.destroy_player(&gid);
            }
        }
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

    let requires_guild = matches!(
        op,
        "updatePlayer"
            | "queueTrack"
            | "skipTrack"
            | "previousTrack"
            | "autoplay"
            | "loop"
            | "shuffleQueue"
            | "clearQueue"
            | "destroyPlayer"
    );
    if requires_guild {
        // REST validates the guild id before it ever reaches the player manager;
        // the WebSocket path did not. Without this an authenticated client could
        // create players keyed by arbitrary (unbounded, non-numeric) strings —
        // growing `players` without limit — and inject newlines into the logs.
        if let Err(e) = security::validate_guild_id(guild_id) {
            let safe_op = security::sanitize_for_log(op);
            let safe_gid = security::sanitize_for_log(guild_id);
            warn!("WS op '{}' rejected: {} guild='{}'", safe_op, e, safe_gid);
            return;
        }
    }

    match op {
        "updatePlayer" => {
            let mut payload = PlayerUpdatePayload::default();
            let mut voice_attached = false;

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
                voice_attached = true;
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

            let safe_gid = security::sanitize_for_log(guild_id);
            match state
                .player_manager
                .update_player(guild_id, payload, false, session_id)
                .await
            {
                Ok(_) => {
                    // Subscribe only after the session successfully claimed the player.
                    if !guild_id.is_empty() {
                        state.session_manager.add_guild(session_id, guild_id);
                    }
                }
                Err(e) => {
                    warn!(
                        "WS updatePlayer failed for guild {} (voice={}): {}",
                        safe_gid, voice_attached, e
                    );
                }
            }
        }
        "queueTrack" => {
            if let Some(encoded) = msg.get("encoded").and_then(|e| e.as_str()) {
                let safe_gid = security::sanitize_for_log(guild_id);
                match state
                    .player_manager
                    .queue_track(guild_id, encoded, session_id)
                    .await
                {
                    Ok(_) => {
                        if !guild_id.is_empty() {
                            state.session_manager.add_guild(session_id, guild_id);
                        }
                    }
                    Err(e) => {
                        warn!("WS queueTrack failed for guild {}: {}", safe_gid, e);
                    }
                }
            }
        }
        "skipTrack" => {
            let safe_gid = security::sanitize_for_log(guild_id);
            match state.player_manager.skip_track(guild_id, session_id).await {
                Ok(_) => {
                    if !guild_id.is_empty() {
                        state.session_manager.add_guild(session_id, guild_id);
                    }
                }
                Err(e) => warn!("WS skipTrack failed for guild {}: {}", safe_gid, e),
            }
        }
        "previousTrack" => {
            let safe_gid = security::sanitize_for_log(guild_id);
            match state
                .player_manager
                .previous_track(guild_id, session_id)
                .await
            {
                Ok(_) => {
                    if !guild_id.is_empty() {
                        state.session_manager.add_guild(session_id, guild_id);
                    }
                }
                Err(e) => warn!("WS previousTrack failed for guild {}: {}", safe_gid, e),
            }
        }
        "autoplay" => {
            let safe_gid = security::sanitize_for_log(guild_id);
            match state
                .player_manager
                .toggle_autoplay(guild_id, session_id)
                .await
            {
                Ok(_) => {
                    if !guild_id.is_empty() {
                        state.session_manager.add_guild(session_id, guild_id);
                    }
                }
                Err(e) => warn!("WS autoplay failed for guild {}: {}", safe_gid, e),
            }
        }
        "loop" => {
            let mode = msg.get("mode").and_then(|m| m.as_str()).unwrap_or("none");
            let loop_mode = match mode {
                "track" => crate::player::queue::LoopMode::Track,
                "queue" => crate::player::queue::LoopMode::Queue,
                _ => crate::player::queue::LoopMode::None,
            };
            let safe_gid = security::sanitize_for_log(guild_id);
            match state
                .player_manager
                .set_loop_mode(guild_id, loop_mode, session_id)
                .await
            {
                Ok(_) => {
                    if !guild_id.is_empty() {
                        state.session_manager.add_guild(session_id, guild_id);
                    }
                }
                Err(e) => warn!("WS loop failed for guild {}: {}", safe_gid, e),
            }
        }
        "shuffleQueue" => {
            let safe_gid = security::sanitize_for_log(guild_id);
            match state
                .player_manager
                .shuffle_queue(guild_id, session_id)
                .await
            {
                Ok(_) => {
                    if !guild_id.is_empty() {
                        state.session_manager.add_guild(session_id, guild_id);
                    }
                }
                Err(e) => warn!("WS shuffleQueue failed for guild {}: {}", safe_gid, e),
            }
        }
        "clearQueue" => {
            let safe_gid = security::sanitize_for_log(guild_id);
            match state.player_manager.clear_queue(guild_id, session_id).await {
                Ok(_) => {
                    if !guild_id.is_empty() {
                        state.session_manager.add_guild(session_id, guild_id);
                    }
                }
                Err(e) => warn!("WS clearQueue failed for guild {}: {}", safe_gid, e),
            }
        }
        "destroyPlayer" => {
            state.session_manager.remove_guild(session_id, guild_id);
            match state
                .player_manager
                .destroy_player_for_session(guild_id, session_id)
            {
                Ok(()) => {}
                Err(e) => {
                    let safe_gid = security::sanitize_for_log(guild_id);
                    warn!("WS destroyPlayer failed for guild {}: {}", safe_gid, e);
                }
            }
        }
        "ping" => {
            let pong = serde_json::json!({ "op": "pong" });
            let _ = direct_tx.send(Message::Text(pong.to_string())).await;
        }
        _ => {
            warn!("Unknown WebSocket op: {}", security::sanitize_for_log(op));
        }
    }
}
