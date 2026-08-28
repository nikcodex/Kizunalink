use crate::models::track::{LavalinkTrack, PlaylistData, PlaylistInfo, TrackInfo};
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, ACCEPT, USER_AGENT};
use serde::Deserialize;
use serde_json::Value;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::RwLock;
use tracing::{info, warn};

const SPOTIFY_TOKEN_URL: &str = "https://accounts.spotify.com/api/token";
const SPOTIFY_API_BASE: &str = "https://api.spotify.com/v1";

/// Spotify client credentials (proper OAuth2 flow)
#[derive(Debug, Clone, Deserialize)]
struct SpotifyClientCredentials {
    access_token: String,
    expires_in: u64,
}

pub struct SpotifySource {
    client: reqwest::Client,
    /// (token, expires_at_millis)
    token: RwLock<Option<(String, u64)>>,
    /// Spotify client ID and secret for proper client_credentials flow
    client_id: Option<String>,
    client_secret: Option<String>,
}

impl SpotifySource {
    pub fn new() -> Arc<Self> {
        let mut headers = HeaderMap::new();
        headers.insert(
            USER_AGENT,
            HeaderValue::from_static(
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36",
            ),
        );
        headers.insert(ACCEPT, HeaderValue::from_static("application/json"));

        let client = reqwest::Client::builder()
            .default_headers(headers)
            .timeout(Duration::from_secs(5))
            .build()
            .expect("Failed to build Reqwest client for Spotify");

        let client_id = std::env::var("SPOTIFY_CLIENT_ID").ok();
        let client_secret = std::env::var("SPOTIFY_CLIENT_SECRET").ok();

        if client_id.is_some() && client_secret.is_some() {
            info!("Spotify: Using client_credentials OAuth2 flow");
        } else {
            warn!(
                "Spotify: No SPOTIFY_CLIENT_ID/SPOTIFY_CLIENT_SECRET set. \
                 Using anonymous token (less reliable). Set credentials for production use."
            );
        }

        Arc::new(Self {
            client,
            token: RwLock::new(None),
            client_id,
            client_secret,
        })
    }

    pub async fn get_token(&self) -> Result<String, String> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        // Check cache first
        {
            let read = self.token.read().await;
            if let Some((tok, exp)) = &*read {
                if now + 60_000 < *exp {
                    return Ok(tok.clone());
                }
            }
        }

        // Double-check after acquiring write lock
        let mut write = self.token.write().await;
        if let Some((tok, exp)) = &*write {
            if now + 60_000 < *exp {
                return Ok(tok.clone());
            }
        }

        // Try client_credentials flow first (more reliable)
        if let (Some(client_id), Some(client_secret)) = (&self.client_id, &self.client_secret) {
            match self.fetch_client_credentials_token(client_id, client_secret).await {
                Ok(creds) => {
                    let expires_at = now + (creds.expires_in * 1000);
                    *write = Some((creds.access_token.clone(), expires_at));
                    info!("Spotify: Refreshed client_credentials token");
                    return Ok(creds.access_token);
                }
                Err(e) => {
                    warn!("Spotify: client_credentials failed ({}), trying anonymous...", e);
                }
            }
        }

