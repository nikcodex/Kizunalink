use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::Json,
};
use serde::Deserialize;
use crate::AppState;
use crate::models::protocol::{PlayerResponse, PlayerUpdatePayload};
use crate::rest::error::LavalinkError;
use tracing::error;

#[derive(Deserialize, Default)]
pub struct NoReplaceQuery {
    #[serde(rename = "noReplace")]
    pub no_replace: Option<bool>,
}

pub async fn get_players(
    State(state): State<AppState>,
    Path(_session_id): Path<String>,
) -> Json<serde_json::Value> {
    let players = state.player_manager.get_all_players().await;
    Json(serde_json::json!({ "players": players }))
}

pub async fn get_player(
    State(state): State<AppState>,
    Path((_session_id, guild_id)): Path<(String, String)>,
) -> Result<Json<PlayerResponse>, LavalinkError> {
    match state.player_manager.get_player(&guild_id).await {
        Some(player) => Ok(Json(player)),
        None => Err(LavalinkError::new(
            StatusCode::NOT_FOUND,
            format!("Player not found for guild: {}", guild_id),
            format!("/v4/sessions/_/players/{}", guild_id),
        )),
    }
}

pub async fn update_player(
    State(state): State<AppState>,
    Path((_session_id, guild_id)): Path<(String, String)>,
    Query(query): Query<NoReplaceQuery>,
    Json(payload): Json<PlayerUpdatePayload>,
) -> Result<Json<PlayerResponse>, LavalinkError> {
    if query.no_replace.unwrap_or(false) {
        if let Some(player) = state.player_manager.get_player(&guild_id).await {
            if player.track.is_some() {
                return Ok(Json(player));
            }
        }
    }

    match state.player_manager.update_player(&guild_id, payload).await {
        Ok(player) => Ok(Json(player)),
        Err(e) => {
            error!("Player update failed for guild {}: {}", guild_id, e);
            Err(LavalinkError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                e.to_string(),
                format!("/v4/sessions/_/players/{}", guild_id),
            ))
        }
    }
}

pub async fn destroy_player(
    State(state): State<AppState>,
    Path((_session_id, guild_id)): Path<(String, String)>,
) -> Result<StatusCode, LavalinkError> {
    if state.player_manager.destroy_player(&guild_id) {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(LavalinkError::new(
            StatusCode::NOT_FOUND,
            format!("Player not found for guild: {}", guild_id),
            format!("/v4/sessions/_/players/{}", guild_id),
        ))
    }
}
