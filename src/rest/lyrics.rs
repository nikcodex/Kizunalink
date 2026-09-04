use axum::{
    extract::{Path, Query, State},
    http::HeaderMap,
    response::Json,
};
use serde::{Deserialize, Serialize};

use crate::rest::auth::require_auth;
use crate::rest::error::LavalinkError;
use crate::AppState;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LyricLine {
    pub timestamp: u64,
    pub line: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LyricsResponse {
    pub name: String,
    pub artist: String,
    pub source: String,
    pub text: String,
    pub lines: Vec<LyricLine>,
    pub synced: bool,
}

#[derive(Deserialize)]
pub struct LyricsQuery {
    pub query: Option<String>,
    pub track: Option<String>,
    #[serde(rename = "trackName")]
    pub track_name: Option<String>,
    #[serde(rename = "artistName")]
    pub artist_name: Option<String>,
}

/// GET /v4/lyrics — Query synchronized or plain lyrics by query or encoded track.
pub async fn get_lyrics_query(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query_params): Query<LyricsQuery>,
) -> Result<Json<LyricsResponse>, LavalinkError> {
    require_auth(&headers, &state.password, "/v4/lyrics")?;

    if let Some(ref enc) = query_params.track {
        if let Ok(track) = crate::track_encoding::decode_track(enc) {
            return resolve_lyrics_internal(
                &state,
                &track.info.title,
                &track.info.author,
                &track.info.identifier,
            )
            .await;
        }
    }

    if let (Some(ref t), Some(ref a)) = (&query_params.track_name, &query_params.artist_name) {
        return resolve_lyrics_internal(&state, t, a, "").await;
    }

    if let Some(ref q) = query_params.query {
        if !q.trim().is_empty() {
            return resolve_lyrics_internal(&state, q, "", "").await;
        }
    }

    Err(LavalinkError::new(
        axum::http::StatusCode::BAD_REQUEST,
        "Missing required query parameter: 'query', 'track', or 'trackName' & 'artistName'",
        "/v4/lyrics",
    ))
}

/// GET /v4/lyrics/:song_id — Legacy endpoint: fetch lyrics by JioSaavn song ID or query.
pub async fn get_lyrics(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(song_id): Path<String>,
) -> Result<Json<LyricsResponse>, LavalinkError> {
    require_auth(&headers, &state.password, "/v4/lyrics")?;

    if song_id.trim().is_empty() {
        return Err(LavalinkError::new(
            axum::http::StatusCode::BAD_REQUEST,
            "Song ID cannot be empty",
            "/v4/lyrics",
        ));
    }

    resolve_lyrics_internal(&state, &song_id, "", &song_id).await
}

/// GET /v4/sessions/:session_id/players/:guild_id/track/lyrics — Fetch lyrics for currently playing track.
pub async fn get_player_current_lyrics(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((_session_id, guild_id)): Path<(String, String)>,
) -> Result<Json<LyricsResponse>, LavalinkError> {
    require_auth(
        &headers,
        &state.password,
        "/v4/sessions/:session_id/players/:guild_id/track/lyrics",
    )?;

    let player = state
        .player_manager
        .get_player(&guild_id)
        .await
        .ok_or_else(|| {
            LavalinkError::new(
                axum::http::StatusCode::NOT_FOUND,
                format!("Player not found for guild: {}", guild_id),
                "/v4/sessions/:session_id/players/:guild_id/track/lyrics",
            )
        })?;

    let track = player.track.ok_or_else(|| {
        LavalinkError::new(
            axum::http::StatusCode::NOT_FOUND,
            "Player has no track currently playing",
            "/v4/sessions/:session_id/players/:guild_id/track/lyrics",
        )
    })?;

    resolve_lyrics_internal(
        &state,
        &track.info.title,
        &track.info.author,
        &track.info.identifier,
    )
    .await
}

/// Core internal lyrics resolver: tries LRCLIB with precise (title, artist),
/// falls back to LRCLIB general search, and finally falls back to JioSaavn.
async fn resolve_lyrics_internal(
    state: &AppState,
    title: &str,
    artist: &str,
    identifier: &str,
) -> Result<Json<LyricsResponse>, LavalinkError> {
    let client = crate::config::http_client();

    // 1. Try LRCLIB exact match if artist is provided
    if !artist.is_empty() {
        if let Some(lyrics) = fetch_lrclib_lyrics(&client, title, artist).await {
            return Ok(Json(lyrics));
        }
    }

    // 2. Try LRCLIB search query
    let search_query = if !artist.is_empty() {
        format!("{} {}", title, artist)
    } else {
        title.to_string()
    };

    if let Some(lyrics) = search_lrclib_lyrics(&client, &search_query).await {
        return Ok(Json(lyrics));
    }

    // 3. Try JioSaavn by identifier or title
    let js_id = if !identifier.is_empty() {
        identifier
    } else {
        title
    };

    if let Ok(Some(js_lyrics)) = state.jiosaavn.get_lyrics(js_id).await {
        let lines = js_lyrics
            .lines()
            .enumerate()
            .map(|(i, l)| LyricLine {
                timestamp: (i as u64) * 3000,
                line: l.trim().to_string(),
            })
            .collect();
        return Ok(Json(LyricsResponse {
            name: title.to_string(),
            artist: artist.to_string(),
            source: "jiosaavn".to_string(),
            text: js_lyrics,
            lines,
            synced: false,
        }));
    }

    Err(LavalinkError::new(
        axum::http::StatusCode::NOT_FOUND,
        format!("No lyrics found for '{}'", title),
        "/v4/lyrics",
    ))
}

