// Copyright (c) 2026 nikcodex (KizunaLink)
// Licensed under the MIT License

use std::sync::{Arc, atomic::Ordering};

use tokio::time::{Duration, timeout};
use tracing::{error, info};

use super::{
    super::context::PlayerContext,
    error::send_load_failed,
    monitor::{MonitorCtx, monitor_loop},
};
use crate::discord::player::manager::lyrics::spawn_lyrics_fetch;
use crate::{
    engine::playback::{PlaybackState, TrackHandle},
    lavalink::protocol::{
        self,
        events::{KizunaLinkEvent, TrackEndReason},
    },
};

pub struct PlaybackStartConfig {
    pub track: String,
    pub session: Arc<dyn crate::common::server_hooks::SessionContext>,
    pub source_manager: Arc<crate::media::sources::SourceManager>,
    pub lyrics_manager: Arc<crate::media::lyrics::LyricsManager>,
    pub routeplanner: Option<Arc<dyn crate::lavalink::routeplanner::RoutePlanner>>,
    /// Cadence at which player-state events are pushed to the client (from
    /// `server.player_update_interval`, N02).
    pub update_interval: Duration,
    pub user_data: Option<serde_json::Value>,
    pub end_time: Option<u64>,
    pub start_time_ms: Option<u64>,
}

