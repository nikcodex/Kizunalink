// Copyright (c) 2026 nikcodex (KizunaLink)
// Licensed under the MIT License

//! Handles incoming WebSocket opcodes dispatched from the WS handler.

use std::sync::Arc;

use kizunalink::discord::player::VoiceConnectionState;
use kizunalink::lavalink::protocol::opcodes::IncomingMessage;

use crate::server::{AppState, Session};

pub async fn handle_op(
    op: IncomingMessage,
    state: &Arc<AppState>,
    session_id: &kizunalink::common::types::SessionId,
) -> Result<(), String> {
    let session: Arc<Session> = state
        .sessions
        .get(session_id)
        .map(|s| s.clone())
        .ok_or_else(|| "Session not found".to_string())?;

    match op {
        IncomingMessage::VoiceUpdate {
            guild_id,
            session_id: voice_session_id,
            channel_id,
            event,
        } => {
            handle_voice_update(
                &session,
                state,
                guild_id,
                voice_session_id,
                channel_id,
                event,
            )
            .await
        }
        IncomingMessage::Play { guild_id, track } => {
            handle_play(&session, state, guild_id, track).await
        }
        IncomingMessage::Stop { guild_id } => {
            if let Some(player_arc) = session.players.get(&guild_id).map(|kv| kv.value().clone()) {
                let mut player = player_arc.write().await;
                player.stop_track();
            }
            Ok(())
        }
        IncomingMessage::Destroy { guild_id } => {
            session.destroy_player(&guild_id).await;
            Ok(())
        }
        IncomingMessage::ConfigureResuming { key, timeout } => {
            // `timeout` of 0 disables resuming. Lavalink's optional `key` is not
            // supported by this node (sessions resume by id); warn so operators
            // notice clients that rely on it.
            if key.is_some() {
                tracing::warn!(
                    "configureResuming: custom resume keys are not supported; \
                     sessions resume by session id. Ignoring key for session={session_id}."
                );
            }
            session
                .resumable
                .store(timeout > 0, std::sync::atomic::Ordering::Relaxed);
            session
                .resume_timeout
                .store(timeout, std::sync::atomic::Ordering::Relaxed);
            tracing::info!(
                "configureResuming: resuming={}, timeout={}s for session={session_id}",
                timeout > 0,
                timeout
            );
            Ok(())
        }
    }
}

