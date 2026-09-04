use crate::models::protocol::{GitInfo, ServerInfo, VersionInfo};
use axum::response::Json;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const BUILD_TIME: u64 = 1724284800000;
pub const GIT_BRANCH: &str = match option_env!("GIT_BRANCH") {
    Some(b) => b,
    None => "main",
};
pub const GIT_COMMIT: &str = match option_env!("GIT_COMMIT") {
    Some(c) => c,
    None => "kizuna-core",
};
pub const GIT_COMMIT_TIME: u64 = BUILD_TIME;

pub async fn get_version() -> &'static str {
    VERSION
}

pub async fn get_info() -> Json<ServerInfo> {
    Json(ServerInfo {
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
    })
}
