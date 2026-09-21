// Copyright (c) 2026 nikcodex (KizunaLink)
// Licensed under the MIT License

use serde::{Deserialize, Serialize};

use crate::common::utils::now_ms;

/// Exception severity levels.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Severity {
    Common,
    Suspicious,
    Fault,
}

/// KizunaLink v4 JSON error response format.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KizunaLinkError {
    /// Unix timestamp in milliseconds.
    pub timestamp: u64,
    /// HTTP status code.
    pub status: u16,
    /// HTTP status reason phrase (e.g. "Bad Request").
    pub error: String,
    /// Human-readable error message.
    pub message: String,
    /// The request path that caused the error.
    pub path: String,
    /// Stack trace (only in non-production).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trace: Option<String>,
}

impl KizunaLinkError {
    /// Creates a 400 Bad Request error.
    pub fn bad_request(message: impl Into<String>, path: impl Into<String>) -> Self {
        Self::new(400, "Bad Request", message, path)
    }

    /// Creates a 404 Not Found error.
    pub fn not_found(message: impl Into<String>, path: impl Into<String>) -> Self {
        Self::new(404, "Not Found", message, path)
    }

    /// Creates a generic error response.
    pub fn new(
        status: u16,
        error: impl Into<String>,
        message: impl Into<String>,
        path: impl Into<String>,
    ) -> Self {
        Self {
            timestamp: now_ms(),
            status,
            error: error.into(),
            message: message.into(),
            path: path.into(),
            trace: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bad_request_has_correct_status() {
        let err = KizunaLinkError::bad_request("test msg", "/v4/test");
        assert_eq!(err.status, 400);
        assert_eq!(err.error, "Bad Request");
        assert_eq!(err.message, "test msg");
        assert_eq!(err.path, "/v4/test");
    }

    #[test]
    fn not_found_has_correct_status() {
        let err = KizunaLinkError::not_found("missing", "/v4/sessions/123");
        assert_eq!(err.status, 404);
        assert_eq!(err.error, "Not Found");
    }

    #[test]
    fn error_has_timestamp() {
        let err = KizunaLinkError::new(500, "Internal", "oops", "/v4/test");
        assert!(err.timestamp > 0);
        assert!(err.trace.is_none());
    }

    #[test]
    fn severity_variants_serialize() {
        // Ensure serde works
        let json = serde_json::to_string(&Severity::Common).unwrap();
        assert_eq!(json, "\"common\"");
        let json = serde_json::to_string(&Severity::Fault).unwrap();
        assert_eq!(json, "\"fault\"");
    }
}
