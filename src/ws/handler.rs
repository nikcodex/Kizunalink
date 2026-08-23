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
use crate::util::constant_time_eq;
use crate::AppState;

static SESSION_STORE: LazyLock<DashMap<String, std::time::Instant>> =
    LazyLock::new(DashMap::new);

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
        "WebSocket connected: client={} user={}",
        client_name, user_id
    );

    ws.on_upgrade(move |socket| handle_socket(socket, state, session_id))
}

async fn handle_socket(
    mut socket: WebSocket,
    state: AppState,
    resume_session_id: Option<String>,
) {
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

    let stats_state = state.clone();
    let stats_task = tokio::spawn(async move {
        let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(60));
        loop {
            interval.tick().await;
            let (total_players, playing_players) =
                stats_state.player_manager.count_players().await;
            let uptime = stats_state.start_time.elapsed().as_millis() as u64;

            let now = std::time::Instant::now();
            SESSION_STORE.retain(|_, last_active| {
                now.duration_since(*last_active).as_secs() < 3600
            });

            let stats = serde_json::json!({
                "op": "stats",
                "players": total_players,
                "playingPlayers": playing_players,
                "uptime": uptime,
                "memory": {
                    "free": 1024 * 1024 * 512u64,
                    "used": 1024 * 1024 * 18u64,
                    "allocated": 1024 * 1024 * 32u64,
                    "reservable": 1024 * 1024 * 512u64
                },
                "cpu": {
                    "cores": num_cpus::get(),
                    "systemLoad": 0.05,
                    "lavalinkLoad": 0.01
                },
                "frameStats": null
            });

            let _ = stats_state.event_tx.send(stats.to_string());
        }
    });

    let update_state = state.clone();
    let update_task = tokio::spawn(async move {
        let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(5));
        loop {
            interval.tick().await;
            for player_response in update_state.player_manager.get_all_players().await {
                if player_response.track.is_some() || player_response.state.connected {
                    let msg = serde_json::json!({
                        "op": "playerUpdate",
                        "guildId": player_response.guild_id,
                        "state": player_response.state,
                    });
                    let _ = update_state.event_tx.send(msg.to_string());
                }
            }
        }
    });

    while let Some(msg) = receiver.next().await {
        match msg {
            Ok(Message::Text(text)) => {
                handle_ws_message(&state, &text).await;
            }
            Ok(Message::Close(_)) => {
                info!("WebSocket client disconnected");
                break;
            }
            Err(e) => {
                error!("WebSocket error: {:?}", e);
                break;
            }
            _ => {}
        }
    }

    event_task.abort();
    stats_task.abort();
    update_task.abort();
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
                .await;
        }
        "queueTrack" => {
            if let Some(encoded) = msg.get("encoded").and_then(|e| e.as_str()) {
                let _ = state.player_manager.queue_track(guild_id, encoded).await;
            }
        }
        "skipTrack" => {
            let _ = state.player_manager.skip_track(guild_id).await;
        }
        "previousTrack" => {
            let _ = state.player_manager.previous_track(guild_id).await;
        }
        "autoplay" => {
            let _ = state.player_manager.toggle_autoplay(guild_id).await;
        }
        "loop" => {
            let mode = msg
                .get("mode")
                .and_then(|m| m.as_str())
                .unwrap_or("none");
            let loop_mode = match mode {
                "track" => crate::player::queue::LoopMode::Track,
                "queue" => crate::player::queue::LoopMode::Queue,
                _ => crate::player::queue::LoopMode::None,
            };
            let _ = state
                .player_manager
                .set_loop_mode(guild_id, loop_mode)
                .await;
        }
        "shuffleQueue" => {
            let _ = state.player_manager.shuffle_queue(guild_id).await;
        }
        "clearQueue" => {
            let _ = state.player_manager.clear_queue(guild_id).await;
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
