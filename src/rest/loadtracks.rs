use axum::{
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
    response::Json,
};
use serde::Deserialize;
use crate::AppState;
use crate::models::track::LoadResult;
use crate::rest::error::LavalinkError;
use tracing::info;

#[derive(Deserialize)]
pub struct LoadTracksQuery {
    pub identifier: Option<String>,
}

pub async fn load_tracks(
    state: State<AppState>,
    headers: HeaderMap,
    Query(query): Query<LoadTracksQuery>,
) -> Result<Json<LoadResult>, LavalinkError> {
    if let Some(auth) = headers.get("authorization") {
        if auth.to_str().unwrap_or("") != state.password {
            return Err(LavalinkError::new(StatusCode::UNAUTHORIZED, "Invalid authorization header", "/v4/loadtracks"));
        }
    }

    let identifier = match query.identifier {
        Some(id) if !id.trim().is_empty() => id.trim().to_string(),
        _ => return Ok(Json(LoadResult::Empty)),
    };

    info!("Resolving track query: \"{}\"", identifier);

    if identifier.starts_with("http://") || identifier.starts_with("https://") {
        if is_direct_audio_url(&identifier) {
            return Ok(Json(LoadResult::Track(
                crate::util::create_http_track(&identifier),
            )));
        }
    }

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

    if identifier.contains("open.spotify.com/track/") {
        if let Some(track_id) = identifier.split("/track/").nth(1).and_then(|s| s.split('?').next()) {
            if let Ok(Some(track)) = state.spotify.resolve_track(track_id).await {
                return Ok(Json(LoadResult::Track(track)));
            }
        }
    } else if identifier.contains("open.spotify.com/playlist/") {
        if let Some(pl_id) = identifier.split("/playlist/").nth(1).and_then(|s| s.split('?').next()) {
            if let Ok(Some(pl)) = state.spotify.resolve_playlist(pl_id).await {
                return Ok(Json(LoadResult::Playlist(pl)));
            }
        }
    } else if let Some(stripped) = identifier.strip_prefix("spsearch:") {
        if let Ok(tracks) = state.spotify.search(stripped.trim(), 10).await {
            return Ok(Json(LoadResult::Search(tracks)));
        }
    }

    if identifier.contains("youtube.com/watch") || identifier.contains("youtu.be/") {
        let video_id = extract_youtube_id(&identifier);
        if let Ok(Some(track)) = state.youtube.resolve_video(video_id).await {
            return Ok(Json(LoadResult::Track(track)));
        }
    } else if let Some(stripped) = identifier.strip_prefix("ytsearch:") {
        if let Ok(tracks) = state.youtube.search(stripped.trim(), 10).await {
            return Ok(Json(LoadResult::Search(tracks)));
        }
    }

    if let Some(stripped) = identifier.strip_prefix("ytmsearch:") {
        if let Ok(tracks) = state.youtube.search(stripped.trim(), 10).await {
            return Ok(Json(LoadResult::Search(tracks)));
        }
    }

    if let Some(stripped) = identifier.strip_prefix("amsearch:")
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
            Ok(Json(LoadResult::Empty))
        }
    }
}

fn is_direct_audio_url(url: &str) -> bool {
    url.ends_with(".mp3")
        || url.ends_with(".wav")
        || url.ends_with(".ogg")
        || url.ends_with(".flac")
        || url.ends_with(".m4a")
        || url.ends_with(".aac")
        || url.contains("cdn.discordapp.com/attachments/")
}

fn extract_youtube_id(url: &str) -> &str {
    if let Some(id) = url.split("v=").nth(1).and_then(|s| s.split('&').next()) {
        id
    } else if let Some(id) = url.split("youtu.be/").nth(1).and_then(|s| s.split('?').next()) {
        id
    } else {
        url
    }
}
