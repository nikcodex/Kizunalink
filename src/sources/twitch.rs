use crate::models::track::{LavalinkTrack, TrackInfo};
use crate::sources::backoff::{with_backoff, BackoffConfig};
use reqwest::Client;
use serde_json::Value;
use std::sync::Arc;
use tracing::warn;

const TWITCH_GQL_URL: &str = "https://gql.twitch.tv/gql";

pub struct TwitchSource {
    client: Client,
}

impl TwitchSource {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            client: crate::config::global_proxy()
                .apply_to_builder(Client::builder())
                .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
                .build()
                .unwrap_or_default(),
        })
    }

    pub async fn resolve_stream(&self, channel: &str) -> Result<Option<String>, String> {
        let client = self.client.clone();
        let channel = channel.to_string();
        let cfg = BackoffConfig::default();

        let body = serde_json::json!({
            "operationName": "PlayerAccessTokenSp",
            "extensions": {
                "persistedQuery": {
                    "version": 1,
                    "sha256Hash": "06e644980b2827a180cbffc048c24e881ed3380f5685e82f2b58ddab3f20e2fc"
                }
            },
            "variables": {
                "isLive": true,
                "login": channel,
                "isVod": false,
                "vodID": "",
                "playerType": "site"
            }
        });

        let res = with_backoff(&cfg, "twitch_resolve", || {
            let client = client.clone();
            let body = body.clone();
            async move {
                let res = client
                    .post(TWITCH_GQL_URL)
                    .header("Client-ID", "kimne78kx3ncx6brgo4mv6wki5h1ko")
                    .header("Content-Type", "application/json")
                    .json(&body)
                    .send()
                    .await
                    .map_err(|e| e.to_string())?;
                let text = res.text().await.map_err(|e| e.to_string())?;
                Ok::<String, String>(text)
            }
        })
        .await?;

        let json: Value = serde_json::from_str(&res).map_err(|e| e.to_string())?;

        let access_token = json
            .get("data")
            .and_then(|d| d.get("streamPlaybackAccessToken"))
            .and_then(|t| t.get("value"))
            .and_then(|v| v.as_str());

        let sig = json
            .get("data")
            .and_then(|d| d.get("streamPlaybackAccessToken"))
            .and_then(|t| t.get("signature"))
            .and_then(|s| s.as_str());

        match (access_token, sig) {
            (Some(token), Some(signature)) => {
                let url = format!(
                    "https://usher.ttvnw.net/api/channel/hls/{}.m3u8?client_id=kimne78kx3ncx6brgo4mv6wki5h1ko&token={}&sig={}&allow_source=true&allow_audio_only=true",
                    urlencoding::encode(&channel),
                    urlencoding::encode(token),
                    urlencoding::encode(signature),
                );
                Ok(Some(url))
            }
            _ => {
                warn!("Twitch: Failed to get access token for channel {}", channel);
                Ok(None)
            }
        }
    }

    pub async fn search(&self, query: &str, _limit: usize) -> Result<Vec<LavalinkTrack>, String> {
        let channel = query.trim().to_lowercase();

        if let Some(stream_url) = self.resolve_stream(&channel).await? {
            let mut track = LavalinkTrack {
                encoded: String::new(),
                info: TrackInfo {
                    identifier: channel.clone(),
                    is_seekable: false,
                    author: channel.clone(),
                    length: 0,
                    is_stream: true,
                    position: 0,
                    title: format!("Twitch: {}", channel),
                    uri: Some(format!("https://twitch.tv/{}", channel)),
                    artwork_url: None,
                    isrc: None,
                    source_name: "twitch".to_string(),
                },
                plugin_info: serde_json::json!({ "streamUrl": stream_url }),
                user_data: serde_json::json!({}),
            };

            if let Ok(enc) = crate::track_encoding::encode_track(&track) {
                track.encoded = enc;
            }
            return Ok(vec![track]);
        }

        Ok(vec![])
    }
}
