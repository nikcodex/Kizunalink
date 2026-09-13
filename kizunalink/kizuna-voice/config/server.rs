// Copyright (c) 2026 nikcodex (KizunaLink)
// Licensed under the MIT License

use std::time::Duration;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ServerConfig {
    /// Server bind address (e.g. "0.0.0.0" or "127.0.0.1").
    #[serde(default = "default_address")]
    pub address: String,
    /// Server port (default: 2333, matching Lavalink convention).
    #[serde(default = "default_port")]
    pub port: u16,
    /// Authorization password required for REST and WebSocket access.
    #[serde(default = "default_authorization")]
    pub authorization: String,
    /// Interval between player state updates sent to the client (N02).
    ///
    /// The TOML wire format is unchanged — a plain integer of **seconds** (`5`) — but the
    /// in-memory type is a `Duration` so call sites cannot forget the unit.
    #[serde(
        default = "default_player_update_interval",
        deserialize_with = "de_duration_secs",
        serialize_with = "ser_duration_secs"
    )]
    pub player_update_interval: Duration,
    /// Interval in **seconds** between stats events sent over WebSocket.
    #[serde(default = "default_stats_interval")]
    pub stats_interval: u64,
    /// Interval in **seconds** between WebSocket ping frames.
    #[serde(default = "default_websocket_ping_interval")]
    pub websocket_ping_interval: u64,
    /// Maximum number of events to queue for a disconnected session.
    #[serde(default = "default_max_event_queue_size")]
    pub max_event_queue_size: usize,
    /// Requests per minute allowed per client IP across REST + WebSocket upgrade
    /// (S06). `0` disables rate limiting entirely. Default 2000 is well above any
    /// sane single-node bot load and below anything an abusive flood would need.
    #[serde(default = "default_rate_limit_per_minute")]
    pub rate_limit_per_minute: u32,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            address: default_address(),
            port: default_port(),
            authorization: default_authorization(),
            player_update_interval: default_player_update_interval(),
            stats_interval: default_stats_interval(),
            websocket_ping_interval: default_websocket_ping_interval(),
            max_event_queue_size: default_max_event_queue_size(),
            rate_limit_per_minute: default_rate_limit_per_minute(),
        }
    }
}

fn default_address() -> String {
    "127.0.0.1".to_string()
}
fn default_port() -> u16 {
    2333
}
fn default_authorization() -> String {
    "youshallnotpass".to_string()
}
fn default_max_event_queue_size() -> usize {
    100
}
fn default_rate_limit_per_minute() -> u32 {
    2000
}
fn default_player_update_interval() -> Duration {
    Duration::from_secs(5)
}

/// Deserialize an integer number of seconds into a [`Duration`] (keeps the on-disk
/// config format byte-compatible with the old `u64` field).
fn de_duration_secs<'de, D>(deserializer: D) -> Result<Duration, D::Error>
where
    D: Deserializer<'de>,
{
    let secs = u64::deserialize(deserializer)?;
    Ok(Duration::from_secs(secs))
}

fn ser_duration_secs<S: Serializer>(value: &Duration, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_u64(value.as_secs())
}
fn default_stats_interval() -> u64 {
    30
}
fn default_websocket_ping_interval() -> u64 {
    20
}

#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub struct LoggingConfig {
    pub level: Option<String>,
    pub filters: Option<String>,
    pub file: Option<LogFileConfig>,
}

#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub struct LogFileConfig {
    pub path: String,
    pub max_lines: u32,
    #[serde(default)]
    pub max_files: u32,
    #[serde(default)]
    pub rotate_daily: bool,
}

#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub struct RoutePlannerConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub cidrs: Vec<String>,
    #[serde(default)]
    pub excluded_ips: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone, Default)]
#[serde(default)]
pub struct MirrorsConfig {
    pub providers: Vec<String>,
    pub best_match: BestMatchConfig,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(default)]
pub struct BestMatchConfig {
    pub scoring: bool,
    pub throttled_prefixes: Vec<String>,
    pub min_similarity: f64,
    pub high_confidence: f64,
    pub immediate_use: f64,
    pub weight_title: f64,
    pub weight_artist: f64,
    pub weight_duration: f64,
    pub duration_tolerance_ms: u64,
}

impl Default for BestMatchConfig {
    fn default() -> Self {
        Self {
            scoring: true,
            throttled_prefixes: vec!["ytmsearch:".into(), "ytsearch:".into()],
            min_similarity: 0.50,
            high_confidence: 0.75,
            immediate_use: 0.88,
            weight_title: 0.50,
            weight_artist: 0.30,
            weight_duration: 0.20,
            duration_tolerance_ms: 3_000,
        }
    }
}

#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub struct ConfigServerConfig {
    pub url: String,
    pub username: Option<String>,
    pub password: Option<String>,
}
