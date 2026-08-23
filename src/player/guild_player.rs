use crate::models::{
    filters::Filters,
    protocol::{PlayerResponse, PlayerState, VoiceStateUpdate},
    track::LavalinkTrack,
};
use crate::player::autoplay::AutoplayEngine;
use crate::player::queue::{LoopMode, TrackQueue};
use crate::util;
use songbird::driver::Driver;
use songbird::events::{context_data, CoreEvent, EventContext, EventHandler};
use songbird::id::{GuildId, UserId};
use songbird::input::HttpRequest;
use songbird::ConnectionInfo;
use songbird::tracks::{Track, TrackHandle};
use std::num::NonZeroU64;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{broadcast, Mutex};
use tracing::{error, info, warn};

use crate::dsp::pipeline::{self, SharedChain};

const SAMPLE_RATE: f64 = 48000.0;

// ---------------------------------------------------------------------------
// Voice disconnect handling
//
// The event payload construction is a pure function so it can be unit-tested
// without any Discord connection: tests feed each DisconnectReason variant
// directly and assert the exact WebSocketClosedEvent JSON emitted to clients.
//
// Lavalink v4 shape:
//   { op: "event", type: "WebSocketClosedEvent", guildId, code, reason, byRemote }
// ---------------------------------------------------------------------------

/// Human-readable reason strings for Discord voice close codes.
fn describe_close_code(code: u16) -> &'static str {
    match code {
        4001 => "Unknown opcode",
        4002 => "Invalid payload",
        4003 => "Not authenticated",
        4004 => "Authentication failed",
        4005 => "Already authenticated",
        4006 => "Session is no longer valid",
        4009 => "Session timed out",
        4011 => "Server not found",
        4012 => "Unknown protocol",
        4014 => "Disconnected (channel closed/removed/kicked)",
        4015 => "Voice server crashed",
        4016 => "Unknown encryption mode",
        _ => "Voice WebSocket closed",
    }
}

/// Map a Songbird disconnect reason to (close_code, reason, by_remote).
///
/// `byRemote == true` mirrors Lavalink semantics: the voice gateway dropped us
/// (client should usually re-request the voice session via the main gateway).
fn classify_disconnect(
    reason: &Option<context_data::DisconnectReason>,
) -> (u16, String, bool) {
    use context_data::DisconnectReason as DR;

    match reason {
        None => (1000, "Voice channel left or changed".to_string(), false),
        Some(DR::Requested) => (1000, "Requested".to_string(), false),
        Some(DR::AttemptDiscarded) => (
            1000,
            "Connection attempt discarded".to_string(),
            false,
        ),
        Some(DR::WsClosed(Some(close_code))) => {
            let code = *close_code as u16;
            (code, describe_close_code(code).to_string(), true)
        }
        Some(DR::WsClosed(None)) => (
            1006,
            "Voice WebSocket closed unexpectedly".to_string(),
            true,
        ),
        Some(DR::TimedOut) => (1006, "Connection timed out".to_string(), true),
        Some(DR::Io) => (1006, "I/O error".to_string(), true),
        Some(DR::ProtocolViolation) => (1006, "Protocol violation".to_string(), true),
        Some(DR::Internal) => (1011, "Internal driver error".to_string(), false),
        // #[non_exhaustive] upstream — future-proof catch-all
        Some(_) => (1006, "Voice connection lost".to_string(), true),
    }
}

/// Build the exact WebSocketClosedEvent JSON payload for a disconnect.
pub fn ws_closed_event_json(
    guild_id: &str,
    reason: &Option<context_data::DisconnectReason>,
) -> serde_json::Value {
    let (code, reason_str, by_remote) = classify_disconnect(reason);
    serde_json::json!({
        "op": "event",
        "type": "WebSocketClosedEvent",
        "guildId": guild_id,
        "code": code,
        "reason": reason_str,
        "byRemote": by_remote,
    })
}

#[derive(Clone)]
struct DisconnectHandler {
    guild_id: String,
    event_tx: broadcast::Sender<String>,
}

impl DisconnectHandler {
    /// Produce and broadcast the WebSocketClosedEvent for a disconnect.
    /// Split out from `act` so the broadcast side is testable too.
    fn handle_disconnect(&self, reason: &Option<context_data::DisconnectReason>) -> String {
        let payload = ws_closed_event_json(&self.guild_id, reason);
        let json = payload.to_string();
        let _ = self.event_tx.send(json.clone());
        json
    }
}

