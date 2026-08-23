use crate::models::track::{LavalinkTrack, TrackInfo};
use reqwest::Client;
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::RwLock;

const SOUNDCLOUD_API: &str = "https://api-v2.soundcloud.com";

#[derive(Clone)]
pub struct SoundCloudSource {
    client: Client,
    client_id: Arc<RwLock<String>>,
}

impl SoundCloudSource {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            client: Client::builder()
                .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
                .build()
                .unwrap_or_default(),
            client_id: Arc::new(RwLock::new("2t9loNfh90kzAzYrI6NvRcqDuq9U6PL2".to_string())),
        })
    }

    pub async fn search(&self, query: &str, limit: usize) -> Result<Vec<LavalinkTrack>, String> {
        let client_id = self.client_id.read().await.clone();
        let url = format!(
            "{}/search/tracks?q={}&client_id={}&limit={}",
            SOUNDCLOUD_API,
            urlencoding::encode(query),
            client_id,
            limit
        );

        let res = self.client.get(&url).send().await.map_err(|e| e.to_string())?;
        let json: Value = res.json().await.map_err(|e| e.to_string())?;

        let mut tracks = Vec::new();
        if let Some(items) = json.get("collection").and_then(|c| c.as_array()) {
            for item in items {
                let id = item.get("id").and_then(|i| i.as_u64()).map(|i| i.to_string()).unwrap_or_default();
                let title = item.get("title").and_then(|t| t.as_str()).unwrap_or("Unknown Title").to_string();
                let author = item.get("user").and_then(|u| u.get("username")).and_then(|un| un.as_str()).unwrap_or("Unknown Artist").to_string();
                let duration_ms = item.get("duration").and_then(|d| d.as_u64()).unwrap_or(0);
                let artwork = item.get("artwork_url").and_then(|a| a.as_str()).map(|s| s.replace("large", "t500x500"));
                let uri = item.get("permalink_url").and_then(|u| u.as_str()).map(|s| s.to_string());

                let mut track = LavalinkTrack {
                    encoded: String::new(),
                    info: TrackInfo {
                        identifier: id,
                        is_seekable: true,
                        author,
                        length: duration_ms,
                        is_stream: false,
                        position: 0,
                        title,
                        uri,
                        artwork_url: artwork,
                        isrc: None,
                        source_name: "soundcloud".to_string(),
                    },
                    plugin_info: serde_json::json!({}),
                    user_data: serde_json::json!({}),
                };

                if let Ok(enc) = crate::track_encoding::encode_track(&track) {
                    track.encoded = enc;
                }

                tracks.push(track);
            }
        }

        Ok(tracks)
    }

    pub async fn resolve_stream(&self, track_id: &str) -> Result<String, String> {
        let client_id = self.client_id.read().await.clone();
        let url = format!(
            "{}/tracks/{}?client_id={}",
            SOUNDCLOUD_API, track_id, client_id
        );

        let res = self.client.get(&url).send().await.map_err(|e| e.to_string())?;
        let json: Value = res.json().await.map_err(|e| e.to_string())?;

        if let Some(stream_url) = json.get("stream_url").and_then(|u| u.as_str()) {
            let mut params = url::form_urlencoded::parse(stream_url.split('?').nth(1).unwrap_or("").as_bytes())
                .map(|(k, v)| (k.into_owned(), v.into_owned()))
                .collect::<Vec<_>>();
            params.push(("client_id".to_string(), client_id));

            let redirect_url = format!(
                "{}?{}",
                stream_url.split('?').next().unwrap_or(stream_url),
                params.iter()
                    .map(|(k, v)| format!("{}={}", k, v))
                    .collect::<Vec<_>>()
                    .join("&")
            );

            let stream_res = self.client.get(&redirect_url).send().await.map_err(|e| e.to_string())?;
            let stream_json: Value = stream_res.json().await.map_err(|e| e.to_string())?;

            if let Some(url) = stream_json.get("url").and_then(|u| u.as_str()) {
                return Ok(url.to_string());
            }
        }

        Err("No stream URL found for SoundCloud track".to_string())
    }
}
