use axum::{
    extract::{Query, State},
    http::HeaderMap,
    response::Json,
};
use serde::Deserialize;
use tracing::info;

use crate::models::track::LoadResult;
use crate::rest::auth::require_auth;
use crate::rest::error::LavalinkError;
use crate::AppState;

#[derive(Deserialize)]
pub struct LoadTracksQuery {
    pub identifier: Option<String>,
}

pub async fn load_tracks(
    state: State<AppState>,
    headers: HeaderMap,
    Query(query): Query<LoadTracksQuery>,
) -> Result<Json<LoadResult>, LavalinkError> {
    require_auth(&headers, &state.password, "/v4/loadtracks")?;

    let identifier = match query.identifier {
        Some(id) if !id.trim().is_empty() => id.trim().to_string(),
        _ => return Ok(Json(LoadResult::Empty)),
    };

    info!("Resolving track query: \"{}\"", identifier);

    // Direct HTTP(S) audio stream
    if (identifier.starts_with("http://") || identifier.starts_with("https://"))
        && is_direct_audio_url(&identifier)
    {
        return Ok(Json(LoadResult::Track(
            crate::util::create_http_track(&identifier),
        )));
    }

    // Recommendation prefixes
    if let Some(stripped) = identifier.strip_prefix("jsrec:") {
        if let Ok(tracks) = state.jiosaavn.get_recommendations(stripped.trim()).await {
            if !tracks.is_empty() {
                return Ok(Json(LoadResult::Search(tracks)));
            }
        }
    }

    if let Some(stripped) = identifier.strip_prefix("scsearch:") {
        if let Ok(tracks) = state.soundcloud.search(stripped.trim(), 10).await {
            if !tracks.is_empty() {
                return Ok(Json(LoadResult::Search(tracks)));
            }
        }
    }

    // Spotify URLs & search
    if identifier.contains("open.spotify.com/track/") {
        if let Some(track_id) = extract_spotify_id(&identifier, "track") {
            if let Ok(Some(track)) = state.spotify.resolve_track(&track_id).await {
                return Ok(Json(LoadResult::Track(track)));
            }
        }
    } else if identifier.contains("open.spotify.com/playlist/") {
        if let Some(pl_id) = extract_spotify_id(&identifier, "playlist") {
            if let Ok(Some(pl)) = state.spotify.resolve_playlist(&pl_id).await {
                return Ok(Json(LoadResult::Playlist(pl)));
            }
        }
    } else if let Some(stripped) = identifier.strip_prefix("spsearch:") {
        if let Ok(tracks) = state.spotify.search(stripped.trim(), 10).await {
            return Ok(Json(LoadResult::Search(tracks)));
        }
    }

    // YouTube URLs & search
    if is_youtube_url(&identifier) {
        let video_id = extract_youtube_id(&identifier);
        if let Ok(Some(track)) = state.youtube.resolve_video(&video_id).await {
            return Ok(Json(LoadResult::Track(track)));
        }
    } else if let Some(stripped) = identifier.strip_prefix("ytsearch:") {
        if let Ok(tracks) = state.youtube.search(stripped.trim(), 10).await {
            if !tracks.is_empty() {
                return Ok(Json(LoadResult::Search(tracks)));
            }
        }
        // Fallback to JioSaavn if YouTube API is dead/rate-limited
        if let Ok(tracks) = state.jiosaavn.search(stripped.trim(), 10).await {
            if !tracks.is_empty() {
                return Ok(Json(LoadResult::Search(tracks)));
            }
        }
    } else if let Some(stripped) = identifier.strip_prefix("ytmsearch:") {
        if let Ok(tracks) = state.youtube.search(stripped.trim(), 10).await {
            if !tracks.is_empty() {
                return Ok(Json(LoadResult::Search(tracks)));
            }
        }
        // Fallback to JioSaavn if YouTube API is dead
        if let Ok(tracks) = state.jiosaavn.search(stripped.trim(), 10).await {
            if !tracks.is_empty() {
                return Ok(Json(LoadResult::Search(tracks)));
            }
        }
    }

    // Apple Music / Deezer search prefixes -> match with JioSaavn / Spotify
    if let Some(stripped) = identifier
        .strip_prefix("amsearch:")
        .or_else(|| identifier.strip_prefix("dzsearch:"))
    {
        if let Ok(tracks) = state.jiosaavn.search(stripped.trim(), 10).await {
            if !tracks.is_empty() {
                return Ok(Json(LoadResult::Search(tracks)));
            }
        }
        if let Ok(tracks) = state.spotify.search(stripped.trim(), 10).await {
            return Ok(Json(LoadResult::Search(tracks)));
        }
    }

    // JioSaavn search or generic query fallback
    let search_term = identifier.strip_prefix("jssearch:").unwrap_or(&identifier).trim();

    match state.jiosaavn.search(search_term, 10).await {
        Ok(tracks) if !tracks.is_empty() => {
            if identifier.starts_with("http") && tracks.len() == 1 {
                Ok(Json(LoadResult::Track(tracks.into_iter().next().unwrap())))
            } else {
                Ok(Json(LoadResult::Search(tracks)))
            }
        }
        _ => {
            if let Ok(yt_tracks) = state.youtube.search(search_term, 10).await {
                if !yt_tracks.is_empty() {
                    return Ok(Json(LoadResult::Search(yt_tracks)));
                }
            }
            if let Ok(sc_tracks) = state.soundcloud.search(search_term, 10).await {
                if !sc_tracks.is_empty() {
                    return Ok(Json(LoadResult::Search(sc_tracks)));
                }
            }
            Ok(Json(LoadResult::Empty))
        }
    }
}

fn is_direct_audio_url(url: &str) -> bool {
    let lower = url.to_lowercase();
    let path = lower.split(['?', '#']).next().unwrap_or(&lower);
    path.ends_with(".mp3")
        || path.ends_with(".wav")
        || path.ends_with(".ogg")
        || path.ends_with(".flac")
        || path.ends_with(".m4a")
        || path.ends_with(".aac")
        || path.ends_with(".opus")
        || path.ends_with(".webm")
        || url.contains("cdn.discordapp.com/attachments/")
}

fn is_youtube_url(url: &str) -> bool {
    url.contains("youtube.com/watch")
        || url.contains("youtu.be/")
        || url.contains("youtube.com/shorts/")
        || url.contains("music.youtube.com/watch")
}

fn extract_youtube_id(url: &str) -> String {
    if let Some(id) = url.split("v=").nth(1).and_then(|s| s.split('&').next()) {
        id.to_string()
    } else if let Some(id) = url.split("youtu.be/").nth(1).and_then(|s| s.split(['?', '&']).next()) {
        id.to_string()
    } else if let Some(id) = url.split("/shorts/").nth(1).and_then(|s| s.split(['?', '&']).next()) {
        id.to_string()
    } else {
        url.to_string()
    }
}

fn extract_spotify_id(url: &str, entity_type: &str) -> Option<String> {
    let pattern = format!("/{}/", entity_type);
    url.split(&pattern).nth(1).and_then(|s| s.split('?').next()).map(|s| s.to_string())
}
