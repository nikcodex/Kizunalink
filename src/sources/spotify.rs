use crate::models::track::{LavalinkTrack, PlaylistData, PlaylistInfo, TrackInfo};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, USER_AGENT};
use serde::Deserialize;
use serde_json::Value;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::RwLock;
use tracing::info;

const SPOTIFY_TOKEN_URL: &str = "https://open.spotify.com/get_access_token?reason=transport&productType=web_player";
const SPOTIFY_API_BASE: &str = "https://api.spotify.com/v1";

#[derive(Debug, Clone, Deserialize)]
struct SpotifyAnonymousToken {
    #[serde(rename = "accessToken")]
    access_token: String,
    #[serde(rename = "accessTokenExpirationTimestampMs")]
    expires_at: u64,
}

pub struct SpotifySource {
    client: reqwest::Client,
    token: RwLock<Option<(String, u64)>>,
}

impl SpotifySource {
    pub fn new() -> Arc<Self> {
        let mut headers = HeaderMap::new();
        headers.insert(USER_AGENT, HeaderValue::from_static("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36"));
        headers.insert(ACCEPT, HeaderValue::from_static("application/json"));

        let client = reqwest::Client::builder()
            .default_headers(headers)
            .build()
            .expect("Failed to build Reqwest client for Spotify");

        Arc::new(Self {
            client,
            token: RwLock::new(None),
        })
    }

    pub async fn get_token(&self) -> Result<String, String> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        {
            let read = self.token.read().await;
            if let Some((tok, exp)) = &*read {
                if now + 60_000 < *exp {
                    return Ok(tok.clone());
                }
            }
        }

        let mut write = self.token.write().await;
        if let Some((tok, exp)) = &*write {
            if now + 60_000 < *exp {
                return Ok(tok.clone());
            }
        }

        let res = self
            .client
            .get(SPOTIFY_TOKEN_URL)
            .send()
            .await
            .map_err(|e| e.to_string())?;

        let token_data: SpotifyAnonymousToken = res.json().await.map_err(|e| e.to_string())?;
        let token = token_data.access_token.clone();
        *write = Some((token_data.access_token, token_data.expires_at));

        info!("🔑 Refreshed Spotify anonymous token successfully");
        Ok(token)
    }

    pub async fn search(&self, query: &str, limit: usize) -> Result<Vec<LavalinkTrack>, String> {
        let token = self.get_token().await?;
        let url = format!(
            "{}/search?type=track&q={}&limit={}",
            SPOTIFY_API_BASE,
            urlencoding::encode(query.trim()),
            limit
        );

        let res = self
            .client
            .get(&url)
            .bearer_auth(token)
            .send()
            .await
            .map_err(|e| e.to_string())?;

        let json: Value = res.json().await.map_err(|e| e.to_string())?;
        let tracks_arr = json
            .get("tracks")
            .and_then(|t| t.get("items"))
            .and_then(|i| i.as_array());

        let items = match tracks_arr {
            Some(arr) => arr,
            None => return Ok(vec![]),
        };

        let mut tracks = Vec::new();
        for item in items {
            if let Some(track) = self.parse_track_item(item) {
                tracks.push(track);
            }
        }

        Ok(tracks)
    }

    pub async fn resolve_track(&self, track_id: &str) -> Result<Option<LavalinkTrack>, String> {
        let token = self.get_token().await?;
        let url = format!("{}/tracks/{}", SPOTIFY_API_BASE, track_id);

        let res = self
            .client
            .get(&url)
            .bearer_auth(token)
            .send()
            .await
            .map_err(|e| e.to_string())?;

        if res.status() == 404 {
            return Ok(None);
        }

        let json: Value = res.json().await.map_err(|e| e.to_string())?;
        Ok(self.parse_track_item(&json))
    }

    pub async fn resolve_playlist(&self, playlist_id: &str) -> Result<Option<PlaylistData>, String> {
        let token = self.get_token().await?;
        let url = format!("{}/playlists/{}", SPOTIFY_API_BASE, playlist_id);

        let res = self
            .client
            .get(&url)
            .bearer_auth(token)
            .send()
            .await
            .map_err(|e| e.to_string())?;

        if res.status() == 404 {
            return Ok(None);
        }

        let json: Value = res.json().await.map_err(|e| e.to_string())?;
        let pl_name = json.get("name").and_then(|n| n.as_str()).unwrap_or("Spotify Playlist").to_string();

        let mut tracks = Vec::new();
        if let Some(items) = json.get("tracks").and_then(|t| t.get("items")).and_then(|i| i.as_array()) {
            for entry in items {
                if let Some(track_val) = entry.get("track") {
                    if let Some(track) = self.parse_track_item(track_val) {
                        tracks.push(track);
                    }
                }
            }
        }

        Ok(Some(PlaylistData {
            info: PlaylistInfo {
                name: pl_name,
                selected_track: 0,
            },
            plugin_info: Value::Object(Default::default()),
            tracks,
        }))
    }

    fn parse_track_item(&self, item: &Value) -> Option<LavalinkTrack> {
        let id = item.get("id").and_then(|v| v.as_str())?.to_string();
        let title = item.get("name").and_then(|v| v.as_str()).unwrap_or("Unknown Title").to_string();

        let artists = item
            .get("artists")
            .and_then(|a| a.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|art| art.get("name").and_then(|n| n.as_str()))
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_else(|| "Unknown Artist".to_string());

        let duration_ms = item.get("duration_ms").and_then(|d| d.as_u64()).unwrap_or(0);

        let artwork = item
            .get("album")
            .and_then(|al| al.get("images"))
            .and_then(|imgs| imgs.as_array())
            .and_then(|arr| arr.first())
            .and_then(|img| img.get("url"))
            .and_then(|u| u.as_str())
            .map(|s| s.to_string());

        let isrc = item
            .get("external_ids")
            .and_then(|ext| ext.get("isrc"))
            .and_then(|i| i.as_str())
            .map(|s| s.to_string());

        let uri = item
            .get("external_urls")
            .and_then(|ext| ext.get("spotify"))
            .and_then(|u| u.as_str())
            .map(|s| s.to_string());

        let encoded = STANDARD.encode(format!("spotify:{}", id));

        let mut plugin_info = serde_json::Map::new();
        if let Some(code) = isrc {
            plugin_info.insert("isrc".to_string(), Value::String(code));
        }

        Some(LavalinkTrack {
            encoded,
            info: TrackInfo {
                identifier: id,
                is_seekable: true,
                author: artists,
                length: duration_ms,
                is_stream: false,
                position: 0,
                title,
                uri,
                artwork_url: artwork,
                source_name: "spotify".to_string(),
                bitrate: Some("320kbps".to_string()),
                stream_url: None,
            },
            plugin_info: Value::Object(plugin_info),
            user_data: Value::Object(Default::default()),
        })
    }
}
