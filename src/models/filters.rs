use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Filters {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub volume: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub equalizer: Option<Vec<Band>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub karaoke: Option<Karaoke>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timescale: Option<Timescale>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tremolo: Option<Tremolo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vibrato: Option<Vibrato>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub distortion: Option<Distortion>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rotation: Option<Rotation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel_mix: Option<ChannelMix>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub low_pass: Option<LowPass>,
    #[serde(flatten)]
    pub plugin_filters: std::collections::HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Band {
    pub band: i32,
    pub gain: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Karaoke {
    #[serde(default = "default_karaoke_level")]
    pub level: f32,
    #[serde(default = "default_karaoke_mono_level")]
    pub mono_level: f32,
    #[serde(default = "default_karaoke_filter_band")]
    pub filter_band: f32,
    #[serde(default = "default_karaoke_filter_width")]
    pub filter_width: f32,
}

fn default_karaoke_level() -> f32 {
    1.0
}

fn default_karaoke_mono_level() -> f32 {
    1.0
}

fn default_karaoke_filter_band() -> f32 {
    220.0
}

fn default_karaoke_filter_width() -> f32 {
    100.0
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Timescale {
    #[serde(default = "default_timescale_speed")]
    pub speed: f64,
    #[serde(default = "default_timescale_pitch")]
    pub pitch: f64,
    #[serde(default = "default_timescale_rate")]
    pub rate: f64,
}

fn default_timescale_speed() -> f64 {
    1.0
}

fn default_timescale_pitch() -> f64 {
    1.0
}

fn default_timescale_rate() -> f64 {
    1.0
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tremolo {
    #[serde(default = "default_tremolo_frequency")]
    pub frequency: f32,
    #[serde(default = "default_tremolo_depth")]
    pub depth: f32,
}

fn default_tremolo_frequency() -> f32 {
    2.0
}

fn default_tremolo_depth() -> f32 {
    0.5
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Vibrato {
    #[serde(default = "default_vibrato_frequency")]
    pub frequency: f32,
    #[serde(default = "default_vibrato_depth")]
    pub depth: f32,
}

fn default_vibrato_frequency() -> f32 {
    2.0
}

fn default_vibrato_depth() -> f32 {
    0.5
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Distortion {
    #[serde(default)]
    pub sin_offset: f32,
    #[serde(default = "default_distortion_sin_scale")]
    pub sin_scale: f32,
    #[serde(default)]
    pub cos_offset: f32,
    #[serde(default = "default_distortion_cos_scale")]
    pub cos_scale: f32,
    #[serde(default)]
    pub tan_offset: f32,
    #[serde(default = "default_distortion_tan_scale")]
    pub tan_scale: f32,
    #[serde(default)]
    pub offset: f32,
    #[serde(default = "default_distortion_scale")]
    pub scale: f32,
}

fn default_distortion_sin_scale() -> f32 {
    1.0
}

fn default_distortion_cos_scale() -> f32 {
    1.0
}

fn default_distortion_tan_scale() -> f32 {
    1.0
}

fn default_distortion_scale() -> f32 {
    1.0
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rotation {
    #[serde(default)]
    pub rotation_hz: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelMix {
    #[serde(default = "default_channel_mix_left_to_left")]
    pub left_to_left: f32,
    #[serde(default)]
    pub left_to_right: f32,
    #[serde(default)]
    pub right_to_left: f32,
    #[serde(default = "default_channel_mix_right_to_right")]
    pub right_to_right: f32,
}

fn default_channel_mix_left_to_left() -> f32 {
    1.0
}

fn default_channel_mix_right_to_right() -> f32 {
    1.0
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LowPass {
    #[serde(default = "default_low_pass_smoothing")]
    pub smoothing: f32,
}

fn default_low_pass_smoothing() -> f32 {
    20.0
}
