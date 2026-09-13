// Copyright (c) 2026 nikcodex (KizunaLink)
// Licensed under the MIT License

use thiserror::Error;

/// Errors that can occur during source resolution and track loading.
#[derive(Debug, Error)]
pub enum SourceError {
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("HTTP status error: {0}")]
    HttpStatus(reqwest::StatusCode),

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

impl From<reqwest::StatusCode> for SourceError {
    fn from(status: reqwest::StatusCode) -> Self {
        Self::HttpStatus(status)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_failed_message_not_found() {
        let err = SourceError::NotFound("test query".into());
        assert!(err.load_failed_message().contains("test query"));
    }

    #[test]
    fn load_failed_message_rate_limited() {
        let err = SourceError::RateLimited;
        assert!(err.load_failed_message().contains("rate limited"));
    }

    #[test]
    fn load_failed_message_unavailable() {
        let err = SourceError::Unavailable("server down".into());
        assert!(err.load_failed_message().contains("server down"));
    }

    #[test]
    fn error_display() {
        let err = SourceError::Cipher("decrypt failed".into());
        assert!(err.to_string().contains("decrypt failed"));
    }

    #[test]
    fn http_error_conversion() {
        let http_err = reqwest::StatusCode::NOT_FOUND;
        let err: SourceError = http_err.into();
        assert!(err.to_string().contains("404"));
    }
}
