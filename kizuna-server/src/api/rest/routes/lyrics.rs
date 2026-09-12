// Copyright (c) 2026 nikcodex (KizunaLink)
// Licensed under the MIT License

use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, Query, State},
    response::IntoResponse,
};

use crate::{
    protocol::{
        models::{GetLyricsQuery, GetPlayerLyricsQuery, KizunaLinkLyrics, KizunaLinkLyricsLine},
        tracks::Track,
    },
    server::AppState,
};

pub async fn subscribe_lyrics(
    State(state): State<Arc<AppState>>,
    Path((session_id, guild_id)): Path<(String, String)>,
) -> axum::http::StatusCode {
    let session_id = kizunalink::common::types::SessionId(session_id);
    let guild_id = kizunalink::common::types::GuildId(guild_id);
    tracing::info!(
        "POST /v4/sessions/{}/players/{}/lyrics/subscribe",
        session_id,
        guild_id
    );

    let Some(session) = state.sessions.get(&session_id) else {
        return axum::http::StatusCode::NOT_FOUND;
    };

    let Some(player_arc) = session.players.get(&guild_id).map(|kv| kv.value().clone()) else {
        return axum::http::StatusCode::NOT_FOUND;
    };

    let player = player_arc.write().await;
    player.subscribe_lyrics();

    if let Some(track) = &player.track_info {
        let lyrics_data_arc = player.lyrics_data.clone();
        let lyrics_manager = state.lyrics_manager.clone();
        let track_info = track.info.clone();
        let session_clone = session.clone();
        let guild_id = player.guild_id.clone();

        tokio::spawn(async move {
            let has_lyrics = lyrics_data_arc.lock().await.is_some();
            if has_lyrics {
                return;
            }

            if let Some(lyrics) = lyrics_manager.load_lyrics(&track_info).await {
                {
                    *lyrics_data_arc.lock().await = Some(lyrics.clone());
                }

                let event = crate::lavalink::protocol::OutgoingMessage::Event {
                    event: Box::new(crate::lavalink::protocol::KizunaLinkEvent::LyricsFound {
                        guild_id,
                        lyrics: crate::lavalink::protocol::models::KizunaLinkLyrics {
                            source_name: track_info.source_name.clone(),
                            provider: Some(lyrics.provider),
                            text: Some(lyrics.text),
                            lines: lyrics.lines.map(|lines| {
                                lines
                                    .into_iter()
                                    .map(|l| crate::lavalink::protocol::models::KizunaLinkLyricsLine {
                                        timestamp: l.timestamp,
                                        duration: Some(l.duration),
                                        line: l.text,
                                        plugin: serde_json::json!({}),
                                    })
                                    .collect()
                            }),
                            plugin: serde_json::json!({}),
                        },
                    }),
                };
                session_clone.send_message(&event);
            } else {
                let event = crate::lavalink::protocol::OutgoingMessage::Event {
                    event: Box::new(crate::lavalink::protocol::KizunaLinkEvent::LyricsNotFound { guild_id }),
                };
                session_clone.send_message(&event);
            }
        });
    }

    axum::http::StatusCode::NO_CONTENT
}

pub async fn unsubscribe_lyrics(
    State(state): State<Arc<AppState>>,
    Path((session_id, guild_id)): Path<(String, String)>,
) -> axum::http::StatusCode {
    let session_id = kizunalink::common::types::SessionId(session_id);
    let guild_id = kizunalink::common::types::GuildId(guild_id);
    tracing::info!(
        "DELETE /v4/sessions/{}/players/{}/lyrics/unsubscribe",
        session_id,
        guild_id
    );

    let Some(session) = state.sessions.get(&session_id) else {
        return axum::http::StatusCode::NOT_FOUND;
    };

    let Some(player_arc) = session.players.get(&guild_id).map(|kv| kv.value().clone()) else {
        return axum::http::StatusCode::NOT_FOUND;
    };

    let player = player_arc.write().await;
    player.unsubscribe_lyrics();
    axum::http::StatusCode::NO_CONTENT
}

pub async fn get_lyrics(
    State(state): State<Arc<AppState>>,
    Query(query): Query<GetLyricsQuery>,
) -> impl IntoResponse {
    tracing::info!(
        "GET /v4/lyrics: track='{}', skipTrackSource={}",
        query.track,
        query.skip_track_source
    );
    let track = match Track::decode(&query.track) {
        Some(t) => t,
        None => {
            return (axum::http::StatusCode::BAD_REQUEST, "Invalid encoded track").into_response();
        }
    };

    match state
        .lyrics_manager
        .load_lyrics_ext(&track.info, query.skip_track_source)
        .await
    {
        Some(lyrics) => {
            let response = KizunaLinkLyrics {
                source_name: track.info.source_name.clone(),
                provider: Some(lyrics.provider),
                text: Some(lyrics.text),
                lines: lyrics
                    .lines
                    .map(|lines: Vec<crate::lavalink::protocol::models::LyricsLine>| {
                        lines
                            .into_iter()
                            .map(|l| KizunaLinkLyricsLine {
                                timestamp: l.timestamp,
                                duration: Some(l.duration),
                                line: l.text,
                                plugin: serde_json::json!({}),
                            })
                            .collect()
                    }),
                plugin: serde_json::json!({}),
            };
            Json(response).into_response()
        }
        None => axum::http::StatusCode::NO_CONTENT.into_response(),
    }
}

pub async fn get_player_lyrics(
    State(state): State<Arc<AppState>>,
    Path((session_id, guild_id)): Path<(String, String)>,
    Query(query): Query<GetPlayerLyricsQuery>,
) -> impl IntoResponse {
    let session_id = kizunalink::common::types::SessionId(session_id);
    let guild_id = kizunalink::common::types::GuildId(guild_id);
    tracing::info!(
        "GET /v4/sessions/{}/players/{}/track/lyrics",
        session_id,
        guild_id
    );

    let Some(session) = state.sessions.get(&session_id) else {
        return axum::http::StatusCode::NOT_FOUND.into_response();
    };

    let Some(player_arc) = session.players.get(&guild_id).map(|kv| kv.value().clone()) else {
        return axum::http::StatusCode::NOT_FOUND.into_response();
    };

    let player = player_arc.read().await;

    let lyrics_data = player.lyrics_data.lock().await;
    match lyrics_data.as_ref() {
        Some(lyrics) => {
            let track_info = player.track_info.as_ref().map(|t| &t.info);
            let response = KizunaLinkLyrics {
                source_name: track_info.map(|i| i.source_name.clone()).unwrap_or_default(),
                provider: Some(lyrics.provider.clone()),
                text: Some(lyrics.text.clone()),
                lines: lyrics.lines.as_ref().map(|lines| {
                    lines
                        .iter()
                        .map(|l| KizunaLinkLyricsLine {
                            timestamp: l.timestamp,
                            duration: Some(l.duration),
                            line: l.text.clone(),
                            plugin: serde_json::json!({}),
                        })
                        .collect()
                }),
                plugin: serde_json::json!({}),
            };
            (axum::http::StatusCode::OK, Json(response)).into_response()
        }
        None => axum::http::StatusCode::NO_CONTENT.into_response(),
    }
}
