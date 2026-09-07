use crate::models::filters::Filters;
use crate::models::track::LavalinkTrack;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct VoiceStateUpdate {
    pub token: String,
    pub endpoint: String,
    #[serde(rename = "sessionId")]
    pub session_id: String,
    #[serde(rename = "channelId", default)]
    pub channel_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TrackPayload {
    pub encoded: Option<String>,
    pub identifier: Option<String>,
    #[serde(rename = "userData", default)]
    pub user_data: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PlayerUpdatePayload {
    pub track: Option<TrackPayload>,
    #[serde(rename = "encodedTrack")]
    pub encoded_track: Option<String>,
    pub identifier: Option<String>,
    pub position: Option<u64>,
    #[serde(rename = "endTime")]
    pub end_time: Option<u64>,
    pub volume: Option<u32>,
    pub paused: Option<bool>,
    pub filters: Option<Filters>,
    pub voice: Option<VoiceStateUpdate>,
    pub autoplay: Option<bool>,
    #[serde(rename = "loop", default)]
    pub loop_mode: Option<String>,
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
    pub filters: Filters,
    pub autoplay: bool,
    #[serde(rename = "loop")]
    pub loop_mode: String,
    #[serde(skip)]
    pub is_playing: bool,
}

impl PlayerResponse {
    pub fn is_actively_playing(&self) -> bool {
        self.is_playing && !self.paused && self.track.is_some()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatsPayload {
    pub players: usize,
    #[serde(rename = "playingPlayers")]
    pub playing_players: usize,
    pub uptime: u64,
    pub memory: MemoryStats,
    pub cpu: CpuStats,
    /// Lavalink v4: `null` when the node has no players, and always `null` on
    /// `GET /v4/stats`. The key itself is always present on the wire.
    #[serde(rename = "frameStats")]
    pub frame_stats: Option<FrameStats>,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FrameStats {
    pub sent: u64,
    pub nulled: u64,
    pub deficit: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionInfo {
    pub semver: String,
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
    #[serde(rename = "preRelease")]
    pub pre_release: Option<String>,
    /// Semver build metadata. Lavalink v4's `/v4/info` always emits this key
    /// (`"build": null` when there is none), so it must be present on the wire.
    pub build: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitInfo {
    pub branch: String,
    pub commit: String,
    #[serde(rename = "commitTime")]
    pub commit_time: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerInfo {
    pub version: VersionInfo,
    #[serde(rename = "buildTime")]
    pub build_time: u64,
    pub git: GitInfo,
    pub jvm: String,
    pub lavaplayer: String,
    #[serde(rename = "sourceManagers")]
    pub source_managers: Vec<String>,
    pub filters: Vec<String>,
    pub plugins: Vec<PluginInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginInfo {
    pub name: String,
    pub version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionUpdate {
    pub resuming: Option<bool>,
    pub timeout: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionResponse {
    pub resuming: bool,
    pub timeout: u64,
}
