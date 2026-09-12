// Copyright (c) 2026 nikcodex (KizunaLink)
// Licensed under the MIT License

use thiserror::Error;

#[derive(Debug, Error)]
pub enum TidalError {
    #[error("HTTP request failed: {0}")]
    Request(#[from] reqwest::Error),

    #[error("HiFi API returned {status}: {body}")]
    ApiError {
        status: reqwest::StatusCode,
        body: String,
    },

    #[error("Failed to parse response: {0}")]
    Parse(#[from] serde_json::Error),

    #[error("Base64 decode error: {0}")]
    Base64(#[from] base64::DecodeError),

    #[error("No HiFi API base URLs configured — add at least one URL to hifi_apis")]
    NoApisConfigured,

    #[error("All HiFi API base URLs failed for this request")]
    AllApisFailed,

    #[error("Unsupported manifest type: {0} (DASH is not directly streamable)")]
    UnsupportedManifest(String),

    #[error("No stream URL found in manifest")]
    NoStreamUrl,

    #[error("{0}")]
    Other(String),
}

pub type TidalResult<T> = Result<T, TidalError>;
