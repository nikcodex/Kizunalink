use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
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
#[serde(rename_all = "camelCase")]
pub struct Band {
    pub band: i32,
    pub gain: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
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
#[serde(rename_all = "camelCase")]
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
#[serde(rename_all = "camelCase")]
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
#[serde(rename_all = "camelCase")]
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
#[serde(rename_all = "camelCase")]
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
#[serde(rename_all = "camelCase")]
pub struct Rotation {
    #[serde(default)]
    pub rotation_hz: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
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
#[serde(rename_all = "camelCase")]
pub struct LowPass {
    #[serde(default = "default_low_pass_smoothing")]
    pub smoothing: f32,
}

fn default_low_pass_smoothing() -> f32 {
    20.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_filters_camelcase_roundtrip() {
        let json = r#"{
            "volume": 1.2,
            "channelMix": { "leftToLeft": 0.5, "leftToRight": 0.2, "rightToLeft": 0.3, "rightToRight": 0.5 },
            "lowPass": { "smoothing": 15.0 },
            "rotation": { "rotationHz": 2.0 },
            "karaoke": { "monoLevel": 0.8, "filterBand": 250.0, "filterWidth": 120.0, "level": 1.0 },
            "distortion": { "sinScale": 1.5, "cosScale": 0.5 }
        }"#;

        let filters: Filters = serde_json::from_str(json).expect("Deserialization failed");
        assert!(filters.channel_mix.is_some());
        assert_eq!(filters.channel_mix.as_ref().unwrap().left_to_left, 0.5);
        assert_eq!(filters.channel_mix.as_ref().unwrap().left_to_right, 0.2);
        assert!(filters.low_pass.is_some());
        assert_eq!(filters.low_pass.as_ref().unwrap().smoothing, 15.0);
        assert_eq!(filters.rotation.as_ref().unwrap().rotation_hz, 2.0);
        assert_eq!(filters.karaoke.as_ref().unwrap().mono_level, 0.8);
        assert_eq!(filters.distortion.as_ref().unwrap().sin_scale, 1.5);
        assert!(
            filters.plugin_filters.is_empty(),
            "Core filter fields absorbed into plugin_filters!"
        );

        let serialized = serde_json::to_string(&filters).unwrap();
        assert!(serialized.contains("channelMix"));
        assert!(serialized.contains("lowPass"));
        assert!(serialized.contains("rotationHz"));
        assert!(!serialized.contains("channel_mix"));
        assert!(!serialized.contains("low_pass"));
    }
}
