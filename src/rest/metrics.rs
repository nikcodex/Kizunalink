use axum::{extract::State, http::HeaderMap, response::Response};

use crate::metrics::Metrics;
use crate::rest::auth::require_auth;
use crate::rest::error::LavalinkError;
use crate::stats::SystemStats;
use crate::AppState;

pub async fn get_metrics(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Response<String>, LavalinkError> {
    require_auth(&headers, &state.password, "/v4/metrics")?;

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
    );

    let body = metrics.encode_text();

    Ok(Response::builder()
        .header("Content-Type", "text/plain; version=0.0.4; charset=utf-8")
        .body(body)
        .unwrap())
}
