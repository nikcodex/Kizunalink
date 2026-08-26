use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub server: ServerConfig,
    pub sources: SourcesConfig,
    #[serde(default)]
    pub ratelimit: RatelimitConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub password: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourcesConfig {
    pub jiosaavn: bool,
    pub spotify: bool,
    pub youtube: bool,
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
}

fn default_strategy() -> String {
    "RotatingIpRoutePlanner".to_string()
}

fn default_retry_limit() -> u32 {
    4
}

impl Default for RatelimitConfig {
    fn default() -> Self {
        Self {
            ip_blocks: Vec::new(),
            strategy: default_strategy(),
            excluded_ips: Vec::new(),
            retry_limit: default_retry_limit(),
        }
    }
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            server: ServerConfig {
                host: "0.0.0.0".to_string(),
                port: 2333,
                password: "youshallnotpass".to_string(),
            },
            sources: SourcesConfig {
                jiosaavn: true,
                spotify: true,
                youtube: true,
            },
            ratelimit: RatelimitConfig::default(),
        }
    }
}

impl AppConfig {
    pub fn load() -> Self {
        if Path::new("config.toml").exists() {
            if let Ok(content) = fs::read_to_string("config.toml") {
                if let Ok(config) = toml::from_str::<AppConfig>(&content) {
                    return config;
                }
            }
        }

        let host = std::env::var("KIZUNA_HOST").unwrap_or_else(|_| "0.0.0.0".to_string());
        let port = std::env::var("KIZUNA_PORT")
            .ok()
            .and_then(|p| p.parse().ok())
            .unwrap_or(2333);
        let password = std::env::var("KIZUNA_PASSWORD").unwrap_or_else(|_| "youshallnotpass".to_string());

        let mut ratelimit = RatelimitConfig::default();
        if let Ok(blocks) = std::env::var("KIZUNA_IP_BLOCKS") {
            ratelimit.ip_blocks = blocks.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
        }
        if let Ok(strategy) = std::env::var("KIZUNA_ROUTEPLANNER_STRATEGY") {
            ratelimit.strategy = strategy;
        }

        Self {
            server: ServerConfig { host, port, password },
            sources: SourcesConfig {
                jiosaavn: true,
                spotify: true,
                youtube: true,
            },
            ratelimit,
        }
    }
}
