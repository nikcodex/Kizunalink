use crate::models::track::{LavalinkTrack, TrackInfo};
use serde_json::Value;
use std::sync::Arc;

const ITUNES_SEARCH_URL: &str = "https://itunes.apple.com/search";
const ITUNES_LOOKUP_URL: &str = "https://itunes.apple.com/lookup";

pub struct AppleMusicSource {
    client: reqwest::Client,
}

impl AppleMusicSource {
    pub fn new() -> Arc<Self> {
        let client = crate::config::global_proxy()
            .apply_to_builder(reqwest::Client::builder())
            .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36")
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .expect("Failed to build Reqwest client for Apple Music");

        Arc::new(Self { client })
    }

    pub async fn search(&self, query: &str, limit: usize) -> Result<Vec<LavalinkTrack>, String> {
        let url = format!(
            "{}?term={}&media=music&entity=song&limit={}",
            ITUNES_SEARCH_URL,
            urlencoding::encode(query.trim()),
            limit
        );

        let res = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| e.to_string())?;
        let json: Value = res.json().await.map_err(|e| e.to_string())?;

        let results = json
            .get("results")
            .and_then(|r| r.as_array())
            .ok_or("No results")?;

        let mut tracks = Vec::new();
        for item in results {
            if let Some(track) = self.parse_track(item) {
                tracks.push(track);
            }
        }

        Ok(tracks)
    }

    pub async fn resolve_track(&self, track_id: &str) -> Result<Option<LavalinkTrack>, String> {
        let url = format!("{}?id={}", ITUNES_LOOKUP_URL, track_id);

        let res = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| e.to_string())?;
        let json: Value = res.json().await.map_err(|e| e.to_string())?;

        let results = json
            .get("results")
            .and_then(|r| r.as_array())
            .and_then(|r| r.first());

        match results {
            Some(item) => Ok(self.parse_track(item)),
            None => Ok(None),
        }
    }

    fn parse_track(&self, item: &Value) -> Option<LavalinkTrack> {
        let track_id = item.get("trackId").and_then(|v| v.as_u64())?.to_string();
        let title = item
            .get("trackName")
            .and_then(|t| t.as_str())
            .unwrap_or("Unknown Title")
            .to_string();
        let artist = item
            .get("artistName")
            .and_then(|a| a.as_str())
            .unwrap_or("Unknown Artist")
            .to_string();
        let duration_ms = item
            .get("trackTimeMillis")
            .and_then(|d| d.as_u64())
            .unwrap_or(0);
        let artwork = item
            .get("artworkUrl100")
            .and_then(|u| u.as_str())
            .map(|s| s.replace("100x100bb", "512x512bb"));
        let preview_url = item
            .get("previewUrl")
            .and_then(|u| u.as_str())
            .map(|s| s.to_string());
        let track_view_url = item
            .get("trackViewUrl")
            .and_then(|u| u.as_str())
            .map(|s| s.to_string());

        let mut track = LavalinkTrack {
            encoded: String::new(),
            info: TrackInfo {
                identifier: track_id,
                is_seekable: true,
                author: artist,
                length: duration_ms,
                is_stream: false,
                position: 0,
                title,
                uri: preview_url.or(track_view_url),
                artwork_url: artwork,
                isrc: None,
                source_name: "applemusic".to_string(),
            },
            plugin_info: Value::Object(Default::default()),
            user_data: Value::Object(Default::default()),
        };

        if let Ok(enc) = crate::track_encoding::encode_track(&track) {
            track.encoded = enc;
        }

        Some(track)
    }
}