#[async_trait::async_trait]
impl EventHandler for DisconnectHandler {
    async fn act(&self, ctx: &EventContext<'_>) -> Option<songbird::events::Event> {
        if let EventContext::DriverDisconnect(data) = ctx {
            let json = self.handle_disconnect(&data.reason);
            info!(
                "Voice disconnected for guild {}: {}",
                self.guild_id, json
            );
        }
        None
    }
}

// ---------------------------------------------------------------------------
// Player
// ---------------------------------------------------------------------------

pub struct GuildPlayer {
    pub guild_id: String,
    pub user_id: String,
    pub volume: u32,
    pub paused: bool,
    pub voice: Option<VoiceStateUpdate>,
    pub filters: Filters,
    pub last_update: u64,
    pub driver: Arc<Mutex<Driver>>,
    track_handle: Option<TrackHandle>,
    pub is_playing: bool,
    pub queue: TrackQueue,
    pub autoplay: AutoplayEngine,
    pub end_time: Option<u64>,
    pub play_started_at: Option<Instant>,
    pub paused_at: Option<Instant>,
    pub paused_position: u64,
    pub event_tx: broadcast::Sender<String>,

    /// DSP filter chain shared with the live audio pipeline (if active).
    pub shared_chain: SharedChain,
    /// True while the current track plays through the filtered pipeline.
    filtered_active: bool,
    /// Stream URL backing the current track, needed for hot-restarts.
    current_stream_url: Option<String>,
}

impl GuildPlayer {
    pub fn new(
        guild_id: String,
        user_id: String,
        event_tx: broadcast::Sender<String>,
    ) -> Self {
        let driver = Driver::new(Default::default());
        Self {
            guild_id,
            user_id,
            volume: 100,
            paused: false,
            voice: None,
            filters: Filters::default(),
            last_update: util::current_timestamp(),
            driver: Arc::new(Mutex::new(driver)),
            track_handle: None,
            is_playing: false,
            queue: TrackQueue::new(),
            autoplay: AutoplayEngine::new(),
            end_time: None,
            play_started_at: None,
            paused_at: None,
            paused_position: 0,
            event_tx,

            shared_chain: pipeline::new_shared_chain(SAMPLE_RATE),
            filtered_active: false,
            current_stream_url: None,
        }
    }

    fn emit_event(&self, event_type: &str, extra: serde_json::Value) {
        let mut event = serde_json::json!({
            "op": "event",
            "type": event_type,
            "guildId": self.guild_id,
        });
        if let Some(obj) = event.as_object_mut() {
            if let Some(extra_obj) = extra.as_object() {
                for (k, v) in extra_obj {
                    obj.insert(k.clone(), v.clone());
                }
            }
        }
        let _ = self.event_tx.send(event.to_string());
    }

    pub fn emit_track_load_failed(&self, track: &LavalinkTrack, exception: &str) {
        self.emit_event("TrackExceptionEvent", serde_json::json!({
            "track": track,
            "exception": {
                "message": exception,
                "severity": "COMMON",
                "cause": "",
                "causeStackTrace": ""
            },
        }));
    }

    fn emit_player_update(&self) {
        let resp = self.to_response();
        if let Ok(json) = serde_json::to_value(&resp) {
            let msg = serde_json::json!({
                "op": "playerUpdate",
                "guildId": self.guild_id,
                "state": json.get("state"),
            });
            let _ = self.event_tx.send(msg.to_string());
        }
    }