/// Start playing a new track on `player`.
pub async fn start_playback(player: &mut PlayerContext, config: PlaybackStartConfig) {
    stop_current_track(player, config.session.as_ref()).await;

    // A client must send an encoded track string produced by `/v4/loadtracks`. If
    // it does not decode there is no metadata to resolve and nothing to play, so
    // fail loudly with `TrackException` + `TrackEnd: LoadFailed`. Previously the
    // node substituted placeholder "Unknown" metadata, ran that through the
    // mirror-search filler (which can match an arbitrary, unrelated track), then
    // silently returned once it failed to build the track response — leaving the
    // client with no event and a half-started player.
    let Some(decoded) = crate::lavalink::protocol::tracks::Track::decode(&config.track) else {
        // Build a minimal stub so the failure event still carries the offending
        // encoded string. No metadata is resolved — the old code substituted
        // placeholder "Unknown" metadata and ran it through the mirror-search
        // filler, matching an arbitrary unrelated track.
        player.track_info = Some(crate::lavalink::protocol::tracks::Track {
            encoded: config.track.clone(),
            info: crate::lavalink::protocol::tracks::TrackInfo::default(),
            plugin_info: serde_json::json!({}),
            user_data: serde_json::json!({}),
        });
        player.track = Some(config.track.clone());
        player.position = 0;
        error!(
            "Rejecting play: encoded track could not be decoded (len={})",
            config.track.len()
        );
        send_load_failed(
            player,
            config.session.as_ref(),
            "Invalid or corrupted encoded track".to_string(),
        )
        .await;
        return;
    };
    let track_info = decoded.info.clone();
    player.track_info = Some(decoded);
    player.track = Some(config.track.clone());
    player.position = 0;
    player.end_time = config.end_time;
    player.user_data = config.user_data.unwrap_or_else(|| serde_json::json!({}));
    player.stop_signal = Arc::new(std::sync::atomic::AtomicBool::new(false));

    let identifier = track_info
        .uri
        .clone()
        .unwrap_or_else(|| track_info.identifier.clone());

    let playable = match timeout(
        Duration::from_secs(30),
        config
            .source_manager
            .resolve_track(&track_info, config.routeplanner),
    )
    .await
    {
        Ok(Ok(t)) => t,
        Ok(Err(e)) => {
            error!("Failed to resolve track: {} (Error: {})", identifier, e);
            send_load_failed(player, config.session.as_ref(), e).await;
            return;
        }
        Err(_) => {
            error!("Track resolution timed out (30 s): {}", identifier);
            send_load_failed(
                player,
                config.session.as_ref(),
                format!("Track resolution timed out: {identifier}"),
            )
            .await;
            return;
        }
    };

    info!(
        "Playback starting: {} (source: {})",
        identifier, track_info.source_name
    );

    let (frame_rx, cmd_tx, err_rx) = playable.start_decoding(player.config.clone());
    let (handle, audio_state, vol, pos, is_buffering) =
        TrackHandle::new(cmd_tx, player.tape_stop.clone());

    handle.set_volume(player.volume as f32 / 100.0);

    {
        let engine = player.engine.lock().await;
        let mut mixer = engine.mixer.lock().await;
        mixer.add_track(
            frame_rx,
            audio_state.clone(),
            vol,
            pos.clone(),
            is_buffering,
            player.config.clone(),
        );
    }

    player.track_handle = Some(handle.clone());

    if let Some(start_ms) = config.start_time_ms
        && start_ms > 0
    {
        handle.seek(start_ms);
    }

    if player.paused {
        handle.pause();
    }

    let Some(track_response) = player.to_player_response().await.track else {
        error!(
            "Failed to build track response for guild {}",
            player.guild_id
        );
        return;
    };

    config
        .session
        .send_message(&protocol::OutgoingMessage::Event {
            event: Box::new(KizunaLinkEvent::TrackStart {
                guild_id: player.guild_id.clone(),
                track: track_response.clone(),
            }),
        });

    spawn_lyrics_fetch(
        player.lyrics_subscribed.clone(),
        player.lyrics_data.clone(),
        track_info.clone(),
        config.lyrics_manager,
        config.session.clone(),
        player.guild_id.clone(),
    );

    if player.config.sponsorblock.enabled {
        let sb_state = player.sponsorblock.clone();
        let sb_config = player.config.sponsorblock.clone();
        let guild_id = player.guild_id.clone();
        let session = config.session.clone();
        let track_clone = track_response.clone();
        tokio::spawn(async move {
            super::sponsorblock::load_segments(
                &sb_state,
                &sb_config,
                &guild_id,
                &track_clone,
                session.as_ref(),
            )
            .await;
        });
    }

    // One mixer tick is 20 ms. Round up so sub-second intervals are not
    // truncated to a single 20 ms update, and never schedule zero ticks.
    let interval_ms = config.update_interval.as_millis().max(20);
    let update_every_n = interval_ms.div_ceil(20) as u64;

    let ctx = MonitorCtx {
        guild_id: player.guild_id.clone(),
        handle: handle.clone(),
        err_rx,
        session: config.session.clone(),
        track: track_response,
        stop_signal: player.stop_signal.clone(),
        ping: player.ping.clone(),
        voice_ready: player.voice_ready.clone(),
        stuck_threshold_ms: player.config.stuck_threshold_ms,
        update_every_n,
        lyrics_subscribed: player.lyrics_subscribed.clone(),
        lyrics_data: player.lyrics_data.clone(),
        last_lyric_index: player.last_lyric_index.clone(),
        sponsorblock: player.sponsorblock.clone(),
        sponsorblock_enabled: player.config.sponsorblock.enabled,
        end_time_ms: player.end_time,
    };

    let track_task = tokio::spawn(monitor_loop(ctx));
    config.session.register_task(track_task.abort_handle());
    player.track_task = Some(track_task);
}

/// Whether a stop cancelled an active track (and emitted `TrackEnd`) or was a
/// no-op because nothing was playing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopOutcome {
    /// A track was active; it was cancelled and a `TrackEnd` event was emitted.
    Stopped,
    /// No active track; nothing was cancelled and no event was emitted.
    Nothing,
}

/// Stops the current track while preserving unrelated mixer tracks and sound
/// effects.
///
/// Emits `TrackEnd: Replaced` for an active track, clears its player metadata,
/// and moves its frame counters into the session's historical totals.
async fn stop_current_track(
    player: &mut PlayerContext,
    session: &dyn crate::common::server_hooks::SessionContext,
) -> StopOutcome {
    stop_current_track_with_reason(player, session, TrackEndReason::Replaced).await
}

