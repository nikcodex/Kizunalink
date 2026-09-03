use crate::models::{
    filters::Filters,
    protocol::{PlayerResponse, PlayerState, VoiceStateUpdate},
    track::LavalinkTrack,
};
use crate::player::autoplay::AutoplayEngine;
use crate::player::queue::{LoopMode, TrackQueue};
use crate::util;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::{broadcast, mpsc, Mutex};
use tracing::{info, warn};

use crate::dsp::pipeline::{self, SharedChain};

const SAMPLE_RATE: f64 = 48000.0;

// ---------------------------------------------------------------------------
// Voice disconnect handling
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Track End Notifier
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Guild Player
// ---------------------------------------------------------------------------

pub struct GuildPlayer {
    pub guild_id: String,
    pub user_id: String,
    pub volume: u32,
    pub paused: bool,
    pub voice: Option<VoiceStateUpdate>,
    pub filters: Filters,
    pub last_update: u64,
    pub kizuna_voice_adapter: Option<Arc<Mutex<crate::player::kizuna_adapter::KizunaVoiceAdapter>>>,
    pub kizuna_track_handle: Option<kizuna_voice::audio::KizunaTrackHandle>,
    pub is_playing: bool,
    pub queue: TrackQueue,
    pub autoplay: AutoplayEngine,
    pub end_time: Option<u64>,
    pub play_started_at: Option<Instant>,
    pub paused_at: Option<Instant>,
    pub paused_position: u64,
    pub event_tx: broadcast::Sender<String>,
    pub track_end_tx: mpsc::UnboundedSender<String>,

    pub shared_chain: SharedChain,
    filtered_active: bool,
    current_stream_url: Option<String>,
}

impl GuildPlayer {
    pub fn new(
        guild_id: String,
        user_id: String,
        event_tx: broadcast::Sender<String>,
        track_end_tx: mpsc::UnboundedSender<String>,
    ) -> Self {
        Self {
            guild_id,
            user_id,
            volume: 100,
            paused: false,
            voice: None,
            filters: Filters::default(),
            last_update: util::current_timestamp(),
            kizuna_voice_adapter: None,
            kizuna_track_handle: None,
            is_playing: false,
            queue: TrackQueue::new(),
            autoplay: AutoplayEngine::new(),
            end_time: None,
            play_started_at: None,
            paused_at: None,
            paused_position: 0,
            event_tx,
            track_end_tx,

            shared_chain: pipeline::new_shared_chain(SAMPLE_RATE),
            filtered_active: false,
            current_stream_url: None,
        }
    }

    pub fn emit_event(&self, event_type: &str, extra: serde_json::Value) {
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
        self.emit_event(
            "TrackExceptionEvent",
            serde_json::json!({
                "track": track,
                "exception": {
                    "message": exception,
                    "severity": "fault",
                    "cause": "",
                    "causeStackTrace": ""
                },
            }),
        );
    }

