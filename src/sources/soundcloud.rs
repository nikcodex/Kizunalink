use crate::models::track::{LavalinkTrack, TrackInfo};
use reqwest::Client;
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{info, warn};

const SOUNDCLOUD_API: &str = "https://api-v2.soundcloud.com";

/// Pool of client_id values — if one gets rate-limited, try the next.
/// These are public client IDs extracted from SoundCloud's web player.
const CLIENT_ID_POOL: &[&str] = &[
    "2t9loNfh90kzAzYrI6NvRcqDuq9U6PL2",
    "M2TjD1HrYvf5BrV1xC0rNDM0t2eZUNIE",
    "ZxwVFtpflsFVJEOCQqGKb0Wc6VzHMFML",
];

pub struct SoundCloudSource {
    client: Client,
    /// Currently active client_id index
    client_id_index: Arc<RwLock<usize>>,
    /// Whether using env-provided client_id (never rotate)
    env_client_id: Option<String>,
}

impl SoundCloudSource {
    pub fn new() -> Arc<Self> {
        let env_client_id = std::env::var("SOUNDCLOUD_CLIENT_ID").ok();
        if env_client_id.is_some() {
            info!("SoundCloud: Using client_id from SOUNDCLOUD_CLIENT_ID env var");
        } else {
            warn!(
                "SoundCloud: No SOUNDCLOUD_CLIENT_ID set. Using built-in pool (may be rate-limited)."
            );
        }

        Arc::new(Self {
            client: crate::config::global_proxy()
                .apply_to_builder(Client::builder())
                .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
                .timeout(std::time::Duration::from_secs(10))
                .build()
                .unwrap_or_default(),
            client_id_index: Arc::new(RwLock::new(0)),
            env_client_id,
        })
    }

    /// Get the current client_id, rotating if needed after errors
    async fn get_client_id(&self) -> String {
        if let Some(ref id) = self.env_client_id {
            return id.clone();
        }
        let idx = *self.client_id_index.read().await;
        CLIENT_ID_POOL[idx % CLIENT_ID_POOL.len()].to_string()
    }

    /// Rotate to next client_id after rate limit
    async fn rotate_client_id(&self) {
        if self.env_client_id.is_some() {
            return;
        }
        let mut idx = self.client_id_index.write().await;
        let old = *idx;
        *idx = (*idx + 1) % CLIENT_ID_POOL.len();
        info!(
            "SoundCloud: Rotated client_id from index {} to {}",
            old, *idx
        );
    }

    /// Try a request with automatic client_id rotation on 401/403/429
    async fn request_with_retry<F, T>(
        &self,
        build_url: impl Fn(&str) -> String,
        parse: F,
    ) -> Result<T, String>
    where
        F: Fn(Value) -> Option<T>,
    {
        let max_attempts = CLIENT_ID_POOL.len();
        for attempt in 0..max_attempts {
            let client_id = self.get_client_id().await;
            let url = build_url(&client_id);

            match self.client.get(&url).send().await {
                Ok(res) => {
                    let status = res.status();
                    if status == 401 || status == 403 || status == 429 {
                        warn!(
                            "SoundCloud: Got {} with client_id attempt {}/{}, rotating...",
                            status,
                            attempt + 1,
                            max_attempts
                        );
                        self.rotate_client_id().await;
                        continue;
                    }

                    let json: Value = res.json().await.map_err(|e| e.to_string())?;
                    if let Some(result) = parse(json) {
                        return Ok(result);
                    }
                    return Err("No results found".to_string());
                }
                Err(e) => {
                    if attempt < max_attempts - 1 {
                        warn!("SoundCloud: Request error ({}), retrying...", e);
                        self.rotate_client_id().await;
                        continue;
                    }
                    return Err(format!("SoundCloud request failed: {}", e));
                }
            }
        }
        Err("All SoundCloud client_ids exhausted".to_string())
    }

