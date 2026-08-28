mod plugins;
mod config;
pub mod dsp;
mod dave;
mod models;
mod player;
pub mod metrics;
mod rest;
mod sources;
mod stats;
mod track_encoding;
mod util;
mod ws;
mod ratelimit;
mod security;

use axum::{
    routing::{get, patch, post},
    Router,
};
use std::sync::Arc;
use tokio::sync::broadcast;
use tower_http::cors::{Any, CorsLayer};
use tracing::{info, warn};

use player::manager::PlayerManager;
use ratelimit::{RateLimiter, RateLimitConfig};
use sources::{
    apple_music::AppleMusicSource, bandcamp::BandcampSource, deezer::DeezerSource,
    jiosaavn::JioSaavnSource, niconico::NicoNicoSource, route_planner::RoutePlanner,
    soundcloud::SoundCloudSource, spotify::SpotifySource, twitch::TwitchSource,
    vimeo::VimeoSource, youtube::YouTubeSource,
};

#[derive(Clone)]
pub struct AppState {
    pub player_manager: Arc<PlayerManager>,
    pub jiosaavn: Arc<JioSaavnSource>,
    pub youtube: Arc<YouTubeSource>,
    pub spotify: Arc<SpotifySource>,
    pub soundcloud: Arc<SoundCloudSource>,
    pub bandcamp: Arc<BandcampSource>,
    pub twitch: Arc<TwitchSource>,
    pub vimeo: Arc<VimeoSource>,
    pub niconico: Arc<NicoNicoSource>,
    pub apple_music: Arc<AppleMusicSource>,
    pub deezer: Arc<DeezerSource>,
    pub plugin_manager: Arc<plugins::PluginManager>,
    pub dave_manager: dave::DaveManager,
    pub route_planner: Option<Arc<RoutePlanner>>,
    pub rate_limiter: Arc<RateLimiter>,
    pub password: String,
    pub start_time: std::time::Instant,
    pub event_tx: broadcast::Sender<String>,
}

