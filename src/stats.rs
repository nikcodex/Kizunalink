use std::sync::OnceLock;
use sysinfo::System;
use tokio::sync::RwLock;

use crate::models::protocol::{CpuStats, MemoryStats};

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
        sys.refresh_all();
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
