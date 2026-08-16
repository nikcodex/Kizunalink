use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub server: ServerConfig,
    pub sources: SourcesConfig,
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

        Self {
            server: ServerConfig { host, port, password },
            sources: SourcesConfig {
                jiosaavn: true,
                spotify: true,
                youtube: true,
            },
        }
    }
}
