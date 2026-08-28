use prometheus::{
    opts, register_gauge_with_registry, register_int_counter_with_registry,
    register_int_gauge_with_registry, Encoder, Gauge, IntCounter, IntGauge, Registry, TextEncoder,
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
    // Source-specific metrics
    pub source_youtube: IntCounter,
    pub source_spotify: IntCounter,
    pub source_soundcloud: IntCounter,
    pub source_jiosaavn: IntCounter,
    pub source_deezer: IntCounter,
    pub source_applemusic: IntCounter,
    pub source_bandcamp: IntCounter,
    pub source_twitch: IntCounter,
    pub source_vimeo: IntCounter,
    pub source_niconico: IntCounter,
    pub source_http: IntCounter,
    // Mirror metrics
    pub mirror_youtube: IntCounter,
    pub mirror_jiosaavn: IntCounter,
    // Error metrics
    pub errors_auth: IntCounter,
    pub errors_rate_limit: IntCounter,
    pub errors_not_found: IntCounter,
    pub errors_internal: IntCounter,
    // Request metrics
    pub requests_total: IntCounter,
    pub requests_latency_ms: IntCounter,
}

static INSTANCE: OnceLock<Metrics> = OnceLock::new();

impl Metrics {
    pub fn global() -> &'static Self {
        INSTANCE.get_or_init(|| {
            let registry = Registry::new();

            let players_total = register_int_gauge_with_registry!(
                opts!("kizunalink_players_total", "Total number of players"),
                registry
            )
            .unwrap();

            let players_playing = register_int_gauge_with_registry!(
                opts!(
                    "kizunalink_players_playing",
                    "Number of actively playing players"
                ),
                registry
            )
            .unwrap();

            let uptime_seconds = register_gauge_with_registry!(
                opts!("kizunalink_uptime_seconds", "Server uptime in seconds"),
                registry
            )
            .unwrap();

            let frames_sent = register_int_counter_with_registry!(
                opts!("kizunalink_frames_sent_total", "Total audio frames sent"),
                registry
            )
            .unwrap();

            let frames_nulled = register_int_counter_with_registry!(
                opts!("kizunalink_frames_nulled_total", "Total null frames sent"),
                registry
            )
            .unwrap();

            let frames_deficit = register_int_counter_with_registry!(
                opts!(
                    "kizunalink_frames_deficit_total",
                    "Total frame deficit events"
                ),
                registry
            )
            .unwrap();

            let bytes_sent = register_int_counter_with_registry!(
                opts!("kizunalink_bytes_sent_total", "Total bytes sent to voice"),
                registry
            )
            .unwrap();

            let tracks_loaded = register_int_counter_with_registry!(
                opts!(
                    "kizunalink_tracks_loaded_total",
                    "Total tracks successfully loaded"
                ),
                registry
            )
            .unwrap();

            let tracks_failed = register_int_counter_with_registry!(
                opts!(
                    "kizunalink_tracks_failed_total",
                    "Total track load failures"
                ),
                registry
            )
            .unwrap();

            let ws_connections = register_int_gauge_with_registry!(
                opts!("kizunalink_ws_connections", "Current WebSocket connections"),
                registry
            )
            .unwrap();

            let memory_used_bytes = register_gauge_with_registry!(
                opts!("kizunalink_memory_used_bytes", "Memory usage in bytes"),
                registry
            )
            .unwrap();

            let cpu_usage = register_gauge_with_registry!(
                opts!("kizunalink_cpu_usage", "CPU usage ratio (0.0-1.0)"),
                registry
            )
            .unwrap();

            let source_requests = register_int_counter_with_registry!(
                opts!(
                    "kizunalink_source_requests_total",
                    "Total source API requests"
                ),
                registry
            )
            .unwrap();

            let active_sessions = register_int_gauge_with_registry!(
                opts!(
                    "kizunalink_active_sessions",
                    "Number of active WebSocket sessions"
                ),
                registry
            )
            .unwrap();

            let source_youtube = register_int_counter_with_registry!(
                opts!("kizunalink_source_youtube_total", "YouTube source requests"),
                registry
            )
            .unwrap();
            let source_spotify = register_int_counter_with_registry!(
                opts!("kizunalink_source_spotify_total", "Spotify source requests"),
                registry
            )
            .unwrap();
            let source_soundcloud = register_int_counter_with_registry!(
                opts!(
                    "kizunalink_source_soundcloud_total",
                    "SoundCloud source requests"
                ),
                registry
            )
            .unwrap();
            let source_jiosaavn = register_int_counter_with_registry!(
                opts!(
                    "kizunalink_source_jiosaavn_total",
                    "JioSaavn source requests"
                ),
                registry
            )
            .unwrap();
            let source_deezer = register_int_counter_with_registry!(
                opts!("kizunalink_source_deezer_total", "Deezer source requests"),
                registry
            )
            .unwrap();
            let source_applemusic = register_int_counter_with_registry!(
                opts!(
                    "kizunalink_source_applemusic_total",
                    "Apple Music source requests"
                ),
                registry
            )
            .unwrap();
            let source_bandcamp = register_int_counter_with_registry!(
                opts!(
                    "kizunalink_source_bandcamp_total",
                    "Bandcamp source requests"
                ),
                registry
            )
            .unwrap();
            let source_twitch = register_int_counter_with_registry!(
                opts!("kizunalink_source_twitch_total", "Twitch source requests"),
                registry
            )
            .unwrap();
            let source_vimeo = register_int_counter_with_registry!(
                opts!("kizunalink_source_vimeo_total", "Vimeo source requests"),
                registry
            )
            .unwrap();
            let source_niconico = register_int_counter_with_registry!(
                opts!(
                    "kizunalink_source_niconico_total",
                    "NicoNico source requests"
                ),
                registry
            )
            .unwrap();
            let source_http = register_int_counter_with_registry!(
                opts!("kizunalink_source_http_total", "HTTP source requests"),
                registry
            )
            .unwrap();

            let mirror_youtube = register_int_counter_with_registry!(
                opts!(
                    "kizunalink_mirror_youtube_total",
                    "Tracks mirrored to YouTube"
                ),
                registry
            )
            .unwrap();
            let mirror_jiosaavn = register_int_counter_with_registry!(
                opts!(
                    "kizunalink_mirror_jiosaavn_total",
                    "Tracks mirrored to JioSaavn"
                ),
                registry
            )
            .unwrap();

            let errors_auth = register_int_counter_with_registry!(
                opts!("kizunalink_errors_auth_total", "Authentication failures"),
                registry
            )
            .unwrap();
            let errors_rate_limit = register_int_counter_with_registry!(
                opts!(
                    "kizunalink_errors_rate_limit_total",
                    "Rate limit rejections"
                ),
                registry
            )
            .unwrap();
            let errors_not_found = register_int_counter_with_registry!(
                opts!("kizunalink_errors_not_found_total", "404 errors"),
                registry
            )
            .unwrap();
            let errors_internal = register_int_counter_with_registry!(
                opts!("kizunalink_errors_internal_total", "500 errors"),
                registry
            )
            .unwrap();

            let requests_total = register_int_counter_with_registry!(
                opts!("kizunalink_requests_total", "Total HTTP requests"),
                registry
            )
            .unwrap();
            let requests_latency_ms = register_int_counter_with_registry!(
                opts!(
                    "kizunalink_requests_latency_ms_total",
                    "Total request latency in ms"
                ),
                registry
            )
            .unwrap();

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
                source_youtube,
                source_spotify,
                source_soundcloud,
                source_jiosaavn,
                source_deezer,
                source_applemusic,
                source_bandcamp,
                source_twitch,
                source_vimeo,
                source_niconico,
                source_http,
                mirror_youtube,
                mirror_jiosaavn,
                errors_auth,
                errors_rate_limit,
                errors_not_found,
                errors_internal,
                requests_total,
                requests_latency_ms,
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

    pub fn update_from_stats(
        &self,
        players: usize,
        playing: usize,
        uptime_ms: u64,
        mem_used: u64,
        cpu: f64,
    ) {
        self.players_total.set(players as i64);
        self.players_playing.set(playing as i64);
        self.uptime_seconds.set(uptime_ms as f64 / 1000.0);
        self.memory_used_bytes.set(mem_used as f64);
        self.cpu_usage.set(cpu);
    }

    /// Increment the counter for a specific source.
    pub fn inc_source(&self, source: &str) {
        match source {
            "youtube" => self.source_youtube.inc(),
            "spotify" => self.source_spotify.inc(),
            "soundcloud" => self.source_soundcloud.inc(),
            "jiosaavn" => self.source_jiosaavn.inc(),
            "deezer" => self.source_deezer.inc(),
            "applemusic" => self.source_applemusic.inc(),
            "bandcamp" => self.source_bandcamp.inc(),
            "twitch" => self.source_twitch.inc(),
            "vimeo" => self.source_vimeo.inc(),
            "niconico" => self.source_niconico.inc(),
            "http" => self.source_http.inc(),
            _ => self.source_requests.inc(),
        }
    }
}
