// Copyright (c) 2026 nikcodex (KizunaLink)
// Licensed under the MIT License

use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
};

use crate::server::AppState;

/// GET /v4/sessions/{sessionId}/players/{guildId}/sponsorblock/categories
///
/// Mirror of the `SponsorBlock-Plugin` endpoint. Returns the set of categories
/// the player currently skips.
pub async fn get_categories(
    Path((session_id, guild_id)): Path<(String, String)>,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    let session_id = kizunalink::common::types::SessionId(session_id);
    let guild_id = kizunalink::common::types::GuildId(guild_id);

    let Some(session) = state.sessions.get(&session_id) else {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "Session not found" })),
        )
            .into_response();
    };
    let Some(player_arc) = session.players.get(&guild_id).map(|kv| kv.value().clone()) else {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "Player not found" })),
        )
            .into_response();
    };

    let player = player_arc.read().await;
    let categories = player.config.sponsorblock.categories.clone();
    (StatusCode::OK, Json(categories)).into_response()
}

/// PUT /v4/sessions/{sessionId}/players/{guildId}/sponsorblock/categories
///
/// Replaces the set of categories the player skips. Must be a subset of the
/// categories the SponsorBlock API knows.
pub async fn set_categories(
    Path((session_id, guild_id)): Path<(String, String)>,
    State(state): State<Arc<AppState>>,
    Json(body): Json<Vec<String>>,
) -> impl IntoResponse {
    let session_id = kizunalink::common::types::SessionId(session_id);
    let guild_id = kizunalink::common::types::GuildId(guild_id);

    let Some(session) = state.sessions.get(&session_id) else {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "Session not found" })),
        )
            .into_response();
    };
    let Some(player_arc) = session.players.get(&guild_id).map(|kv| kv.value().clone()) else {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "Player not found" })),
        )
            .into_response();
    };

    let cleaned: Vec<String> = body
        .into_iter()
        .filter(|c| {
            kizunalink::lavalink::sponsorblock::KNOWN_CATEGORIES
                .iter()
                .any(|k| k == c)
        })
        .collect();

    let mut player = player_arc.write().await;
    player.config.sponsorblock.categories = cleaned.clone();
    (StatusCode::OK, Json(cleaned)).into_response()
}

/// DELETE /v4/sessions/{sessionId}/players/{guildId}/sponsorblock/categories
///
/// Disables SponsorBlock for this player by clearing all categories.
pub async fn delete_categories(
    Path((session_id, guild_id)): Path<(String, String)>,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    let session_id = kizunalink::common::types::SessionId(session_id);
    let guild_id = kizunalink::common::types::GuildId(guild_id);

    let Some(session) = state.sessions.get(&session_id) else {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "Session not found" })),
        )
            .into_response();
    };
    let Some(player_arc) = session.players.get(&guild_id).map(|kv| kv.value().clone()) else {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "Player not found" })),
        )
            .into_response();
    };

    let mut player = player_arc.write().await;
    player.config.sponsorblock.categories.clear();
    player.config.sponsorblock.enabled = false;
    (StatusCode::OK, Json(serde_json::json!({}))).into_response()
}

/// GET /v4/sessions/{sessionId}/players/{guildId}/sponsorblock/segments
///
/// Returns the currently-cached segments for the active track (or `[]`).
pub async fn get_segments(
    Path((session_id, guild_id)): Path<(String, String)>,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    let session_id = kizunalink::common::types::SessionId(session_id);
    let guild_id = kizunalink::common::types::GuildId(guild_id);

    let Some(session) = state.sessions.get(&session_id) else {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "Session not found" })),
        )
            .into_response();
    };
    let Some(player_arc) = session.players.get(&guild_id).map(|kv| kv.value().clone()) else {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "Player not found" })),
        )
            .into_response();
    };

    let player = player_arc.read().await;
    let segments = player.sponsorblock.segments.read().await.clone();
    (StatusCode::OK, Json(segments)).into_response()
}
