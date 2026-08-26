mod plugins;
mod config;
pub mod dsp;
mod models;
mod player;
mod rest;
mod sources;
mod track_encoding;
mod util;
mod ws;

use axum::{
    routing::{get, patch, post},
    Router,
};
use std::sync::Arc;
use tokio::sync::broadcast;
use tower_http::cors::{Any, CorsLayer};
use tracing::info;

use player::manager::PlayerManager;
use sources::{
    jiosaavn::JioSaavnSource, route_planner::RoutePlanner, soundcloud::SoundCloudSource,
    spotify::SpotifySource, youtube::YouTubeSource,
};

#[derive(Clone)]
pub struct AppState {
    pub player_manager: Arc<PlayerManager>,
    pub jiosaavn: Arc<JioSaavnSource>,
    pub youtube: Arc<YouTubeSource>,
    pub spotify: Arc<SpotifySource>,
    pub soundcloud: Arc<SoundCloudSource>,
    pub plugin_manager: Arc<plugins::PluginManager>,
    pub route_planner: Option<Arc<RoutePlanner>>,
    pub password: String,
    pub start_time: std::time::Instant,
    pub event_tx: broadcast::Sender<String>,
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

    let player_manager = Arc::new(PlayerManager::new(
        event_tx.clone(),
        jiosaavn.clone(),
        youtube.clone(),
        spotify.clone(),
        soundcloud.clone(),
    ));

    let state = AppState {
        player_manager,
        jiosaavn,
        youtube,
        spotify,
        soundcloud,
        plugin_manager,
        route_planner,
        password,
        start_time: std::time::Instant::now(),
        event_tx,
    };

    // Clone state for global broadcast tasks before the router consumes it
    let stats_state = state.clone();
    let update_state = state.clone();

    let app = Router::new()
        .route("/version", get(|| async { "4.2.1" }))
        .route("/v4/info", get(rest::info::get_info))
        .route("/v4/stats", get(rest::stats::get_stats))
        .route("/v4/loadtracks", get(rest::loadtracks::load_tracks))
        .route("/v4/decodetrack", get(rest::decodetrack::decode_track))
        .route("/v4/decodetracks", post(rest::decodetrack::decode_tracks))
        .route("/v4/sessions/:session_id", patch(rest::session::update_session))
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
        loop {
            interval.tick().await;
            let (total_players, playing_players) =
                stats_state.player_manager.count_players().await;
            let uptime = stats_state.start_time.elapsed().as_millis() as u64;

            let stats = serde_json::json!({
                "op": "stats",
                "players": total_players,
                "playingPlayers": playing_players,
                "uptime": uptime,
                "memory": {
                    "free": 1024 * 1024 * 512u64,
                    "used": 1024 * 1024 * 18u64,
                    "allocated": 1024 * 1024 * 32u64,
                    "reservable": 1024 * 1024 * 512u64
                },
                "cpu": {
                    "cores": num_cpus::get(),
                    "systemLoad": 0.05,
                    "lavalinkLoad": 0.01
                },
                "frameStats": null
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

    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
