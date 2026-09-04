use crate::models::protocol::{GitInfo, ServerInfo, VersionInfo};
use crate::rest::auth::require_auth;
use crate::rest::error::LavalinkError;
use crate::AppState;
use axum::{extract::State, http::HeaderMap, response::Json};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub const fn parse_u64_or_zero(s: &str) -> u64 {
    let bytes = s.as_bytes();
    let mut val = 0u64;
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] >= b'0' && bytes[i] <= b'9' {
            val = val * 10 + (bytes[i] - b'0') as u64;
        } else {
            return 0;
        }
        i += 1;
    }
    val
}

pub const BUILD_TIME: u64 = match option_env!("BUILD_TIME") {
    Some(s) => parse_u64_or_zero(s),
    None => 0,
};
pub const GIT_BRANCH: &str = match option_env!("GIT_BRANCH") {
    Some(b) => b,
    None => "main",
};
pub const GIT_COMMIT: &str = match option_env!("GIT_COMMIT") {
    Some(c) => c,
    None => "unknown",
};
pub const GIT_COMMIT_TIME: u64 = match option_env!("GIT_COMMIT_TIME") {
    Some(s) => parse_u64_or_zero(s),
    None => BUILD_TIME,
};

pub async fn get_version() -> &'static str {
    VERSION
}

pub async fn get_info(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<ServerInfo>, LavalinkError> {
    require_auth(&headers, &state.password, "/v4/info")?;

    Ok(Json(ServerInfo {
        version: VersionInfo {
            semver: VERSION.to_string(),
            major: env!("CARGO_PKG_VERSION_MAJOR").parse().unwrap_or(4),
            minor: env!("CARGO_PKG_VERSION_MINOR").parse().unwrap_or(2),
            patch: env!("CARGO_PKG_VERSION_PATCH").parse().unwrap_or(1),
            pre_release: None,
        },
        build_time: BUILD_TIME,
        git: GitInfo {
            branch: GIT_BRANCH.to_string(),
            commit: GIT_COMMIT.to_string(),
            commit_time: GIT_COMMIT_TIME,
        },
        jvm: "Rust (Tokio)".to_string(),
        lavaplayer: "symphonia-0.5".to_string(),
        source_managers: vec![
            "jiosaavn".to_string(),
            "youtube".to_string(),
            "spotify".to_string(),
            "soundcloud".to_string(),
            "bandcamp".to_string(),
            "twitch".to_string(),
            "vimeo".to_string(),
            "niconico".to_string(),
            "http".to_string(),
            "applemusic".to_string(),
            "deezer".to_string(),
        ],
        filters: vec![
            "volume".to_string(),
            "equalizer".to_string(),
            "karaoke".to_string(),
            "timescale".to_string(),
            "tremolo".to_string(),
            "vibrato".to_string(),
            "distortion".to_string(),
            "rotation".to_string(),
            "channelMix".to_string(),
            "lowPass".to_string(),
        ],
        plugins: vec![],
    }))
}
