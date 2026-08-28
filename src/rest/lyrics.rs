use axum::{
    extract::{Path, State},
    http::HeaderMap,
    response::Json,
};

use crate::rest::auth::require_auth;
use crate::rest::error::LavalinkError;
use crate::AppState;

/// GET /v4/lyrics/:id — Fetch lyrics for a JioSaavn song by ID.
///
/// Returns the lyrics as plain text or a 404-style error if not found.
pub async fn get_lyrics(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(song_id): Path<String>,
) -> Result<Json<serde_json::Value>, LavalinkError> {
    require_auth(&headers, &state.password, "/v4/lyrics")?;

    if song_id.trim().is_empty() {
        return Err(LavalinkError::new(
            axum::http::StatusCode::BAD_REQUEST,
            "Song ID cannot be empty",
            "/v4/lyrics",
        ));
    }

    match state.jiosaavn.get_lyrics(&song_id).await {
        Ok(Some(lyrics)) => Ok(Json(serde_json::json!({
            "lyrics": lyrics,
            "source": "jiosaavn",
            "songId": song_id,
        }))),
        Ok(None) => Err(LavalinkError::new(
            axum::http::StatusCode::NOT_FOUND,
            format!("No lyrics found for song ID: {}", song_id),
            "/v4/lyrics",
        )),
        Err(e) => Err(LavalinkError::new(
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to fetch lyrics: {}", e),
            "/v4/lyrics",
        )),
    }
}