    pub fn emit_player_update(&self) {
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

    pub async fn set_voice(&mut self, new_voice: VoiceStateUpdate) -> bool {
        // Merge with existing voice update if partially received
        let merged = match &self.voice {
            Some(existing) => VoiceStateUpdate {
                token: if !new_voice.token.is_empty() {
                    new_voice.token
                } else {
                    existing.token.clone()
                },
                endpoint: if !new_voice.endpoint.is_empty() {
                    new_voice.endpoint
                } else {
                    existing.endpoint.clone()
                },
                session_id: if !new_voice.session_id.is_empty() {
                    new_voice.session_id
                } else {
                    existing.session_id.clone()
                },
                channel_id: new_voice.channel_id.or_else(|| existing.channel_id.clone()),
            },
            None => new_voice,
        };

        if merged.token.is_empty() || merged.endpoint.is_empty() || merged.session_id.is_empty() {
            self.voice = Some(merged);
            return false;
        }


        let mut adapter = crate::player::kizuna_adapter::KizunaVoiceAdapter::new(
            merged.session_id.clone(),
            merged.token.clone(),
            merged.endpoint.clone(),
            self.guild_id.clone(),
        );
        let _ = adapter
            .connect(self.guild_id.clone(), self.user_id.clone())
            .await;
        self.kizuna_voice_adapter = Some(std::sync::Arc::new(tokio::sync::Mutex::new(adapter)));

        info!("Voice connected for guild: {}", self.guild_id);

        self.voice = Some(merged);
        self.last_update = util::current_timestamp();
        self.emit_player_update();
        true
    }

    fn extension_hint(url: &str) -> Option<String> {
        let path = url.split(['?', '#']).next()?;
        let name = path.rsplit('/').next()?;
        let dot = name.rfind('.')?;
        let ext = &name[dot + 1..];
        let ext = ext.to_ascii_lowercase();
        match ext.as_str() {
            "mp3" | "m4a" | "mp4" | "aac" | "ogg" | "opus" | "webm" | "flac" | "wav" => Some(ext),
            _ => None,
        }
    }

    fn stop_handle_silently(&mut self) {
        if let Some(handle) = self.kizuna_track_handle.take() {
            tokio::spawn(async move {
                let _ = handle.stop().await;
            });
        }
    }

    async fn restart_at(&mut self, position_ms: u64) {
        let Some(url) = self.current_stream_url.clone() else {
            return;
        };
        let Some(_track) = self.queue.current.clone() else {
            return;
        };

        self.stop_handle_silently();

        let was_paused = self.paused;
        let filtered = self.shared_chain.lock().unwrap().is_active();

        if let Some(adapter_arc) = &self.kizuna_voice_adapter {
            match crate::dsp::pipeline::create_kizuna_source(
                crate::config::http_client(),
                url.clone(),
                None,
                self.shared_chain.clone(),
                0,
            )
            .await
            {
                Ok(k_source) => {
                    let k_src = Arc::new(Mutex::new(k_source));
                    let mut adapter = adapter_arc.lock().await;
                    let k_handle = adapter.play_source(k_src, self.user_id.clone());

                    let guild_id = self.guild_id.clone();
                    let tx = self.track_end_tx.clone();
                    let kh_clone = k_handle.clone();

                    // TrackEndNotifier replacement loop
                    tokio::spawn(async move {
                        while let Ok(event) = kh_clone.next_event().await {
                            if matches!(
                                event,
                                kizuna_voice::audio::TrackEvent::Ended
                                    | kizuna_voice::audio::TrackEvent::Error(_)
                            ) {
                                let _ = tx.send(guild_id.clone());
                                break;
                            }
                        }
                    });

                    self.kizuna_track_handle = Some(k_handle);
                }
                Err(e) => {
                    warn!("Failed to recreate audio source on restart for guild {}: {}", self.guild_id, e);
                    return;
                }
            }
        }

        let factor = self
            .shared_chain
            .lock()
            .unwrap()
            .duration_factor()
            .max(1e-6);
        let wall_offset_ms = (position_ms as f64 / factor) as u64;

        if was_paused {
            if let Some(k_handle) = &self.kizuna_track_handle {
                let k = k_handle.clone();
                tokio::spawn(async move {
                    let _ = k.pause().await;
                });
            }
            self.paused_at = Some(Instant::now());
            self.paused_position = position_ms;
            self.play_started_at = None;
        } else {
            self.play_started_at = Some(Instant::now() - Duration::from_millis(wall_offset_ms));
            self.paused_at = None;
            self.paused_position = 0;
        }

        self.filtered_active = filtered;

        info!(
            "Restarted playback at ~{} ms for guild {} (filtered={})",
            position_ms, self.guild_id, filtered
        );
    }

    pub async fn play_track(&mut self, track: LavalinkTrack, stream_url: String) -> bool {
        if let Some(old_handle) = &self.kizuna_track_handle {
            let k = old_handle.clone();
            tokio::spawn(async move {
                let _ = k.stop().await;
            });
            if let Some(old_track) = self.queue.current.take() {
                self.emit_event(
                    "TrackEndEvent",
                    serde_json::json!({
                        "track": old_track,
                        "reason": "replaced",
                    }),
                );
            }
        }

        let filtered = self.shared_chain.lock().unwrap().is_active();

        if let Some(adapter_arc) = &self.kizuna_voice_adapter {
            match crate::dsp::pipeline::create_kizuna_source(
                crate::config::http_client(),
                stream_url.clone(),
                None,
                self.shared_chain.clone(),
                0,
            )
            .await
            {
                Ok(k_source) => {
                    let k_src = Arc::new(Mutex::new(k_source));
                    let mut adapter = adapter_arc.lock().await;
                    let k_handle = adapter.play_source(k_src, self.user_id.clone());

                    let guild_id = self.guild_id.clone();
                    let tx = self.track_end_tx.clone();
                    let kh_clone = k_handle.clone();

                    // TrackEndNotifier replacement loop
                    tokio::spawn(async move {
                        while let Ok(event) = kh_clone.next_event().await {
                            if matches!(
                                event,
                                kizuna_voice::audio::TrackEvent::Ended
                                    | kizuna_voice::audio::TrackEvent::Error(_)
                            ) {
                                let _ = tx.send(guild_id.clone());
                                break;
                            }
                        }
                    });

                    self.kizuna_track_handle = Some(k_handle);
                }
                Err(e) => {
                    warn!("Failed to create audio source for guild {}: {}", self.guild_id, e);
                    self.emit_track_load_failed(&track, &format!("Failed to open audio stream: {}", e));
                    return false;
                }
            }
        } else {
            warn!("Cannot play track: voice not connected for guild {}", self.guild_id);
            self.emit_track_load_failed(&track, "Voice connection not established");
            return false;
        }

        if let Some(k_handle) = &self.kizuna_track_handle {
            let k = k_handle.clone();
            let vol = self.volume as f32 / 100.0;
            tokio::spawn(async move {
                let _ = k.set_volume(vol).await;
            });
        }

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
        self.emit_event(
            "TrackStartEvent",
            serde_json::json!({
                "track": track,
            }),
        );
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
            self.emit_event(
                "TrackEndEvent",
                serde_json::json!({
                    "track": track,
                    "reason": "stopped",
                }),
            );
        }
        self.emit_player_update();
        old_track
    }

    pub fn set_paused(&mut self, paused: bool) {
        self.paused = paused;
        self.last_update = util::current_timestamp();
        self.emit_player_update();
    }

    pub fn set_volume(&mut self, volume: u32) {
        self.volume = volume.clamp(0, 1000);
        self.last_update = util::current_timestamp();
        self.emit_player_update();
    }

    pub async fn apply_filters(&mut self) {
        let structural = {
            let mut chain = self.shared_chain.lock().unwrap();
            chain.update_from_lavalink(&self.filters)
        };

        if structural && self.is_playing && self.queue.current.is_some() {
            let pos = self.get_position();
            info!("Structural filter change; restarting at {} ms", pos);
            self.restart_at(pos).await;
        } else if !structural && !self.filtered_active {
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
            if self.is_playing {
                self.restart_at(position).await;
            }
            self.play_started_at = Some(Instant::now() - Duration::from_millis(position));
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
            LoopMode::Track => self.queue.current.clone(),
            LoopMode::Queue => {
                if let Some(track) = self.queue.tracks.pop_front() {
                    self.queue.tracks.push_back(track.clone());
                    Some(track)
                } else {
                    self.queue.current.clone()
                }
            }
            LoopMode::None => self.queue.tracks.pop_front(),
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
