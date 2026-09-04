use axum::{
    extract::Request,
    middleware::{self, Next},
    response::Response,
    routing::{get, patch, post},
    Router,
};
use std::sync::Arc;
use tokio::sync::broadcast;
use tracing::{info, warn};

use kizunalink::*;
use kizunalink::player::manager::PlayerManager;
use kizunalink::ratelimit::{RateLimitConfig, RateLimiter};
use kizunalink::sources::{
    apple_music::AppleMusicSource, bandcamp::BandcampSource, deezer::DeezerSource,
    jiosaavn::JioSaavnSource, niconico::NicoNicoSource, route_planner::RoutePlanner,
    soundcloud::SoundCloudSource, spotify::SpotifySource, twitch::TwitchSource, vimeo::VimeoSource,
    youtube::YouTubeSource,
};


async fn track_requests(req: Request, next: Next) -> Response {
    if req.uri().path() != "/v4/websocket" {
        crate::metrics::Metrics::global().requests_total.inc();
    }
    next.run(req).await
}

async fn health_check() -> axum::Json<serde_json::Value> {
    axum::Json(serde_json::json!({
        "status": "ok",
        "version": VERSION,
        "timestamp": util::current_timestamp(),
    }))
}

async fn version_handler() -> axum::Json<serde_json::Value> {
    axum::Json(serde_json::json!({
        "version": VERSION,
    }))
}

#[tokio::main]
async fn main() {
    let config = config::AppConfig::load();

    // Initialize structured logging from config
    let log_level = std::env::var("RUST_LOG").unwrap_or_else(|_| config.logging.level.clone());
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(&log_level)),
        )
        .with_target(true)
        .with_thread_ids(false)
        .init();

    info!(
        "Loaded config: host={}, port={}",
        config.server.host, config.server.port
    );

    // Initialize the Cross-Platform WASM Plugin Manager
    // TODO: Plugin system is dead code / never invoked. Re-enable load_all() when plugin hooks are integrated.
    let plugin_manager = plugins::PluginManager::new();
    // plugin_manager.load_all();
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

    // Initialize global proxy config for all HTTP clients
    config::init_proxy(config.proxy.clone());
    // Initialize global security config
    config::init_security(config.security.clone());

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
        crate::player::manager::SourceBundle {
            jiosaavn: jiosaavn.clone(),
            youtube: youtube.clone(),
            spotify: spotify.clone(),
            soundcloud: soundcloud.clone(),
            deezer: deezer.clone(),
            apple_music: apple_music.clone(),
        },
        config.queue_max_history,
    ));

    let dave_manager = dave::DaveManager::new();
    let rate_limiter = RateLimiter::new(RateLimitConfig {
        max_requests: config.ratelimit.max_requests,
        window: std::time::Duration::from_secs(config.ratelimit.window_secs),
        burst: config.ratelimit.burst,
        ..RateLimitConfig::default()
    });

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
        .route("/version", get(version_handler))
        .route("/health", get(health_check))
        .route("/v4/info", get(rest::info::get_info))
        .route("/v4/stats", get(rest::stats::get_stats))
        .route("/v4/loadtracks", get(rest::loadtracks::load_tracks))
        .route("/v4/decodetrack", get(rest::decodetrack::decode_track))
        .route("/v4/decodetracks", post(rest::decodetrack::decode_tracks))
        .route(
            "/v4/sessions/:session_id",
            patch(rest::session::update_session),
        )
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
        .route("/v4/lyrics", get(rest::lyrics::get_lyrics_query))
        .route("/v4/lyrics/:song_id", get(rest::lyrics::get_lyrics))
        .route(
            "/v4/sessions/:session_id/players/:guild_id/track/lyrics",
            get(rest::lyrics::get_player_current_lyrics),
        )
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
        .route("/", get(|| async { VERSION }))
        .route("/v4/websocket", get(ws::handler::ws_handler))
        .layer(tower_http::limit::RequestBodyLimitLayer::new(
            config.security.max_body_size,
        ))
        .with_state(state)
        .layer(middleware::from_fn(track_requests));

    let addr = format!("{}:{}", config.server.host, config.server.port);
    info!("⛩️ KizunaLink v{} listening on {}", VERSION, addr);

    // Global broadcast tasks — run once for all clients (not per-connection)
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(60));
        let system_stats = crate::stats::SystemStats::global();
        loop {
            interval.tick().await;
            system_stats.refresh().await;
            let (total_players, playing_players) = stats_state.player_manager.count_players().await;
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
                if player_response.is_actively_playing() {
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

    let listener = match tokio::net::TcpListener::bind(&addr).await {
        Ok(l) => l,
        Err(e) => {
            tracing::error!("Failed to bind server to {}: {}", addr, e);
            std::process::exit(1);
        }
    };

    // Graceful shutdown via SIGINT (Ctrl+C) or SIGTERM
    let shutdown_signal = async {
        let ctrl_c = async {
            tokio::signal::ctrl_c()
                .await
                .expect("Failed to install Ctrl+C handler");
        };

        #[cfg(unix)]
        let terminate = async {
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                .expect("Failed to install SIGTERM handler")
                .recv()
                .await;
        };

        #[cfg(not(unix))]
        let terminate = std::future::pending::<()>();

        tokio::select! {
            _ = ctrl_c => {
                warn!("Received Ctrl+C (SIGINT), shutting down gracefully...");
            }
            _ = terminate => {
                warn!("Received SIGTERM, shutting down gracefully...");
            }
        }
    };

    info!("⛩️ KizunaLink v{} listening on {}", VERSION, addr);

    if let Err(e) = axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal)
        .await
    {
        tracing::error!("Server error: {}", e);
    }

    // Cleanup on shutdown
    info!("Shutting down players...");
    for item in shutdown_manager.get_all_players().await {
        shutdown_manager.destroy_player(&item.guild_id);
    }
    info!("KizunaLink shut down cleanly.");
}
