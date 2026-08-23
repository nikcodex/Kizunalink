use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
};

use crate::rest::auth::require_auth;
use crate::rest::error::LavalinkError;
use crate::AppState;

pub async fn get_routeplanner_status(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<StatusCode, LavalinkError> {
    require_auth(&headers, &state.password, "/v4/routeplanner/status")?;
    // Real Lavalink nodes return 204 No Content when routeplanner is not configured/disabled
    Ok(StatusCode::NO_CONTENT)
}

pub async fn free_routeplanner_address(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<StatusCode, LavalinkError> {
    require_auth(&headers, &state.password, "/v4/routeplanner/free/address")?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn free_routeplanner_all(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<StatusCode, LavalinkError> {
    require_auth(&headers, &state.password, "/v4/routeplanner/free/all")?;
    Ok(StatusCode::NO_CONTENT)
}
