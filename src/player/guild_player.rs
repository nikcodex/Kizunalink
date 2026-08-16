use crate::models::{
    protocol::{PlayerResponse, PlayerState, VoiceStateUpdate},
    track::LavalinkTrack,
};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone)]
pub struct GuildPlayer {
    pub guild_id: String,
    pub current_track: Option<LavalinkTrack>,
    pub volume: u32,
    pub paused: bool,
    pub position: u64,
    pub voice: Option<VoiceStateUpdate>,
    pub filters: serde_json::Value,
    pub last_update: u64,
}

impl GuildPlayer {
    pub fn new(guild_id: String) -> Self {
        Self {
            guild_id,
            current_track: None,
            volume: 100,
            paused: false,
            position: 0,
            voice: None,
            filters: serde_json::json!({}),
            last_update: current_timestamp(),
        }
    }

    pub fn set_track(&mut self, track: LavalinkTrack) {
        self.current_track = Some(track);
        self.position = 0;
        self.paused = false;
        self.last_update = current_timestamp();
    }

    pub fn stop(&mut self) {
        self.current_track = None;
        self.position = 0;
        self.paused = false;
        self.last_update = current_timestamp();
    }

    pub fn set_paused(&mut self, paused: bool) {
        self.paused = paused;
        self.last_update = current_timestamp();
    }

    pub fn set_volume(&mut self, volume: u32) {
        self.volume = volume.clamp(0, 1000);
        self.last_update = current_timestamp();
    }

    pub fn seek(&mut self, position: u64) {
        self.position = position;
        self.last_update = current_timestamp();
    }

    pub fn set_voice(&mut self, voice: VoiceStateUpdate) {
        self.voice = Some(voice);
        self.last_update = current_timestamp();
    }

    pub fn to_response(&self) -> PlayerResponse {
        let is_connected = self.voice.is_some();
        PlayerResponse {
            guild_id: self.guild_id.clone(),
            track: self.current_track.clone(),
            volume: self.volume,
            paused: self.paused,
            state: PlayerState {
                time: current_timestamp(),
                position: self.position,
                connected: is_connected,
                ping: if is_connected { 12 } else { -1 },
            },
            voice: self.voice.clone().unwrap_or(VoiceStateUpdate {
                token: "".to_string(),
                endpoint: "".to_string(),
                session_id: "".to_string(),
            }),
            filters: self.filters.clone(),
        }
    }
}

fn current_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