/// Applies a voice-state update and starts a gateway task when the state changed
/// or no task is running.
///
/// Returns an error when the event has no string `token` or `endpoint`. If the
/// session has no user ID, its voice state is not updated and no task is started.
async fn handle_voice_update(
    session: &Arc<Session>,
    state: &Arc<AppState>,
    guild_id: kizunalink::common::types::GuildId,
    voice_session_id: String,
    channel_id: Option<String>,
    event: serde_json::Value,
) -> Result<(), String> {
    // A null channel is Discord's "leave voice" signal. Tear the gateway down
    // explicitly instead of failing to parse a now-absent token/endpoint.
    if channel_id.is_none() {
        if let Some(player_arc) = session.players.get(&guild_id).map(|kv| kv.value().clone()) {
            let mut player = player_arc.write().await;
            if let Some(task) = player.gateway_task.take() {
                task.abort();
                let _ = task.await;
            }
            player
                .voice_ready
                .store(false, std::sync::atomic::Ordering::Release);
            player.voice = Default::default();
        }
        tracing::info!(
            "[{}] Voice disconnect (channel_id is null); gateway stopped",
            guild_id
        );
        return Ok(());
    }

    let token = event
        .get("token")
        .and_then(|v| v.as_str())
        .ok_or("Missing token in voice update event")?
        .to_string();
    let endpoint = event
        .get("endpoint")
        .and_then(|v| v.as_str())
        .ok_or("Missing endpoint in voice update event")?
        .to_string();

    let Some(uid) = session.user_id else {
        // Without a user id DAVE key exchange and the voice session cannot be built, so
        // the update would otherwise vanish in silence while the node still reported
        // itself as healthy.
        tracing::warn!(
            "[{}] Dropping voice update: session has no user id (missing or unparseable User-Id header)",
            guild_id
        );
        return Ok(());
    };

    let player_arc = session.get_or_create_player(guild_id.clone(), state.clone());

    // Update the voice state and decide whether a gateway task must be (re)spawned
    // under a single write lock, so the decision cannot race with a concurrent
    // voice update between the check and the spawn.
    let spawn = {
        let mut player = player_arc.write().await;
        let changed = player.voice.token != token
            || player.voice.endpoint != endpoint
            || player.voice.session_id != voice_session_id
            || player.voice.channel_id != channel_id;

        if changed {
            player.voice = VoiceConnectionState {
                token,
                endpoint,
                session_id: voice_session_id,
                channel_id,
            };
        }

        if changed || player.gateway_task.is_none() {
            if let Some(task) = player.gateway_task.take() {
                task.abort();
                let _ = task.await;
            }

            Some((
                player.engine.clone(),
                player.guild_id.clone(),
                player.voice.clone(),
                player.filter_chain.clone(),
                player.ping.clone(),
                player.voice_ready.clone(),
                player.frames_sent.clone(),
                player.frames_nulled.clone(),
            ))
        } else {
            None
        }
    };

    if let Some((
        engine,
        guild,
        voice_state,
        filter_chain,
        ping,
        voice_ready,
        frames_sent,
        frames_nulled,
    )) = spawn
    {
        let session_clone = session.clone();
        // Bounded so a stalled WebSocket consumer cannot grow this queue without bound.
        let (event_tx, mut event_rx) = tokio::sync::mpsc::channel(256);
        tokio::spawn(async move {
            while let Some(event) = event_rx.recv().await {
                let msg = kizunalink::lavalink::protocol::OutgoingMessage::Event {
                    event: Box::new(event),
                };
                session_clone.send_message(&msg);
            }
        });

        let new_task = crate::server::connect_voice(crate::server::voice::VoiceConnectConfig {
            engine,
            guild_id: guild,
            user_id: uid,
            voice: voice_state,
            filter_chain,
            ping,
            voice_ready,
            event_tx: Some(event_tx),
            frames_sent,
            frames_nulled,
        })
        .await;

        // Re-acquire the lock to install. If a concurrent voice update spawned a
        // newer task while we were connecting, keep theirs and drop ours — the
        // later update is authoritative.
        let mut player = player_arc.write().await;
        if player.gateway_task.is_some() {
            new_task.abort();
        } else {
            session.register_task(new_task.abort_handle());
            player.gateway_task = Some(new_task);
        }
    } else {
        // Identical voice update arriving while a gateway task is already running:
        // there is nothing to (re)start, so this is a normal no-op, not a drop.
        tracing::debug!(
            "[{}] Voice update unchanged; existing gateway task kept",
            guild_id
        );
    }

    Ok(())
}

async fn handle_play(
    session: &Arc<Session>,
    state: &Arc<AppState>,
    guild_id: kizunalink::common::types::GuildId,
    track: String,
) -> Result<(), String> {
    let player_arc = session.get_or_create_player(guild_id, state.clone());
    let mut player = player_arc.write().await;

    // P10: track-load latency histogram (resolution + arm).
    let load_start = std::time::Instant::now();
    kizunalink::discord::player::start_playback(
        &mut player,
        kizunalink::discord::player::manager::start::PlaybackStartConfig {
            track,
            session: session.clone(),
            source_manager: state.source_manager.clone(),
            lyrics_manager: state.lyrics_manager.clone(),
            routeplanner: state.routeplanner.clone(),
            update_interval: state.config.server.player_update_interval,
            user_data: None,
            end_time: None,
            start_time_ms: None,
        },
    )
    .await;
    crate::monitoring::prometheus::observe_track_load(load_start.elapsed().as_secs_f64());

    Ok(())
}
