use axum::{
    extract::{Query, State},
    http::HeaderMap,
    response::Json,
};
use serde::Deserialize;
use tracing::{info, warn};

use crate::models::track::{LavalinkTrack, LoadResult, TrackInfo};
use crate::ratelimit::extract_ip;
use crate::rest::auth::require_auth;
use crate::rest::error::LavalinkError;
use crate::security;
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

    // Rate limit check
    let ip = extract_ip(&headers, "0.0.0.0");
    if !state.rate_limiter.check(&ip) {
        return Err(LavalinkError::new(
            axum::http::StatusCode::TOO_MANY_REQUESTS,
            "Rate limit exceeded",
            "/v4/loadtracks",
        )
        .with_retry_after(state.rate_limiter.window_secs()));
    }

    let identifier = match query.identifier {
        Some(id) if !id.trim().is_empty() => id.trim().to_string(),
        _ => return Ok(Json(LoadResult::Empty)),
    };

    // Validate identifier
    if let Err(e) = security::validate_identifier(&identifier) {
        return Err(LavalinkError::new(
            axum::http::StatusCode::BAD_REQUEST,
            e,
            "/v4/loadtracks",
        ));
    }

    info!("Resolving track query: \"{}\"", security::sanitize_for_log(&identifier));

    // Local audio file playback (e.g. /media/song.mp3 or file:///media/song.mp3)
    let local_path = if let Some(stripped) = identifier.strip_prefix("file://") {
        Some(stripped)
    } else if identifier.starts_with('/') && is_direct_audio_url(&identifier) {
        Some(identifier.as_str())
    } else {
        None
    };

    if let Some(path_str) = local_path {
        if !state.sources.local {
            return Ok(Json(LoadResult::Empty));
        }

        // Validate local path: prevent directory traversal and access to sensitive system directories
        let clean = path_str.trim_start_matches('/');
        if path_str.contains("..")
            || clean == "etc"
            || clean.starts_with("etc/")
            || clean == "proc"
            || clean.starts_with("proc/")
            || clean == "sys"
            || clean.starts_with("sys/")
            || clean == "dev"
            || clean.starts_with("dev/")
        {
            return Err(LavalinkError::new(
                axum::http::StatusCode::BAD_REQUEST,
                "Access to sensitive or relative paths is forbidden",
                "/v4/loadtracks",
            ));
        }

        let path = std::path::Path::new(path_str);
        if path.is_file() {
            let file_name = path
                .file_name()
                .and_then(|f| f.to_str())
                .unwrap_or("Local Audio");
            let mut track = LavalinkTrack {
                encoded: String::new(),
                info: TrackInfo {
                    identifier: path_str.to_string(),
                    is_seekable: true,
                    author: "Local File".to_string(),
                    length: 0,
                    is_stream: false,
                    position: 0,
                    title: file_name.to_string(),
                    uri: Some(format!("file://{}", path_str)),
                    artwork_url: None,
                    isrc: None,
                    source_name: "local".to_string(),
                },
                plugin_info: serde_json::Value::Null,
                user_data: serde_json::Value::Null,
            };
            if let Ok(enc) = crate::track_encoding::encode_track(&track) {
                track.encoded = enc;
            }
            let m = crate::metrics::Metrics::global();
            m.inc_source("local");
            m.tracks_loaded.inc();
            return Ok(Json(LoadResult::Track(track)));
        }
    }

    // SSRF protection: validate URL for identifier-based loads
    if identifier.starts_with("http://") || identifier.starts_with("https://") {
        if let Err(e) = security::validate_url(&identifier) {
            warn!("SSRF blocked for '{}': {}", security::sanitize_for_log(&identifier), e);
            return Ok(Json(LoadResult::Error(crate::models::track::ErrorInfo {
                message: Some(format!("URL rejected: {}", e)),
                severity: "fault".to_string(),
                cause: "SSRF protection".to_string(),
                cause_stack_trace: String::new(),
            })));
        }
    }

    // Direct HTTP(S) audio stream
    if (identifier.starts_with("http://") || identifier.starts_with("https://"))
        && is_direct_audio_url(&identifier)
    {
        if !state.sources.http {
            return Ok(Json(LoadResult::Empty));
        }
        let m = crate::metrics::Metrics::global();
        m.inc_source("http");
        m.tracks_loaded.inc();
        return Ok(Json(LoadResult::Track(crate::util::create_http_track(
            &identifier,
        ))));
    }

    // Bandcamp URLs
    if identifier.contains("bandcamp.com") || identifier.contains("bcvc.live") {
        if !state.sources.bandcamp {
            return Ok(Json(LoadResult::Empty));
        }
        if let Ok(Some(track)) = state.bandcamp.resolve_track(&identifier).await {
            let m = crate::metrics::Metrics::global();
            m.inc_source("bandcamp");
            m.tracks_loaded.inc();
            return Ok(Json(LoadResult::Track(track)));
        }
    }

    // Twitch URLs
    if identifier.contains("twitch.tv/") {
        if !state.sources.twitch {
            return Ok(Json(LoadResult::Empty));
        }
        let channel = identifier
            .split("twitch.tv/")
            .nth(1)
            .and_then(|s| s.split('/').next())
            .unwrap_or(&identifier);
        if let Ok(tracks) = state.twitch.search(channel, 1).await {
            if let Some(track) = tracks.into_iter().next() {
                let m = crate::metrics::Metrics::global();
                m.inc_source("twitch");
                m.tracks_loaded.inc();
                return Ok(Json(LoadResult::Track(track)));
            }
        }
    }

    // Vimeo URLs
    if identifier.contains("vimeo.com/") {
        if !state.sources.vimeo {
            return Ok(Json(LoadResult::Empty));
        }
        let video_id = identifier
            .split("vimeo.com/")
            .nth(1)
            .and_then(|s| s.split('/').next())
            .unwrap_or(&identifier);
        if let Ok(Some(track)) = state.vimeo.resolve_video(video_id).await {
            let m = crate::metrics::Metrics::global();
            m.inc_source("vimeo");
            m.tracks_loaded.inc();
            return Ok(Json(LoadResult::Track(track)));
        }
    }

    // NicoNico URLs
    if identifier.contains("nicovideo.jp/") || identifier.contains("nico.ms/") {
        if !state.sources.niconico {
            return Ok(Json(LoadResult::Empty));
        }
        let video_id = identifier
            .split("nicovideo.jp/watch/")
            .nth(1)
            .or_else(|| identifier.split("nico.ms/").nth(1))
            .and_then(|s| s.split('?').next())
            .unwrap_or(&identifier);
        if let Ok(Some(track)) = state.niconico.resolve_video(video_id).await {
            let m = crate::metrics::Metrics::global();
            m.inc_source("niconico");
            m.tracks_loaded.inc();
            return Ok(Json(LoadResult::Track(track)));
        }
    }

    // Check per-source rate limits using RateLimiter::check_source
    let check_source_limit = |source: &str| -> Result<(), LavalinkError> {
        if !state.rate_limiter.check_source(&ip, source) {
            Err(LavalinkError::new(
                axum::http::StatusCode::TOO_MANY_REQUESTS,
                format!("Rate limit exceeded for source: {}", source),
                "/v4/loadtracks",
            ))
        } else {
            Ok(())
        }
    };

    // Recommendation prefixes
    if let Some(stripped) = identifier.strip_prefix("jsrec:") {
        if !state.sources.jiosaavn {
            return Ok(Json(LoadResult::Empty));
        }
        check_source_limit("jiosaavn")?;
        if let Ok(tracks) = state.jiosaavn.get_recommendations(stripped.trim()).await {
            if !tracks.is_empty() {
                crate::metrics::Metrics::global().inc_source("jiosaavn");
                return Ok(Json(LoadResult::Search(tracks)));
            }
        }
    }

    if let Some(stripped) = identifier.strip_prefix("scsearch:") {
        if !state.sources.soundcloud {
            return Ok(Json(LoadResult::Empty));
        }
        check_source_limit("soundcloud")?;
        if let Ok(tracks) = state.soundcloud.search(stripped.trim(), 10).await {
            if !tracks.is_empty() {
                crate::metrics::Metrics::global().inc_source("soundcloud");
                return Ok(Json(LoadResult::Search(tracks)));
            }
        }
    }

    // SoundCloud set / playlist URLs
    if identifier.contains("soundcloud.com/") && identifier.contains("/sets/") {
        if !state.sources.soundcloud {
            return Ok(Json(LoadResult::Empty));
        }
        check_source_limit("soundcloud")?;
        if let Ok(Some(pl)) = state.soundcloud.resolve_set(&identifier).await {
            crate::metrics::Metrics::global().inc_source("soundcloud");
            return Ok(Json(LoadResult::Playlist(pl)));
        }
    }

    // Spotify URLs & search
    if identifier.contains("open.spotify.com/track/") {
        if !state.sources.spotify {
            return Ok(Json(LoadResult::Empty));
        }
        check_source_limit("spotify")?;
        if let Some(track_id) = extract_spotify_id(&identifier, "track") {
            if let Ok(Some(track)) = state.spotify.resolve_track(&track_id).await {
                crate::metrics::Metrics::global().inc_source("spotify");
                return Ok(Json(LoadResult::Track(track)));
            }
        }
    } else if identifier.contains("open.spotify.com/playlist/") {
        if !state.sources.spotify {
            return Ok(Json(LoadResult::Empty));
        }
        check_source_limit("spotify")?;
        if let Some(pl_id) = extract_spotify_id(&identifier, "playlist") {
            if let Ok(Some(pl)) = state.spotify.resolve_playlist(&pl_id).await {
                crate::metrics::Metrics::global().inc_source("spotify");
                return Ok(Json(LoadResult::Playlist(pl)));
            }
        }
    } else if let Some(stripped) = identifier.strip_prefix("spsearch:") {
        if !state.sources.spotify {
            return Ok(Json(LoadResult::Empty));
        }
        check_source_limit("spotify")?;
        if let Ok(tracks) = state.spotify.search(stripped.trim(), 10).await {
            crate::metrics::Metrics::global().inc_source("spotify");
            return Ok(Json(LoadResult::Search(tracks)));
        }
    }

    // YouTube URLs & search
    if is_youtube_url(&identifier) {
        if !state.sources.youtube {
            return Ok(Json(LoadResult::Empty));
        }
        check_source_limit("youtube")?;
        if let Some(list_id) = extract_youtube_playlist_id(&identifier) {
            if let Ok(Some(pl)) = state.youtube.resolve_playlist(&list_id).await {
                crate::metrics::Metrics::global().inc_source("youtube");
                return Ok(Json(LoadResult::Playlist(pl)));
            }
        }
        let video_id = extract_youtube_id(&identifier);
        if let Ok(Some(track)) = state.youtube.resolve_video(&video_id).await {
            crate::metrics::Metrics::global().inc_source("youtube");
            return Ok(Json(LoadResult::Track(track)));
        }
    } else if let Some(stripped) = identifier.strip_prefix("ytsearch:") {
        if !state.sources.youtube {
            return Ok(Json(LoadResult::Empty));
        }
        check_source_limit("youtube")?;
        if let Ok(tracks) = state.youtube.search(stripped.trim(), 10).await {
            if !tracks.is_empty() {
                crate::metrics::Metrics::global().inc_source("youtube");
                return Ok(Json(LoadResult::Search(tracks)));
            }
        }
        // Fallback to JioSaavn if YouTube API is dead/rate-limited
        if state.sources.jiosaavn && state.rate_limiter.check_source(&ip, "jiosaavn") {
            if let Ok(tracks) = state.jiosaavn.search(stripped.trim(), 10).await {
                if !tracks.is_empty() {
                    crate::metrics::Metrics::global().inc_source("jiosaavn");
                    return Ok(Json(LoadResult::Search(tracks)));
                }
            }
        }
    } else if let Some(stripped) = identifier.strip_prefix("ytmsearch:") {
        if !state.sources.youtube {
            return Ok(Json(LoadResult::Empty));
        }
        check_source_limit("youtube")?;
        if let Ok(tracks) = state.youtube.search(stripped.trim(), 10).await {
            if !tracks.is_empty() {
                crate::metrics::Metrics::global().inc_source("youtube");
                return Ok(Json(LoadResult::Search(tracks)));
            }
        }
        // Fallback to JioSaavn if YouTube API is dead
        if state.sources.jiosaavn && state.rate_limiter.check_source(&ip, "jiosaavn") {
            if let Ok(tracks) = state.jiosaavn.search(stripped.trim(), 10).await {
                if !tracks.is_empty() {
                    crate::metrics::Metrics::global().inc_source("jiosaavn");
                    return Ok(Json(LoadResult::Search(tracks)));
                }
            }
        }
    }

    // Apple Music URLs
    if identifier.contains("music.apple.com/") {
        if !state.sources.applemusic {
            return Ok(Json(LoadResult::Empty));
        }
        check_source_limit("applemusic")?;
        if let Some(track_id) = extract_apple_music_id(&identifier) {
            if let Ok(Some(track)) = state.apple_music.resolve_track(&track_id).await {
                let m = crate::metrics::Metrics::global();
                m.inc_source("applemusic");
                m.tracks_loaded.inc();
                return Ok(Json(LoadResult::Track(track)));
            }
        }
    } else if let Some(stripped) = identifier.strip_prefix("amsearch:") {
        if !state.sources.applemusic {
            return Ok(Json(LoadResult::Empty));
        }
        check_source_limit("applemusic")?;
        if let Ok(tracks) = state.apple_music.search(stripped.trim(), 10).await {
            if !tracks.is_empty() {
                crate::metrics::Metrics::global().inc_source("applemusic");
                return Ok(Json(LoadResult::Search(tracks)));
            }
        }
    }

    // Deezer URLs
    if identifier.contains("deezer.com/") {
        if !state.sources.deezer {
            return Ok(Json(LoadResult::Empty));
        }
        check_source_limit("deezer")?;
        if let Some(track_id) = extract_deezer_id(&identifier, "track") {
            if let Ok(Some(track)) = state.deezer.resolve_track(&track_id).await {
                let m = crate::metrics::Metrics::global();
                m.inc_source("deezer");
                m.tracks_loaded.inc();
                return Ok(Json(LoadResult::Track(track)));
            }
        } else if let Some(pl_id) = extract_deezer_id(&identifier, "playlist") {
            if let Ok(Some(pl)) = state.deezer.resolve_playlist(&pl_id).await {
                crate::metrics::Metrics::global().inc_source("deezer");
                return Ok(Json(LoadResult::Playlist(pl)));
            }
        }
    } else if let Some(stripped) = identifier.strip_prefix("dzsearch:") {
        if !state.sources.deezer {
            return Ok(Json(LoadResult::Empty));
        }
        check_source_limit("deezer")?;
        if let Ok(tracks) = state.deezer.search(stripped.trim(), 10).await {
            if !tracks.is_empty() {
                crate::metrics::Metrics::global().inc_source("deezer");
                return Ok(Json(LoadResult::Search(tracks)));
            }
        }
    }

    // Bandcamp search
    if let Some(stripped) = identifier.strip_prefix("bcsearch:") {
        if !state.sources.bandcamp {
            return Ok(Json(LoadResult::Empty));
        }
        if let Ok(tracks) = state.bandcamp.search(stripped.trim(), 10).await {
            if !tracks.is_empty() {
                crate::metrics::Metrics::global().inc_source("bandcamp");
                return Ok(Json(LoadResult::Search(tracks)));
            }
        }
    }

    // NicoNico search
    if let Some(stripped) = identifier.strip_prefix("nisearch:") {
        if !state.sources.niconico {
            return Ok(Json(LoadResult::Empty));
        }
        if let Ok(tracks) = state.niconico.search(stripped.trim(), 10).await {
            if !tracks.is_empty() {
                crate::metrics::Metrics::global().inc_source("niconico");
                return Ok(Json(LoadResult::Search(tracks)));
            }
        }
    }

    // Twitch stream resolve
    if let Some(stripped) = identifier.strip_prefix("twsearch:") {
        if !state.sources.twitch {
            return Ok(Json(LoadResult::Empty));
        }
        if let Ok(tracks) = state.twitch.search(stripped.trim(), 10).await {
            if !tracks.is_empty() {
                crate::metrics::Metrics::global().inc_source("twitch");
                return Ok(Json(LoadResult::Search(tracks)));
            }
        }
    }

    // Vimeo search
    if let Some(stripped) = identifier.strip_prefix("vmsearch:") {
        if !state.sources.vimeo {
            return Ok(Json(LoadResult::Empty));
        }
        if let Ok(tracks) = state.vimeo.search(stripped.trim(), 10).await {
            if !tracks.is_empty() {
                crate::metrics::Metrics::global().inc_source("vimeo");
                return Ok(Json(LoadResult::Search(tracks)));
            }
        }
    }

    // JioSaavn search or generic query fallback
    let search_term = identifier
        .strip_prefix("jssearch:")
        .unwrap_or(&identifier)
        .trim();

    if state.sources.jiosaavn {
        check_source_limit("jiosaavn")?;
        if let Ok(tracks) = state.jiosaavn.search(search_term, 10).await {
            if !tracks.is_empty() {
                crate::metrics::Metrics::global().inc_source("jiosaavn");
                if identifier.starts_with("http") && tracks.len() == 1 {
                    return Ok(Json(LoadResult::Track(tracks.into_iter().next().unwrap())));
                } else {
                    return Ok(Json(LoadResult::Search(tracks)));
                }
            }
        }
    }

    if state.sources.youtube && state.rate_limiter.check_source(&ip, "youtube") {
        if let Ok(yt_tracks) = state.youtube.search(search_term, 10).await {
            if !yt_tracks.is_empty() {
                crate::metrics::Metrics::global().inc_source("youtube");
                return Ok(Json(LoadResult::Search(yt_tracks)));
            }
        }
    }
    if state.sources.soundcloud && state.rate_limiter.check_source(&ip, "soundcloud") {
        if let Ok(sc_tracks) = state.soundcloud.search(search_term, 10).await {
            if !sc_tracks.is_empty() {
                crate::metrics::Metrics::global().inc_source("soundcloud");
                return Ok(Json(LoadResult::Search(sc_tracks)));
            }
        }
    }
    if state.sources.bandcamp {
        if let Ok(bc_tracks) = state.bandcamp.search(search_term, 10).await {
            if !bc_tracks.is_empty() {
                crate::metrics::Metrics::global().inc_source("bandcamp");
                return Ok(Json(LoadResult::Search(bc_tracks)));
            }
        }
    }
    Ok(Json(LoadResult::Empty))
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
        || url.contains("youtube.com/playlist")
}

