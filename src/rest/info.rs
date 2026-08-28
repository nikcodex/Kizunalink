use axum::response::Json;
use crate::models::protocol::{GitInfo, ServerInfo, VersionInfo};

pub async fn get_version() -> &'static str {
    "4.2.1"
}

pub async fn get_info() -> Json<ServerInfo> {
    Json(ServerInfo {
        version: VersionInfo {
            semver: "4.2.1".to_string(),
            major: 4,
            minor: 2,
            patch: 1,
            pre_release: None,
        },
        build_time: 1724284800000,
        git: GitInfo {
            branch: "main".to_string(),
            commit: "kizuna-core".to_string(),
            commit_time: 1724284800000,
        },
        jvm: "Rust (Tokio)".to_string(),
        lavaplayer: "2.2.6".to_string(),
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