async fn fetch_lrclib_lyrics(
    client: &reqwest::Client,
    title: &str,
    artist: &str,
) -> Option<LyricsResponse> {
    let url = format!(
        "https://lrclib.net/api/get?track_name={}&artist_name={}",
        urlencoding::encode(title),
        urlencoding::encode(artist)
    );

    let res = client
        .get(&url)
        .header("User-Agent", "KizunaLink/4.2 (Audio-Engine)")
        .send()
        .await
        .ok()?;

    if !res.status().is_success() {
        return None;
    }

    let json: serde_json::Value = res.json().await.ok()?;
    let name = json
        .get("name")
        .or_else(|| json.get("trackName"))
        .and_then(|v| v.as_str())
        .unwrap_or(title)
        .to_string();
    let artist_str = json
        .get("artistName")
        .and_then(|v| v.as_str())
        .unwrap_or(artist)
        .to_string();
    let synced_lrc = json.get("syncedLyrics").and_then(|v| v.as_str());
    let plain_lrc = json.get("plainLyrics").and_then(|v| v.as_str());

    if synced_lrc.is_none() && plain_lrc.is_none() {
        return None;
    }

    let (lines, synced) = if let Some(lrc) = synced_lrc {
        (parse_lrc(lrc), true)
    } else if let Some(plain) = plain_lrc {
        let lines = plain
            .lines()
            .enumerate()
            .map(|(i, l)| LyricLine {
                timestamp: (i as u64) * 3000,
                line: l.trim().to_string(),
            })
            .collect();
        (lines, false)
    } else {
        (Vec::new(), false)
    };

    let text = plain_lrc.map(|s| s.to_string()).unwrap_or_else(|| {
        lines
            .iter()
            .map(|l| l.line.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    });

    Some(LyricsResponse {
        name,
        artist: artist_str,
        source: "lrclib".to_string(),
        text,
        lines,
        synced,
    })
}

async fn search_lrclib_lyrics(client: &reqwest::Client, query: &str) -> Option<LyricsResponse> {
    let url = format!(
        "https://lrclib.net/api/search?q={}",
        urlencoding::encode(query)
    );

    let res = client
        .get(&url)
        .header("User-Agent", "KizunaLink/4.2 (Audio-Engine)")
        .send()
        .await
        .ok()?;

    if !res.status().is_success() {
        return None;
    }

    let items: Vec<serde_json::Value> = res.json().await.ok()?;
    let first = items.into_iter().next()?;

    let name = first
        .get("name")
        .or_else(|| first.get("trackName"))
        .and_then(|v| v.as_str())
        .unwrap_or(query)
        .to_string();
    let artist_str = first
        .get("artistName")
        .and_then(|v| v.as_str())
        .unwrap_or("Unknown")
        .to_string();
    let synced_lrc = first.get("syncedLyrics").and_then(|v| v.as_str());
    let plain_lrc = first.get("plainLyrics").and_then(|v| v.as_str());

    if synced_lrc.is_none() && plain_lrc.is_none() {
        return None;
    }

    let (lines, synced) = if let Some(lrc) = synced_lrc {
        (parse_lrc(lrc), true)
    } else if let Some(plain) = plain_lrc {
        let lines = plain
            .lines()
            .enumerate()
            .map(|(i, l)| LyricLine {
                timestamp: (i as u64) * 3000,
                line: l.trim().to_string(),
            })
            .collect();
        (lines, false)
    } else {
        (Vec::new(), false)
    };

    let text = plain_lrc.map(|s| s.to_string()).unwrap_or_else(|| {
        lines
            .iter()
            .map(|l| l.line.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    });

    Some(LyricsResponse {
        name,
        artist: artist_str,
        source: "lrclib".to_string(),
        text,
        lines,
        synced,
    })
}

pub fn parse_lrc(lrc: &str) -> Vec<LyricLine> {
    let mut lines = Vec::new();
    for raw_line in lrc.lines() {
        let trimmed = raw_line.trim();
        if let Some(rest) = trimmed.strip_prefix('[') {
            if let Some((time_part, text)) = rest.split_once(']') {
                let (min_part, sec_part) = if let Some((m, s)) = time_part.split_once(':') {
                    (m, s)
                } else {
                    // Fallback for timestamps using dots instead of colon, e.g. "01.23.45"
                    let dot_parts: Vec<&str> = time_part.split('.').collect();
                    if dot_parts.len() >= 3 {
                        (
                            dot_parts[0],
                            time_part
                                .strip_prefix(dot_parts[0])
                                .unwrap_or("")
                                .trim_start_matches('.'),
                        )
                    } else {
                        continue;
                    }
                };

                // Clean min part (strip any extra dots)
                let min_str = min_part.split('.').next().unwrap_or(min_part);

                // Handle malformed extra dots in seconds gracefully (e.g. "23.45.67" -> "23.45", "23..45" -> "23.45")
                let sec_cleaned = {
                    let non_empty: Vec<&str> =
                        sec_part.split('.').filter(|s| !s.is_empty()).collect();
                    match non_empty.len() {
                        0 => "0".to_string(),
                        1 => non_empty[0].to_string(),
                        _ => format!("{}.{}", non_empty[0], non_empty[1]),
                    }
                };

                if let (Ok(min), Ok(sec)) = (min_str.parse::<f64>(), sec_cleaned.parse::<f64>()) {
                    if !min.is_finite()
                        || !sec.is_finite()
                        || min < 0.0
                        || min > 99.0
                        || sec < 0.0
                        || sec > 59.99
                    {
                        continue;
                    }
                    let ms = ((min * 60.0 + sec) * 1000.0).round() as u64;
                    lines.push(LyricLine {
                        timestamp: ms,
                        line: text.trim().to_string(),
                    });
                }
            }
        }
    }
    lines
}
