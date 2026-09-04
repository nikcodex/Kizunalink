use axum::{extract::State, http::HeaderMap, response::Response};

use crate::metrics::Metrics;
use crate::ratelimit::extract_ip;
use crate::rest::auth::require_auth;
use crate::rest::error::LavalinkError;
use crate::stats::SystemStats;
use crate::AppState;

pub async fn get_metrics(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Response<String>, LavalinkError> {
    require_auth(&headers, &state.password, "/v4/metrics")?;

    // Rate limit check
    let ip = extract_ip(&headers, "0.0.0.0");
    if !state.rate_limiter.check(&ip) {
        return Err(LavalinkError::new(
            axum::http::StatusCode::TOO_MANY_REQUESTS,
            "Rate limit exceeded",
            "/v4/metrics",
        ));
    }

    let system_stats = SystemStats::global();
    let metrics = Metrics::global();

    // Update metrics from current stats
    let (total_players, playing_players) = state.player_manager.count_players().await;
    let uptime = state.start_time.elapsed().as_millis() as u64;
    let memory = system_stats.get_memory_stats().await;
    let cpu = system_stats.get_cpu_stats().await;
    metrics.update_from_stats(
        total_players,
        playing_players,
        uptime,
        memory.used,
        cpu.lavalink_load,
        crate::models::protocol::FrameStats {
            sent: crate::stats::FrameCounters::global()
                .sent
                .load(std::sync::atomic::Ordering::Relaxed),
            nulled: crate::stats::FrameCounters::global()
                .nulled
                .load(std::sync::atomic::Ordering::Relaxed),
            deficit: crate::stats::FrameCounters::global()
                .deficit
                .load(std::sync::atomic::Ordering::Relaxed),
        },
    );

    let body = metrics.encode_text();

    Ok(Response::builder()
        .header("Content-Type", "text/plain; version=0.0.4; charset=utf-8")
        .body(body)
        .unwrap())
}
