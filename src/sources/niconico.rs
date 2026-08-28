use crate::models::track::{LavalinkTrack, TrackInfo};
use crate::sources::backoff::{with_backoff, BackoffConfig};
use reqwest::Client;
use serde_json::Value;
use std::sync::Arc;

const NICONICO_API: &str = "https://ext.nicovideo.jp/api/getthumbinfo/";

pub struct NicoNicoSource {
    client: Client,
}

impl NicoNicoSource {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            client: Client::builder()
                .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36")
                .build()
                .unwrap_or_default(),
        })
    }

    pub async fn search(&self, query: &str, limit: usize) -> Result<Vec<LavalinkTrack>, String> {
        let url = format!(
            "https://snapshot.search.nicovideo.jp/api/v2/snapshot/video/contents/search?q={}&targets=title,tags&fields=contentId,title,userId,lengthSeconds,viewCounter,_score&filters[login]=no&filters[n_hindsen]=no&_sort=-viewCounter&_limit={}",
            urlencoding::encode(query),
            limit
        );

        let client = self.client.clone();
        let cfg = BackoffConfig::default();
        let json = with_backoff(&cfg, "niconico_search", || {
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
        if let Some(data) = json.get("data").and_then(|d| d.as_array()) {
            for item in data.iter().take(limit) {
                if let Some(track) = self.parse_video(item) {
                    tracks.push(track);
                }
            }
        }

        Ok(tracks)
    }

    pub async fn resolve_video(&self, video_id: &str) -> Result<Option<LavalinkTrack>, String> {
        let clean_id = video_id.trim_start_matches("sm").trim_start_matches("so");
        let api_url = format!("{}{}", NICONICO_API, clean_id);

        let client = self.client.clone();
        let cfg = BackoffConfig::default();
        let xml = with_backoff(&cfg, "niconico_resolve", || {
            let client = client.clone();
            let url = api_url.clone();
            async move {
                let res = client.get(&url).send().await.map_err(|e| e.to_string())?;
                let text = res.text().await.map_err(|e| e.to_string())?;
                Ok::<String, String>(text)
            }
        })
        .await?;

        let title = extract_xml_tag(&xml, "title").unwrap_or_else(|| "Unknown".to_string());
        let author = extract_xml_tag(&xml, "uploader").unwrap_or_else(|| "Unknown Artist".to_string());
        let length_str = extract_xml_tag(&xml, "length").unwrap_or_else(|| "0".to_string());
        let length_ms = length_str.parse::<u64>().map(|s| s * 1000).unwrap_or(0);
        let thumb_url = extract_xml_tag(&xml, "thumbnail_url");

        let stream_url = format!("https://nicovideo.jp/watch/{}", clean_id);

        let mut track = LavalinkTrack {
            encoded: String::new(),
            info: TrackInfo {
                identifier: clean_id.to_string(),
                is_seekable: true,
                author,
                length: length_ms,
                is_stream: false,
                position: 0,
                title,
                uri: Some(stream_url),
                artwork_url: thumb_url,
                isrc: None,
                source_name: "niconico".to_string(),
            },
            plugin_info: serde_json::json!({}),
            user_data: serde_json::json!({}),
        };

        if let Ok(enc) = crate::track_encoding::encode_track(&track) {
            track.encoded = enc;
        }

        Ok(Some(track))
    }

    fn parse_video(&self, item: &Value) -> Option<LavalinkTrack> {
        let id = item.get("contentId").and_then(|i| i.as_str())?.to_string();
        let title = item.get("title").and_then(|t| t.as_str()).unwrap_or("Unknown").to_string();
        let length_ms = item.get("lengthSeconds").and_then(|d| d.as_u64()).map(|d| d * 1000).unwrap_or(0);
        let uri = Some(format!("https://nicovideo.jp/watch/{}", id));
        let artwork = Some(format!("https://nicovideo.jp/watch/{}", id));

        let mut track = LavalinkTrack {
            encoded: String::new(),
            info: TrackInfo {
                identifier: id,
                is_seekable: true,
                author: "NicoNico".to_string(),
                length: length_ms,
                is_stream: false,
                position: 0,
                title,
                uri,
                artwork_url: artwork,
                isrc: None,
                source_name: "niconico".to_string(),
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

fn extract_xml_tag(xml: &str, tag: &str) -> Option<String> {
    let open = format!("<{}>", tag);
    let close = format!("</{}>", tag);
    let start = xml.find(&open)? + open.len();
    let end = xml[start..].find(&close)?;
    Some(xml[start..start + end].to_string())
}