    pub async fn set_voice(&mut self, voice: VoiceStateUpdate) -> bool {
        let endpoint_raw = voice.endpoint.clone();
        let endpoint = endpoint_raw
            .trim_start_matches("wss://")
            .trim_start_matches("ws://")
            .split(':')
            .next()
            .unwrap_or(&endpoint_raw)
            .to_string();

        let guild_num = self.guild_id.parse::<u64>().unwrap_or(0);
        let user_num = self.user_id.parse::<u64>().unwrap_or(0);

        let guild_nz = NonZeroU64::new(guild_num).unwrap_or(NonZeroU64::new(1).unwrap());
        let user_nz = NonZeroU64::new(user_num).unwrap_or(NonZeroU64::new(1).unwrap());

        let channel_id_val = voice.channel_id.parse::<u64>().ok();

        let info = ConnectionInfo {
            endpoint,
            guild_id: GuildId::from(guild_nz),
            channel_id: channel_id_val.map(|id| {
                let nz = NonZeroU64::new(id).unwrap_or(NonZeroU64::new(1).unwrap());
                songbird::id::ChannelId::from(nz)
            }),
            session_id: voice.session_id.clone(),
            token: voice.token.clone(),
            user_id: UserId::from(user_nz),
        };

        let mut driver_lock = self.driver.lock().await;
        if let Err(e) = driver_lock.connect(info).await {
            error!("Voice connection failed for guild {}: {:?}", self.guild_id, e);
            self.voice = Some(voice);
            self.last_update = util::current_timestamp();
            return false;
        }
        drop(driver_lock);
        info!("Voice connected for guild: {}", self.guild_id);

        let disconnect_handler = DisconnectHandler {
            guild_id: self.guild_id.clone(),
            event_tx: self.event_tx.clone(),
        };
        {
            let mut driver_lock = self.driver.lock().await;
            driver_lock.add_global_event(
                CoreEvent::DriverDisconnect.into(),
                disconnect_handler,
            );
        }

        self.voice = Some(voice);
        self.last_update = util::current_timestamp();
        self.emit_player_update();
        true
    }

    /// Guess a file extension hint for symphonia probing from the URL.
    fn extension_hint(url: &str) -> Option<String> {
        let path = url.split(['?', '#']).next()?;
        let name = path.rsplit('/').next()?;
        let dot = name.rfind('.')?;
        let ext = &name[dot + 1..];
        let ext = ext.to_ascii_lowercase();
        match ext.as_str() {
            "mp3" | "m4a" | "mp4" | "aac" | "ogg" | "opus" | "webm" | "flac" | "wav" => {
                Some(ext)
            }
            _ => None,
        }
    }

    /// Build the playback Input for `stream_url`, routing through the DSP
    /// pipeline when the filter chain is active. Returns (input, filtered).
    async fn build_input(
        &self,
        stream_url: &str,
        start_offset_ms: u64,
    ) -> (songbird::input::Input, bool) {
        let chain_active = self.shared_chain.lock().unwrap().is_active();

        if chain_active {
            match pipeline::create_filtered_input(
                reqwest::Client::new(),
                stream_url.to_string(),
                Self::extension_hint(stream_url),
                self.shared_chain.clone(),
                start_offset_ms * (SAMPLE_RATE as u64 / 1000),
            )
            .await
            {
                Ok(input) => return (input, true),
                Err(e) => {
                    warn!(
                        "Filtered pipeline setup failed ({}), falling back to direct input",
                        e
                    );
                }
            }
        }

        (
            HttpRequest::new(reqwest::Client::new(), stream_url.to_string()).into(),
            false,
        )
    }

    /// Stop the current handle without emitting TrackEndEvent (internal swaps).
    fn stop_handle_silently(&mut self) {
        if let Some(handle) = &self.track_handle {
            let _ = handle.stop();
        }
        self.track_handle = None;
    }

    /// (Re)start the current track at `position_ms`, choosing the right
    /// pipeline based on current filter state. Used by filter updates and
    /// seeks on non-seekable filtered streams. Emits no track lifecycle events.
    async fn restart_at(&mut self, position_ms: u64) {
        let Some(url) = self.current_stream_url.clone() else {
            return;
        };
        let Some(_track) = self.queue.current.clone() else {
            return;
        };

        self.stop_handle_silently();

        let was_paused = self.paused;
        let (input, filtered) = self.build_input(&url, position_ms).await;

        let mut driver_lock = self.driver.lock().await;
        let handle = driver_lock.play(Track::new(input));
        drop(driver_lock);

        if let Err(e) = handle.set_volume(self.volume as f32 / 100.0) {
            warn!("Failed to set volume during restart: {:?}", e);
        }

        // Align reported position: source_pos = wall_elapsed * duration_factor
        let factor = self.shared_chain.lock().unwrap().duration_factor().max(1e-6);
        let wall_offset_ms = (position_ms as f64 / factor) as u64;

        if was_paused {
            let _ = handle.pause();
            self.paused_at = Some(Instant::now());
            self.paused_position = position_ms;
            self.play_started_at = None;
        } else {
            self.play_started_at =
                Some(Instant::now() - Duration::from_millis(wall_offset_ms));
            self.paused_at = None;
            self.paused_position = 0;
        }

        self.filtered_active = filtered;
        self.track_handle = Some(handle);

        info!(
            "Restarted playback at ~{} ms for guild {} (filtered={})",
            position_ms, self.guild_id, filtered
        );
    }

