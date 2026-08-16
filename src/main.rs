#![allow(dead_code, unused_imports, unused_variables)]

// KizunaLink — High-Performance Standalone Discord Audio Engine in Rust.

pub mod config;
pub mod models;
pub mod player;
pub mod sources;

use config::AppConfig;
use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Path, Query, State,
    },
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Json},
    routing::get,
    Router,
};
use models::{
    protocol::{CpuStats, MemoryStats, PlayerResponse, PlayerUpdatePayload, StatsPayload},
    track::{ErrorInfo, LoadResult},
};
use player::manager::PlayerManager;
use serde::Deserialize;
use serde_json::json;
use sources::{
    jiosaavn::JioSaavnSource, soundcloud::SoundCloudSource, spotify::SpotifySource,
    youtube::YouTubeSource,
};
use std::{net::SocketAddr, sync::Arc, time::Instant};
use tracing::{error, info, Level};
use tracing_subscriber::FmtSubscriber;

#[derive(Clone)]
pub struct AppState {
    pub jiosaavn: Arc<JioSaavnSource>,
    pub soundcloud: Arc<SoundCloudSource>,
    pub spotify: Arc<SpotifySource>,
    pub youtube: Arc<YouTubeSource>,
    pub player_manager: Arc<PlayerManager>,
    pub password: String,
    pub start_time: Instant,
}

#[derive(Deserialize)]
pub struct LoadTracksQuery {
    pub identifier: Option<String>,
}

#[derive(Deserialize)]
pub struct LoadLyricsQuery {
    #[serde(rename = "encodedTrack")]
    pub encoded_track: Option<String>,
}

#[tokio::main]
async fn main() {
    // 1. Initialize Logging
    let subscriber = FmtSubscriber::builder()
        .with_max_level(Level::INFO)
        .finish();
    tracing::subscriber::set_global_default(subscriber)
        .expect("Failed to initialize tracing subscriber");

    info!("🌸 Initializing KizunaLink v0.1.0 (Rust Audio Core)...");

    let cfg = AppConfig::load();
    let jiosaavn = JioSaavnSource::new();
    let soundcloud = SoundCloudSource::new();
    let spotify = SpotifySource::new();
    let youtube = YouTubeSource::new();
    let player_manager = PlayerManager::new(jiosaavn.clone());
    let password = cfg.server.password.clone();

    let state = AppState {
        jiosaavn,
        soundcloud,
        spotify,
        youtube,
        player_manager,
        password,
        start_time: Instant::now(),
    };

    // 2. Build Full Lavalink v4 Protocol Router
    let app = Router::new()
        .route("/version", get(get_version))
        .route("/v4/info", get(get_info))
        .route("/v4/stats", get(get_stats))
        .route("/v4/websocket", get(ws_handler))
        .route("/v4/loadtracks", get(load_tracks))
        .route("/v4/loadlyrics", get(load_lyrics))
        .route("/v4/decodetrack", get(decode_track))
        .route("/v4/decodetracks", axum::routing::post(decode_tracks))
        .route("/v4/sessions/:session_id", axum::routing::patch(update_session))
        .route("/v4/sessions/:session_id/players", get(get_players))
        .route(
            "/v4/sessions/:session_id/players/:guild_id",
            get(get_player)
                .patch(update_player)
                .delete(destroy_player),
        )
        .with_state(state);

    let addr: SocketAddr = format!("{}:{}", cfg.server.host, cfg.server.port)
        .parse()
        .expect("Invalid host or port configuration");

    info!("⛩️ KizunaLink Audio Core listening on http://{}", addr);

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("Failed to bind TCP listener");
    axum::serve(listener, app)
        .await
        .expect("KizunaLink server encountered an error");
}

/// Server Info Endpoint (/v4/info)
async fn get_info() -> impl IntoResponse {
    Json(json!({
        "version": {
            "semver": "0.1.0",
            "major": 0,
            "minor": 1,
            "patch": 0
        },
        "buildTime": chrono_timestamp(),
        "git": {
            "branch": "main",
            "commit": "kizuna-core"
        },
        "isKizunaLink": true,
        "engine": "Rust (Tokio + Symphonia + Songbird)",
        "sourceManagers": ["jiosaavn", "youtube", "spotify", "soundcloud", "http"],
        "filters": ["volume", "equalizer", "karaoke", "timescale", "tremolo", "vibrato", "rotation", "distortion"],
        "plugins": []
    }))
}

/// Server Metrics & Stats Endpoint (/v4/stats)
async fn get_stats(State(state): State<AppState>) -> impl IntoResponse {
    let (total_players, playing_players) = state.player_manager.count_players();
    let uptime = state.start_time.elapsed().as_millis() as u64;

    Json(StatsPayload {
        players: total_players,
        playing_players,
        uptime,
        memory: MemoryStats {
            free: 1024 * 1024 * 512,
            used: 1024 * 1024 * 18,
            allocated: 1024 * 1024 * 32,
            reservable: 1024 * 1024 * 512,
        },
        cpu: CpuStats {
            cores: num_cpus::get(),
            system_load: 0.05,
            lavalink_load: 0.01,
        },
    })
}

