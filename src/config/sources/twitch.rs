// Copyright (c) 2026 nikcodex (KizunaLink)
// Licensed under the MIT License

use serde::{Deserialize, Serialize};

use crate::config::sources::HttpProxyConfig;

#[derive(Debug, Deserialize, Serialize, Clone, Default)]
#[serde(default)]
pub struct TwitchConfig {
    pub enabled: bool,
    pub client_id: Option<String>,
    pub proxy: Option<HttpProxyConfig>,
}
