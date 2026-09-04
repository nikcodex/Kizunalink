use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::Json,
};

use crate::models::protocol::{SessionResponse, SessionUpdate};
use crate::rest::auth::require_auth;
use crate::rest::error::LavalinkError;
use crate::security;
use crate::AppState;

pub async fn update_session(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(session_id): Path<String>,
    Json(payload): Json<SessionUpdate>,
) -> Result<Json<SessionResponse>, LavalinkError> {
    let path = format!("/v4/sessions/{}", session_id);
    require_auth(&headers, &state.password, &path)?;

    if let Err(e) = security::validate_session_id(&session_id) {
        return Err(LavalinkError::new(StatusCode::BAD_REQUEST, e, path));
    }

    let (resuming, timeout) = state
        .session_manager
        .update_session(&session_id, payload.resuming, payload.timeout);

    Ok(Json(SessionResponse { resuming, timeout }))
}
