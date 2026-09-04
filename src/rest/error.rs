use axum::{
    body::Bytes,
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
    /// Optional pre-built trace payload; `None` serializes as `"trace": null`.
    pub trace: Option<serde_json::Value>,
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

        // Keep error metrics live.
        let m = crate::metrics::Metrics::global();
        match status.as_u16() {
            404 => m.errors_not_found.inc(),
            429 => m.errors_rate_limit.inc(),
            s if s >= 500 => m.errors_internal.inc(),
            _ => {}
        }

        Self {
            status: status.as_u16(),
            error: status.canonical_reason().unwrap_or("Unknown").to_string(),
            message: message.into(),
            path: path.into(),
            headers,
            trace: None,
        }
    }

    pub fn with_trace(mut self, trace: serde_json::Value) -> Self {
        self.trace = Some(trace);
        self
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
            "trace": self.trace.unwrap_or(serde_json::Value::Null),
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

/// Build a safe, deterministic trace string from an existing error body.
/// Never includes request headers, tokens, cookies, or passwords.
fn build_trace_string(value: &serde_json::Value) -> String {
    let error = value
        .get("error")
        .and_then(|e| e.as_str())
        .unwrap_or("Error");
    let message = value
        .get("message")
        .and_then(|m| m.as_str())
        .unwrap_or_default();
    let path = value
        .get("path")
        .and_then(|p| p.as_str())
        .unwrap_or_default();
    let backtrace = std::backtrace::Backtrace::force_capture();
    format!(
        "kizunalink::{}: {}\n  path: {}\n{}::backtrace-end",
        error, message, path, backtrace
    )
}

/// Lavalink-compatible `?trace=true` handling.
///
/// When `trace_requested` is true and the response carries a Lavalink error JSON
/// body (with a null `trace` field), replaces the null with a useful trace
/// string. All other responses pass through untouched.
pub async fn maybe_inject_trace(response: Response, trace_requested: bool) -> Response {
    if !trace_requested {
        return response;
    }
    let status = response.status();
    if !status.is_client_error() && !status.is_server_error() {
        return response;
    }

    let (parts, body) = response.into_parts();
    let Ok(bytes) = axum::body::to_bytes(body, 1024 * 1024).await else {
        // Body was consumed; return the status-only response.
        return axum::response::Response::from_parts(parts, axum::body::Body::empty());
    };

    if let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) {
        let trace_is_null = value.get("trace").map(|t| t.is_null()).unwrap_or(false);
        if trace_is_null {
            let trace_str = build_trace_string(&value);
            if let Some(mut obj) = value.as_object().cloned() {
                obj.insert("trace".to_string(), serde_json::Value::String(trace_str));
                if let Ok(new_bytes) = serde_json::to_vec(&serde_json::Value::Object(obj)) {
                    let mut new_parts = parts;
                    new_parts.headers.remove(axum::http::header::CONTENT_LENGTH);
                    if let Ok(len) = HeaderValue::from_str(&new_bytes.len().to_string()) {
                        new_parts
                            .headers
                            .insert(axum::http::header::CONTENT_LENGTH, len);
                    }
                    return axum::response::Response::from_parts(
                        new_parts,
                        axum::body::Body::from(Bytes::from(new_bytes)),
                    );
                }
            }
        }
    }

    axum::response::Response::from_parts(parts, axum::body::Body::from(bytes))
}

impl std::fmt::Display for LavalinkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.error, self.message)
    }
}

impl std::error::Error for LavalinkError {}

#[cfg(test)]
mod tests {
    use super::*;

    async fn response_body(resp: Response) -> serde_json::Value {
        let body = resp.into_body();
        let bytes = axum::body::to_bytes(body, 1024 * 1024)
            .await
            .expect("read body");
        serde_json::from_slice(&bytes).expect("json body")
    }

    #[tokio::test]
    async fn test_error_response_trace_null_by_default() {
        let err = LavalinkError::new(
            StatusCode::NOT_FOUND,
            "Session not found: abc",
            "/v4/sessions/abc",
        );
        let resp = err.into_response();
        let body = response_body(resp).await;
        assert!(body.get("trace").unwrap().is_null());
        assert_eq!(body.get("status").unwrap().as_u64(), Some(404));
        assert_eq!(body.get("error").unwrap().as_str(), Some("Not Found"));
        assert_eq!(body.get("path").unwrap().as_str(), Some("/v4/sessions/abc"));
        assert!(body.get("timestamp").is_some());
    }

    #[tokio::test]
    async fn test_trace_injected_when_requested() {
        let err = LavalinkError::new(
            StatusCode::NOT_FOUND,
            "Session not found: abc",
            "/v4/sessions/abc",
        );
        let injected = maybe_inject_trace(err.into_response(), true).await;
        assert_eq!(injected.status(), StatusCode::NOT_FOUND);
        let body = response_body(injected).await;
        let trace = body.get("trace").unwrap();
        assert!(trace.is_string());
        let trace_str = trace.as_str().unwrap();
        assert!(trace_str.contains("Session not found: abc"));
        assert!(trace_str.contains("/v4/sessions/abc"));
        // All Lavalink error fields are preserved.
        for key in ["timestamp", "status", "error", "message", "path"] {
            assert!(body.get(key).is_some(), "missing {}", key);
        }
    }

    #[tokio::test]
    async fn test_trace_not_injected_when_not_requested() {
        let err = LavalinkError::new(
            StatusCode::NOT_FOUND,
            "Session not found: abc",
            "/v4/sessions/abc",
        );
        let resp = maybe_inject_trace(err.into_response(), false).await;
        let body = response_body(resp).await;
        assert!(body.get("trace").unwrap().is_null());
    }

    #[tokio::test]
    async fn test_trace_does_not_leak_secrets() {
        let err = LavalinkError::new(
            StatusCode::UNAUTHORIZED,
            "Unauthorized: Invalid authorization header",
            "/v4/info",
        );
        let injected = maybe_inject_trace(err.into_response(), true).await;
        let body = response_body(injected).await;
        let trace_str = body.get("trace").unwrap().as_str().unwrap();
        // The trace must never contain an actual credential value or auth scheme.
        for secret in ["youshallnotpass", "Bearer ", "ghp_", "xoxb-"] {
            assert!(!trace_str.contains(secret), "trace leaked '{}'", secret);
        }
    }

    #[tokio::test]
    async fn test_trace_skips_success_responses() {
        let resp = axum::response::Json(serde_json::json!({"ok": true})).into_response();
        let resp = maybe_inject_trace(resp, true).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = response_body(resp).await;
        assert_eq!(body.get("ok").unwrap().as_bool(), Some(true));
    }
}
