use crate::rest::error::LavalinkError;
use crate::util::constant_time_eq;
use axum::http::{HeaderMap, StatusCode};

pub fn require_auth(
    headers: &HeaderMap,
    expected_password: &str,
    path: &str,
) -> Result<(), LavalinkError> {
    let auth = match headers.get("authorization").and_then(|h| h.to_str().ok()) {
        Some(a) if !a.is_empty() => a,
        _ => {
            return Err(LavalinkError::new(
                StatusCode::UNAUTHORIZED,
                "Unauthorized: Missing authorization header",
                path,
            ));
        }
    };

    if !constant_time_eq(auth, expected_password) {
        return Err(LavalinkError::new(
            StatusCode::UNAUTHORIZED,
            "Unauthorized: Invalid authorization header",
            path,
        ));
    }

    Ok(())
}
