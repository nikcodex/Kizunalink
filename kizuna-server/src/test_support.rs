// Copyright (c) 2026 nikcodex (KizunaLink)
// Licensed under the MIT License

//! In-process test harness for REST + WebSocket integration tests.
//!
//! Builds a real [`AppState`] and the real axum [`Router`] (exactly what
//! `kizuna_server::main::run` installs) so tests exercise the full middleware
//! stack — auth, rate limiting, response headers — without binding a socket or
//! reaching the network. Source managers are constructed but never queried, so
//! integration tests remain hermetic and deterministic.

use std::sync::Arc;

use axum::Router;
use dashmap::DashMap;

use crate::{
    api::{RateLimiter, rest},
    server::AppState,
};

/// Authorization token used across tests (mirrors the config default).
pub const AUTH_TOKEN: &str = "youshallnotpass";
/// Default User-Id used to create sessions in integration tests.
pub const TEST_USER_ID: u64 = 424242;

/// Build a fresh [`AppState`] with sane defaults.
///
/// Source managers are real but inert (never queried), so tests are hermetic.
pub fn test_state() -> Arc<AppState> {
    // `AppConfig` parses from TOML (no `Default` impl); every `ServerConfig`
    // field carries a serde default, so an empty `[server]` is a valid config.
    let config: kizunalink::config::AppConfig =
        toml::from_str("[server]").expect("minimal config parses");

    let source_manager = Arc::new(kizunalink::media::sources::SourceManager::new(&config));
    let lyrics_manager = Arc::new(kizunalink::media::lyrics::LyricsManager::new(&config));

    // `ProcessStat::cur` reads the process CPU time; on Linux (CI + prod) this
    // always succeeds. Tests never read the value, so a hard failure is fine.
    let process_stat =
        perf_monitor::cpu::ProcessStat::cur().expect("ProcessStat::cur on supported platform");

    let rate_limiter = Arc::new(RateLimiter::new(config.server.rate_limit_per_minute));

    Arc::new(AppState {
        start_time: std::time::Instant::now(),
        sessions: DashMap::new(),
        resumable_sessions: DashMap::new(),
        routeplanner: None,
        source_manager,
        lyrics_manager,
        config,
        youtube: None,
        system_state: parking_lot::Mutex::new(sysinfo::System::new_all()),
        last_system_refresh: parking_lot::Mutex::new(std::time::Instant::now()),
        process_stat: parking_lot::Mutex::new(process_stat),
        rate_limiter,
    })
}

/// Build the production axum [`Router`] (including the WS route, auth, and rate
/// limiting) around a given state. Mirrors the wiring in `main.rs`.
pub fn test_router(state: Arc<AppState>) -> Router {
    use axum::routing::get;

    let mut app = Router::new()
        .route("/health", get(crate::health::health_check))
        .route("/v4/websocket", get(crate::api::ws::websocket_handler))
        .with_state(state.clone())
        .merge(rest::router(state.clone()))
        .layer(tower_http::trace::TraceLayer::new_for_http())
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            crate::api::rate_limit::rate_limit,
        ));

    if state.config.metrics.prometheus.enabled {
        app = app.route(
            &state.config.metrics.prometheus.endpoint,
            get(crate::monitoring::prometheus::metrics_handler),
        );
        app = app.layer(axum::middleware::from_fn(
            crate::monitoring::prometheus::observe_request_latency,
        ));
    }

    app
}

/// Spawn an in-process axum server on an ephemeral port and return the base URL
/// plus the associated [`AppState`] (so tests can inspect side effects).
pub async fn spawn_test_server() -> (String, Arc<AppState>) {
    let state = test_state();
    let app = test_router(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral port");
    let addr = listener.local_addr().expect("local addr");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("test server serve");
    });
    (format!("http://{addr}"), state)
}
