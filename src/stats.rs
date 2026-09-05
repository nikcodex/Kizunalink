use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;
use sysinfo::System;
use tokio::sync::RwLock;

use crate::models::protocol::{CpuStats, FrameStats, MemoryStats};

/// Global frame counters for /v4/stats and /v4/metrics.
pub struct FrameCounters {
    pub sent: AtomicU64,
    pub nulled: AtomicU64,
    pub deficit: AtomicU64,
}

static FRAME_COUNTERS: OnceLock<FrameCounters> = OnceLock::new();

impl FrameCounters {
    pub fn global() -> &'static Self {
        FRAME_COUNTERS.get_or_init(|| Self {
            sent: AtomicU64::new(0),
            nulled: AtomicU64::new(0),
            deficit: AtomicU64::new(0),
        })
    }

    /// Record successfully sent audio frames.
    pub fn record_sent(&self, frames: u64) {
        self.sent.fetch_add(frames, Ordering::Relaxed);
    }

    /// Record nulled / silence frames sent.
    pub fn record_nulled(&self, frames: u64) {
        self.nulled.fetch_add(frames, Ordering::Relaxed);
    }

    /// Record frame deficit / dropped frames.
    pub fn record_deficit(&self, frames: u64) {
        self.deficit.fetch_add(frames, Ordering::Relaxed);
    }

    /// `frameStats` for the per-minute WebSocket broadcast.
    ///
    /// Lavalink v4 sends `null` here when the node has no players, so the
    /// counters are only reported while there is something to report.
    pub fn ws_frame_stats(players: usize) -> serde_json::Value {
        if players == 0 {
            return serde_json::Value::Null;
        }
        let snapshot = Self::global().snapshot();
        serde_json::json!({
            "sent": snapshot.sent,
            "nulled": snapshot.nulled,
            "deficit": snapshot.deficit
        })
    }

    pub fn snapshot(&self) -> FrameStats {
        FrameStats {
            sent: self.sent.load(Ordering::Relaxed),
            nulled: self.nulled.load(Ordering::Relaxed),
            deficit: self.deficit.load(Ordering::Relaxed),
        }
    }
}

/// Global system monitor — initialized once, updated periodically.
pub struct SystemStats {
    system: RwLock<System>,
}

static INSTANCE: OnceLock<SystemStats> = OnceLock::new();

impl SystemStats {
    pub fn global() -> &'static Self {
        INSTANCE.get_or_init(|| Self {
            system: RwLock::new(System::new_all()),
        })
    }

    /// Refresh CPU and memory stats. Should be called every ~5s.
    pub async fn refresh(&self) {
        let mut sys = self.system.write().await;
        // `refresh_all` walks /proc for every process on the host and can block
        // for tens of milliseconds. Running it inline stalls the async worker —
        // the same pool that paces audio frames — so hand it to the blocking
        // pool. The write guard is held throughout, so readers still see a
        // consistent snapshot.
        let mut taken = std::mem::replace(&mut *sys, System::new());
        let join = tokio::task::spawn_blocking(move || {
            taken.refresh_all();
            taken
        });
        match join.await {
            Ok(refreshed) => *sys = refreshed,
            Err(e) => {
                // The task panicked or the runtime is shutting down. `sys` keeps
                // the empty placeholder, which the next refresh repopulates.
                tracing::warn!("system stats refresh failed: {}", e);
            }
        }
    }

    pub async fn get_memory_stats(&self) -> MemoryStats {
        let sys = self.system.read().await;
        let total_mem = sys.total_memory();
        let used_mem = sys.used_memory();

        MemoryStats {
            free: total_mem.saturating_sub(used_mem),
            used: used_mem,
            allocated: total_mem,
            reservable: total_mem,
        }
    }

    pub async fn get_cpu_stats(&self) -> CpuStats {
        let sys = self.system.read().await;
        let cores = sys.cpus().len();
        let system_load = sys.global_cpu_usage() as f64 / 100.0;

        // Estimate KizunaLink's own CPU usage from process
        let pid = sysinfo::get_current_pid().ok();
        let lavalink_load = if let Some(pid) = pid {
            if let Some(proc) = sys.process(pid) {
                proc.cpu_usage() as f64 / (cores as f64 * 100.0)
            } else {
                0.01
            }
        } else {
            0.01
        };

        CpuStats {
            cores,
            system_load,
            lavalink_load,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::protocol::StatsPayload;

    #[test]
    fn ws_frame_stats_is_null_without_players() {
        // Lavalink v4: `frameStats` is null when the node has no players.
        assert!(FrameCounters::ws_frame_stats(0).is_null());
    }

    #[test]
    fn ws_frame_stats_reports_counters_with_players() {
        let stats = FrameCounters::ws_frame_stats(2);
        let obj = stats.as_object().expect("frame stats object");
        assert!(obj.contains_key("sent"));
        assert!(obj.contains_key("nulled"));
        assert!(obj.contains_key("deficit"));
    }

    #[test]
    fn rest_stats_payload_carries_frame_stats_as_null() {
        let payload = StatsPayload {
            players: 0,
            playing_players: 0,
            uptime: 1,
            memory: MemoryStats {
                free: 0,
                used: 0,
                allocated: 0,
                reservable: 0,
            },
            cpu: CpuStats {
                cores: 1,
                system_load: 0.0,
                lavalink_load: 0.0,
            },
            frame_stats: None,
        };
        let value = serde_json::to_value(&payload).expect("stats serialise");
        let obj = value.as_object().expect("stats object");
        // The key is always present on /v4/stats, and always null there.
        assert!(obj.contains_key("frameStats"));
        assert!(obj["frameStats"].is_null());
    }
}
