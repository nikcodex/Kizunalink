use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::Json,
};
use serde::Deserialize;
use tracing::error;

use crate::models::protocol::{PlayerResponse, PlayerUpdatePayload};
use crate::rest::auth::require_auth;
use crate::rest::error::LavalinkError;
use crate::AppState;

#[derive(Deserialize, Default)]
pub struct NoReplaceQuery {
    #[serde(rename = "noReplace")]
    pub no_replace: Option<bool>,
}

pub async fn get_players(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(session_id): Path<String>,
) -> Result<Json<Vec<PlayerResponse>>, LavalinkError> {
    let path = format!("/v4/sessions/{}/players", session_id);
    require_auth(&headers, &state.password, &path)?;

    let players = state.player_manager.get_all_players().await;
    Ok(Json(players))
}

pub async fn get_player(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((session_id, guild_id)): Path<(String, String)>,
) -> Result<Json<PlayerResponse>, LavalinkError> {
    let path = format!("/v4/sessions/{}/players/{}", session_id, guild_id);
    require_auth(&headers, &state.password, &path)?;

    match state.player_manager.get_player(&guild_id).await {
        Some(player) => Ok(Json(player)),
        None => Err(LavalinkError::new(
            StatusCode::NOT_FOUND,
            format!("Player not found for guild: {}", guild_id),
            path,
        )),
    }
}

pub async fn update_player(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((session_id, guild_id)): Path<(String, String)>,
    Query(query): Query<NoReplaceQuery>,
    Json(payload): Json<PlayerUpdatePayload>,
) -> Result<Json<PlayerResponse>, LavalinkError> {
    let path = format!("/v4/sessions/{}/players/{}", session_id, guild_id);
    require_auth(&headers, &state.password, &path)?;

    let no_replace = query.no_replace.unwrap_or(false);

    match state
        .player_manager
        .update_player(&guild_id, payload, no_replace)
        .await
    {
        Ok(player) => Ok(Json(player)),
        Err(e) => {
            error!("Player update failed for guild {}: {}", guild_id, e);
            Err(LavalinkError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                e,
                path,
            ))
        }
    }
}

pub async fn destroy_player(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((session_id, guild_id)): Path<(String, String)>,
) -> Result<StatusCode, LavalinkError> {
    let path = format!("/v4/sessions/{}/players/{}", session_id, guild_id);
    require_auth(&headers, &state.password, &path)?;

    if state.player_manager.destroy_player(&guild_id) {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(LavalinkError::new(
            StatusCode::NOT_FOUND,
            format!("Player not found for guild: {}", guild_id),
            path,
        ))
    }
}
