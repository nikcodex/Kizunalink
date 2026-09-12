// Copyright (c) 2026 nikcodex (KizunaLink)
// Licensed under the MIT License

use thiserror::Error;

/// Errors that can occur during source resolution and track loading.
#[derive(Debug, Error)]
pub enum SourceError {
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Track not found: {0}")]
    NotFound(String),

    #[error("Source unavailable: {0}")]
    Unavailable(String),

    #[error("Invalid response: {0}")]
    InvalidResponse(String),

    #[error("Cipher error: {0}")]
    Cipher(String),

    #[error("Authentication required: {0}")]
    AuthRequired(String),

    #[error("Rate limited")]
    RateLimited,

    #[error("{0}")]
    Other(String),
}

impl SourceError {
    /// Create a load-failed error message suitable for the Lavalink protocol.
    pub fn load_failed_message(&self) -> String {
        match self {
            Self::NotFound(id) => format!("No matches found for '{id}'"),
            Self::Unavailable(msg) => format!("Source unavailable: {msg}"),
            Self::RateLimited => "Source rate limited, try again later".to_string(),
            other => format!("Load failed: {other}"),
        }
    }
}
