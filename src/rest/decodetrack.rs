use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::Json,
    Json as JsonBody,
};
use serde::Deserialize;
use crate::AppState;
use crate::models::track::{LavalinkTrack, TrackInfo};
use crate::rest::error::LavalinkError;

#[derive(Deserialize)]
pub struct DecodeTrackQuery {
    #[serde(rename = "encodedTrack")]
    pub encoded_track: Option<String>,
}

pub async fn decode_track(
    Query(query): Query<DecodeTrackQuery>,
    State(_state): State<AppState>,
) -> Result<Json<LavalinkTrack>, LavalinkError> {
    let encoded = match query.encoded_track {
        Some(e) if !e.trim().is_empty() => e,
        _ => return Err(LavalinkError::new(StatusCode::BAD_REQUEST, "Missing parameter 'encodedTrack'", "/v4/decodetrack")),
    };

    Ok(Json(crate::util::decode_track(&encoded)))
}

#[derive(Deserialize)]
pub struct DecodeTracksRequest {
    pub tracks: Vec<String>,
}

#[derive(serde::Serialize)]
pub struct DecodeTracksResponse {
    pub tracks: Vec<LavalinkTrack>,
}

pub async fn decode_tracks(
    State(_state): State<AppState>,
    JsonBody(payload): JsonBody<DecodeTracksRequest>,
) -> Json<DecodeTracksResponse> {
    Json(DecodeTracksResponse {
        tracks: payload
            .tracks
            .iter()
            .map(|e| crate::util::decode_track(e))
            .collect(),
    })
}
