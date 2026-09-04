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
    #[serde(rename = "noReplace", alias = "no_replace")]
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

    if state.session_manager.get_session(&session_id).is_none() {
        return Err(LavalinkError::new(
            StatusCode::NOT_FOUND,
            format!("Session not found: {}", session_id),
            path,
        ));
    }

    // Only players owned by this session are visible to it.
    let players = state
        .player_manager
        .get_players_for_session(&session_id)
        .await;
    Ok(Json(players))
}

pub async fn get_player(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((session_id, guild_id)): Path<(String, String)>,
) -> Result<Json<PlayerResponse>, LavalinkError> {
    let path = format!("/v4/sessions/{}/players/{}", session_id, guild_id);
    require_auth(&headers, &state.password, &path)?;

    if state.session_manager.get_session(&session_id).is_none() {
        return Err(LavalinkError::new(
            StatusCode::NOT_FOUND,
            format!("Session not found: {}", session_id),
            &path,
        ));
    }

    if let Err(e) = security::validate_guild_id(&guild_id) {
        return Err(LavalinkError::new(StatusCode::BAD_REQUEST, e, path));
    }

    // The player must belong to this session — a player owned by another session
    // is indistinguishable from a missing one.
    match state
        .player_manager
        .get_player_for_session(&guild_id, &session_id)
        .await
    {
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

    if state.session_manager.get_session(&session_id).is_none() {
        return Err(LavalinkError::new(
            StatusCode::NOT_FOUND,
            format!("Session not found: {}", session_id),
            &path,
        ));
    }

    // Rate limit check
    let ip = extract_ip(&headers, "0.0.0.0");
    if !state.rate_limiter.check(&ip) {
        return Err(LavalinkError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "Rate limit exceeded",
            &path,
        )
        .with_retry_after(state.rate_limiter.window_secs()));
    }

    // Validate guild ID
    if let Err(e) = security::validate_guild_id(&guild_id) {
        return Err(LavalinkError::new(StatusCode::BAD_REQUEST, e, &path));
    }

    let no_replace = query.no_replace.unwrap_or(false);

    // Atomically claim (create) or reuse the player for this session. If the
    // guild's player is owned by a different session, this fails — a PATCH never
    // establishes ownership over another session's player.
    match state
        .player_manager
        .update_player(&guild_id, payload, no_replace, &session_id)
        .await
    {
        Ok(player) => {
            state.session_manager.add_guild(&session_id, &guild_id);
            Ok(Json(player))
        }
        Err(crate::player::manager::PlayerManagerError::NotFound(_)) => Err(LavalinkError::new(
            StatusCode::NOT_FOUND,
            format!("Player not found for guild: {}", guild_id),
            path,
        )),
        Err(crate::player::manager::PlayerManagerError::LimitReached(n)) => {
            error!("Player limit reached for guild {}", guild_id);
            Err(LavalinkError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                format!("Player limit reached: maximum {} players allowed", n),
                path,
            ))
        }
        Err(e) => {
            error!("Player update failed for guild {}: {}", guild_id, e);
            Err(LavalinkError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                e.to_string(),
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

    if state.session_manager.get_session(&session_id).is_none() {
        return Err(LavalinkError::new(
            StatusCode::NOT_FOUND,
            format!("Session not found: {}", session_id),
            &path,
        ));
    }

    if let Err(e) = security::validate_guild_id(&guild_id) {
        return Err(LavalinkError::new(StatusCode::BAD_REQUEST, e, path));
    }

    // Destroy only the player owned by this session.
    match state
        .player_manager
        .destroy_player_for_session(&guild_id, &session_id)
    {
        Ok(()) => {
            state.session_manager.remove_guild(&session_id, &guild_id);
            Ok(StatusCode::NO_CONTENT)
        }
        Err(_) => Err(LavalinkError::new(
            StatusCode::NOT_FOUND,
            format!("Player not found for guild: {}", guild_id),
            path,
        )),
    }
}