        // Fall back to anonymous token
        let (token, expires_at) = self.fetch_anonymous_token().await?;
        *write = Some((token.clone(), expires_at));
        Ok(token)
    }

    async fn fetch_client_credentials_token(
        &self,
        client_id: &str,
        client_secret: &str,
    ) -> Result<SpotifyClientCredentials, String> {
        let auth = base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            format!("{}:{}", client_id, client_secret),
        );

        let res = self
            .client
            .post(SPOTIFY_TOKEN_URL)
            .header(AUTHORIZATION, format!("Basic {}", auth))
            .form(&[("grant_type", "client_credentials")])
            .send()
            .await
            .map_err(|e| e.to_string())?;

        if !res.status().is_success() {
            return Err(format!("HTTP {}", res.status()));
        }

        res.json().await.map_err(|e| e.to_string())
    }

    async fn fetch_anonymous_token(&self) -> Result<(String, u64), String> {
        let url = "https://open.spotify.com/get_access_token?reason=transport&productType=web_player";

        let res = self
            .client
            .get(url)
            .send()
            .await
            .map_err(|e| e.to_string())?;

        #[derive(Deserialize)]
        struct AnonymousToken {
            #[serde(rename = "accessToken")]
            access_token: String,
            #[serde(rename = "accessTokenExpirationTimestampMs")]
            expires_at: u64,
        }

        let token_data: AnonymousToken = res.json().await.map_err(|e| e.to_string())?;
        info!("Spotify: Refreshed anonymous token");
        Ok((token_data.access_token, token_data.expires_at))
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
            .bearer_auth(&token)
            .send()
            .await
            .map_err(|e| e.to_string())?;

        // Handle 401 — token might be expired, force refresh
        if res.status() == 401 {
            self.token.write().await.take();
            let token = self.get_token().await?;
            let res = self
                .client
                .get(&url)
                .bearer_auth(&token)
                .send()
                .await
                .map_err(|e| e.to_string())?;
            let json: Value = res.json().await.map_err(|e| e.to_string())?;
            return self.parse_search_results(&json, limit);
        }

        let json: Value = res.json().await.map_err(|e| e.to_string())?;
        self.parse_search_results(&json, limit)
    }

    fn parse_search_results(
        &self,
        json: &Value,
        limit: usize,
    ) -> Result<Vec<LavalinkTrack>, String> {
        let items = json
            .get("tracks")
            .and_then(|t| t.get("items"))
            .and_then(|i| i.as_array());

        let items = match items {
            Some(arr) => arr,
            None => return Ok(vec![]),
        };

        let mut tracks = Vec::new();
        for item in items {
            if let Some(track) = self.parse_track_item(item) {
                tracks.push(track);
                if tracks.len() >= limit {
                    break;
                }
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
            .bearer_auth(&token)
            .send()
            .await
            .map_err(|e| e.to_string())?;

        if res.status() == 404 {
            return Ok(None);
        }

        if res.status() == 401 {
            self.token.write().await.take();
            let token = self.get_token().await?;
            let res = self
                .client
                .get(&url)
                .bearer_auth(&token)
                .send()
                .await
                .map_err(|e| e.to_string())?;
            let json: Value = res.json().await.map_err(|e| e.to_string())?;
            return Ok(self.parse_track_item(&json));
        }

        let json: Value = res.json().await.map_err(|e| e.to_string())?;
        Ok(self.parse_track_item(&json))
    }

    pub async fn resolve_playlist(
        &self,
        playlist_id: &str,
    ) -> Result<Option<PlaylistData>, String> {
        let token = self.get_token().await?;
        let url = format!("{}/playlists/{}", SPOTIFY_API_BASE, playlist_id);

        let res = self
            .client
            .get(&url)
            .bearer_auth(&token)
            .send()
            .await
            .map_err(|e| e.to_string())?;

        if res.status() == 404 {
            return Ok(None);
        }

        if res.status() == 401 {
            self.token.write().await.take();
            let token = self.get_token().await?;
            let res = self
                .client
                .get(&url)
                .bearer_auth(&token)
                .send()
                .await
                .map_err(|e| e.to_string())?;
            if res.status() == 404 {
                return Ok(None);
            }
            let json: Value = res.json().await.map_err(|e| e.to_string())?;
            return self.parse_playlist_response(&json, &token).await;
        }

        let json: Value = res.json().await.map_err(|e| e.to_string())?;
        self.parse_playlist_response(&json, &token).await
    }

    async fn parse_playlist_response(&self, json: &Value, token: &str) -> Result<Option<PlaylistData>, String> {
        let pl_name = json
            .get("name")
            .and_then(|n| n.as_str())
            .unwrap_or("Spotify Playlist")
            .to_string();

        let mut tracks = Vec::new();

        // Collect first page
        if let Some(items) = json
            .get("tracks")
            .and_then(|t| t.get("items"))
            .and_then(|i| i.as_array())
        {
            for entry in items {
                if let Some(track_val) = entry.get("track") {
                    if let Some(track) = self.parse_track_item(track_val) {
                        tracks.push(track);
                    }
                }
            }
        }

        // Follow pagination (up to 500 tracks)
        let mut next_url = json
            .get("tracks")
            .and_then(|t| t.get("next"))
            .and_then(|n| n.as_str())
            .map(|s| s.to_string());

        while let Some(url) = next_url.take() {
            if tracks.len() >= 500 {
                break;
            }
            let page_res = self.client.get(&url).bearer_auth(token).send().await;
            let page_json: Value = match page_res {
                Ok(r) => r.json().await.unwrap_or_default(),
                Err(_) => break,
            };
            if let Some(items) = page_json.get("items").and_then(|i| i.as_array()) {
                for entry in items {
                    if let Some(track_val) = entry.get("track") {
                        if let Some(track) = self.parse_track_item(track_val) {
                            tracks.push(track);
                        }
                    }
                }
            }
            next_url = page_json
                .get("next")
                .and_then(|n| n.as_str())
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string());
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
        let title = item
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown Title")
            .to_string();

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

        let mut plugin_info = serde_json::Map::new();
        if let Some(code) = &isrc {
            plugin_info.insert("isrc".to_string(), Value::String(code.clone()));
        }

        let mut track = LavalinkTrack {
            encoded: String::new(),
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
                isrc,
                source_name: "spotify".to_string(),
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
