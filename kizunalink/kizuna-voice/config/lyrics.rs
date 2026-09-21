// Copyright (c) 2026 nikcodex (KizunaLink)
// Licensed under the MIT License

use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub struct LyricsConfig {
    #[serde(default)]
    pub youtubemusic: bool,
    #[serde(default)]
    pub lrclib: bool,
    #[serde(default)]
    pub genius: bool,
    #[serde(default)]
    pub deezer: bool,
    #[serde(default)]
    pub musixmatch: bool,
    #[serde(default)]
    pub letrasmus: bool,
    #[serde(default)]
    pub yandex: bool,
    #[serde(default)]
    pub netease: bool,
}

#[derive(Debug, Deserialize, Serialize, Clone, Default)]
#[serde(default)]
pub struct YandexLyricsConfig {
    pub access_token: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone, Default)]
#[serde(default)]
pub struct YandexConfig {
    pub lyrics: Option<YandexLyricsConfig>,
}
