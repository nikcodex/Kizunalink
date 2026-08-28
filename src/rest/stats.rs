use axum::{
    extract::State,
    http::HeaderMap,
    response::Json,
};

use crate::models::protocol::StatsPayload;
use crate::rest::auth::require_auth;
use crate::rest::error::LavalinkError;
use crate::stats::{FrameCounters, SystemStats};
use crate::AppState;

pub async fn get_stats(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<StatsPayload>, LavalinkError> {
    require_auth(&headers, &state.password, "/v4/stats")?;

    let system_stats = SystemStats::global();
    system_stats.refresh().await;

    let (total_players, playing_players) = state.player_manager.count_players().await;
    let uptime = state.start_time.elapsed().as_millis() as u64;
    let memory = system_stats.get_memory_stats().await;
    let cpu = system_stats.get_cpu_stats().await;
    let frame_stats = Some(FrameCounters::global().snapshot());

    Ok(Json(StatsPayload {
        players: total_players,
        playing_players,
        uptime,
        memory,
        cpu,
        frame_stats,
    }))
}
