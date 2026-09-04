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
    #[serde(rename = "isrc")]
    pub isrc: Option<String>,
    #[serde(rename = "sourceName")]
    pub source_name: String,
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
pub struct PlaylistInfo {
    pub name: String,
    #[serde(rename = "selectedTrack")]
    pub selected_track: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaylistData {
    pub info: PlaylistInfo,
    #[serde(rename = "pluginInfo")]
    pub plugin_info: serde_json::Value,
    pub tracks: Vec<LavalinkTrack>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorInfo {
    pub message: Option<String>,
    pub severity: String,
    pub cause: String,
    #[serde(rename = "causeStackTrace")]
    pub cause_stack_trace: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "loadType", content = "data")]
#[allow(clippy::large_enum_variant)]
pub enum LoadResult {
    #[serde(rename = "track")]
    Track(LavalinkTrack),
    #[serde(rename = "playlist")]
    Playlist(PlaylistData),
    #[serde(rename = "search")]
    Search(Vec<LavalinkTrack>),
    #[serde(rename = "empty")]
    Empty,
    #[serde(rename = "error")]
    Error(ErrorInfo),
}

impl Serialize for LoadResult {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        match self {
            LoadResult::Track(t) => {
                let mut s = serializer.serialize_struct("LoadResult", 2)?;
                s.serialize_field("loadType", "track")?;
                s.serialize_field("data", t)?;
                s.end()
            }
            LoadResult::Playlist(p) => {
                let mut s = serializer.serialize_struct("LoadResult", 2)?;
                s.serialize_field("loadType", "playlist")?;
                s.serialize_field("data", p)?;
                s.end()
            }
            LoadResult::Search(tracks) => {
                let mut s = serializer.serialize_struct("LoadResult", 2)?;
                s.serialize_field("loadType", "search")?;
                s.serialize_field("data", tracks)?;
                s.end()
            }
            LoadResult::Empty => {
                let mut s = serializer.serialize_struct("LoadResult", 2)?;
                s.serialize_field("loadType", "empty")?;
                s.serialize_field("data", &serde_json::Value::Null)?;
                s.end()
            }
            LoadResult::Error(e) => {
                let mut s = serializer.serialize_struct("LoadResult", 2)?;
                s.serialize_field("loadType", "error")?;
                s.serialize_field("data", e)?;
                s.end()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load_result_empty_serializes_with_data_null() {
        let res = LoadResult::Empty;
        let json = serde_json::to_string(&res).unwrap();
        assert_eq!(json, r#"{"loadType":"empty","data":null}"#);
    }
}
