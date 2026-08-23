use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::Serialize;
use serde_json::json;

#[derive(Debug, Clone)]
pub struct LavalinkError {
    pub status: u16,
    pub error: String,
    pub message: String,
    pub path: String,
}

impl LavalinkError {
    pub fn new(status: StatusCode, message: impl Into<String>, path: impl Into<String>) -> Self {
        Self {
            status: status.as_u16(),
            error: status.canonical_reason().unwrap_or("Unknown").to_string(),
            message: message.into(),
            path: path.into(),
        }
    }
}

impl IntoResponse for LavalinkError {
    fn into_response(self) -> Response {
        let body = json!({
            "timestamp": crate::util::current_timestamp(),
            "status": self.status,
            "error": self.error,
            "message": self.message,
            "path": self.path,
        });
        (
            StatusCode::from_u16(self.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
            Json(body),
        )
            .into_response()
    }
}
