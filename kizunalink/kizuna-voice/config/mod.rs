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
    /// Resolve the configuration file path.
    ///
    /// Resolution order:
    /// 1. `KIZUNA_CONFIG_PATH` (explicit override, must exist — fails fast)
    /// 2. `config.toml` in the current working directory
    /// 3. `config.example.toml` in the current working directory (with a warning)
    ///
    /// Operators commonly launch the node from elsewhere (systemd, a process manager,
    /// a Docker `WORKDIR` that differs from the config mount). The environment override
    /// removes the "must `cd` into the config directory" papercut without changing the
    /// default behaviour.
    fn resolve_config_path() -> String {
        if let Ok(path) = std::env::var("KIZUNA_CONFIG_PATH") {
            if Path::new(&path).exists() {
                return path;
            }
            return path; // Let the read fail fast with a clear "No such file" error.
        }

        if Path::new("config.toml").exists() {
            "config.toml".to_string()
        } else if Path::new("config.example.toml").exists() {
            tracing::warn!(
                "config.toml not found — falling back to config.example.toml. \
                 For production, copy config.example.toml to config.toml and customize it."
            );
            "config.example.toml".to_string()
        } else {
            "config.toml".to_string() // Produces a clear read error naming the path.
        }
    }

    /// The `log_println!` lines here intentionally write to the console before the
    /// tracing subscriber exists (and mirror to the log file once it does) — hence the
    /// local N05 `print_stdout` allow.
    #[allow(clippy::print_stdout)]
    pub async fn load() -> AnyResult<Self> {
        let config_path = Self::resolve_config_path();
        let config_path = config_path.as_str();

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
            let mut config: Self = toml::from_str(&remote_toml)?;
            // KIZUNA_* overrides must win regardless of where the TOML came from, so
            // secrets injected through the environment still take effect with a remote
            // config server (where embedding them in the TOML is least safe).
            config.apply_env_overrides()?;
            config.validate()?;
            return Ok(config);
        }

        let mut config: Self = toml::from_str(&raw)?;

        // 12-factor overrides: KIZUNA_* env vars win over the TOML file. This is the
        // supported way to inject secrets (e.g. `KIZUNA_AUTHORIZATION` from a Docker
        // secret or k8s Secret) without embedding them in a config file.
        config.apply_env_overrides()?;

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
    ///
    /// A variable that is *set but empty* (e.g. `KIZUNA_ADDRESS=` exported by a
    /// shell or `environment: - KIZUNA_ADDRESS` in Compose) is treated as a
    /// misconfiguration and reported by name, rather than being applied as an
    /// empty string that later fails with a cryptic parse error.
    fn apply_env_overrides(&mut self) -> AnyResult<()> {
        if let Some(v) = non_empty_env("KIZUNA_ADDRESS")? {
            self.server.address = v;
        }
        if let Some(v) = non_empty_env("KIZUNA_PORT")? {
            // Malformed values must fail startup with a configuration error, not
            // panic from a background task.
            self.server.port = v
                .parse()
                .map_err(|e| format!("KIZUNA_PORT must be a number, got {v:?}: {e}"))?;
        }
        if let Some(v) = non_empty_env("KIZUNA_AUTHORIZATION")? {
            self.server.authorization = v;
        }
        if let Some(v) = non_empty_env("KIZUNA_RATE_LIMIT_PER_MINUTE")? {
            self.server.rate_limit_per_minute = v.parse().map_err(|e| {
                format!("KIZUNA_RATE_LIMIT_PER_MINUTE must be a number, got {v:?}: {e}")
            })?;
        }
        if let Some(v) = non_empty_env("KIZUNA_TLS_ENABLED")? {
            self.server.tls.enabled = v
                .parse()
                .map_err(|e| format!("KIZUNA_TLS_ENABLED must be true or false, got {v:?}: {e}"))?;
        }
        if let Some(v) = non_empty_env("KIZUNA_TLS_CERT")? {
            self.server.tls.cert_path = Some(v);
        }
        if let Some(v) = non_empty_env("KIZUNA_TLS_KEY")? {
            self.server.tls.key_path = Some(v);
        }
        if let Some(v) = non_empty_env("KIZUNA_LOG_LEVEL")? {
            let logging = self.logging.get_or_insert_with(Default::default);
            logging.level = Some(v);
        }
        if let Some(v) = non_empty_env("KIZUNA_METRICS_ENABLED")? {
            self.metrics.prometheus.enabled = v.parse().map_err(|e| {
                format!("KIZUNA_METRICS_ENABLED must be true or false, got {v:?}: {e}")
            })?;
        }
        Ok(())
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

        // A zero-second stats/ping interval makes `tokio::time::interval(Duration::ZERO)`
        // panic inside the WebSocket handler task, which kills the connection and
        // prevents clients from reconnecting reliably. Reject it at startup.
        if self.server.stats_interval == 0 {
            return Err("server.stats_interval must be > 0 seconds".into());
        }
        if self.server.websocket_ping_interval == 0 {
            return Err("server.websocket_ping_interval must be > 0 seconds".into());
        }

        // Bound the resumable event queue count so a paused session cannot grow
        // without limit. (The queue also enforces a byte budget at runtime.)
        if self.server.max_event_queue_size > 10_000 {
            return Err(format!(
                "server.max_event_queue_size must be <= 10000, got {}",
                self.server.max_event_queue_size
            )
            .into());
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

        if self.server.authorization.trim().is_empty() {
            return Err(
                "server.authorization is required but missing or empty. Set a strong, unique \
                 secret in config.toml or via the KIZUNA_AUTHORIZATION environment variable."
                    .into(),
            );
        }

        // Reject well-known placeholders on *every* bind address. A guessable
        // secret is no better on loopback than on a public interface: any local
        // process, a misconfigured reverse proxy, or a container port-forward
        // can still reach it. There is no safe bind for a known default.
        if is_insecure_placeholder(&self.server.authorization) {
            return Err(format!(
                "server.authorization is set to the known insecure placeholder {}. \
                 Choose a strong, unique secret (and rotate it if it was ever exposed).",
                redacted_placeholder(&self.server.authorization)
            )
            .into());
        }

        Ok(())
    }
}