async fn health_check() -> axum::Json<serde_json::Value> {
    axum::Json(serde_json::json!({
        "status": "ok",
        "version": "4.2.1",
        "timestamp": util::current_timestamp(),
    }))
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();
    let config = config::AppConfig::load();
    info!(
        "Loaded config: host={}, port={}",
        config.server.host, config.server.port
    );

    // Initialize the Cross-Platform WASM Plugin Manager
    let mut plugin_manager = plugins::PluginManager::new();
    plugin_manager.load_all();
    let plugin_manager = Arc::new(plugin_manager);

    // Initialize the Route Planner (if configured)
    let route_planner = RoutePlanner::new(
        &config.ratelimit.ip_blocks,
        &config.ratelimit.strategy,
        &config.ratelimit.excluded_ips,
    )
    .map(Arc::new);

    let (event_tx, _) = broadcast::channel::<String>(256);
    let password = config.server.password.clone();

    let jiosaavn = JioSaavnSource::new();
    let youtube = YouTubeSource::new(route_planner.clone());
    let spotify = SpotifySource::new();
    let soundcloud = SoundCloudSource::new();
    let bandcamp = BandcampSource::new();
    let twitch = TwitchSource::new();
    let vimeo = VimeoSource::new();
    let niconico = NicoNicoSource::new();
    let apple_music = AppleMusicSource::new();
    let deezer = DeezerSource::new();

    let player_manager = Arc::new(PlayerManager::new(
        event_tx.clone(),
        jiosaavn.clone(),
        youtube.clone(),
        spotify.clone(),
        soundcloud.clone(),
    ));

    let dave_manager = dave::DaveManager::new();
    let rate_limiter = RateLimiter::new(RateLimitConfig::default());

    let state = AppState {
        player_manager,
        jiosaavn,
        youtube,
        spotify,
        soundcloud,
        bandcamp,
        twitch,
        vimeo,
        niconico,
        apple_music,
        deezer,
        plugin_manager,
        dave_manager,
        route_planner,
        rate_limiter,
        password,
        start_time: std::time::Instant::now(),
        event_tx,
    };

    // Clone state for global broadcast tasks and shutdown cleanup before the router consumes it
    let stats_state = state.clone();
    let update_state = state.clone();
    let shutdown_manager = state.player_manager.clone();
    let rate_limiter = state.rate_limiter.clone();

    let app = Router::new()
        .route("/version", get(|| async { "4.2.1" }))
        .route("/health", get(health_check))
        .route("/v4/info", get(rest::info::get_info))
        .route("/v4/stats", get(rest::stats::get_stats))
        .route("/v4/loadtracks", get(rest::loadtracks::load_tracks))
        .route("/v4/decodetrack", get(rest::decodetrack::decode_track))
        .route("/v4/decodetracks", post(rest::decodetrack::decode_tracks))
        .route("/v4/sessions/:session_id", patch(rest::session::update_session))
        .route("/v4/players/all", get(rest::players::get_all_players))
        .route(
            "/v4/sessions/:session_id/players",
            get(rest::players::get_players),
        )
        .route(
            "/v4/sessions/:session_id/players/:guild_id",
            get(rest::players::get_player),
        )
        .route(
            "/v4/sessions/:session_id/players/:guild_id",
            patch(rest::players::update_player).delete(rest::players::destroy_player),
        )
        .route("/v4/lyrics/:song_id", get(rest::lyrics::get_lyrics))
        .route("/v4/sessions", get(rest::sessions::list_sessions))
        .route("/v4/metrics", get(rest::metrics::get_metrics))
        .route(
            "/v4/routeplanner/status",
            get(rest::routeplanner::get_routeplanner_status),
        )
        .route(
            "/v4/routeplanner/free/address",
            post(rest::routeplanner::free_routeplanner_address),
        )
        .route(
            "/v4/routeplanner/free/all",
            post(rest::routeplanner::free_routeplanner_all),
        )
        .route("/", get(|| async { "4.2.1" }))
        .route("/v4/websocket", get(ws::handler::ws_handler))
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods(Any)
                .allow_headers(Any),
        )
        .with_state(state);

    let addr = format!("{}:{}", config.server.host, config.server.port);
    info!("⛩️ KizunaLink v4.2.1 listening on {}", addr);

    // Global broadcast tasks — run once for all clients (not per-connection)
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(60));
        let system_stats = crate::stats::SystemStats::global();
        loop {
            interval.tick().await;
            system_stats.refresh().await;
            let (total_players, playing_players) =
                stats_state.player_manager.count_players().await;
            let uptime = stats_state.start_time.elapsed().as_millis() as u64;
            let memory = system_stats.get_memory_stats().await;
            let cpu = system_stats.get_cpu_stats().await;

            let stats = serde_json::json!({
                "op": "stats",
                "players": total_players,
                "playingPlayers": playing_players,
                "uptime": uptime,
                "memory": {
                    "free": memory.free,
                    "used": memory.used,
                    "allocated": memory.allocated,
                    "reservable": memory.reservable
                },
                "cpu": {
                    "cores": cpu.cores,
                    "systemLoad": cpu.system_load,
                    "lavalinkLoad": cpu.lavalink_load
                },
                "frameStats": {
                    "sent": crate::stats::FrameCounters::global().sent.load(std::sync::atomic::Ordering::Relaxed),
                    "nulled": crate::stats::FrameCounters::global().nulled.load(std::sync::atomic::Ordering::Relaxed),
                    "deficit": crate::stats::FrameCounters::global().deficit.load(std::sync::atomic::Ordering::Relaxed)
                }
            });

            let _ = stats_state.event_tx.send(stats.to_string());
        }
    });


    tokio::spawn(async move {
        let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(5));
        loop {
            interval.tick().await;
            for player_response in update_state.player_manager.get_all_players().await {
                if player_response.track.is_some() || player_response.state.connected {
                    let msg = serde_json::json!({
                        "op": "playerUpdate",
                        "guildId": player_response.guild_id,
                        "state": player_response.state,
                    });
                    let _ = update_state.event_tx.send(msg.to_string());
                }
            }
        }
    });

    // Rate limiter cleanup task
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(300));
        loop {
            interval.tick().await;
            rate_limiter.cleanup().await;
        }
    });

    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();

    // Graceful shutdown via Ctrl+C
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    ctrlc::set_handler(move || {
        warn!("Received Ctrl+C, shutting down gracefully...");
        let _ = shutdown_tx.send(true);
    })
    .expect("Error setting Ctrl+C handler");

    let shutdown_signal = async move {
        let _ = shutdown_rx.clone().changed().await;
    };

    info!("⛩️ KizunaLink v4.2.1 listening on {}", addr);

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal)
        .await
        .unwrap();

    // Cleanup on shutdown
    info!("Shutting down players...");
    for item in shutdown_manager.get_all_players().await {
        shutdown_manager.destroy_player(&item.guild_id);
    }
    info!("KizunaLink shut down cleanly.");
}
