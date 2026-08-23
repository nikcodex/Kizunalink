use axum::{
    extract::{Path, State},
    http::HeaderMap,
    response::Json,
};

use crate::models::protocol::{SessionResponse, SessionUpdate};
use crate::rest::auth::require_auth;
use crate::rest::error::LavalinkError;
use crate::AppState;

pub async fn update_session(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(session_id): Path<String>,
    Json(payload): Json<SessionUpdate>,
) -> Result<Json<SessionResponse>, LavalinkError> {
    let path = format!("/v4/sessions/{}", session_id);
    require_auth(&headers, &state.password, &path)?;

    Ok(Json(SessionResponse {
        resuming: payload.resuming.unwrap_or(false),
        timeout: payload.timeout.unwrap_or(60),
    }))
}
