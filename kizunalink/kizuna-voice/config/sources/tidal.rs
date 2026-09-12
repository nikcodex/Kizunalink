// Copyright (c) 2026 nikcodex (KizunaLink)
// Licensed under the MIT License

use serde::{Deserialize, Serialize};

use super::HttpProxyConfig;
use crate::config::media::sources::{
    default_country_code, default_limit_20, default_limit_50, default_tidal_quality, default_true,
};

fn default_hifi_qualities() -> Vec<String> {
    vec!["LOSSLESS".to_string(), "HIGH".to_string(), "LOW".to_string()]
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct TidalConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_country_code")]
    pub country_code: String,
    #[serde(default)]
    pub hifi_apis: Vec<String>,
    #[serde(default = "default_hifi_qualities")]
    pub hifi_qualities: Vec<String>,
    #[serde(default = "default_limit_50")]
    pub playlist_load_limit: usize,
    #[serde(default = "default_limit_50")]
    pub album_load_limit: usize,
    #[serde(default = "default_limit_20")]
    pub artist_load_limit: usize,
    pub proxy: Option<HttpProxyConfig>,
}

impl Default for TidalConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            country_code: default_country_code(),
            hifi_apis: vec![],
            hifi_qualities: default_hifi_qualities(),
            playlist_load_limit: 50,
            album_load_limit: 50,
            artist_load_limit: 20,
            proxy: None,
        }
    }
}