/// Like [`stop_current_track`] but lets the caller choose the `TrackEnd` reason
/// (`op: stop` uses `Stopped`, a replacing play uses `Replaced`).
async fn stop_current_track_with_reason(
    player: &mut PlayerContext,
    session: &dyn crate::common::server_hooks::SessionContext,
    reason: TrackEndReason,
) -> StopOutcome {
    let mut outcome = StopOutcome::Nothing;
    if let Some(handle) = &player.track_handle
        && handle.get_state() != PlaybackState::Stopped
        && let Some(track) = player.to_player_response().await.track
    {
        session.send_message(&protocol::OutgoingMessage::Event {
            event: Box::new(KizunaLinkEvent::TrackEnd {
                guild_id: player.guild_id.clone(),
                track,
                reason,
            }),
        });
        outcome = StopOutcome::Stopped;
    }

    player.stop_signal.store(true, Ordering::Release);

    if let Some(task) = player.track_task.take() {
        task.abort();
    }

    if let Some(handle) = player.track_handle.take() {
        handle.stop();
        // Remove only this track from the mixer — `stop_all` would also kill any
        // other mixer tracks and active sound-effect layers.
        let engine = player.engine.lock().await;
        engine.mixer.lock().await.stop_track(&handle.state_arc());
    }
    player.track = None;
    player.track_info = None;
    player.position = 0;
    player.end_time = None;

    session.total_sent_historical().fetch_add(
        player.frames_sent.swap(0, Ordering::Relaxed),
        Ordering::Relaxed,
    );
    session.total_nulled_historical().fetch_add(
        player.frames_nulled.swap(0, Ordering::Relaxed),
        Ordering::Relaxed,
    );

    outcome
}