    pub async fn play_track(&mut self, track: LavalinkTrack, stream_url: String) -> bool {
        if let Some(old_handle) = &self.track_handle {
            let _ = old_handle.stop();
            if let Some(old_track) = self.queue.current.take() {
                self.emit_event("TrackEndEvent", serde_json::json!({
                    "track": old_track,
                    "reason": "replaced",
                }));
            }
        }

        let (input, filtered) = self.build_input(&stream_url, 0).await;

        let mut driver_lock = self.driver.lock().await;
        let handle = driver_lock.play(Track::new(input));
        drop(driver_lock);

        if let Err(e) = handle.set_volume(self.volume as f32 / 100.0) {
            warn!("Failed to set volume on handle: {:?}", e);
        }

        self.track_handle = Some(handle);
        self.filtered_active = filtered;
        self.current_stream_url = Some(stream_url);
        self.queue.current = Some(track.clone());
        self.paused = false;
        self.is_playing = true;
        self.play_started_at = Some(Instant::now());
        self.paused_at = None;
        self.paused_position = 0;
        self.last_update = util::current_timestamp();

        self.autoplay.record_track(&track);
        self.emit_event("TrackStartEvent", serde_json::json!({
            "track": track,
        }));
        self.emit_player_update();
        info!(
            "Started playback for guild: {} (filtered={})",
            self.guild_id, filtered
        );
        true
    }

    pub fn stop(&mut self) -> Option<LavalinkTrack> {
        self.stop_handle_silently();
        let old_track = self.queue.current.take();
        self.is_playing = false;
        self.filtered_active = false;
        self.current_stream_url = None;
        self.play_started_at = None;
        self.paused_at = None;
        self.paused_position = 0;
        self.end_time = None;
        self.last_update = util::current_timestamp();

        if let Some(track) = &old_track {
            self.emit_event("TrackEndEvent", serde_json::json!({
                "track": track,
                "reason": "stopped",
            }));
        }
        self.emit_player_update();
        old_track
    }

    pub fn set_paused(&mut self, paused: bool) {
        if let Some(handle) = &self.track_handle {
            if paused {
                let _ = handle.pause();
                self.paused_at = Some(Instant::now());
            } else {
                let _ = handle.play();
                self.paused_at = None;
                self.play_started_at = Some(
                    Instant::now() - Duration::from_millis(self.paused_position),
                );
            }
        }
        self.paused = paused;
        self.last_update = util::current_timestamp();
        self.emit_player_update();
    }

    pub fn set_volume(&mut self, volume: u32) {
        self.volume = volume.clamp(0, 1000);
        if let Some(handle) = &self.track_handle {
            let _ = handle.set_volume(self.volume as f32 / 100.0);
        }
        self.last_update = util::current_timestamp();
        self.emit_player_update();
    }

    /// Push current `self.filters` into the shared DSP chain.
    ///
    /// Non-structural changes (EQ gains, tremolo depth, volume, ...) take
    /// effect immediately on samples already flowing through the pipeline.
    /// Structural changes (Timescale enabled/disabled/re-parameterised)
    /// trigger a hot-restart of the current track at its present position,
    /// mirroring Lavalink's behaviour of rebuilding the filter graph.
    pub async fn apply_filters(&mut self) {
        let structural = {
            let mut chain = self.shared_chain.lock().unwrap();
            chain.update_from_lavalink(&self.filters)
        };

        // Mixer-level volume stays bound to the player volume; the DSP chain
        // applies its own volume filter independently.
        if let Some(handle) = &self.track_handle {
            let _ = handle.set_volume(self.volume as f32 / 100.0);
        }

        if structural && self.is_playing && self.queue.current.is_some() {
            let pos = self.get_position();
            info!("Structural filter change; restarting at {} ms", pos);
            self.restart_at(pos).await;
        } else if !structural && !self.filtered_active {
            // Filters became active while a plain (unfiltered) track runs;
            // switch pipelines to pick them up.
            if self.shared_chain.lock().unwrap().is_active()
                && self.is_playing
                && self.queue.current.is_some()
            {
                let pos = self.get_position();
                self.restart_at(pos).await;
            }
        }

        self.last_update = util::current_timestamp();
        self.emit_player_update();
    }