/// Track Search & Load Endpoint (/v4/loadtracks?identifier=...)
async fn load_tracks(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<LoadTracksQuery>,
) -> Result<Json<LoadResult>, StatusCode> {
    if let Some(auth) = headers.get("authorization") {
        if auth.to_str().unwrap_or("") != state.password {
            return Err(StatusCode::UNAUTHORIZED);
        }
    }

    let identifier = match query.identifier {
        Some(id) if !id.trim().is_empty() => id.trim().to_string(),
        _ => return Ok(Json(LoadResult::Empty(json!({})))),
    };

    info!("🔍 Resolving track query: \"{}\"", identifier);

    // 1. Direct HTTP Audio Stream Handling
    if identifier.starts_with("http://") || identifier.starts_with("https://") {
        if identifier.ends_with(".mp3")
            || identifier.ends_with(".wav")
            || identifier.ends_with(".ogg")
            || identifier.ends_with(".flac")
            || identifier.ends_with(".m4a")
            || identifier.ends_with(".aac")
            || identifier.contains("cdn.discordapp.com/attachments/")
        {
            use base64::{engine::general_purpose::STANDARD, Engine as _};
            let title = identifier
                .split('/')
                .last()
                .unwrap_or("Direct Audio Stream")
                .split('?')
                .next()
                .unwrap_or("Direct Audio Stream")
                .to_string();

            let encoded = STANDARD.encode(format!("http:{}", identifier));
            let track = crate::models::track::LavalinkTrack {
                encoded,
                info: crate::models::track::TrackInfo {
                    identifier: identifier.clone(),
                    is_seekable: true,
                    author: "HTTP Stream".to_string(),
                    length: 0,
                    is_stream: true,
                    position: 0,
                    title,
                    uri: Some(identifier.clone()),
                    artwork_url: None,
                    source_name: "http".to_string(),
                    bitrate: Some("320kbps".to_string()),
                    stream_url: Some(identifier.clone()),
                },
                plugin_info: serde_json::json!({}),
                user_data: serde_json::json!({}),
            };
            return Ok(Json(LoadResult::Track(track)));
        }
    }

    // 2. JioSaavn Radio Recommendations (jsrec:<query>)
    if let Some(stripped) = identifier.strip_prefix("jsrec:") {
        if let Ok(tracks) = state.jiosaavn.get_recommendations(stripped.trim()).await {
            if !tracks.is_empty() {
                return Ok(Json(LoadResult::Search(tracks)));
            }
        }
    }

    // 3. SoundCloud Search & Prefix (scsearch:<query>)
    if let Some(stripped) = identifier.strip_prefix("scsearch:") {
        if let Ok(tracks) = state.soundcloud.search(stripped.trim(), 10).await {
            if !tracks.is_empty() {
                return Ok(Json(LoadResult::Search(tracks)));
            }
        }
    }

    // 4. Spotify URL Handling
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

    // 5. YouTube URL Handling & Prefix
    if identifier.contains("youtube.com/watch") || identifier.contains("youtu.be/") {
        let video_id = if let Some(id) = identifier.split("v=").nth(1).and_then(|s| s.split('&').next()) {
            id
        } else if let Some(id) = identifier.split("youtu.be/").nth(1).and_then(|s| s.split('?').next()) {
            id
        } else {
            &identifier
        };

        if let Ok(Some(track)) = state.youtube.resolve_video(video_id).await {
            return Ok(Json(LoadResult::Track(track)));
        }
    } else if let Some(stripped) = identifier.strip_prefix("ytsearch:") {
        if let Ok(tracks) = state.youtube.search(stripped.trim(), 10).await {
            return Ok(Json(LoadResult::Search(tracks)));
        }
    }

    // Additional Prefixes (ytmsearch:, amsearch:, dzsearch:)
    if let Some(stripped) = identifier.strip_prefix("ytmsearch:") {
        if let Ok(tracks) = state.youtube.search(stripped.trim(), 10).await {
            return Ok(Json(LoadResult::Search(tracks)));
        }
    }
    if let Some(stripped) = identifier.strip_prefix("amsearch:").or_else(|| identifier.strip_prefix("dzsearch:")) {
        if let Ok(tracks) = state.jiosaavn.search(stripped.trim(), 10).await {
            if !tracks.is_empty() {
                return Ok(Json(LoadResult::Search(tracks)));
            }
        }
        if let Ok(tracks) = state.spotify.search(stripped.trim(), 10).await {
            return Ok(Json(LoadResult::Search(tracks)));
        }
    }

    // 6. JioSaavn Primary Search & Fallback
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
            // Fallback to YouTube
            if let Ok(yt_tracks) = state.youtube.search(search_term, 10).await {
                if !yt_tracks.is_empty() {
                    return Ok(Json(LoadResult::Search(yt_tracks)));
                }
            }
            Ok(Json(LoadResult::Empty(json!({}))))
        }
    }
}