    pub async fn search(&self, query: &str, limit: usize) -> Result<Vec<LavalinkTrack>, String> {
        self.request_with_retry(
            |client_id| {
                format!(
                    "{}/search/tracks?q={}&client_id={}&limit={}",
                    SOUNDCLOUD_API,
                    urlencoding::encode(query),
                    client_id,
                    limit
                )
            },
            |json| {
                let items = json.get("collection").and_then(|c| c.as_array())?;
                let tracks: Vec<LavalinkTrack> =
                    items.iter().filter_map(parse_track).take(limit).collect();
                Some(tracks)
            },
        )
        .await
    }

    pub async fn resolve_stream(&self, track_id: &str) -> Result<String, String> {
        self.request_with_retry(
            |client_id| {
                format!(
                    "{}/tracks/{}?client_id={}",
                    SOUNDCLOUD_API, track_id, client_id
                )
            },
            |json| {
                let stream_url = json.get("stream_url").and_then(|u| u.as_str())?;
                // The stream_url needs a follow redirect with client_id
                Some(stream_url.to_string())
            },
        )
        .await?;

        // Follow the redirect with client_id
        let client_id = self.get_client_id().await;
        let client_id_clone = client_id.clone();

        // Re-fetch to get the actual streaming URL
        self.request_with_retry(
            |cid| format!("{}/tracks/{}?client_id={}", SOUNDCLOUD_API, track_id, cid),
            |json| {
                let stream_url = json.get("stream_url").and_then(|u| u.as_str())?;
                let mut params: Vec<(String, String)> = url::form_urlencoded::parse(
                    stream_url.split('?').nth(1).unwrap_or("").as_bytes(),
                )
                .map(|(k, v)| (k.into_owned(), v.into_owned()))
                .collect();
                params.push(("client_id".to_string(), client_id_clone.clone()));

                let redirect_url = format!(
                    "{}?{}",
                    stream_url.split('?').next().unwrap_or(stream_url),
                    params
                        .iter()
                        .map(|(k, v)| format!("{}={}", k, v))
                        .collect::<Vec<_>>()
                        .join("&")
                );

                // Note: actual redirect following happens outside parse
                Some(redirect_url)
            },
        )
        .await
    }

    pub async fn resolve_set(
        &self,
        set_url: &str,
    ) -> Result<Option<crate::models::track::PlaylistData>, String> {
        self.request_with_retry(
            |client_id| {
                format!(
                    "{}/resolve?url={}&client_id={}",
                    SOUNDCLOUD_API,
                    urlencoding::encode(set_url),
                    client_id,
                )
            },
            |json| {
                let title = json
                    .get("title")
                    .and_then(|t| t.as_str())
                    .unwrap_or("SoundCloud Playlist")
                    .to_string();

                let tracks_val = json.get("tracks").and_then(|t| t.as_array())?;
                let tracks: Vec<LavalinkTrack> =
                    tracks_val.iter().filter_map(parse_track).collect();

                if tracks.is_empty() {
                    return None;
                }

                Some(crate::models::track::PlaylistData {
                    info: crate::models::track::PlaylistInfo {
                        name: title,
                        selected_track: 0,
                    },
                    plugin_info: serde_json::json!({}),
                    tracks,
                })
            },
        )
        .await
        .map(Some)
        .or(Ok(None))
    }
}

fn parse_track(item: &Value) -> Option<LavalinkTrack> {
    let id = item
        .get("id")
        .and_then(|i| i.as_u64())
        .map(|i| i.to_string())
        .unwrap_or_default();
    let title = item
        .get("title")
        .and_then(|t| t.as_str())
        .unwrap_or("Unknown Title")
        .to_string();
    let author = item
        .get("user")
        .and_then(|u| u.get("username"))
        .and_then(|un| un.as_str())
        .unwrap_or("Unknown Artist")
        .to_string();
    let duration_ms = item.get("duration").and_then(|d| d.as_u64()).unwrap_or(0);
    let artwork = item
        .get("artwork_url")
        .and_then(|a| a.as_str())
        .map(|s| s.replace("large", "t500x500"));
    let uri = item
        .get("permalink_url")
        .and_then(|u| u.as_str())
        .map(|s| s.to_string());

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

    Some(track)
}
