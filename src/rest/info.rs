use axum::response::Json;
use crate::models::protocol::{ServerInfo, VersionInfo, GitInfo, JvmInfo};

pub async fn get_version() -> &'static str {
    "4.0.0"
}

pub async fn get_info() -> Json<ServerInfo> {
    Json(ServerInfo {
        version: VersionInfo {
            semver: "4.0.0".to_string(),
            major: 4,
            minor: 0,
            patch: 0,
            pre_release: Some("kizuna".to_string()),
        },
        build_time: 1724284800000,
        git: GitInfo {
            branch: "main".to_string(),
            commit: "kizuna-core".to_string(),
            commit_time: 1724284800000,
        },
        source_managers: vec![
            "jiosaavn".to_string(),
            "youtube".to_string(),
            "spotify".to_string(),
            "soundcloud".to_string(),
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
        jvm: JvmInfo {
            version: "21".to_string(),
            vm: "Rust (Tokio)".to_string(),
            vendor: "KizunaLink".to_string(),
        },
    })
}
