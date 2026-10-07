// Copyright (c) 2026 nikcodex (KizunaLink)
// Licensed under the MIT License

use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub struct LocalSourceConfig {
    #[serde(default)]
    pub enabled: bool,
    /// Restrict local playback to this directory and its descendants.
    #[serde(default)]
    pub media_dir: Option<String>,
}
