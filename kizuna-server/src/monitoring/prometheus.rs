// Copyright (c) 2026 nikcodex (KizunaLink)
// Licensed under the MIT License

use std::{
    sync::{Arc, LazyLock},
    time::Duration,
};

use axum::{
    extract::Request,
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
};
use prometheus::{
    Encoder, Gauge, Histogram, HistogramOpts, HistogramVec, Opts, Registry, TextEncoder,
};
use tokio::time::interval;
use tracing::{error, info};

use crate::server::AppState;

const NAMESPACE: &str = "lavalink";

struct NodeMetrics {
    players: Gauge,
    playing_players: Gauge,
    uptime: Gauge,
    memory_free: Gauge,
    memory_used: Gauge,
    memory_allocated: Gauge,
    memory_reservable: Gauge,
    cpu_cores: Gauge,
    cpu_system_load: Gauge,
    cpu_lavalink_load: Gauge,
}

static REGISTRY: LazyLock<Registry> = LazyLock::new(Registry::new);

static METRICS: LazyLock<NodeMetrics> = LazyLock::new(|| {
    // These `expect`s are intentionally loud: a metric whose name collides with
    // an existing registration means a naming bug, and silently ignoring it
    // would corrupt the exposition. The error message names the metric.
    macro_rules! gauge {
        ($name:expr, $help:expr) => {
            Gauge::with_opts(Opts::new($name, $help).namespace(NAMESPACE))
                .expect("duplicate metric name at startup")
        };
    }
    macro_rules! register {
        ($metric:expr, $name:expr) => {
            REGISTRY
                .register(Box::new($metric))
                .expect(concat!("unable to register metric ", $name))
        };
    }

    let metrics = NodeMetrics {
        players: gauge!("players_total", "Total connected players"),
        playing_players: gauge!("playing_players_total", "Players currently playing"),
        uptime: gauge!("uptime_milliseconds", "Node uptime in ms"),
        memory_free: gauge!("memory_free_bytes", "Free memory"),
        memory_used: gauge!("memory_used_bytes", "Used memory"),
        memory_allocated: gauge!("memory_allocated_bytes", "Allocated memory"),
        memory_reservable: gauge!("memory_reservable_bytes", "Reservable memory"),
        cpu_cores: gauge!("cpu_cores", "CPU cores count"),
        cpu_system_load: gauge!("cpu_system_load_percentage", "System CPU load"),
        cpu_lavalink_load: gauge!("cpu_lavalink_load_percentage", "Process CPU load"),
    };

    register!(metrics.players.clone(), "players_total");
    register!(metrics.playing_players.clone(), "playing_players_total");
    register!(metrics.uptime.clone(), "uptime_milliseconds");
    register!(metrics.memory_free.clone(), "memory_free_bytes");
    register!(metrics.memory_used.clone(), "memory_used_bytes");
    register!(metrics.memory_allocated.clone(), "memory_allocated_bytes");
    register!(metrics.memory_reservable.clone(), "memory_reservable_bytes");
    register!(metrics.cpu_cores.clone(), "cpu_cores");
    register!(
        metrics.cpu_system_load.clone(),
        "cpu_system_load_percentage"
    );
    register!(
        metrics.cpu_lavalink_load.clone(),
        "cpu_lavalink_load_percentage"
    );

    metrics
});

/// Initializes the metrics system and starts the background observer.
pub fn init(state: Arc<AppState>) {
    let config = &state.config.metrics.prometheus;
    if !config.enabled {
        return;
    }

    info!("Initializing Prometheus metrics at {}", config.endpoint);

    // Ensure metrics are initialized
    LazyLock::force(&METRICS);

    tokio::spawn(async move {
        let mut ticker = interval(Duration::from_secs(state.config.server.stats_interval));
        loop {
            ticker.tick().await;
            update_metrics(&state);
        }
    });
}

fn update_metrics(state: &AppState) {
    let stats = crate::monitoring::collect_stats(state, None);

    METRICS.players.set(stats.players as f64);
    METRICS.playing_players.set(stats.playing_players as f64);
    METRICS.uptime.set(stats.uptime as f64);
    METRICS.memory_free.set(stats.memory.free as f64);
    METRICS.memory_used.set(stats.memory.used as f64);
    METRICS.memory_allocated.set(stats.memory.allocated as f64);
    METRICS
        .memory_reservable
        .set(stats.memory.reservable as f64);
    METRICS.cpu_cores.set(stats.cpu.cores as f64);
    METRICS.cpu_system_load.set(stats.cpu.system_load);
    METRICS.cpu_lavalink_load.set(stats.cpu.lavalink_load);
}

/// P10: HTTP latency histogram. Labels are deliberately `{method, status}` only —
/// request paths contain session/guild ids and would explode label cardinality.
static REQUEST_LATENCY: LazyLock<HistogramVec> = LazyLock::new(|| {
    let opts = HistogramOpts::new(
        "http_request_duration_seconds",
        "End-to-end HTTP request latency by method and status code",
    )
    .namespace(NAMESPACE)
    .buckets(prometheus::exponential_buckets(0.0005, 2.0, 15).expect("constant bucket spec"));
    let vec = HistogramVec::new(opts, &["method", "status"]).expect("valid label set");
    REGISTRY
        .register(Box::new(vec.clone()))
        .expect("first registration");
    vec
});

/// P10: time from API request to playback armed (source resolution + decoder start).
static TRACK_LOAD_DURATION: LazyLock<Histogram> = LazyLock::new(|| {
    let h = Histogram::with_opts(
        HistogramOpts::new(
            "track_load_duration_seconds",
            "Time spent resolving a track request and arming playback",
        )
        .namespace(NAMESPACE)
        .buckets(vec![0.01, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0]),
    )
    .expect("constant bucket spec");
    REGISTRY
        .register(Box::new(h.clone()))
        .expect("first registration");
    h
});

/// Observe one track-load duration (seconds) from the API layer.
pub fn observe_track_load(seconds: f64) {
    TRACK_LOAD_DURATION.observe(seconds);
}

/// Middleware: record end-to-end latency for every request (P10).
pub async fn observe_request_latency(req: Request, next: Next) -> Response {
    let method = req.method().as_str().to_owned();
    let start = std::time::Instant::now();
    let resp = next.run(req).await;
    let status = resp.status().as_u16().to_string();
    REQUEST_LATENCY
        .with_label_values(&[&method, &status])
        .observe(start.elapsed().as_secs_f64());
    resp
}

/// Axum handler for Prometheus metrics.
pub async fn metrics_handler() -> Response {
    let encoder = TextEncoder::new();
    let metric_families = REGISTRY.gather();
    let mut buffer = Vec::new();

    if let Err(e) = encoder.encode(&metric_families, &mut buffer) {
        error!("Failed to encode prometheus metrics: {}", e);
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }

    Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", encoder.format_type())
        .body(buffer.into())
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}
