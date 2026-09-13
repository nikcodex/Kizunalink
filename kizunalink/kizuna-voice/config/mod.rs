// Copyright (c) 2026 nikcodex (KizunaLink)
// Licensed under the MIT License

pub mod filters;
pub mod lyrics;
pub mod metrics;
pub mod player;
pub mod server;
pub mod sources;

use std::{fs, path::Path};

pub use filters::*;
pub use lyrics::*;
pub use metrics::*;
pub use player::*;
use serde::Deserialize;
pub use server::*;
pub use sources::*;

use crate::common::types::AnyResult;

#[derive(Debug, Deserialize, Clone)]
pub struct AppConfig {
    pub server: ServerConfig,
    #[serde(default)]
    pub route_planner: RoutePlannerConfig,
    #[serde(default)]
    pub sources: SourcesConfig,
    #[serde(default)]
    pub lyrics: LyricsConfig,
    pub logging: Option<LoggingConfig>,
    #[serde(default)]
    pub filters: FiltersConfig,
    #[serde(default)]
    pub player: PlayerConfig,
    #[serde(default)]
    pub metrics: MetricsConfig,
    #[serde(default)]
    pub config_server: Option<ConfigServerConfig>,
}

impl AppConfig {
    /// The `log_println!` lines here intentionally write to the console before the
    /// tracing subscriber exists (and mirror to the log file once it does) — hence the
    /// local N05 `print_stdout` allow.
    #[allow(clippy::print_stdout)]
    pub async fn load() -> AnyResult<Self> {
        let config_path = if Path::new("config.toml").exists() {
            "config.toml"
        } else if Path::new("config.example.toml").exists() {
            tracing::warn!(
                "config.toml not found — falling back to config.example.toml. \
                 For production, copy config.example.toml to config.toml and customize it."
            );
            "config.example.toml"
        } else {
            return Err("config.toml or config.example.toml not found — please create one from config.example.toml".into());
        };

        crate::log_println!("Loading configuration from: {}", config_path);

        let raw = fs::read_to_string(config_path)?;
        if raw.is_empty() {
            return Err(format!("{} is empty", config_path).into());
        }

        let raw_val: toml::Value = toml::from_str(&raw)?;

        if let Some(cs_val) = raw_val.get("config_server") {
            let cs: ConfigServerConfig = cs_val.clone().try_into()?;

            let client = reqwest::Client::new();
            let mut request = client.get(&cs.url);

            if let (Some(u), Some(p)) = (&cs.username, &cs.password) {
                use base64::{Engine as _, engine::general_purpose};
                let auth = format!("{}:{}", u, p);
                let encoded = general_purpose::STANDARD.encode(auth);
                request = request.header("Authorization", format!("Basic {}", encoded));
            }

            let response = request.send().await?;
            if !response.status().is_success() {
                return Err(format!(
                    "Failed to fetch remote config: status {}",
                    response.status()
                )
                .into());
            }

            let remote_toml = response.text().await?;
            let config: Self = toml::from_str(&remote_toml)?;
            config.validate()?;
            return Ok(config);
        }

        let config: Self = toml::from_str(&raw)?;

        // P06: Validate configuration at startup so bad values fail fast.
        config.validate()?;

        Ok(config)
    }

    /// Validate configuration values. Called at startup to catch errors early.
    fn validate(&self) -> AnyResult<()> {
        // Validate opus encoding quality range
        if self.player.opus_encoding_quality == 0 || self.player.opus_encoding_quality > 10 {
            return Err(format!(
                "player.opus_encoding_quality must be 1-10, got {}",
                self.player.opus_encoding_quality
            )
            .into());
        }

        // Validate port is not zero
        if self.server.port == 0 {
            return Err("server.port must be non-zero".into());
        }

        // N02: a zero-second update interval would flood clients with events.
        if self.server.player_update_interval.is_zero() {
            return Err("server.player_update_interval must be > 0 seconds".into());
        }

        // Warn if authorization is the default
        if self.server.authorization == "youshallnotpass" {
            tracing::warn!(
                "server.authorization is set to the default 'youshallnotpass'. \
                 Change this for production deployments!"
            );
        }

        Ok(())
    }
}
