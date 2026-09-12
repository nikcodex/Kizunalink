// Copyright (c) 2026 nikcodex (KizunaLink)
// Licensed under the MIT License

use serde::{Deserialize, Serialize};

use crate::config::sources::sources::HttpProxyConfig;

#[derive(Debug, Deserialize, Serialize, Clone, Default)]
#[serde(default)]
pub struct RedditConfig {
    pub enabled: bool,
    pub proxy: Option<HttpProxyConfig>,
}
