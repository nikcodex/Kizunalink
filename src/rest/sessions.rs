use axum::{
    extract::State,
    http::HeaderMap,
    response::Json,
};

use crate::rest::auth::require_auth;
use crate::rest::error::LavalinkError;
use crate::ws::handler::get_session_ids;
use crate::AppState;

/// GET /v4/sessions — List all active WebSocket session IDs.
pub async fn list_sessions(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, LavalinkError> {
    require_auth(&headers, &state.password, "/v4/sessions")?;

    let sessions: Vec<String> = get_session_ids()
        .into_iter()
        .collect();

    Ok(Json(serde_json::json!({
        "sessions": sessions,
        "count": sessions.len(),
    })))
}
