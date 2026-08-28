use prometheus::{
    Encoder, Gauge, IntCounter, IntGauge, Registry, TextEncoder,
    opts, register_int_counter_with_registry, register_int_gauge_with_registry,
    register_gauge_with_registry,
};
use std::sync::OnceLock;

pub struct Metrics {
    pub registry: Registry,
    pub players_total: IntGauge,
    pub players_playing: IntGauge,
    pub uptime_seconds: Gauge,
    pub frames_sent: IntCounter,
    pub frames_nulled: IntCounter,
    pub frames_deficit: IntCounter,
    pub bytes_sent: IntCounter,
    pub tracks_loaded: IntCounter,
    pub tracks_failed: IntCounter,
    pub ws_connections: IntGauge,
    pub memory_used_bytes: Gauge,
    pub cpu_usage: Gauge,
    pub source_requests: IntCounter,
    pub active_sessions: IntGauge,
}

static INSTANCE: OnceLock<Metrics> = OnceLock::new();

impl Metrics {
    pub fn global() -> &'static Self {
        INSTANCE.get_or_init(|| {
            let registry = Registry::new();

            let players_total = register_int_gauge_with_registry!(
                opts!("kizunalink_players_total", "Total number of players"),
                registry
            ).unwrap();

            let players_playing = register_int_gauge_with_registry!(
                opts!("kizunalink_players_playing", "Number of actively playing players"),
                registry
            ).unwrap();

            let uptime_seconds = register_gauge_with_registry!(
                opts!("kizunalink_uptime_seconds", "Server uptime in seconds"),
                registry
            ).unwrap();

            let frames_sent = register_int_counter_with_registry!(
                opts!("kizunalink_frames_sent_total", "Total audio frames sent"),
                registry
            ).unwrap();

            let frames_nulled = register_int_counter_with_registry!(
                opts!("kizunalink_frames_nulled_total", "Total null frames sent"),
                registry
            ).unwrap();

            let frames_deficit = register_int_counter_with_registry!(
                opts!("kizunalink_frames_deficit_total", "Total frame deficit events"),
                registry
            ).unwrap();

            let bytes_sent = register_int_counter_with_registry!(
                opts!("kizunalink_bytes_sent_total", "Total bytes sent to voice"),
                registry
            ).unwrap();

            let tracks_loaded = register_int_counter_with_registry!(
                opts!("kizunalink_tracks_loaded_total", "Total tracks successfully loaded"),
                registry
            ).unwrap();

            let tracks_failed = register_int_counter_with_registry!(
                opts!("kizunalink_tracks_failed_total", "Total track load failures"),
                registry
            ).unwrap();

            let ws_connections = register_int_gauge_with_registry!(
                opts!("kizunalink_ws_connections", "Current WebSocket connections"),
                registry
            ).unwrap();

            let memory_used_bytes = register_gauge_with_registry!(
                opts!("kizunalink_memory_used_bytes", "Memory usage in bytes"),
                registry
            ).unwrap();

            let cpu_usage = register_gauge_with_registry!(
                opts!("kizunalink_cpu_usage", "CPU usage ratio (0.0-1.0)"),
                registry
            ).unwrap();

            let source_requests = register_int_counter_with_registry!(
                opts!("kizunalink_source_requests_total", "Total source API requests"),
                registry
            ).unwrap();

            let active_sessions = register_int_gauge_with_registry!(
                opts!("kizunalink_active_sessions", "Number of active WebSocket sessions"),
                registry
            ).unwrap();

            Metrics {
                registry,
                players_total,
                players_playing,
                uptime_seconds,
                frames_sent,
                frames_nulled,
                frames_deficit,
                bytes_sent,
                tracks_loaded,
                tracks_failed,
                ws_connections,
                memory_used_bytes,
                cpu_usage,
                source_requests,
                active_sessions,
            }
        })
    }

    pub fn encode_text(&self) -> String {
        let encoder = TextEncoder::new();
        let metric_families = self.registry.gather();
        let mut buffer = Vec::new();
        encoder.encode(&metric_families, &mut buffer).unwrap();
        String::from_utf8(buffer).unwrap()
    }

    pub fn update_from_stats(&self, players: usize, playing: usize, uptime_ms: u64, mem_used: u64, cpu: f64) {
        self.players_total.set(players as i64);
        self.players_playing.set(playing as i64);
        self.uptime_seconds.set(uptime_ms as f64 / 1000.0);
        self.memory_used_bytes.set(mem_used as f64);
        self.cpu_usage.set(cpu);
    }
}
