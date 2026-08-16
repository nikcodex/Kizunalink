use crate::models::track::LavalinkTrack;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct VoiceStateUpdate {
    pub token: String,
    pub endpoint: String,
    #[serde(rename = "sessionId")]
    pub session_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PlayerUpdatePayload {
    #[serde(rename = "encodedTrack")]
    pub encoded_track: Option<String>,
    pub identifier: Option<String>,
    pub position: Option<u64>,
    #[serde(rename = "endTime")]
    pub end_time: Option<u64>,
    pub volume: Option<u32>,
    pub paused: Option<bool>,
    pub filters: Option<serde_json::Value>,
    pub voice: Option<VoiceStateUpdate>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlayerState {
    pub time: u64,
    pub position: u64,
    pub connected: bool,
    pub ping: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlayerResponse {
    #[serde(rename = "guildId")]
    pub guild_id: String,
    pub track: Option<LavalinkTrack>,
    pub volume: u32,
    pub paused: bool,
    pub state: PlayerState,
    pub voice: VoiceStateUpdate,
    pub filters: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op")]
#[allow(clippy::large_enum_variant)]
pub enum OutboundWsMessage {
    #[serde(rename = "ready")]
    Ready {
        resumed: bool,
        #[serde(rename = "sessionId")]
        session_id: String,
    },
    #[serde(rename = "playerUpdate")]
    PlayerUpdate {
        #[serde(rename = "guildId")]
        guild_id: String,
        state: PlayerState,
    },
    #[serde(rename = "event")]
    Event(PlayerEvent),
    #[serde(rename = "stats")]
    Stats(StatsPayload),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum PlayerEvent {
    TrackStartEvent {
        #[serde(rename = "guildId")]
        guild_id: String,
        track: LavalinkTrack,
    },
    TrackEndEvent {
        #[serde(rename = "guildId")]
        guild_id: String,
        track: LavalinkTrack,
        reason: String,
    },
    TrackExceptionEvent {
        #[serde(rename = "guildId")]
        guild_id: String,
        track: LavalinkTrack,
        exception: serde_json::Value,
    },
    TrackStuckEvent {
        #[serde(rename = "guildId")]
        guild_id: String,
        track: LavalinkTrack,
        #[serde(rename = "thresholdMs")]
        threshold_ms: u64,
    },
    WebSocketClosedEvent {
        #[serde(rename = "guildId")]
        guild_id: String,
        code: u32,
        reason: String,
        #[serde(rename = "byRemote")]
        by_remote: bool,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatsPayload {
    pub players: usize,
    #[serde(rename = "playingPlayers")]
    pub playing_players: usize,
    pub uptime: u64,
    pub memory: MemoryStats,
    pub cpu: CpuStats,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryStats {
    pub free: u64,
    pub used: u64,
    pub allocated: u64,
    pub reservable: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CpuStats {
    pub cores: usize,
    #[serde(rename = "systemLoad")]
    pub system_load: f64,
    #[serde(rename = "lavalinkLoad")]
    pub lavalink_load: f64,
}
