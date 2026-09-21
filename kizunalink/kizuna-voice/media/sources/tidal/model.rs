// Copyright (c) 2026 nikcodex (KizunaLink)
// Licensed under the MIT License

use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaybackData {
    pub manifest: String,
    pub manifest_mime_type: String,
    pub mime_type: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub urls: Vec<String>,
    pub mime_type: Option<String>,
}
