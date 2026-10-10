// Copyright (c) 2026 nikcodex (KizunaLink)
// Licensed under the MIT License

use std::sync::Arc;

use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Json},
};

use crate::api::rest::json::{ApiJson, ApiQuery};
use crate::{
    protocol,
    protocol::{models::*, tracks::Track},
    server::AppState,
};

/// GET /v4/loadtracks?identifier=...
pub async fn load_tracks(
    ApiQuery(params): ApiQuery<LoadTracksQuery>,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    let identifier = params.identifier;
    if identifier.len() > 16 * 1024 {
        return (
            StatusCode::BAD_REQUEST,
            Json(kizunalink::common::KizunaLinkError::bad_request(
                "Track identifier is too long",
                "/v4/loadtracks",
            )),
        )
            .into_response();
    }
    tracing::info!(
        "GET /v4/loadtracks: identifier_bytes={}, source_prefix={}",
        identifier.len(),
        identifier.split(':').next().unwrap_or("unknown")
    );

    (
        StatusCode::OK,
        Json(
            state
                .source_manager
                .load(&identifier, state.routeplanner.clone())
                .await,
        ),
    )
        .into_response()
}

pub async fn load_search(
    ApiQuery(params): ApiQuery<LoadSearchQuery>,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    let query = params.query;
    let types_str = params.types.unwrap_or_default();
    if query.len() > 4 * 1024 || types_str.len() > 512 {
        return (
            StatusCode::BAD_REQUEST,
            Json(kizunalink::common::KizunaLinkError::bad_request(
                "Search query is too long",
                "/v4/loadsearch",
            )),
        )
            .into_response();
    }

    tracing::info!(
        "GET /v4/loadsearch: query_bytes={}, type_count={}",
        query.len(),
        types_str
            .split(',')
            .filter(|s| !s.trim().is_empty())
            .count()
    );

    let types: Vec<String> = types_str
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .filter(|s| {
            matches!(
                s.as_str(),
                "track" | "album" | "artist" | "playlist" | "text"
            )
        })
        .collect();

    match state
        .source_manager
        .load_search(&query, &types, state.routeplanner.clone())
        .await
    {
        Some(result) => (StatusCode::OK, Json(result)).into_response(),
        None => StatusCode::NO_CONTENT.into_response(),
    }
}

pub async fn decode_track(ApiQuery(params): ApiQuery<DecodeTrackQuery>) -> impl IntoResponse {
    let encoded = params.encoded_track.clone().or(params.track);
    tracing::info!(
        "GET /v4/decodetrack: encoded_track_bytes={}",
        encoded.as_ref().map_or(0, String::len)
    );

    let Some(encoded) = encoded else {
        return (
            StatusCode::BAD_REQUEST,
            Json(kizunalink::common::KizunaLinkError::bad_request(
                "No track to decode provided",
                "/v4/decodetrack",
            )),
        )
            .into_response();
    };

    match Track::decode(&encoded) {
        Some(track) => (StatusCode::OK, Json(track)).into_response(),
        None => (
            StatusCode::BAD_REQUEST,
            Json(kizunalink::common::KizunaLinkError::bad_request(
                "Invalid track encoding",
                "/v4/decodetrack",
            )),
        )
            .into_response(),
    }
}

pub async fn decode_tracks(ApiJson(body): ApiJson<protocol::EncodedTracks>) -> impl IntoResponse {
    let tracks_input = body.0;
    tracing::info!("POST /v4/decodetracks: count={}", tracks_input.len());

    if tracks_input.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(kizunalink::common::KizunaLinkError::bad_request(
                "No tracks to decode provided",
                "/v4/decodetracks",
            )),
        )
            .into_response();
    }
    if tracks_input.len() > 256
        || tracks_input.iter().map(String::len).sum::<usize>() > 2 * 1024 * 1024
    {
        return (
            StatusCode::BAD_REQUEST,
            Json(kizunalink::common::KizunaLinkError::bad_request(
                "Too many or too-large tracks to decode",
                "/v4/decodetracks",
            )),
        )
            .into_response();
    }

    let mut tracks = Vec::with_capacity(tracks_input.len());
    for encoded in &tracks_input {
        let Some(t) = Track::decode(encoded) else {
            return (
                StatusCode::BAD_REQUEST,
                Json(kizunalink::common::KizunaLinkError::bad_request(
                    "Invalid track encoding",
                    "/v4/decodetracks",
                )),
            )
                .into_response();
        };
        tracks.push(t);
    }

    (StatusCode::OK, Json(tracks)).into_response()
}
