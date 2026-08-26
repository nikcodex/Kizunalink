use crate::models::track::{LavalinkTrack, TrackInfo};
use crate::sources::route_planner::RoutePlanner;
use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, USER_AGENT};
use serde_json::Value;
use std::sync::Arc;
use tracing::warn;

const PIPED_API_INSTANCES: &[&str] = &[
    "https://pipedapi.kavin.rocks",
    "https://api.piped.privacydev.net",
    "https://piped-api.lunar.icu",
    "https://cf.piped.video",
    "https://piped.video",
];

pub struct YouTubeSource {
    default_client: reqwest::Client,
    route_planner: Option<Arc<RoutePlanner>>,
}

impl YouTubeSource {
    pub fn new(route_planner: Option<Arc<RoutePlanner>>) -> Arc<Self> {
        let mut headers = HeaderMap::new();
        headers.insert(
            USER_AGENT,
            HeaderValue::from_static(
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36",
            ),
        );
        headers.insert(ACCEPT, HeaderValue::from_static("application/json"));

        let default_client = reqwest::Client::builder()
            .default_headers(headers)
            .timeout(std::time::Duration::from_secs(5))
            .build()
            .expect("Failed to build Reqwest client for YouTube");

        Arc::new(Self {
            default_client,
            route_planner,
        })
    }

    fn get_http_client(&self) -> (reqwest::Client, Option<std::net::IpAddr>) {
        if let Some(rp) = &self.route_planner {
            rp.get_client()
        } else {
            (self.default_client.clone(), None)
        }
    }

    pub async fn search(&self, query: &str, limit: usize) -> Result<Vec<LavalinkTrack>, String> {
        let clean_query = query.trim();
        if clean_query.is_empty() {
            return Ok(vec![]);
        }

        for instance in PIPED_API_INSTANCES {
            let (client, bound_ip) = self.get_http_client();
            let url = format!(
                "{}/search?q={}&filter=videos",
                instance,
                urlencoding::encode(clean_query)
            );

            match client.get(&url).send().await {
                Ok(res) => {
                    let status = res.status();
                    if status.as_u16() == 429 || status.as_u16() == 403 {
                        if let (Some(rp), Some(ip)) = (&self.route_planner, bound_ip) {
                            warn!("YouTube instance {} returned {} on {}. Rotating IP...", instance, status, ip);
                            rp.mark_failed(ip);
                        }
                        continue;
                    }

                    if let Ok(json) = res.json::<Value>().await {
                        if let Some(items) = json.get("items").and_then(|i| i.as_array()) {
                            let mut tracks = Vec::new();
                            for item in items.iter().take(limit) {
                                if let Some(track) = self.parse_item(item) {
                                    tracks.push(track);
                                }
                            }
                            if !tracks.is_empty() {
                                return Ok(tracks);
                            }
                        }
                    }
                }
                Err(e) => {
                    if let (Some(rp), Some(ip)) = (&self.route_planner, bound_ip) {
                        warn!("YouTube connection error on {}: {}. Marking failed...", ip, e);
                        rp.mark_failed(ip);
                    }
                }
            }
        }

        Ok(vec![])
    }

    pub async fn resolve_video(&self, video_id: &str) -> Result<Option<LavalinkTrack>, String> {
        for instance in PIPED_API_INSTANCES {
            let (client, bound_ip) = self.get_http_client();
            let url = format!("{}/streams/{}", instance, video_id);

            match client.get(&url).send().await {
                Ok(res) => {
                    let status = res.status();
                    if status.as_u16() == 429 || status.as_u16() == 403 {
                        if let (Some(rp), Some(ip)) = (&self.route_planner, bound_ip) {
                            warn!("YouTube stream resolve returned {} on {}. Rotating IP...", status, ip);
                            rp.mark_failed(ip);
                        }
                        continue;
                    }

                    if let Ok(json) = res.json::<Value>().await {
                        let title = json
                            .get("title")
                            .and_then(|t| t.as_str())
                            .unwrap_or("Unknown Title")
                            .to_string();
                        let author = json
                            .get("uploader")
                            .and_then(|u| u.as_str())
                            .unwrap_or("Unknown Artist")
                            .to_string();
                        let duration_sec = json.get("duration").and_then(|d| d.as_u64()).unwrap_or(0);
                        let artwork = json
                            .get("thumbnailUrl")
                            .and_then(|t| t.as_str())
                            .map(|s| s.to_string());

                        let audio_stream = json
                            .get("audioStreams")
                            .and_then(|s| s.as_array())
                            .and_then(|arr| arr.first())
                            .and_then(|stream| stream.get("url"))
                            .and_then(|u| u.as_str())
                            .map(|s| s.to_string());

                        let mut track = LavalinkTrack {
                            encoded: String::new(),
                            info: TrackInfo {
                                identifier: video_id.to_string(),
                                is_seekable: true,
                                author,
                                length: duration_sec * 1000,
                                is_stream: false,
                                position: 0,
                                title,
                                uri: audio_stream.or_else(|| {
                                    Some(format!("https://www.youtube.com/watch?v={}", video_id))
                                }),
                                artwork_url: artwork,
                                isrc: None,
                                source_name: "youtube".to_string(),
                            },
                            plugin_info: Value::Object(Default::default()),
                            user_data: Value::Object(Default::default()),
                        };

                        if let Ok(enc) = crate::track_encoding::encode_track(&track) {
                            track.encoded = enc;
                        }

                        return Ok(Some(track));
                    }
                }
                Err(e) => {
                    if let (Some(rp), Some(ip)) = (&self.route_planner, bound_ip) {
                        rp.mark_failed(ip);
                    }
                }
            }
        }

        Ok(None)
    }

    fn parse_item(&self, item: &Value) -> Option<LavalinkTrack> {
        let url_path = item.get("url").and_then(|u| u.as_str())?;
        let video_id = url_path
            .strip_prefix("/watch?v=")
            .unwrap_or(url_path)
            .to_string();

        let title = item
            .get("title")
            .and_then(|t| t.as_str())
            .unwrap_or("Unknown Title")
            .to_string();
        let author = item
            .get("uploaderName")
            .and_then(|u| u.as_str())
            .unwrap_or("Unknown Artist")
            .to_string();
        let duration_sec = item.get("duration").and_then(|d| d.as_u64()).unwrap_or(0);
        let artwork = item
            .get("thumbnail")
            .and_then(|t| t.as_str())
            .map(|s| s.to_string());

        let mut track = LavalinkTrack {
            encoded: String::new(),
            info: TrackInfo {
                identifier: video_id.clone(),
                is_seekable: true,
                author,
                length: duration_sec * 1000,
                is_stream: false,
                position: 0,
                title,
                uri: Some(format!("https://www.youtube.com/watch?v={}", video_id)),
                artwork_url: artwork,
                isrc: None,
                source_name: "youtube".to_string(),
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
