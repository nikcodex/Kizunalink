use axum::{
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
    response::Json,
};
use serde::Deserialize;

use crate::models::track::LavalinkTrack;
use crate::rest::auth::require_auth;
use crate::rest::error::LavalinkError;
use crate::security;
use crate::AppState;

const MAX_DECODE_BATCH_SIZE: usize = 100;

#[derive(Deserialize)]
pub struct DecodeTrackQuery {
    #[serde(rename = "encodedTrack")]
    pub encoded_track: Option<String>,
}

#[derive(Deserialize)]
#[serde(untagged)]
pub enum DecodeTracksPayload {
    Array(Vec<String>),
    Object { tracks: Vec<String> },
}

pub async fn decode_track(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<DecodeTrackQuery>,
) -> Result<Json<LavalinkTrack>, LavalinkError> {
    require_auth(&headers, &state.password, "/v4/decodetrack")?;

    let encoded = match query.encoded_track {
        Some(e) if !e.trim().is_empty() => e,
        _ => {
            return Err(LavalinkError::new(
                StatusCode::BAD_REQUEST,
                "Missing query parameter 'encodedTrack'",
                "/v4/decodetrack",
            ));
        }
    };

    match crate::track_encoding::decode_track(&encoded) {
        Ok(track) => Ok(Json(track)),
        Err(e) => Err(LavalinkError::new(
            StatusCode::BAD_REQUEST,
            format!("Failed to decode track: {}", e),
            "/v4/decodetrack",
        )),
    }
}

pub async fn decode_tracks(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<DecodeTracksPayload>,
) -> Result<Json<Vec<LavalinkTrack>>, LavalinkError> {
    require_auth(&headers, &state.password, "/v4/decodetracks")?;

    let track_strings = match payload {
        DecodeTracksPayload::Array(arr) => arr,
        DecodeTracksPayload::Object { tracks } => tracks,
    };

    if track_strings.len() > MAX_DECODE_BATCH_SIZE {
        return Err(LavalinkError::new(
            StatusCode::BAD_REQUEST,
            format!("Batch size {} exceeds maximum of {}", track_strings.len(), MAX_DECODE_BATCH_SIZE),
            "/v4/decodetracks",
        ));
    }

    let mut result = Vec::with_capacity(track_strings.len());
    for s in track_strings {
        match crate::track_encoding::decode_track(&s) {
            Ok(track) => result.push(track),
            Err(e) => {
                return Err(LavalinkError::new(
                    StatusCode::BAD_REQUEST,
                    format!("Failed to decode track: {}", e),
                    "/v4/decodetracks",
                ));
            }
        }
    }

    Ok(Json(result))
}
