use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::Json,
};
use serde::Deserialize;
use tracing::error;

use crate::models::protocol::{PlayerResponse, PlayerUpdatePayload};
use crate::ratelimit::extract_ip;
use crate::rest::auth::require_auth;
use crate::rest::error::LavalinkError;
use crate::security;
use crate::AppState;

#[derive(Deserialize, Default)]
pub struct NoReplaceQuery {
    #[serde(rename = "noReplace")]
    pub no_replace: Option<bool>,
}

#[derive(Deserialize, Default)]
pub struct AllPlayersQuery {
    /// Filter to only players that are actively playing (track loaded and not paused).
    pub playing: Option<bool>,
    /// Filter to only players that are connected to a voice channel.
    pub connected: Option<bool>,
}

/// GET /v4/players/all — List all players across all sessions with optional filters.
pub async fn get_all_players(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<AllPlayersQuery>,
) -> Result<Json<serde_json::Value>, LavalinkError> {
    require_auth(&headers, &state.password, "/v4/players/all")?;

    let all = state.player_manager.get_all_players().await;

    let filtered: Vec<PlayerResponse> = all
        .into_iter()
        .filter(|p| {
            if let Some(playing) = query.playing {
                let is_playing = p.track.is_some() && !p.paused;
                if is_playing != playing {
                    return false;
                }
            }
            if let Some(connected) = query.connected {
                if p.state.connected != connected {
                    return false;
                }
            }
            true
        })
        .collect();

    Ok(Json(serde_json::json!({
        "players": filtered,
        "count": filtered.len(),
    })))
}

pub async fn get_players(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(session_id): Path<String>,
) -> Result<Json<Vec<PlayerResponse>>, LavalinkError> {
    let path = format!("/v4/sessions/{}/players", session_id);
    require_auth(&headers, &state.password, &path)?;

    // Single-session server: return all players.
    // In a multi-session setup, filter by session_id stored on each player.
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

    if let Err(e) = security::validate_guild_id(&guild_id) {
        return Err(LavalinkError::new(StatusCode::BAD_REQUEST, e, path));
    }

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

    // Rate limit check
    let ip = extract_ip(&headers, "0.0.0.0");
    if !state.rate_limiter.check(&ip) {
        return Err(LavalinkError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "Rate limit exceeded",
            &path,
        ));
    }

    // Validate guild ID
    if let Err(e) = security::validate_guild_id(&guild_id) {
        return Err(LavalinkError::new(StatusCode::BAD_REQUEST, e, &path));
    }

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

    if let Err(e) = security::validate_guild_id(&guild_id) {
        return Err(LavalinkError::new(StatusCode::BAD_REQUEST, e, path));
    }

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
