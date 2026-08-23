use axum::{
    extract::{Path, State},
    response::Json,
};
use crate::AppState;
use crate::models::protocol::{SessionUpdate, SessionResponse};

pub async fn update_session(
    Path(_session_id): Path<String>,
    Json(payload): Json<SessionUpdate>,
) -> Json<SessionResponse> {
    Json(SessionResponse {
        resuming: payload.resuming.unwrap_or(false),
        timeout: payload.timeout.unwrap_or(60),
    })
}
