// Copyright (c) 2026 nikcodex (KizunaLink)
// Licensed under the MIT License

use serde::{Deserialize, Serialize};

use crate::config::sources::{HttpProxyConfig, default_false, default_true};

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct YouTubeConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub clients: YouTubeClientsConfig,
    #[serde(default)]
    pub cipher: YouTubeCipherConfig,
    #[serde(default)]
    pub refresh_tokens: Vec<String>,
    #[serde(default = "default_false")]
    pub get_oauth_token: bool,
    /// Route all YouTube requests (innertube, player, CDN fetches) through this proxy.
    ///
    /// YouTube aggressively blocks datacenter/cloud IPs ("Sign in to confirm you're not
    /// a bot" / HTTP 403 from `googlevideo.com`). Routing through a residential or
    /// otherwise clean egress is the most reliable mitigation.
    #[serde(default)]
    pub proxy: Option<HttpProxyConfig>,
}

impl Default for YouTubeConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            clients: YouTubeClientsConfig::default(),
            cipher: YouTubeCipherConfig::default(),
            refresh_tokens: Vec::new(),
            get_oauth_token: false,
            proxy: None,
        }
    }
}

#[derive(Debug, Deserialize, Serialize, Clone, Default)]
#[serde(default)]
pub struct YouTubeCipherConfig {
    pub url: Option<String>,
    pub token: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct YouTubeClientsConfig {
    #[serde(default = "default_search_clients")]
    pub search: Vec<String>,
    #[serde(default = "default_playback_clients")]
    pub playback: Vec<String>,
    #[serde(default = "default_resolve_clients")]
    pub resolve: Vec<String>,
}

impl Default for YouTubeClientsConfig {
    fn default() -> Self {
        Self {
            search: default_search_clients(),
            playback: default_playback_clients(),
            resolve: default_resolve_clients(),
        }
    }
}

fn default_search_clients() -> Vec<String> {
    vec![
        "MUSIC_ANDROID".to_string(),
        "MUSIC_WEB".to_string(),
        "ANDROID".to_string(),
        "WEB".to_string(),
    ]
}

fn default_playback_clients() -> Vec<String> {
    vec![
        "TV_CAST".to_string(),
        "TV_EMBEDDED".to_string(),
        "ANDROID_VR".to_string(),
        "WEB_EMBEDDED".to_string(),
        "TV".to_string(),
        "ANDROID_MUSIC".to_string(),
        "WEB".to_string(),
    ]
}

fn default_resolve_clients() -> Vec<String> {
    vec![
        "WEB".to_string(),
        "MUSIC_WEB".to_string(),
        "ANDROID".to_string(),
        "TVHTML5_SIMPLY".to_string(),
    ]
}
