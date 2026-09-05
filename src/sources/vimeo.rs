use crate::models::track::{LavalinkTrack, TrackInfo};
use crate::sources::backoff::{with_backoff, BackoffConfig};
use reqwest::Client;
use serde_json::Value;
use std::sync::Arc;

pub struct VimeoSource {
    client: Client,
}

impl VimeoSource {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            // Every other source caps its HTTP client; without a timeout a hung
            // upstream keeps the request (and its task) alive indefinitely.
            client: crate::config::global_proxy()
                .apply_to_builder(Client::builder())
                .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
                .timeout(crate::config::source_timeout_secs(10))
                .build()
                .unwrap_or_default(),
        })
    }

    pub async fn search(&self, query: &str, limit: usize) -> Result<Vec<LavalinkTrack>, String> {
        let url = format!(
            "https://vimeo.com/api/rest/v2.0?method=vimeo.videos.search&query={}&per_page={}&format=json",
            urlencoding::encode(query),
            limit
        );

        let client = self.client.clone();
        let cfg = BackoffConfig::default();
        let json = with_backoff(&cfg, "vimeo_search", || {
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
        if let Some(videos) = json
            .get("videos")
            .and_then(|v| v.get("video"))
            .and_then(|v| v.as_array())
        {
            for video in videos.iter().take(limit) {
                if let Some(track) = self.parse_video(video) {
                    tracks.push(track);
                }
            }
        }

        Ok(tracks)
    }

    pub async fn resolve_video(&self, video_id: &str) -> Result<Option<LavalinkTrack>, String> {
        let url = format!(
            "https://vimeo.com/api/oembed.json?url=https://vimeo.com/{}",
            video_id
        );
        let client = self.client.clone();
        let cfg = BackoffConfig::default();

        let json = with_backoff(&cfg, "vimeo_oembed", || {
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

        let title = json
            .get("title")
            .and_then(|t| t.as_str())
            .unwrap_or("Unknown")
            .to_string();
        let author_name = json
            .get("author_name")
            .and_then(|a| a.as_str())
            .unwrap_or("Unknown Artist")
            .to_string();
        let duration = json
            .get("duration")
            .and_then(|d| d.as_u64())
            .map(|d| d * 1000)
            .unwrap_or(0);
        let thumbnail = json
            .get("thumbnail_url")
            .and_then(|t| t.as_str())
            .map(|s| s.to_string());

        let page_url = format!("https://vimeo.com/{}", video_id);
        let page_client = self.client.clone();
        let page_html = with_backoff(&cfg, "vimeo_page", || {
            let client = page_client.clone();
            let url = page_url.clone();
            async move {
                let res = client.get(&url).send().await.map_err(|e| e.to_string())?;
                let text = res.text().await.map_err(|e| e.to_string())?;
                Ok::<String, String>(text)
            }
        })
        .await?;

        let mut stream_url = None;

        if let Some(config_start) = page_html.find("\"progressive\":[") {
            let search_area = &page_html[config_start..];
            if let Some(url_start) = search_area.find("\"url\":\"") {
                let url_area = &search_area[url_start + 7..];
                if let Some(url_end) = url_area.find('"') {
                    let raw_url = &url_area[..url_end];
                    let unescaped = raw_url.replace("\\u0026", "&").replace("\\/", "/");
                    stream_url = Some(unescaped);
                }
            }
        }

        let mut track = LavalinkTrack {
            encoded: String::new(),
            info: TrackInfo {
                identifier: video_id.to_string(),
                is_seekable: true,
                author: author_name,
                length: duration,
                is_stream: false,
                position: 0,
                title,
                uri: Some(format!("https://vimeo.com/{}", video_id)),
                artwork_url: thumbnail,
                isrc: None,
                source_name: "vimeo".to_string(),
            },
            plugin_info: match stream_url {
                Some(ref u) => serde_json::json!({ "streamUrl": u }),
                None => serde_json::json!({}),
            },
            user_data: serde_json::json!({}),
        };

        if let Ok(enc) = crate::track_encoding::encode_track(&track) {
            track.encoded = enc;
        }

        Ok(Some(track))
    }

    fn parse_video(&self, video: &Value) -> Option<LavalinkTrack> {
        let id = video.get("id").and_then(|i| i.as_u64())?.to_string();
        let title = video
            .get("title")
            .and_then(|t| t.as_str())
            .unwrap_or("Unknown")
            .to_string();
        let author = video
            .get("user")
            .and_then(|u| u.get("display_name"))
            .and_then(|n| n.as_str())
            .unwrap_or("Unknown Artist")
            .to_string();
        let duration = video
            .get("duration")
            .and_then(|d| d.as_u64())
            .map(|d| d * 1000)
            .unwrap_or(0);
        let thumbnail = video
            .get("thumbs")
            .and_then(|t| t.get("large"))
            .and_then(|l| l.as_str())
            .map(|s| s.to_string());
        let uri = Some(format!("https://vimeo.com/{}", id));

        let mut track = LavalinkTrack {
            encoded: String::new(),
            info: TrackInfo {
                identifier: id,
                is_seekable: true,
                author,
                length: duration,
                is_stream: false,
                position: 0,
                title,
                uri,
                artwork_url: thumbnail,
                isrc: None,
                source_name: "vimeo".to_string(),
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