/// Plain Version Endpoint (/version)
async fn get_version() -> impl IntoResponse {
    "0.1.0"
}

#[derive(Deserialize)]
pub struct DecodeTrackQuery {
    #[serde(rename = "encodedTrack")]
    pub encoded_track: Option<String>,
}

/// Decode single track endpoint (/v4/decodetrack?encodedTrack=...)
async fn decode_track(
    Query(query): Query<DecodeTrackQuery>,
    State(_state): State<AppState>,
) -> Result<Json<crate::models::track::TrackInfo>, StatusCode> {
    let encoded = match query.encoded_track {
        Some(e) if !e.trim().is_empty() => e,
        _ => return Err(StatusCode::BAD_REQUEST),
    };

    use base64::{engine::general_purpose::STANDARD, Engine as _};
    let decoded_str = STANDARD
        .decode(&encoded)
        .ok()
        .and_then(|b| String::from_utf8(b).ok())
        .unwrap_or_else(|| encoded.clone());

    let (source, id) = if let Some(idx) = decoded_str.find(':') {
        (&decoded_str[..idx], &decoded_str[idx + 1..])
    } else {
        ("jiosaavn", decoded_str.as_str())
    };

    Ok(Json(crate::models::track::TrackInfo {
        identifier: id.to_string(),
        is_seekable: true,
        author: "Kizuna Audio".to_string(),
        length: 210000,
        is_stream: false,
        position: 0,
        title: id.to_string(),
        uri: None,
        artwork_url: None,
        source_name: source.to_string(),
        bitrate: Some("320kbps".to_string()),
        stream_url: None,
    }))
}

/// Decode batch of tracks endpoint (POST /v4/decodetracks)
async fn decode_tracks(
    State(_state): State<AppState>,
    Json(encoded_tracks): Json<Vec<String>>,
) -> Json<Vec<crate::models::track::LavalinkTrack>> {
    use base64::{engine::general_purpose::STANDARD, Engine as _};
    let mut tracks = Vec::new();
    for encoded in encoded_tracks {
        let decoded_str = STANDARD
            .decode(&encoded)
            .ok()
            .and_then(|b| String::from_utf8(b).ok())
            .unwrap_or_else(|| encoded.clone());

        let (source, id) = if let Some(idx) = decoded_str.find(':') {
            (&decoded_str[..idx], &decoded_str[idx + 1..])
        } else {
            ("jiosaavn", decoded_str.as_str())
        };

        tracks.push(crate::models::track::LavalinkTrack {
            encoded: encoded.clone(),
            info: crate::models::track::TrackInfo {
                identifier: id.to_string(),
                is_seekable: true,
                author: "Kizuna Audio".to_string(),
                length: 210000,
                is_stream: false,
                position: 0,
                title: id.to_string(),
                uri: None,
                artwork_url: None,
                source_name: source.to_string(),
                bitrate: Some("320kbps".to_string()),
                stream_url: None,
            },
            plugin_info: serde_json::json!({}),
            user_data: serde_json::json!({}),
        });
    }
    Json(tracks)
}

#[derive(Deserialize)]
pub struct SessionUpdatePayload {
    pub resuming: Option<bool>,
    pub timeout: Option<u64>,
}

/// Update session endpoint (PATCH /v4/sessions/:sessionId)
async fn update_session(
    Path(_session_id): Path<String>,
    Json(payload): Json<SessionUpdatePayload>,
) -> Json<serde_json::Value> {
    Json(json!({
        "resuming": payload.resuming.unwrap_or(false),
        "timeout": payload.timeout.unwrap_or(60)
    }))
}

/// Native Lyrics Endpoint (/v4/loadlyrics?encodedTrack=...)
async fn load_lyrics(
    State(state): State<AppState>,
    Query(query): Query<LoadLyricsQuery>,
) -> impl IntoResponse {
    let encoded = match query.encoded_track {
        Some(e) if !e.trim().is_empty() => e,
        _ => return (StatusCode::BAD_REQUEST, Json(json!({ "loadType": "empty" }))),
    };

    use base64::{engine::general_purpose::STANDARD, Engine as _};
    let song_id = if let Ok(decoded_bytes) = STANDARD.decode(&encoded) {
        let raw = String::from_utf8_lossy(&decoded_bytes);
        if let Some(id) = raw.strip_prefix("jiosaavn:") {
            id.to_string()
        } else {
            raw.to_string()
        }
    } else {
        encoded
    };

    match state.jiosaavn.get_lyrics(&song_id).await {
        Ok(Some(lyrics_text)) => {
            (StatusCode::OK, Json(json!({
                "loadType": "lyrics",
                "data": {
                    "source": "JioSaavn",
                    "text": lyrics_text,
                    "lines": []
                }
            })))
        }
        Ok(None) => (StatusCode::OK, Json(json!({ "loadType": "empty" }))),
        Err(_) => (StatusCode::OK, Json(json!({ "loadType": "empty" }))),
    }
}