    pub async fn seek(&mut self, position: u64) {
        let chain_active = self.shared_chain.lock().unwrap().is_active();

        if chain_active || self.filtered_active {
            // Filtered streams are not seekable byte-wise; hot-restart instead.
            if self.is_playing {
                self.restart_at(position).await;
            }
        } else if let Some(handle) = &self.track_handle {
            let _ = handle.seek(Duration::from_millis(position));
            self.play_started_at =
                Some(Instant::now() - Duration::from_millis(position));
        }
        self.paused_position = position;
        self.last_update = util::current_timestamp();
        self.emit_player_update();
    }

    pub fn get_position(&self) -> u64 {
        if self.paused {
            return self.paused_position;
        }
        match self.play_started_at {
            Some(started) => {
                let wall = started.elapsed().as_millis() as f64;
                let factor = if self.filtered_active {
                    self.shared_chain.lock().unwrap().duration_factor()
                } else {
                    1.0
                };
                (wall * factor.max(0.0)) as u64
            }
            None => 0,
        }
    }

    pub fn is_actively_playing(&self) -> bool {
        self.is_playing && !self.paused && self.queue.current.is_some()
    }

    pub fn is_playing(&self) -> bool {
        self.is_playing
    }

    pub fn set_loop(&mut self, mode: LoopMode) {
        self.queue.set_loop(mode);
        self.emit_player_update();
    }

    pub fn toggle_loop(&mut self) -> LoopMode {
        let mode = self.queue.toggle_loop();
        self.emit_player_update();
        mode
    }

    pub fn add_to_queue(&mut self, track: LavalinkTrack) {
        self.queue.add(track);
        self.emit_player_update();
    }

    pub fn add_to_queue_front(&mut self, track: LavalinkTrack) {
        self.queue.add_at(0, track);
        self.emit_player_update();
    }

    pub fn remove_from_queue(&mut self, index: usize) -> Option<LavalinkTrack> {
        let result = self.queue.remove(index);
        self.emit_player_update();
        result
    }

    pub fn clear_queue(&mut self) {
        self.queue.tracks.clear();
        self.emit_player_update();
    }

    pub fn shuffle_queue(&mut self) {
        self.queue.shuffle();
        self.emit_player_update();
    }

    pub fn skip_to_next(&mut self) -> Option<LavalinkTrack> {
        self.stop();
        self.queue.next()
    }

    pub fn skip_to_previous(&mut self) -> Option<LavalinkTrack> {
        self.stop();
        self.queue.previous_track()
    }

    pub fn get_next_track_for_autoplay(&mut self) -> Option<LavalinkTrack> {
        if let Some(end_time) = self.end_time {
            let position = self.get_position();
            if position >= end_time {
                self.end_time = None;
            }
        }

        match self.queue.loop_mode {
            LoopMode::Track => {
                self.queue.current.clone()
            }
            LoopMode::Queue => {
                if let Some(track) = self.queue.tracks.pop_front() {
                    self.queue.tracks.push_back(track.clone());
                    Some(track)
                } else {
                    self.queue.current.clone()
                }
            }
            LoopMode::None => {
                self.queue.tracks.pop_front()
            }
        }
    }

    pub fn to_response(&self) -> PlayerResponse {
        let is_connected = self.voice.is_some();
        let current = self.queue.current.clone();
        PlayerResponse {
            guild_id: self.guild_id.clone(),
            track: current,
            volume: self.volume,
            paused: self.paused,
            state: PlayerState {
                time: util::current_timestamp(),
                position: self.get_position(),
                connected: is_connected,
                ping: if is_connected { 12 } else { -1 },
            },
            voice: self.voice.clone().unwrap_or_default(),
            filters: self.filters.clone(),
        }
    }
}

// ---------------------------------------------------------------------------
// Tests: disconnect event emission without any Discord connection
// ---------------------------------------------------------------------------

#[cfg(test)]
mod disconnect_tests {
    use super::*;
    use songbird::events::context_data::DisconnectReason as DR;
    use songbird::model::CloseCode;

    #[test]
    fn ws_closed_with_session_invalid_code_4006() {
        let payload = ws_closed_event_json(
            "123456789012345678",
            &Some(DR::WsClosed(Some(CloseCode::SessionInvalid))),
        );
        assert_eq!(payload["op"], "event");
        assert_eq!(payload["type"], "WebSocketClosedEvent");
        assert_eq!(payload["guildId"], "123456789012345678");
        assert_eq!(payload["code"], 4006);
        assert_eq!(payload["reason"], "Session is no longer valid");
        assert_eq!(payload["byRemote"], true);
    }