/// Explicit `op: stop`. Emits `TrackEnd: Stopped` when a track was playing
/// (matching the REST `PATCH .../players/{guildId}` path); a redundant stop is a
/// no-op so it cannot produce a spurious event.
pub async fn stop_playback(
    player: &mut PlayerContext,
    session: &dyn crate::common::server_hooks::SessionContext,
) -> StopOutcome {
    stop_current_track_with_reason(player, session, TrackEndReason::Stopped).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex, atomic::AtomicU64};

    use crate::common::server_hooks::SessionContext;
    use crate::common::types::GuildId;
    use crate::engine::playback::TrackHandle;
    use crate::engine::processor::DecoderCommand;
    use crate::lavalink::protocol::OutgoingMessage;
    use crate::lavalink::protocol::tracks::{Track, TrackInfo};

    /// Captures every outgoing message as JSON so tests can assert on emitted
    /// events without requiring `Deserialize` on the protocol types.
    #[derive(Default)]
    struct CapturingSession {
        sent: Mutex<Vec<serde_json::Value>>,
        sent_frames: AtomicU64,
        nulled_frames: AtomicU64,
    }

    impl CapturingSession {
        fn events(&self) -> Vec<serde_json::Value> {
            self.sent
                .lock()
                .unwrap()
                .iter()
                .filter(|m| m["op"] == "event")
                .cloned()
                .collect()
        }
    }

    impl crate::common::server_hooks::ServerContext for CapturingSession {}

    impl SessionContext for CapturingSession {
        fn send_message(&self, msg: &OutgoingMessage) {
            self.sent
                .lock()
                .unwrap()
                .push(serde_json::to_value(msg).expect("serialize outgoing message"));
        }
        fn register_task(&self, _handle: tokio::task::AbortHandle) {}
        fn total_sent_historical(&self) -> &AtomicU64 {
            &self.sent_frames
        }
        fn total_nulled_historical(&self) -> &AtomicU64 {
            &self.nulled_frames
        }
        fn get_player(
            &self,
            _guild_id: &GuildId,
        ) -> Option<Arc<tokio::sync::RwLock<PlayerContext>>> {
            None
        }
    }

    fn sample_track() -> Track {
        Track::new(TrackInfo {
            identifier: "abc".to_string(),
            is_seekable: true,
            author: "Author".to_string(),
            length: 1000,
            is_stream: false,
            position: 0,
            title: "Title".to_string(),
            uri: Some("https://example.test/abc".to_string()),
            artwork_url: None,
            isrc: None,
            source_name: "test".to_string(),
        })
    }

    fn dummy_handle() -> TrackHandle {
        let (tx, rx) = flume::unbounded::<DecoderCommand>();
        let handle = TrackHandle::new(tx, Arc::new(std::sync::atomic::AtomicBool::new(false))).0;
        // `get_state()` reports `Stopped` once the decoder receiver is dropped,
        // so leak the receiver to model a live decoder for the test's lifetime.
        std::mem::forget(rx);
        handle
    }

    fn make_player(state: Arc<dyn crate::common::server_hooks::ServerContext>) -> PlayerContext {
        PlayerContext::new(
            GuildId("1".to_string()),
            &crate::config::player::PlayerConfig::default(),
            state,
        )
    }

    #[tokio::test]
    async fn stop_playback_emits_track_end_stopped_for_active_track() {
        let session = Arc::new(CapturingSession::default());
        let mut player = make_player(session.clone());
        player.track_handle = Some(dummy_handle());
        player.track_info = Some(sample_track());
        player.track = Some("encoded".to_string());

        let outcome = stop_playback(&mut player, session.as_ref()).await;

        assert_eq!(outcome, StopOutcome::Stopped);
        let events = session.events();
        assert_eq!(events.len(), 1, "exactly one TrackEnd expected: {events:?}");
        assert_eq!(events[0]["type"], "TrackEndEvent");
        assert_eq!(events[0]["reason"], "stopped");
        assert!(player.track_handle.is_none());
        assert!(player.track.is_none());
    }

    #[tokio::test]
    async fn stop_playback_is_a_noop_when_idle() {
        let session = Arc::new(CapturingSession::default());
        let mut player = make_player(session.clone());

        let outcome = stop_playback(&mut player, session.as_ref()).await;

        assert_eq!(outcome, StopOutcome::Nothing);
        assert!(
            session.sent.lock().unwrap().is_empty(),
            "idle stop must emit nothing"
        );
    }

    #[tokio::test]
    async fn stop_playback_is_a_noop_for_already_stopped_handle() {
        let session = Arc::new(CapturingSession::default());
        let mut player = make_player(session.clone());
        let handle = dummy_handle();
        handle.stop();
        player.track_handle = Some(handle);
        player.track_info = Some(sample_track());
        player.track = Some("encoded".to_string());

        let outcome = stop_playback(&mut player, session.as_ref()).await;

        assert_eq!(outcome, StopOutcome::Nothing);
        assert!(session.sent.lock().unwrap().is_empty());
    }

    #[test]
    fn malformed_encoded_track_fails_to_decode() {
        assert!(Track::decode("not-a-real-encoded-track").is_none());
    }

    #[tokio::test]
    async fn malformed_track_emits_exception_and_load_failed() {
        let session = Arc::new(CapturingSession::default());
        let mut player = make_player(session.clone());
        let config: crate::config::AppConfig =
            toml::from_str("[server]\nauthorization = \"test-suite-secret-9f2c\"\n")
                .expect("minimal config parses");
        let source_manager = Arc::new(crate::media::sources::SourceManager::new(&config));
        let lyrics_manager = Arc::new(crate::media::lyrics::LyricsManager::new(&config));

        start_playback(
            &mut player,
            PlaybackStartConfig {
                track: "not-a-real-encoded-track".to_string(),
                session: session.clone(),
                source_manager,
                lyrics_manager,
                routeplanner: None,
                update_interval: Duration::from_secs(5),
                user_data: None,
                end_time: None,
                start_time_ms: None,
            },
        )
        .await;

        // The node must fail loudly instead of silently hanging a half-started
        // player: `TrackException` followed by `TrackEnd: loadFailed`.
        let events = session.events();
        let types: Vec<&str> = events.iter().map(|e| e["type"].as_str().unwrap()).collect();
        assert_eq!(
            types,
            vec!["TrackExceptionEvent", "TrackEndEvent"],
            "{events:?}"
        );
        assert_eq!(events[1]["reason"], "loadFailed");
        // No metadata resolution, so no bogus track start.
        assert!(player.track_handle.is_none());
        assert!(player.track.is_none());
    }
}
