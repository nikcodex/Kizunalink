use axum::{
    http::{HeaderMap, HeaderName, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;

#[derive(Debug)]
pub struct LavalinkError {
    pub status: u16,
    pub error: String,
    pub message: String,
    pub path: String,
    pub headers: Option<Box<HeaderMap>>,
}

impl LavalinkError {
    pub fn new(status: StatusCode, message: impl Into<String>, path: impl Into<String>) -> Self {
        let mut headers = None;
        if status == StatusCode::TOO_MANY_REQUESTS {
            if let Ok(val) = HeaderValue::from_str("60") {
                let mut map = HeaderMap::new();
                map.insert(axum::http::header::RETRY_AFTER, val);
                headers = Some(Box::new(map));
            }
        }
        Self {
            status: status.as_u16(),
            error: status.canonical_reason().unwrap_or("Unknown").to_string(),
            message: message.into(),
            path: path.into(),
            headers,
        }
    }

    pub fn with_header(mut self, name: HeaderName, value: HeaderValue) -> Self {
        self.headers
            .get_or_insert_with(|| Box::new(HeaderMap::new()))
            .insert(name, value);
        self
    }

    pub fn with_retry_after(mut self, retry_after_secs: u64) -> Self {
        if let Ok(val) = HeaderValue::from_str(&retry_after_secs.to_string()) {
            self.headers
                .get_or_insert_with(|| Box::new(HeaderMap::new()))
                .insert(axum::http::header::RETRY_AFTER, val);
        }
        self
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
        let status = StatusCode::from_u16(self.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        let mut resp = (status, Json(body)).into_response();
        if let Some(headers) = self.headers {
            resp.headers_mut().extend(*headers);
        }
        resp
    }
}

impl std::fmt::Display for LavalinkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.error, self.message)
    }
}

impl std::error::Error for LavalinkError {}
