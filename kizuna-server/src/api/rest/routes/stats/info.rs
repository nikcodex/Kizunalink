// Copyright (c) 2026 nikcodex (KizunaLink)
// Licensed under the MIT License

use std::sync::Arc;

use axum::{extract::State, response::Json};

use crate::{discord::player::Filters, protocol, server::AppState};

/// GET /v4/info
pub async fn get_info(State(state): State<Arc<AppState>>) -> Json<protocol::Info> {
    tracing::info!("GET /v4/info");

    let version_str = env!("CARGO_PKG_VERSION");
    let (crate_major, minor, patch, mut pre_release) = parse_semver(version_str);

    let mut semver = version_str.to_string();
    if pre_release.is_none()
        && let Some(pre) = option_env!("KIZUNALINK_PRE_RELEASE")
    {
        pre_release = Some(pre.to_string());
        semver = format!("{}-{}", version_str, pre);
    }

    // Clients compare `version.major` against the *Lavalink protocol* version
    // (official nodes report `4.x.y` here). The crate's own semver (1.x.y) is
    // the server release, not the wire protocol — reporting it made clients
    // that gate on `major >= 4` refuse this node.
    let major = if crate_major == 0 { 4 } else { crate_major };
    let major = if major < 4 { 4 } else { major };

    Json(protocol::Info {
        version: protocol::Version {
            semver,
            major,
            minor,
            patch,
            pre_release,
        },
        build_time: option_env!("BUILD_TIME")
            .and_then(|s| s.parse().ok())
            .unwrap_or(0),
        git: protocol::GitInfo {
            branch: option_env!("GIT_BRANCH").unwrap_or("unknown").to_string(),
            commit: option_env!("GIT_COMMIT").unwrap_or("unknown").to_string(),
            commit_time: option_env!("GIT_COMMIT_TIME")
                .and_then(|s| s.parse().ok())
                .unwrap_or(0),
        },
        jvm: "Rust".to_string(),
        lavaplayer: "symphonia".to_string(),
        source_managers: state.source_manager.source_names(),
        filters: Filters::names()
            .into_iter()
            .filter(|name| state.config.filters.is_enabled(name))
            .collect(),
        plugins: vec![],
    })
}

fn parse_semver(v: &str) -> (u32, u32, u32, Option<String>) {
    let mut parts = v.split('.');
    let major = parts.next().and_then(|s| s.parse().ok()).unwrap_or(4);
    let minor = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);

    let patch_raw = parts.next().unwrap_or("0");
    let (patch_str, pre_release) = if let Some(idx) = patch_raw.find('-') {
        (&patch_raw[..idx], Some(patch_raw[idx + 1..].to_string()))
    } else {
        (patch_raw, None)
    };

    let patch = patch_str.parse().ok().unwrap_or(0);

    (major, minor, patch, pre_release)
}

pub async fn get_stats(State(state): State<Arc<AppState>>) -> Json<protocol::Stats> {
    tracing::info!("GET /v4/stats");
    Json(crate::monitoring::collect_stats(&state, None))
}

pub async fn get_version() -> String {
    tracing::info!("GET /version");
    env!("CARGO_PKG_VERSION").to_string()
}
