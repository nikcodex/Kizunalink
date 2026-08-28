use crate::models::track::{LavalinkTrack, TrackInfo};
use reqwest::header::{HeaderMap, HeaderValue, USER_AGENT};
use serde_json::Value;
use std::sync::Arc;

const DEEZER_API_BASE: &str = "https://api.deezer.com";

pub struct DeezerSource {
    client: reqwest::Client,
}

impl DeezerSource {
    pub fn new() -> Arc<Self> {
        let mut headers = HeaderMap::new();
        headers.insert(
            USER_AGENT,
            HeaderValue::from_static(
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36",
            ),
        );

        let client = crate::config::global_proxy()
            .apply_to_builder(reqwest::Client::builder())
            .default_headers(headers)
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .expect("Failed to build Reqwest client for Deezer");

        Arc::new(Self { client })
    }

    pub async fn search(&self, query: &str, limit: usize) -> Result<Vec<LavalinkTrack>, String> {
        let url = format!(
            "{}/search/track?q={}&limit={}",
            DEEZER_API_BASE,
            urlencoding::encode(query.trim()),
            limit
        );

        let res = self.client.get(&url).send().await.map_err(|e| e.to_string())?;
        let json: Value = res.json().await.map_err(|e| e.to_string())?;

        let data = json
            .get("data")
            .and_then(|d| d.as_array())
            .ok_or("No results")?;

        let mut tracks = Vec::new();
        for item in data {
            if let Some(track) = self.parse_track(item) {
                tracks.push(track);
            }
        }

        Ok(tracks)
    }

    pub async fn resolve_track(&self, track_id: &str) -> Result<Option<LavalinkTrack>, String> {
        let url = format!("{}/track/{}", DEEZER_API_BASE, track_id);

        let res = self.client.get(&url).send().await.map_err(|e| e.to_string())?;

        if res.status() == 404 {
            return Ok(None);
        }

        let json: Value = res.json().await.map_err(|e| e.to_string())?;

        // Check for error response
        if json.get("error").is_some() {
            return Ok(None);
        }

        Ok(self.parse_track(&json))
    }

    pub async fn resolve_playlist(
        &self,
        playlist_id: &str,
    ) -> Result<Option<crate::models::track::PlaylistData>, String> {
        let url = format!("{}/playlist/{}", DEEZER_API_BASE, playlist_id);

        let res = self.client.get(&url).send().await.map_err(|e| e.to_string())?;

        if res.status() == 404 {
            return Ok(None);
        }

        let json: Value = res.json().await.map_err(|e| e.to_string())?;

        if json.get("error").is_some() {
            return Ok(None);
        }

        let title = json
            .get("title")
            .and_then(|t| t.as_str())
            .unwrap_or("Deezer Playlist")
            .to_string();

        let mut tracks = Vec::new();
        if let Some(items) = json.get("tracks").and_then(|t| t.get("data")).and_then(|d| d.as_array()) {
            for item in items {
                if let Some(track) = self.parse_track(item) {
                    tracks.push(track);
                }
            }
        }

        // Follow pagination
        let mut next = json
            .get("tracks")
            .and_then(|t| t.get("next"))
            .and_then(|n| n.as_str())
            .map(|s| s.to_string());

        while let Some(next_url) = next.take() {
            if tracks.len() >= 500 {
                break;
            }
            let page_res = self.client.get(&next_url).send().await;
            let page_json: Value = match page_res {
                Ok(r) => r.json().await.unwrap_or_default(),
                Err(_) => break,
            };
            if let Some(items) = page_json.get("data").and_then(|d| d.as_array()) {
                for item in items {
                    if let Some(track) = self.parse_track(item) {
                        tracks.push(track);
                    }
                }
            }
            next = page_json
                .get("next")
                .and_then(|n| n.as_str())
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string());
        }

        Ok(Some(crate::models::track::PlaylistData {
            info: crate::models::track::PlaylistInfo {
                name: title,
                selected_track: 0,
            },
            plugin_info: Value::Object(Default::default()),
            tracks,
        }))
    }

    fn parse_track(&self, item: &Value) -> Option<LavalinkTrack> {
        let id = item.get("id").and_then(|v| v.as_u64())?.to_string();
        let title = item
            .get("title")
            .and_then(|t| t.as_str())
            .unwrap_or("Unknown Title")
            .to_string();
        let artist = item
            .get("artist")
            .and_then(|a| a.get("name"))
            .and_then(|n| n.as_str())
            .unwrap_or("Unknown Artist")
            .to_string();
        let duration = item.get("duration").and_then(|d| d.as_u64()).unwrap_or(0);
        let artwork = item
            .get("album")
            .and_then(|a| a.get("cover_big"))
            .and_then(|c| c.as_str())
            .or_else(|| {
                item.get("album")
                    .and_then(|a| a.get("cover_medium"))
                    .and_then(|c| c.as_str())
            })
            .map(|s| s.to_string());
        let preview = item
            .get("preview")
            .and_then(|p| p.as_str())
            .map(|s| s.to_string());
        let link = item
            .get("link")
            .and_then(|l| l.as_str())
            .map(|s| s.to_string());

        // Extract ISRC if available (Deezer provides it)
        let isrc = item
            .get("isrc")
            .and_then(|i| i.as_str())
            .map(|s| s.to_string());

        let album_id = item
            .get("album")
            .and_then(|a| a.get("id"))
            .and_then(|id| id.as_u64());

        let mut plugin_info = serde_json::Map::new();
        if let Some(code) = &isrc {
            plugin_info.insert("isrc".to_string(), Value::String(code.clone()));
        }
        if let Some(aid) = album_id {
            plugin_info.insert("deezerAlbumId".to_string(), Value::Number(aid.into()));
        }

        let mut track = LavalinkTrack {
            encoded: String::new(),
            info: TrackInfo {
                identifier: id,
                is_seekable: true,
                author: artist,
                length: duration * 1000,
                is_stream: false,
                position: 0,
                title,
                uri: preview.or(link),
                artwork_url: artwork,
                isrc,
                source_name: "deezer".to_string(),
            },
            plugin_info: Value::Object(plugin_info),
            user_data: Value::Object(Default::default()),
        };

        if let Ok(enc) = crate::track_encoding::encode_track(&track) {
            track.encoded = enc;
        }

        Some(track)
    }
}
