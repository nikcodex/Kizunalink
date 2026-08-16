use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackInfo {
    pub identifier: String,
    #[serde(rename = "isSeekable")]
    pub is_seekable: bool,
    pub author: String,
    pub length: u64,
    #[serde(rename = "isStream")]
    pub is_stream: bool,
    pub position: u64,
    pub title: String,
    pub uri: Option<String>,
    #[serde(rename = "artworkUrl")]
    pub artwork_url: Option<String>,
    #[serde(rename = "sourceName")]
    pub source_name: String,
    pub bitrate: Option<String>,
    #[serde(rename = "streamUrl")]
    pub stream_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LavalinkTrack {
    pub encoded: String,
    pub info: TrackInfo,
    #[serde(rename = "pluginInfo")]
    pub plugin_info: serde_json::Value,
    #[serde(rename = "userData")]
    pub user_data: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "loadType", content = "data")]
pub enum LoadResult {
    #[serde(rename = "track")]
    Track(Box<LavalinkTrack>),
    #[serde(rename = "playlist")]
    Playlist(Box<PlaylistData>),
    #[serde(rename = "search")]
    Search(Vec<LavalinkTrack>),
    #[serde(rename = "empty")]
    Empty(serde_json::Value),
    #[serde(rename = "error")]
    Error(Box<ErrorInfo>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaylistData {
    pub info: PlaylistInfo,
    #[serde(rename = "pluginInfo")]
    pub plugin_info: serde_json::Value,
    pub tracks: Vec<LavalinkTrack>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaylistInfo {
    pub name: String,
    #[serde(rename = "selectedTrack")]
    pub selected_track: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorInfo {
    pub message: String,
    pub severity: String,
    pub cause: String,
}