    #[test]
    fn ws_closed_with_server_crash_4015() {
        let payload = ws_closed_event_json(
            "111",
            &Some(DR::WsClosed(Some(CloseCode::VoiceServerCrash))),
        );
        assert_eq!(payload["code"], 4015);
        assert_eq!(payload["byRemote"], true);
    }

    #[test]
    fn ws_closed_with_none_code_falls_back_to_1006() {
        let payload = ws_closed_event_json("222", &Some(DR::WsClosed(None)));
        assert_eq!(payload["code"], 1006);
        assert_eq!(payload["reason"], "Voice WebSocket closed unexpectedly");
        assert_eq!(payload["byRemote"], true);
    }

    #[test]
    fn timeout_maps_to_1006_by_remote() {
        let payload = ws_closed_event_json("333", &Some(DR::TimedOut));
        assert_eq!(payload["code"], 1006);
        assert_eq!(payload["reason"], "Connection timed out");
        assert_eq!(payload["byRemote"], true);
    }

    #[test]
    fn io_error_maps_to_1006_by_remote() {
        let payload = ws_closed_event_json("444", &Some(DR::Io));
        assert_eq!(payload["code"], 1006);
        assert_eq!(payload["reason"], "I/O error");
        assert_eq!(payload["byRemote"], true);
    }

    #[test]
    fn protocol_violation_maps_to_1006_by_remote() {
        let payload = ws_closed_event_json("555", &Some(DR::ProtocolViolation));
        assert_eq!(payload["code"], 1006);
        assert_eq!(payload["reason"], "Protocol violation");
        assert_eq!(payload["byRemote"], true);
    }

    #[test]
    fn clean_user_disconnect_is_not_by_remote() {
        let payload = ws_closed_event_json("666", &None);
        assert_eq!(payload["code"], 1000);
        assert_eq!(payload["byRemote"], false);
        assert!(payload["reason"].as_str().unwrap().contains("left"));
    }

    #[test]
    fn requested_disconnect_is_not_by_remote() {
        let payload = ws_closed_event_json("777", &Some(DR::Requested));
        assert_eq!(payload["code"], 1000);
        assert_eq!(payload["reason"], "Requested");
        assert_eq!(payload["byRemote"], false);
    }

    #[test]
    fn internal_error_maps_to_1011() {
        let payload = ws_closed_event_json("888", &Some(DR::Internal));
        assert_eq!(payload["code"], 1011);
        assert_eq!(payload["byRemote"], false);
    }

    #[tokio::test]
    async fn handler_broadcasts_exact_payload_to_subscribers() {
        let (tx, mut rx) = broadcast::channel::<String>(8);
        let handler = DisconnectHandler {
            guild_id: "42".to_string(),
            event_tx: tx,
        };

        handler.handle_disconnect(&Some(DR::WsClosed(Some(CloseCode::SessionInvalid))));

        let received = rx.recv().await.unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&received).unwrap();
        assert_eq!(parsed["type"], "WebSocketClosedEvent");
        assert_eq!(parsed["guildId"], "42");
        assert_eq!(parsed["code"], 4006);
        assert_eq!(parsed["byRemote"], true);
        // Field order/format sanity: parses as an object with exactly the
        // six Lavalink fields.
        let obj = parsed.as_object().unwrap();
        assert_eq!(obj.len(), 6);
    }

    #[test]
    fn payload_is_valid_json_with_expected_fields() {
        for reason in [
            None,
            Some(DR::Requested),
            Some(DR::AttemptDiscarded),
            Some(DR::Internal),
            Some(DR::Io),
            Some(DR::TimedOut),
            Some(DR::ProtocolViolation),
            Some(DR::WsClosed(None)),
            Some(DR::WsClosed(Some(CloseCode::UnknownOpcode))),
            Some(DR::WsClosed(Some(CloseCode::UnknownEncryptionMode))),
        ] {
            let payload = ws_closed_event_json("guild", &reason);
            let obj = payload.as_object().expect("must be object");
            assert!(obj.contains_key("op"));
            assert!(obj.contains_key("type"));
            assert!(obj.contains_key("guildId"));
            assert!(obj.contains_key("code"));
            assert!(obj.contains_key("reason"));
            assert!(obj.contains_key("byRemote"));
            assert_eq!(obj["code"].as_u64().unwrap(), obj["code"].as_u64().unwrap());
            assert!(!obj["reason"].as_str().unwrap().is_empty());
        }
    }
}