/// Get all players in a session (/v4/sessions/:sessionId/players)
async fn get_players(
    State(state): State<AppState>,
    Path(_session_id): Path<String>,
) -> Json<Vec<PlayerResponse>> {
    Json(state.player_manager.get_all_players().await)
}

/// Get single player state (/v4/sessions/:sessionId/players/:guildId)
async fn get_player(
    State(state): State<AppState>,
    Path((_session_id, guild_id)): Path<(String, String)>,
) -> Result<Json<PlayerResponse>, StatusCode> {
    state
        .player_manager
        .get_player(&guild_id)
        .await
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

/// Update player state via Lavalink v4 PATCH (/v4/sessions/:sessionId/players/:guildId)
async fn update_player(
    State(state): State<AppState>,
    Path((_session_id, guild_id)): Path<(String, String)>,
    Json(payload): Json<PlayerUpdatePayload>,
) -> Result<Json<PlayerResponse>, StatusCode> {
    match state.player_manager.update_player(&guild_id, payload).await {
        Ok(response) => Ok(Json(response)),
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

/// Destroy player and leave voice channel (/v4/sessions/:sessionId/players/:guildId)
async fn destroy_player(
    State(state): State<AppState>,
    Path((_session_id, guild_id)): Path<(String, String)>,
) -> StatusCode {
    if state.player_manager.destroy_player(&guild_id) {
        StatusCode::NO_CONTENT
    } else {
        StatusCode::NOT_FOUND
    }
}

/// Real-time Voice Control WebSocket Handler (/v4/websocket)
async fn ws_handler(
    ws: WebSocketUpgrade,
    headers: HeaderMap,
    State(state): State<AppState>,
) -> Result<impl IntoResponse, StatusCode> {
    if let Some(auth) = headers.get("authorization") {
        if auth.to_str().unwrap_or("") != state.password {
            return Err(StatusCode::UNAUTHORIZED);
        }
    }

    if let Some(user_id) = headers
        .get("user-id")
        .or_else(|| headers.get("User-Id"))
        .and_then(|h| h.to_str().ok())
    {
        let mut id_lock = state.player_manager.bot_user_id.write().await;
        *id_lock = user_id.to_string();
        info!("🤖 Registered Bot Client User ID: {}", user_id);
    }

    Ok(ws.on_upgrade(handle_socket))
}

async fn handle_socket(socket: WebSocket) {
    use futures_util::{SinkExt, StreamExt};
    info!("🔗 New WebSocket connection established with Discord Bot");

    let (mut sender, mut receiver) = socket.split();

    let ready_payload = json!({
        "op": "ready",
        "resumed": false,
        "sessionId": format!("kizuna-session-{}", uuid_simple())
    });

    if sender.send(Message::Text(ready_payload.to_string())).await.is_err() {
        return;
    }

    let sender = Arc::new(tokio::sync::Mutex::new(sender));
    let sender_clone = sender.clone();

    // Heartbeat sender (every 20s) to keep client active
    let heartbeat_handle = tokio::spawn(async move {
        let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(20));
        loop {
            interval.tick().await;
            let stats = json!({
                "op": "stats",
                "players": 0,
                "playingPlayers": 0,
                "uptime": 10000,
                "memory": {
                    "free": 1024 * 1024 * 512,
                    "used": 1024 * 1024 * 2,
                    "allocated": 1024 * 1024 * 4,
                    "reservable": 1024 * 1024 * 512
                },
                "cpu": {
                    "cores": 4,
                    "systemLoad": 0.01,
                    "lavalinkLoad": 0.01
                },
                "frameStats": {
                    "sent": 0,
                    "nulled": 0,
                    "deficit": 0
                }
            });

            let mut lock = sender_clone.lock().await;
            if lock.send(Message::Text(stats.to_string())).await.is_err() {
                break;
            }
        }
    });

    while let Some(Ok(msg)) = receiver.next().await {
        match msg {
            Message::Text(text) => {
                info!("📩 Inbound Lavalink OP payload: {}", text);
            }
            Message::Close(_) => {
                info!("🔌 WebSocket client disconnected");
                break;
            }
            _ => {}
        }
    }

    heartbeat_handle.abort();
}

fn chrono_timestamp() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn uuid_simple() -> u128 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
}
