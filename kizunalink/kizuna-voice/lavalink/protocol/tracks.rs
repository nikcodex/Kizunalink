// Copyright (c) 2026 nikcodex (KizunaLink)
// Licensed under the MIT License

use serde::{
    Deserialize, Serialize,
    ser::{SerializeMap, Serializer},
};

use crate::lavalink::protocol::codec::{decode_track, encode_track};

/// A single audio track with encoded data and metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Track {
    /// Base64-encoded track data.
    pub encoded: String,
    /// Track metadata.
    pub info: TrackInfo,
    /// Plugin-specific info — free JSON object whose shape is defined by the plugin.
    #[serde(default = "serde_json::Value::default")]
    pub plugin_info: serde_json::Value,
    /// User-provided data attached to the track.
    #[serde(default = "serde_json::Value::default")]
    pub user_data: serde_json::Value,
}
impl Track {
    /// Create a new Track from info and encode it.
    pub fn new(info: TrackInfo) -> Self {
        let mut track = Self {
            encoded: String::new(),
            info,
            plugin_info: serde_json::json!({}),
            user_data: serde_json::json!({}),
        };
        track.encoded = track.encode();
        track
    }

    /// Encode the track into a base64 string.
    pub fn encode(&self) -> String {
        encode_track(&self.info, &self.user_data).unwrap_or_else(|_| self.encoded.clone())
    }

    /// Decode a track from a base64 string.
    pub fn decode(encoded: &str) -> Option<Self> {
        decode_track(encoded).ok()
    }
}

/// Metadata for an audio track.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct TrackInfo {
    pub identifier: String,
    pub is_seekable: bool,
    pub author: String,
    /// Duration in milliseconds. 0 for live streams.
    pub length: u64,
    pub is_stream: bool,
    /// Current playback position in milliseconds.
    pub position: u64,
    pub title: String,
    pub uri: Option<String>,
    pub artwork_url: Option<String>,
    pub isrc: Option<String>,
    pub source_name: String,
}

/// Result of a track load operation.
#[derive(Debug, Deserialize)]
#[serde(tag = "loadType", content = "data", rename_all = "camelCase")]
pub enum LoadResult {
    /// A single track was loaded.
    Track(Track),
    /// A playlist was loaded.
    Playlist(PlaylistData),
    /// A search returned results.
    Search(Vec<Track>),
    /// No matches found.
    ///
    /// Serializes as `"data": null` to byte-match the official Lavalink v4
    /// wire format (`NoMatches` holds `data: null`), not `{}`.
    Empty {},
    /// An error occurred during loading.
    Error(LoadError),
}

impl Serialize for LoadResult {
    /// Manual impl so the `empty` variant emits `"data": null` exactly like
    /// the official `NoMatches` serializer (a derived adjacently-tagged enum
    /// would emit `"data": {}`, which differs on the wire).
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(None)?;
        match self {
            LoadResult::Track(track) => {
                map.serialize_entry("loadType", "track")?;
                map.serialize_entry("data", track)?;
            }
            LoadResult::Playlist(playlist) => {
                map.serialize_entry("loadType", "playlist")?;
                map.serialize_entry("data", playlist)?;
            }
            LoadResult::Search(tracks) => {
                map.serialize_entry("loadType", "search")?;
                map.serialize_entry("data", tracks)?;
            }
            LoadResult::Empty {} => {
                map.serialize_entry("loadType", "empty")?;
                map.serialize_entry("data", &None::<LoadError>)?;
            }
            LoadResult::Error(error) => {
                map.serialize_entry("loadType", "error")?;
                map.serialize_entry("data", error)?;
            }
        }
        map.end()
    }
}

/// Playlist data returned from a load operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaylistData {
    pub info: PlaylistInfo,
    pub plugin_info: serde_json::Value,
    pub tracks: Vec<Track>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextData {
    pub text: String,
    pub plugin: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResult {
    pub tracks: Vec<Track>,
    pub albums: Vec<PlaylistData>,
    pub artists: Vec<PlaylistData>,
    pub playlists: Vec<PlaylistData>,
    pub texts: Vec<TextData>,
    pub plugin: serde_json::Value,
}

/// Playlist metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaylistInfo {
    pub name: String,
    /// Index of the selected track, or -1 if none.
    pub selected_track: i32,
}

/// Error from a failed track load.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoadError {
    /// Human-readable error message.
    pub message: Option<String>,
    /// How severe the error is.
    pub severity: crate::common::Severity,
    /// Exception class / short cause description.
    pub cause: String,
    /// Full stack trace. Official Lavalink types `causeStackTrace` as a
    /// non-null string; a missing trace serializes as `""` so clients that
    /// unconditionally read the field never see `undefined`/`null`.
    #[serde(default)]
    pub cause_stack_trace: Option<String>,
}

impl Serialize for LoadError {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut st = serializer.serialize_struct("LoadError", 4)?;
        st.serialize_field("message", &self.message)?;
        st.serialize_field("severity", &self.severity)?;
        st.serialize_field("cause", &self.cause)?;
        st.serialize_field(
            "causeStackTrace",
            self.cause_stack_trace.as_deref().unwrap_or(""),
        )?;
        st.end()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_load_result_serializes_null_data() {
        let json = serde_json::to_string(&LoadResult::Empty {}).unwrap();
        assert_eq!(json, r#"{"loadType":"empty","data":null}"#);
    }

    #[test]
    fn load_error_serializes_non_null_cause_stack_trace() {
        let err = LoadError {
            message: Some("boom".into()),
            severity: crate::common::Severity::Fault,
            cause: "panic".into(),
            cause_stack_trace: None,
        };
        let v: serde_json::Value = serde_json::to_value(&err).unwrap();
        assert_eq!(v["causeStackTrace"], "");
        assert_eq!(v["message"], "boom");
    }
}
