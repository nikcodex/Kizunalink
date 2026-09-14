// Copyright (c) 2026 nikcodex (KizunaLink)
// Licensed under the MIT License

use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct PlayerConfig {
    #[serde(default = "default_stuck_threshold_ms")]
    pub stuck_threshold_ms: u64,
    #[serde(default = "default_buffer_duration_ms")]
    pub buffer_duration_ms: u64,
    #[serde(default = "default_frame_buffer_duration_ms")]
    pub frame_buffer_duration_ms: u64,
    #[serde(default)]
    pub resampling_quality: ResamplingQuality,
    #[serde(default = "default_opus_encoding_quality")]
    pub opus_encoding_quality: u8,
    #[serde(default)]
    pub tape: TapeConfig,
    #[serde(default)]
    pub mirrors: Option<crate::config::server::MirrorsConfig>,
    #[serde(default)]
    pub sponsorblock: SponsorBlockConfig,
}

/// SponsorBlock integration: automatically skip sponsored/annoying segments on
/// YouTube tracks. Mirror-compatible with the `SponsorBlock-Plugin` REST API
/// (`/v4/sessions/{s}/players/{g}/sponsorblock/categories`) but implemented fully
/// in-process, no JVM needed.
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct SponsorBlockConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_sponsorblock_categories")]
    pub categories: Vec<String>,
    #[serde(default = "default_sponsorblock_api_url")]
    pub api_url: String,
}

impl Default for SponsorBlockConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            categories: default_sponsorblock_categories(),
            api_url: default_sponsorblock_api_url(),
        }
    }
}

fn default_sponsorblock_categories() -> Vec<String> {
    crate::lavalink::sponsorblock::DEFAULT_CATEGORIES
        .iter()
        .map(|s| s.to_string())
        .collect()
}

fn default_sponsorblock_api_url() -> String {
    crate::lavalink::sponsorblock::DEFAULT_API_URL.to_string()
}

#[derive(Debug, Deserialize, Serialize, Clone, Default, Copy, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ResamplingQuality {
    Low,
    #[default]
    Medium,
    High,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct TapeConfig {
    #[serde(default)]
    pub tape_stop: bool,
    #[serde(default = "default_tape_stop_duration_ms")]
    pub tape_stop_duration_ms: u64,
    #[serde(default)]
    pub curve: TapeCurve,
}

#[derive(Debug, Deserialize, Serialize, Clone, Default, Copy, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum TapeCurve {
    Linear,
    Exponential,
    #[default]
    Sinusoidal,
}

impl TapeCurve {
    pub fn value(self, t: f32) -> f32 {
        match self {
            Self::Linear => t,
            Self::Exponential => t * t,
            Self::Sinusoidal => 0.5 * (1.0 - (t * std::f32::consts::PI).cos()),
        }
    }
}

impl Default for PlayerConfig {
    fn default() -> Self {
        Self {
            stuck_threshold_ms: default_stuck_threshold_ms(),
            buffer_duration_ms: default_buffer_duration_ms(),
            frame_buffer_duration_ms: default_frame_buffer_duration_ms(),
            resampling_quality: ResamplingQuality::default(),
            opus_encoding_quality: default_opus_encoding_quality(),
            tape: TapeConfig::default(),
            mirrors: None,
            sponsorblock: SponsorBlockConfig::default(),
        }
    }
}

impl Default for TapeConfig {
    fn default() -> Self {
        Self {
            tape_stop: false,
            tape_stop_duration_ms: default_tape_stop_duration_ms(),
            curve: TapeCurve::default(),
        }
    }
}

fn default_stuck_threshold_ms() -> u64 {
    10000
}
fn default_buffer_duration_ms() -> u64 {
    400
}
fn default_frame_buffer_duration_ms() -> u64 {
    5000
}
fn default_opus_encoding_quality() -> u8 {
    10
}
fn default_tape_stop_duration_ms() -> u64 {
    500
}
