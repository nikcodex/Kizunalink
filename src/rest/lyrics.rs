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
        let Some(rest) = trimmed.strip_prefix('[') else {
            continue;
        };
        let Some((time_part, text)) = rest.split_once(']') else {
            continue;
        };

        let (min_str, sec_rest) = if let Some((m, s)) = time_part.split_once(':') {
            (m, s)
        } else if let Some((m, s)) = time_part.split_once('.') {
            (m, s)
        } else {
            continue;
        };

        let Ok(minutes) = min_str.trim().parse::<u64>() else {
            continue;
        };
        if minutes > 999 {
            continue;
        }

        let (seconds, millis) = if let Some((sec_s, frac_s)) = sec_rest.split_once('.') {
            let Ok(s) = sec_s.trim().parse::<u64>() else {
                continue;
            };
            if s >= 60 {
                continue;
            }
            let frac_clean: String = frac_s.chars().filter(|c| c.is_ascii_digit()).collect();
            let ms = match frac_clean.len() {
                0 => 0,
                1 => frac_clean.parse::<u64>().unwrap_or(0) * 100,
                2 => frac_clean.parse::<u64>().unwrap_or(0) * 10,
                3 => frac_clean[..3].parse::<u64>().unwrap_or(0),
                _ => frac_clean[..3].parse::<u64>().unwrap_or(0),
            };
            (s, ms)
        } else {
            let Ok(s) = sec_rest.trim().parse::<u64>() else {
                continue;
            };
            if s >= 60 {
                continue;
            }
            (s, 0)
        };

        let total_ms = minutes
            .saturating_mul(60)
            .saturating_mul(1000)
            .saturating_add(seconds.saturating_mul(1000))
            .saturating_add(millis);

        lines.push(LyricLine {
            timestamp: total_ms,
            line: text.trim().to_string(),
        });
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_lrc_standard_timestamps() {
        let lrc = "[00:12.34]Line 1\n[01:02.500]Line 2\n[02:00]Line 3";
        let parsed = parse_lrc(lrc);
        assert_eq!(parsed.len(), 3);
        assert_eq!(parsed[0].timestamp, 12340);
        assert_eq!(parsed[0].line, "Line 1");
        assert_eq!(parsed[1].timestamp, 62500);
        assert_eq!(parsed[1].line, "Line 2");
        assert_eq!(parsed[2].timestamp, 120000);
        assert_eq!(parsed[2].line, "Line 3");
    }

    #[test]
    fn test_parse_lrc_rejects_invalid() {
        let lrc = "[invalid]No time\n[01:65.00]Invalid seconds\n[-01:20]Negative\n[00:05.12]Valid line";
        let parsed = parse_lrc(lrc);
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].timestamp, 5120);
        assert_eq!(parsed[0].line, "Valid line");
    }
}
