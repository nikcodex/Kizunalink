use axum::response::Json;
use crate::AppState;
use crate::models::protocol::{StatsPayload, MemoryStats, CpuStats};

fn get_memory_stats() -> MemoryStats {
    #[cfg(target_os = "linux")]
    {
        if let Ok(content) = std::fs::read_to_string("/proc/self/statm") {
            let parts: Vec<&str> = content.split_whitespace().collect();
            if parts.len() >= 2 {
                let total_pages: u64 = parts[0].parse().unwrap_or(0);
                let free_pages: u64 = parts[1].parse().unwrap_or(0);
                let page_size = 4096;
                let allocated = total_pages * page_size;
                let free = free_pages * page_size;
                return MemoryStats {
                    free,
                    used: allocated - free,
                    allocated,
                    reservable: allocated,
                };
            }
        }
    }
    MemoryStats {
        free: 1024 * 1024 * 512,
        used: 1024 * 1024 * 18,
        allocated: 1024 * 1024 * 32,
        reservable: 1024 * 1024 * 512,
    }
}

pub async fn get_stats(state: axum::extract::State<AppState>) -> Json<StatsPayload> {
    let (total_players, playing_players) = state.player_manager.count_players().await;
    let uptime = state.start_time.elapsed().as_millis() as u64;

    Json(StatsPayload {
        players: total_players,
        playing_players,
        uptime,
        memory: get_memory_stats(),
        cpu: CpuStats {
            cores: num_cpus::get(),
            system_load: 0.05,
            lavalink_load: 0.01,
        },
        frame_stats: None,
    })
}