/// Read an environment variable, distinguishing "unset" from "set but blank".
///
/// Returns `Ok(None)` when the variable is not present, `Ok(Some(value))` with
/// the raw value when it is present and non-blank, and `Err` when it is present
/// but blank or not valid UTF-8. A blank value is treated as a misconfiguration
/// rather than an override that silently clears the target field.
fn non_empty_env(name: &str) -> AnyResult<Option<String>> {
    match std::env::var(name) {
        Ok(value) if value.trim().is_empty() => Err(format!(
            "{name} is set but empty. Unset it to keep the configured value, or set it to a \
             non-empty value."
        )
        .into()),
        Ok(value) => Ok(Some(value)),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(std::env::VarError::NotUnicode(_)) => {
            Err(format!("{name} is set to a non-UTF-8 value, which is not supported.").into())
        }
    }
}

/// Publicly documented placeholder credentials that must never be used as a real
/// secret. Comparison is case-insensitive after trimming, so copies such as
/// `YoushallNotPass` cannot slip through.
const INSECURE_PLACEHOLDERS: &[&str] = &[
    "youshallnotpass", // Lavalink's historical default
    "password",
    "changeme",
    "change-me",
    "replace-with-your-strong-secret", // shipped in this repo's README/example
    "replace-with-your-own-long-random-secret", // shipped in this repo's README
    "your-password",
    "yourpassword",
    "secret",
    "admin",
    "test",
];

fn is_insecure_placeholder(value: &str) -> bool {
    let normalized = value.trim().to_ascii_lowercase();
    INSECURE_PLACEHOLDERS.contains(&normalized.as_str())
}

