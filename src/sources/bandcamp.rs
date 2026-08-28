use crate::models::track::{LavalinkTrack, TrackInfo};
use crate::sources::backoff::{with_backoff, BackoffConfig};
use reqwest::Client;
use serde_json::Value;
use std::sync::Arc;

const BANDCAMP_API: &str = "https://bandcamp.com/api";

pub struct BandcampSource {
    client: Client,
}

impl BandcampSource {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            client: Client::builder()
                .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
                .build()
                .unwrap_or_default(),
        })
    }

    pub async fn search(&self, query: &str, limit: usize) -> Result<Vec<LavalinkTrack>, String> {
        let url = format!(
            "{}/fuzzysearch/2/autocomplete?q={}",
            BANDCAMP_API,
            urlencoding::encode(query)
        );

        let client = self.client.clone();
        let cfg = BackoffConfig::default();
        let json = with_backoff(&cfg, "bandcamp_search", || {
            let client = client.clone();
            let url = url.clone();
            async move {
                let res = client.get(&url).send().await.map_err(|e| e.to_string())?;
                let text = res.text().await.map_err(|e| e.to_string())?;
                Ok::<String, String>(text)
            }
        })
        .await?;

        let json: Value = serde_json::from_str(&json).map_err(|e| e.to_string())?;

        let mut tracks = Vec::new();

        if let Some(results) = json.get("results").and_then(|r| r.get("auto")).and_then(|a| a.as_array()) {
            for item in results.iter().take(limit) {
                if let Some(track) = self.parse_search_item(item) {
                    tracks.push(track);
                }
            }
        }

        if let Some(results) = json.get("results").and_then(|r| r.get("bulk")).and_then(|a| a.as_array()) {
            for item in results.iter().take(limit.saturating_sub(tracks.len())) {
                if let Some(track) = self.parse_search_item(item) {
                    tracks.push(track);
                }
            }
        }

        Ok(tracks)
    }

    pub async fn resolve_track(&self, url: &str) -> Result<Option<LavalinkTrack>, String> {
        let client = self.client.clone();
        let cfg = BackoffConfig::default();
        let page_html = with_backoff(&cfg, "bandcamp_resolve", || {
            let client = client.clone();
            let url = url.to_string();
            async move {
                let res = client.get(&url).send().await.map_err(|e| e.to_string())?;
                let text = res.text().await.map_err(|e| e.to_string())?;
                Ok::<String, String>(text)
            }
        })
        .await?;

        if let Some(start) = page_html.find("var TralbumData = ") {
            let json_start = start + "var TralbumData = ".len();
            if let Some(end) = page_html[json_start..].find("};") {
                let json_str = &page_html[json_start..json_start + end + 1];
                if let Ok(tralbum) = serde_json::from_str::<Value>(json_str) {
                    return Ok(self.parse_tralbum(&tralbum));
                }
            }
        }

        Ok(None)
    }

    fn parse_search_item(&self, item: &Value) -> Option<LavalinkTrack> {
        let title = item.get("title").and_then(|t| t.as_str())?.to_string();
        let artist = item.get("artist").and_then(|a| a.as_str()).unwrap_or("Unknown Artist").to_string();
        let url = item.get("url").and_then(|u| u.as_str()).map(|s| s.to_string());
        let id = item.get("id").and_then(|i| i.as_u64()).unwrap_or(0).to_string();

        let mut track = LavalinkTrack {
            encoded: String::new(),
            info: TrackInfo {
                identifier: id,
                is_seekable: true,
                author: artist,
                length: 0,
                is_stream: false,
                position: 0,
                title,
                uri: url,
                artwork_url: None,
                isrc: None,
                source_name: "bandcamp".to_string(),
            },
            plugin_info: serde_json::json!({}),
            user_data: serde_json::json!({}),
        };

        if let Ok(enc) = crate::track_encoding::encode_track(&track) {
            track.encoded = enc;
        }

        Some(track)
    }

    fn parse_tralbum(&self, tralbum: &Value) -> Option<LavalinkTrack> {
        let title = tralbum.get("current").and_then(|c| c.get("title")).and_then(|t| t.as_str()).unwrap_or("Unknown").to_string();
        let artist = tralbum.get("artist").and_then(|a| a.as_str()).unwrap_or("Unknown Artist").to_string();
        let duration_ms = tralbum.get("current").and_then(|c| c.get("duration")).and_then(|d| d.as_u64()).map(|d| d * 1000).unwrap_or(0);
        let artwork = tralbum.get("art_fullsize_url").and_then(|a| a.as_str()).map(|s| s.to_string());

        let mut stream_url = None;
        if let Some(url) = tralbum.get("free_download_url").and_then(|u| u.as_str()) {
            stream_url = Some(url.to_string());
        } else if let Some(url) = tralbum.get("track_stream_url").and_then(|u| u.as_str()) {
            stream_url = Some(url.to_string());
        }

        let url = stream_url.clone().or_else(|| {
            tralbum.get("track_submission_url").and_then(|u| u.as_str()).map(|s| s.to_string())
        });

        let id = url.as_deref().and_then(|u| {
            u.rsplit('/').next().map(|s| s.to_string())
        }).unwrap_or_else(|| title.clone());

        let mut track = LavalinkTrack {
            encoded: String::new(),
            info: TrackInfo {
                identifier: id,
                is_seekable: true,
                author: artist,
                length: duration_ms,
                is_stream: false,
                position: 0,
                title,
                uri: stream_url,
                artwork_url: artwork,
                isrc: None,
                source_name: "bandcamp".to_string(),
            },
            plugin_info: serde_json::json!({}),
            user_data: serde_json::json!({}),
        };

        if let Ok(enc) = crate::track_encoding::encode_track(&track) {
            track.encoded = enc;
        }

        Some(track)
    }
}
