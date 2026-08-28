use axum::{
    extract::{Path, State},
    http::HeaderMap,
    response::Json,
};

use crate::models::protocol::{SessionResponse, SessionUpdate};
use crate::rest::auth::require_auth;
use crate::rest::error::LavalinkError;
use crate::AppState;

use crate::ws::handler::update_session_state;

pub async fn update_session(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(session_id): Path<String>,
    Json(payload): Json<SessionUpdate>,
) -> Result<Json<SessionResponse>, LavalinkError> {
    let path = format!("/v4/sessions/{}", session_id);
    require_auth(&headers, &state.password, &path)?;

    let (resuming, timeout) = update_session_state(
        &session_id,
        payload.resuming,
        payload.timeout,
    );

    Ok(Json(SessionResponse {
        resuming,
        timeout,
    }))
}