/// A human-readable label for a rejected placeholder, safe to include in a
/// startup error. Only *known, public* placeholder strings ever reach this
/// function (validation rejects real secrets before it), so naming the matched
/// value leaks nothing; the operator's actual secret is never formatted here.
fn redacted_placeholder(value: &str) -> &'static str {
    match value.trim().to_ascii_lowercase().as_str() {
        "youshallnotpass" => "\"youshallnotpass\" (the Lavalink default)",
        "replace-with-your-strong-secret" => {
            "\"replace-with-your-strong-secret\" (the example value)"
        }
        _ => "a known default or placeholder value",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // `apply_env_overrides` reads process-global env vars; serialize tests that
    // mutate them to avoid cross-test interference.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    /// A minimal, *valid* config. `authorization` is explicit because it no
    /// longer has a default — omitting it must fail validation.
    const BASE_TOML: &str =
        "server.address='127.0.0.1'\nserver.port=28453\nserver.authorization='base-valid-secret'\n";

    fn cfg_from_toml(raw: &str) -> AppConfig {
        toml::from_str(raw).expect("config parses")
    }

    /// Apply the given `KIZUNA_*` env vars on top of [`BASE_TOML`], returning any
    /// error instead of panicking so the empty-secret guard can be asserted.
    ///
    /// Tests must be hermetic: the documented live-node workflow exports
    /// `KIZUNA_*` vars (address, port, authorization, ...). Any that leak into
    /// the test process would be picked up by `apply_env_overrides` below and
    /// make results depend on the caller's shell, so snapshot and clear them
    /// all first, then restore afterwards.
    fn try_cfg_with_envs(pairs: &[(&str, &str)]) -> AnyResult<AppConfig> {
        let _guard = ENV_LOCK.lock().unwrap();

        let ambient: Vec<(String, String)> = std::env::vars()
            .filter(|(k, _)| k.starts_with("KIZUNA_"))
            .collect();
        for (k, _) in &ambient {
            unsafe { std::env::remove_var(k) };
        }

        for (k, v) in pairs {
            unsafe { std::env::set_var(k, v) };
        }
        let mut cfg = cfg_from_toml(BASE_TOML);
        let result = cfg.apply_env_overrides();

        for (k, _) in pairs {
            unsafe { std::env::remove_var(k) };
        }
        for (k, v) in ambient {
            unsafe { std::env::set_var(k, v) };
        }
        result.map(|()| cfg)
    }

    /// Like [`try_cfg_with_envs`] but panics on failure (the common case).
    fn cfg_with_envs(pairs: &[(&str, &str)]) -> AppConfig {
        try_cfg_with_envs(pairs).expect("env overrides apply cleanly")
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

    #[test]
    fn missing_authorization_is_rejected() {
        // No `authorization` key at all: the field defaults to empty (there is
        // deliberately no serde default secret) and validation must refuse it.
        let cfg = cfg_from_toml("[server]\naddress='127.0.0.1'\n");
        assert_eq!(cfg.server.authorization, "");
        let err = cfg.validate().unwrap_err().to_string();
        assert!(
            err.contains("authorization") && err.contains("required"),
            "missing secret must be a hard error, got: {err}"
        );
    }

    #[test]
    fn empty_authorization_is_rejected() {
        // Empty / whitespace-only supplied through TOML.
        for raw in ["", "   "] {
            let cfg = cfg_from_toml(&format!("[server]\nauthorization={raw:?}\n"));
            let err = cfg.validate().unwrap_err().to_string();
            assert!(
                err.contains("required") || err.contains("empty"),
                "empty secret must be rejected, got: {err}"
            );
        }

        // Empty / whitespace-only supplied through the environment fails even
        // earlier, in `apply_env_overrides`.
        for value in ["", "   "] {
            let err = try_cfg_with_envs(&[("KIZUNA_AUTHORIZATION", value)])
                .expect_err("empty KIZUNA_AUTHORIZATION must be rejected")
                .to_string();
            assert!(
                err.contains("KIZUNA_AUTHORIZATION"),
                "expected env-specific error, got: {err}"
            );
        }
    }

    #[test]
    fn empty_env_value_is_a_named_misconfiguration_for_every_var() {
        // A set-but-empty variable must be reported by name for every override,
        // so a blank `KIZUNA_ADDRESS` cannot surface later as a cryptic
        // `AddrParseError(Ip)` and a blank scalar cannot silently clear a field.
        for name in [
            "KIZUNA_ADDRESS",
            "KIZUNA_PORT",
            "KIZUNA_AUTHORIZATION",
            "KIZUNA_RATE_LIMIT_PER_MINUTE",
            "KIZUNA_TLS_ENABLED",
            "KIZUNA_TLS_CERT",
            "KIZUNA_TLS_KEY",
            "KIZUNA_LOG_LEVEL",
            "KIZUNA_METRICS_ENABLED",
        ] {
            for value in ["", "   "] {
                let err = try_cfg_with_envs(&[(name, value)])
                    .expect_err("blank env value must be rejected")
                    .to_string();
                assert!(
                    err.contains(name),
                    "expected {name}-specific error, got: {err}"
                );
            }
        }
    }

    #[test]
    fn unsetting_or_setting_a_real_env_value_behaves_as_before() {
        // Unset variables keep the TOML value; non-blank overrides still apply.
        let cfg = cfg_with_envs(&[]);
        assert_eq!(cfg.server.address, "127.0.0.1");

        let cfg = cfg_with_envs(&[("KIZUNA_ADDRESS", "0.0.0.0"), ("KIZUNA_PORT", "1234")]);
        assert_eq!(cfg.server.address, "0.0.0.0");
        assert_eq!(cfg.server.port, 1234);
    }

    #[test]
    fn default_placeholder_authorization_rejected_on_every_bind() {
        // The historical default must be refused on loopback *and* public binds:
        // there is no safe interface for a known credential.
        for address in ["127.0.0.1", "0.0.0.0", "::1"] {
            let cfg = cfg_from_toml(&format!(
                "[server]\naddress={address:?}\nauthorization='youshallnotpass'\n"
            ));
            let err = cfg.validate().unwrap_err().to_string();
            assert!(
                err.contains("placeholder"),
                "default secret must be rejected on {address}, got: {err}"
            );
        }

        // Case and surrounding whitespace must not smuggle it through.
        let cfg = cfg_from_toml("[server]\nauthorization='  YouShallNotPass  '\n");
        assert!(cfg.validate().is_err());

        // And via the environment on a loopback bind.
        let err = cfg_with_envs(&[
            ("KIZUNA_ADDRESS", "127.0.0.1"),
            ("KIZUNA_AUTHORIZATION", "youshallnotpass"),
        ])
        .validate()
        .unwrap_err()
        .to_string();
        assert!(err.contains("placeholder"), "got: {err}");
    }

    #[test]
    fn other_placeholder_authorizations_rejected() {
        for value in [
            "password",
            "changeme",
            "replace-with-your-strong-secret",
            "your-password",
            "secret",
            "admin",
            "test",
        ] {
            let cfg = cfg_from_toml(&format!("[server]\nauthorization={value:?}\n"));
            assert!(
                cfg.validate().is_err(),
                "placeholder {value:?} must be rejected"
            );
        }
    }

    #[test]
    fn valid_authorization_passes_on_loopback_and_public() {
        for address in ["127.0.0.1", "0.0.0.0"] {
            let cfg = cfg_from_toml(&format!(
                "[server]\naddress={address:?}\nauthorization='a-genuinely-strong-secret'\n"
            ));
            cfg.validate()
                .unwrap_or_else(|e| panic!("valid secret on {address} must pass: {e}"));
        }

        // A short-but-not-placeholder secret is still accepted (we do not
        // invent length policy; the contract is only "not missing, not known").
        let cfg = cfg_with_envs(&[("KIZUNA_AUTHORIZATION", "s3cr3t!")]);
        cfg.validate().expect("non-placeholder secret passes");
    }

    #[test]
    fn validation_error_never_contains_the_secret() {
        // Rejecting a placeholder must not echo an unrelated secret value. The
        // error text for a placeholder is a fixed label; the operator's own
        // secret is never interpolated into any validation message.
        let cfg = cfg_from_toml("[server]\nauthorization='youshallnotpass'\n");
        let err = cfg.validate().unwrap_err().to_string();
        // The forbidden placeholder is named (a public constant), which is safe…
        assert!(err.contains("youshallnotpass"));

        // …but a real, rejected-for-emptiness config must not leak its trimmed
        // content anywhere.
        let cfg = cfg_from_toml("[server]\nauthorization='   '\n");
        let err = cfg.validate().unwrap_err().to_string();
        assert!(!err.contains("   "));
    }

    #[test]
    fn config_path_override_is_honoured() {
        let _guard = ENV_LOCK.lock().unwrap();
        let previous = std::env::var("KIZUNA_CONFIG_PATH").ok();

        unsafe {
            std::env::set_var("KIZUNA_CONFIG_PATH", "/tmp/kizunalink-does-not-exist.toml");
        }
        // The override is returned verbatim (so the subsequent read fails fast with a
        // clear path instead of silently falling back to a different file).
        assert_eq!(
            AppConfig::resolve_config_path(),
            "/tmp/kizunalink-does-not-exist.toml"
        );

        match previous {
            Some(v) => unsafe { std::env::set_var("KIZUNA_CONFIG_PATH", v) },
            None => unsafe { std::env::remove_var("KIZUNA_CONFIG_PATH") },
        }
    }
}
