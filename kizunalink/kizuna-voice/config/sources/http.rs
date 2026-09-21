// Copyright (c) 2026 nikcodex (KizunaLink)
// Licensed under the MIT License

use serde::{Deserialize, Serialize};

use crate::config::sources::default_true;

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct HttpSourceConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
}

impl Default for HttpSourceConfig {
    fn default() -> Self {
        Self { enabled: true }
    }
}
