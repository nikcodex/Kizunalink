use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
use std::sync::OnceLock;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub server: ServerConfig,
    pub sources: SourcesConfig,
    #[serde(default)]
    pub ratelimit: RatelimitConfig,
    #[serde(default)]
    pub logging: LoggingConfig,
    #[serde(default)]
    pub metrics: MetricsConfig,
    #[serde(default)]
    pub security: SecurityConfig,
    #[serde(default)]
    pub proxy: ProxyConfig,
    #[serde(default = "default_queue_max_history")]
    pub queue_max_history: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub password: String,
    #[serde(default = "default_max_connections")]
    pub max_connections: usize,
    #[serde(default = "default_request_timeout")]
    pub request_timeout_secs: u64,
    #[serde(default = "default_queue_max_history")]
    pub queue_max_history: usize,
}

fn default_max_connections() -> usize {
    1000
}

fn default_request_timeout() -> u64 {
    30
}

fn default_queue_max_history() -> usize {
    50
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourcesConfig {
    pub jiosaavn: bool,
    pub spotify: bool,
    pub youtube: bool,
    #[serde(default = "default_true")]
    pub soundcloud: bool,
    #[serde(default = "default_true")]
    pub bandcamp: bool,
    #[serde(default = "default_true")]
    pub twitch: bool,
    #[serde(default)]
    pub vimeo: bool,
    #[serde(default)]
    pub niconico: bool,
    #[serde(default)]
    pub http: bool,
    #[serde(default)]
    pub local: bool,
    #[serde(default = "default_true")]
    pub applemusic: bool,
    #[serde(default = "default_true")]
    pub deezer: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RatelimitConfig {
    #[serde(default)]
    pub ip_blocks: Vec<String>,
    #[serde(default = "default_strategy")]
    pub strategy: String,
    #[serde(default)]
    pub excluded_ips: Vec<String>,
    #[serde(default = "default_retry_limit")]
    pub retry_limit: u32,
    #[serde(default = "default_max_requests")]
    pub max_requests: u32,
    #[serde(default = "default_window_secs")]
    pub window_secs: u64,
    #[serde(default = "default_burst")]
    pub burst: u32,
}

fn default_strategy() -> String {
    "RotatingIpRoutePlanner".to_string()
}

fn default_retry_limit() -> u32 {
    4
}

fn default_max_requests() -> u32 {
    60
}

fn default_window_secs() -> u64 {
    60
}

fn default_burst() -> u32 {
    10
}

impl Default for RatelimitConfig {
    fn default() -> Self {
        Self {
            ip_blocks: Vec::new(),
            strategy: default_strategy(),
            excluded_ips: Vec::new(),
            retry_limit: default_retry_limit(),
            max_requests: default_max_requests(),
            window_secs: default_window_secs(),
            burst: default_burst(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoggingConfig {
    #[serde(default = "default_log_level")]
    pub level: String,
    #[serde(default)]
    pub file: Option<String>,
    #[serde(default = "default_true")]
    pub colored: bool,
}

fn default_log_level() -> String {
    "info".to_string()
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: default_log_level(),
            file: None,
            colored: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricsConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_metrics_port")]
    pub port: u16,
}

fn default_metrics_port() -> u16 {
    9090
}

impl Default for MetricsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            port: default_metrics_port(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecurityConfig {
    #[serde(default = "default_max_body_size")]
    pub max_body_size: usize,
    #[serde(default = "default_true")]
    pub ssrf_protection: bool,
    #[serde(default)]
    pub blocked_hosts: Vec<String>,
}

fn default_max_body_size() -> usize {
    1_048_576 // 1MB
}

impl Default for SecurityConfig {
    fn default() -> Self {
        Self {
            max_body_size: default_max_body_size(),
            ssrf_protection: true,
            blocked_hosts: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxyConfig {
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub password: Option<String>,
    #[serde(default = "default_proxy_timeout")]
    pub timeout_secs: u64,
}

fn default_proxy_timeout() -> u64 {
    30
}

impl Default for ProxyConfig {
    fn default() -> Self {
        Self {
            url: None,
            username: None,
            password: None,
            timeout_secs: default_proxy_timeout(),
        }
    }
}

impl ProxyConfig {
    pub fn apply_to_builder(&self, builder: reqwest::ClientBuilder) -> reqwest::ClientBuilder {
        if let Some(ref url) = self.url {
            let mut proxy = reqwest::Proxy::all(url).expect("Invalid proxy URL");
            if let (Some(ref user), Some(ref pass)) = (&self.username, &self.password) {
                proxy = proxy.basic_auth(user.as_str(), pass.as_str());
            }
            builder.proxy(proxy)
        } else {
            builder
        }
    }
}

/// Create a reqwest::ClientBuilder with proxy settings from config applied.
pub fn http_client_builder(proxy: &ProxyConfig) -> reqwest::ClientBuilder {
    proxy.apply_to_builder(reqwest::Client::builder())
}

static GLOBAL_PROXY: OnceLock<ProxyConfig> = OnceLock::new();

/// Initialize the global proxy config (call once at startup).
pub fn init_proxy(config: ProxyConfig) {
    let _ = GLOBAL_PROXY.set(config);
}

/// Get the global proxy config (returns default if not initialized).
pub fn global_proxy() -> &'static ProxyConfig {
    GLOBAL_PROXY.get_or_init(ProxyConfig::default)
}

/// Create a reqwest::Client with proxy settings from config applied.
pub fn http_client() -> reqwest::Client {
    global_proxy()
        .apply_to_builder(reqwest::Client::builder())
        .build()
        .expect("Failed to build HTTP client")
}

static GLOBAL_SECURITY: OnceLock<SecurityConfig> = OnceLock::new();

/// Initialize the global security config (call once at startup).
pub fn init_security(config: SecurityConfig) {
    let _ = GLOBAL_SECURITY.set(config);
}

/// Get the global security config (returns default if not initialized).
pub fn global_security() -> &'static SecurityConfig {
    GLOBAL_SECURITY.get_or_init(SecurityConfig::default)
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            server: ServerConfig {
                host: "0.0.0.0".to_string(),
                port: 2333,
                password: "youshallnotpass".to_string(),
                max_connections: default_max_connections(),
                request_timeout_secs: default_request_timeout(),
                queue_max_history: default_queue_max_history(),
            },
            sources: SourcesConfig {
                jiosaavn: true,
                spotify: true,
                youtube: true,
                soundcloud: true,
                bandcamp: true,
                twitch: true,
                vimeo: false,
                niconico: false,
                http: true,
                local: false,
                applemusic: true,
                deezer: true,
            },
            ratelimit: RatelimitConfig::default(),
            logging: LoggingConfig::default(),
            metrics: MetricsConfig::default(),
            security: SecurityConfig::default(),
            proxy: ProxyConfig::default(),
            queue_max_history: default_queue_max_history(),
        }
    }
}

impl AppConfig {
    /// Load application configuration.
    ///
    /// Configuration priority:
    /// 1. Environment variables (loaded via dotenvy / process environment) override config.toml values.
    /// 2. `config.toml` file in working directory.
    /// 3. Default fallback values.
    pub fn load() -> Self {
        dotenvy::dotenv().ok();

        let mut config = if Path::new("config.toml").exists() {
            if let Ok(content) = fs::read_to_string("config.toml") {
                if let Ok(mut c) = toml::from_str::<AppConfig>(&content) {
                    if c.server.queue_max_history != 50 && c.queue_max_history == 50 {
                        c.queue_max_history = c.server.queue_max_history;
                    } else if c.queue_max_history != 50 && c.server.queue_max_history == 50 {
                        c.server.queue_max_history = c.queue_max_history;
                    }
                    c
                } else {
                    Self::default()
                }
            } else {
                Self::default()
            }
        } else {
            Self::default()
        };

        // Environment variables override config.toml values
        if let Ok(host) = std::env::var("KIZUNA_HOST") {
            config.server.host = host;
        }
        if let Some(port) = std::env::var("KIZUNA_PORT").ok().and_then(|p| p.parse().ok()) {
            config.server.port = port;
        }
        if let Ok(password) = std::env::var("KIZUNA_PASSWORD") {
            config.server.password = password;
        }
        if let Ok(blocks) = std::env::var("KIZUNA_IP_BLOCKS") {
            config.ratelimit.ip_blocks = blocks
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
        }
        if let Ok(strategy) = std::env::var("KIZUNA_ROUTEPLANNER_STRATEGY") {
            config.ratelimit.strategy = strategy;
        }
        if let Some(max_req) = std::env::var("KIZUNA_MAX_REQUESTS").ok().and_then(|v| v.parse().ok()) {
            config.ratelimit.max_requests = max_req;
        }
        if let Ok(level) = std::env::var("KIZUNA_LOG_LEVEL") {
            config.logging.level = level;
        }
        if let Ok(file) = std::env::var("KIZUNA_LOG_FILE") {
            config.logging.file = Some(file);
        }
        if let Ok(colored) = std::env::var("KIZUNA_LOG_COLORED") {
            config.logging.colored = colored != "false";
        }
        if let Ok(proxy_url) = std::env::var("KIZUNA_PROXY_URL") {
            config.proxy.url = Some(proxy_url);
        }
        if let Ok(proxy_user) = std::env::var("KIZUNA_PROXY_USER") {
            config.proxy.username = Some(proxy_user);
        }
        if let Ok(proxy_pass) = std::env::var("KIZUNA_PROXY_PASS") {
            config.proxy.password = Some(proxy_pass);
        }
        if let Some(proxy_timeout) = std::env::var("KIZUNA_PROXY_TIMEOUT").ok().and_then(|v| v.parse().ok()) {
            config.proxy.timeout_secs = proxy_timeout;
        }
        if let Some(max_conn) = std::env::var("KIZUNA_MAX_CONNECTIONS").ok().and_then(|v| v.parse().ok()) {
            config.server.max_connections = max_conn;
        }
        if let Some(history) = std::env::var("KIZUNA_QUEUE_MAX_HISTORY").ok().and_then(|v| v.parse().ok()) {
            config.queue_max_history = history;
            config.server.queue_max_history = history;
        }

        config
    }
}
