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

            let client = reqwest::Client::builder()
                .connect_timeout(std::time::Duration::from_secs(10))
                .timeout(std::time::Duration::from_secs(30))
                .build()?;
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

        let mut config: Self = toml::from_str(&raw)?;

        // 12-factor overrides: KIZUNA_* env vars win over the TOML file. This is the
        // supported way to inject secrets (e.g. `KIZUNA_AUTHORIZATION` from a Docker
        // secret or k8s Secret) without embedding them in a config file.
        config.apply_env_overrides();

        // P06: Validate configuration at startup so bad values fail fast.
        config.validate()?;

        Ok(config)
    }

    /// Apply `KIZUNA_*` environment-variable overrides on top of the parsed TOML.
    ///
    /// Supported variables:
    ///
    /// | Variable | Overrides |
    /// |---|---|
    /// | `KIZUNA_ADDRESS` | `server.address` |
    /// | `KIZUNA_PORT` | `server.port` |
    /// | `KIZUNA_AUTHORIZATION` | `server.authorization` (secret) |
    /// | `KIZUNA_RATE_LIMIT_PER_MINUTE` | `server.rate_limit_per_minute` |
    /// | `KIZUNA_TLS_ENABLED` | `server.tls.enabled` (`true`/`false`) |
    /// | `KIZUNA_TLS_CERT` | `server.tls.cert_path` |
    /// | `KIZUNA_TLS_KEY` | `server.tls.key_path` |
    /// | `KIZUNA_LOG_LEVEL` | `logging.level` |
    /// | `KIZUNA_METRICS_ENABLED` | `metrics.prometheus.enabled` (`true`/`false`) |
    ///
    /// A malformed value fails fast rather than being silently ignored.
    fn apply_env_overrides(&mut self) {
        if let Ok(v) = std::env::var("KIZUNA_ADDRESS") {
            self.server.address = v;
        }
        if let Ok(v) = std::env::var("KIZUNA_PORT") {
            self.server.port = v
                .parse()
                .unwrap_or_else(|e| panic!("KIZUNA_PORT must be a number: {e}"));
        }
        if let Ok(v) = std::env::var("KIZUNA_AUTHORIZATION") {
            self.server.authorization = v;
        }
        if let Ok(v) = std::env::var("KIZUNA_RATE_LIMIT_PER_MINUTE") {
            self.server.rate_limit_per_minute = v
                .parse()
                .unwrap_or_else(|e| panic!("KIZUNA_RATE_LIMIT_PER_MINUTE must be a number: {e}"));
        }
        if let Ok(v) = std::env::var("KIZUNA_TLS_ENABLED") {
            self.server.tls.enabled = v
                .parse()
                .unwrap_or_else(|e| panic!("KIZUNA_TLS_ENABLED must be true or false: {e}"));
        }
        if let Ok(v) = std::env::var("KIZUNA_TLS_CERT") {
            self.server.tls.cert_path = Some(v);
        }
        if let Ok(v) = std::env::var("KIZUNA_TLS_KEY") {
            self.server.tls.key_path = Some(v);
        }
        if let Ok(v) = std::env::var("KIZUNA_LOG_LEVEL") {
            let logging = self.logging.get_or_insert_with(Default::default);
            logging.level = Some(v);
        }
        if let Ok(v) = std::env::var("KIZUNA_METRICS_ENABLED") {
            self.metrics.prometheus.enabled = v
                .parse()
                .unwrap_or_else(|e| panic!("KIZUNA_METRICS_ENABLED must be true or false: {e}"));
        }
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

        // TLS must have a cert + key when enabled.
        if self.server.tls.enabled
            && (self.server.tls.cert_path.is_none() || self.server.tls.key_path.is_none())
        {
            return Err(
                "server.tls.enabled = true requires server.tls.cert_path and server.tls.key_path"
                    .into(),
            );
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // `apply_env_overrides` reads process-global env vars; serialize tests that
    // mutate them to avoid cross-test interference.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    /// Parse a minimal config (equivalent to what `load()` does from disk) and
    /// apply the given `KIZUNA_*` env vars on top.
    fn cfg_with_envs(pairs: &[(&str, &str)]) -> AppConfig {
        let _guard = ENV_LOCK.lock().unwrap();
        for (k, _) in pairs {
            unsafe { std::env::remove_var(k) };
        }
        for (k, v) in pairs {
            unsafe { std::env::set_var(k, v) };
        }
        let mut cfg: AppConfig = toml::from_str("server.address='127.0.0.1'\nserver.port=28453\n")
            .expect("minimal TOML parses");
        cfg.apply_env_overrides();
        for (k, _) in pairs {
            unsafe { std::env::remove_var(k) };
        }
        cfg
    }

    #[test]
    fn env_overrides_apply_on_top_of_toml() {
        let cfg = cfg_with_envs(&[
            ("KIZUNA_ADDRESS", "0.0.0.0"),
            ("KIZUNA_PORT", "9999"),
            ("KIZUNA_AUTHORIZATION", "s3cr3t"),
            ("KIZUNA_RATE_LIMIT_PER_MINUTE", "42"),
            ("KIZUNA_LOG_LEVEL", "debug"),
            ("KIZUNA_METRICS_ENABLED", "true"),
        ]);

        assert_eq!(cfg.server.address, "0.0.0.0");
        assert_eq!(cfg.server.port, 9999);
        assert_eq!(cfg.server.authorization, "s3cr3t");
        assert_eq!(cfg.server.rate_limit_per_minute, 42);
        assert_eq!(cfg.logging.unwrap().level.as_deref(), Some("debug"));
        assert!(cfg.metrics.prometheus.enabled);
    }

    #[test]
    fn env_tls_overrides_work_together() {
        let cfg = cfg_with_envs(&[
            ("KIZUNA_TLS_ENABLED", "true"),
            ("KIZUNA_TLS_CERT", "/certs/fullchain.pem"),
            ("KIZUNA_TLS_KEY", "/certs/privkey.pem"),
        ]);

        assert!(cfg.server.tls.enabled);
        assert_eq!(
            cfg.server.tls.cert_path.as_deref(),
            Some("/certs/fullchain.pem")
        );
        assert_eq!(
            cfg.server.tls.key_path.as_deref(),
            Some("/certs/privkey.pem")
        );
    }

    #[test]
    fn missing_env_vars_leave_toml_values_untouched() {
        let cfg = cfg_with_envs(&[]);
        assert_eq!(cfg.server.address, "127.0.0.1");
        assert_eq!(cfg.server.port, 28453);
        assert!(!cfg.server.tls.enabled);
    }

    #[test]
    fn load_applies_env_overrides_and_validation() {
        // Env-provided TLS should pass validation with cert+key …
        let cfg = cfg_with_envs(&[
            ("KIZUNA_TLS_ENABLED", "true"),
            ("KIZUNA_TLS_CERT", "/certs/c.pem"),
            ("KIZUNA_TLS_KEY", "/certs/k.pem"),
        ]);
        cfg.validate()
            .expect("TLS env override with cert+key validates");

        // … and fail when only `enabled` is set.
        let bad = cfg_with_envs(&[("KIZUNA_TLS_ENABLED", "true")]);
        let err = bad.validate().unwrap_err().to_string();
        assert!(
            err.contains("cert"),
            "expected TLS validation error, got: {err}"
        );
    }
}
