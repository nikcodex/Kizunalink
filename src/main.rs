// Copyright (c) 2026 nikcodex (KizunaLink)
// Licensed under the MIT License

use std::{net::SocketAddr, sync::Arc};

use axum::{Router, routing::get};
use dashmap::DashMap;
use kizunalink::{common::types::AnyResult, monitoring, rest, server::AppState, ws};
use tracing::info;

#[tokio::main]
async fn main() -> AnyResult<()> {
    let config = kizunalink::config::AppConfig::load().await?;

    kizunalink::common::logger::init(
        config
            .logging
            .as_ref()
            .unwrap_or(&kizunalink::config::LoggingConfig::default()),
    );

    kizunalink::common::banner::print_banner(&kizunalink::common::banner::BannerInfo::default());

    info!("KizunaLink Server starting...");

    let routeplanner = if config.route_planner.enabled && !config.route_planner.cidrs.is_empty() {
        Some(
            Arc::new(kizunalink::routeplanner::BalancingIpRoutePlanner::new(
                config.route_planner.cidrs.clone(),
            )) as Arc<dyn kizunalink::routeplanner::RoutePlanner>,
        )
    } else {
        None
    };

    let source_manager = Arc::new(kizunalink::sources::SourceManager::new(&config));
    let lyrics_manager = Arc::new(kizunalink::lyrics::LyricsManager::new(&config));
    let youtube_ctx = source_manager.youtube_stream_ctx.clone();

    let process_stat = perf_monitor::cpu::ProcessStat::cur().map_err(|e| {
        Box::new(std::io::Error::other(format!(
            "failed to init ProcessStat: {e}"
        ))) as kizunalink::common::types::AnyError
    })?;

    let shared_state = Arc::new(AppState {
        start_time: std::time::Instant::now(),
        sessions: DashMap::new(),
        resumable_sessions: DashMap::new(),
        routeplanner,
        source_manager,
        lyrics_manager,
        config: config.clone(),
        youtube: youtube_ctx,
        system_state: parking_lot::Mutex::new(sysinfo::System::new_all()),
        last_system_refresh: parking_lot::Mutex::new(std::time::Instant::now()),
        process_stat: parking_lot::Mutex::new(process_stat),
    });

    monitoring::prometheus::init(shared_state.clone());

    let mut app = Router::new()
        .route("/v4/websocket", get(ws::websocket_handler))
        .with_state(shared_state.clone())
        .merge(rest::router(shared_state.clone()))
        .layer(tower_http::trace::TraceLayer::new_for_http());

    if config.metrics.prometheus.enabled {
        app = app.route(
            &config.metrics.prometheus.endpoint,
            get(monitoring::prometheus::metrics_handler),
        );
    }

    let ip: std::net::IpAddr = config.server.address.parse()?;
    let address = SocketAddr::from((ip, config.server.port));
    info!("KizunaLink Server listening on {}", address);

    let listener = tokio::net::TcpListener::bind(address).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
