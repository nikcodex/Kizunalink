// Copyright (c) 2026 nikcodex (KizunaLink)
// Licensed under the MIT License

use std::{net::SocketAddr, sync::Arc};

use axum::{Router, routing::get};
use dashmap::DashMap;
use kizuna_server::{
    api::{rest, ws},
    server::AppState,
};
use kizunalink::common::types::AnyResult;
use tracing::info;

/// Entry point.
///
/// Deliberately *not* `#[tokio::main]`: we must configure the FPU (FTZ/DAZ, P02) on the
/// thread that will later build the runtime, because Linux `clone()` copies the FPU control
/// word into child threads — every tokio worker and every audio thread spawned from this
/// main thread then inherits the denormal-free setting for free.
fn main() -> AnyResult<()> {
    kizunalink::engine::disable_denormals();

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    runtime.block_on(run())
}

async fn run() -> AnyResult<()> {
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
        Some(Arc::new(
            kizuna_server::lavalink::routeplanner::BalancingIpRoutePlanner::new(
                config.route_planner.cidrs.clone(),
            ),
        )
            as Arc<dyn kizunalink::lavalink::routeplanner::RoutePlanner>)
    } else {
        None
    };

    let source_manager = Arc::new(kizunalink::media::sources::SourceManager::new(&config));
    let lyrics_manager = Arc::new(kizunalink::media::lyrics::LyricsManager::new(&config));
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
        rate_limiter: Arc::new(kizuna_server::api::RateLimiter::new(
            config.server.rate_limit_per_minute,
        )),
    });

    kizuna_server::monitoring::prometheus::init(shared_state.clone());

    let mut app = Router::new()
        .route("/v4/websocket", get(ws::websocket_handler))
        .with_state(shared_state.clone())
        .merge(rest::router(shared_state.clone()))
        .layer(tower_http::trace::TraceLayer::new_for_http())
        // S06: outermost layer so flood traffic is rejected before auth work.
        // Needs ConnectInfo below, hence `into_make_service_with_connect_info`.
        .layer(axum::middleware::from_fn_with_state(
            shared_state.clone(),
            kizuna_server::api::rate_limit::rate_limit,
        ));

    if config.metrics.prometheus.enabled {
        app = app.route(
            &config.metrics.prometheus.endpoint,
            get(kizuna_server::monitoring::prometheus::metrics_handler),
        );
        // P10: per-request latency histogram, labeled by method + status only —
        // paths carry unbounded session ids, so they stay out of the label set.
        app = app.layer(axum::middleware::from_fn(
            kizuna_server::monitoring::prometheus::observe_request_latency,
        ));
    }

    if !shared_state.rate_limiter.disabled() {
        let limiter = shared_state.rate_limiter.clone();
        tokio::spawn(async move {
            // Bound memory under IP churn: drop buckets idle for 10+ minutes.
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(300)).await;
                limiter.cleanup(
                    std::time::Instant::now(),
                    std::time::Duration::from_secs(600),
                );
            }
        });
    }

    let ip: std::net::IpAddr = config.server.address.parse()?;
    let address = SocketAddr::from((ip, config.server.port));
    info!("KizunaLink Server listening on {}", address);

    let listener = tokio::net::TcpListener::bind(address).await?;
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;

    Ok(())
}