fn extract_youtube_playlist_id(url: &str) -> Option<String> {
    url.split("list=")
        .nth(1)
        .and_then(|s| s.split('&').next())
        .map(|s| s.to_string())
}

fn extract_youtube_id(url: &str) -> String {
    if let Some(id) = url.split("v=").nth(1).and_then(|s| s.split('&').next()) {
        id.to_string()
    } else if let Some(id) = url
        .split("youtu.be/")
        .nth(1)
        .and_then(|s| s.split(['?', '&']).next())
    {
        id.to_string()
    } else if let Some(id) = url
        .split("/shorts/")
        .nth(1)
        .and_then(|s| s.split(['?', '&']).next())
    {
        id.to_string()
    } else {
        url.to_string()
    }
}

fn extract_spotify_id(url: &str, entity_type: &str) -> Option<String> {
    let pattern = format!("/{}/", entity_type);
    url.split(&pattern)
        .nth(1)
        .and_then(|s| s.split('?').next())
        .map(|s| s.to_string())
}

fn extract_apple_music_id(url: &str) -> Option<String> {
    // URLs like: https://music.apple.com/us/album/song-name/123456?i=789
    // The track ID is the `i=` parameter, or the last path segment
    if let Some(i_param) = url.split("i=").nth(1).and_then(|s| s.split('&').next()) {
        return Some(i_param.to_string());
    }
    url.rsplit('/')
        .next()
        .and_then(|s| s.split('?').next())
        .map(|s| s.to_string())
}

fn extract_deezer_id(url: &str, entity_type: &str) -> Option<String> {
    let pattern = format!("/{}/", entity_type);
    url.split(&pattern)
        .nth(1)
        .and_then(|s| s.split('?').next())
        .map(|s| s.to_string())
}
