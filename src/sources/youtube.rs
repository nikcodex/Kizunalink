use crate::models::track::{LavalinkTrack, TrackInfo};
use crate::sources::route_planner::RoutePlanner;
use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, USER_AGENT};
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{info, warn};

const INNERTUBE_API_KEY: &str = "AIzaSyAO_FJ2SlqU8Q4STEHLGCilw_Y9_11qcW8";

/// InnerTube client configuration — each client has different capabilities.
/// YouTube may block one client but not another, so we try them in order.
struct InnerTubeClient {
    name: &'static str,
    client_version: &'static str,
    user_agent: &'static str,
    /// Client ID number sent in X-YouTube-Client-Name header
    client_id: i32,
}

const INNERTUBE_CLIENTS: &[InnerTubeClient] = &[
    InnerTubeClient {
        name: "WEB",
        client_version: "2.20241126.01.00",
        user_agent: "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36",
        client_id: 1,
    },
    InnerTubeClient {
        name: "ANDROID_VR",
        client_version: "1.61.48",
        user_agent: "Mozilla/5.0 (Linux; Android 12; Quest 3) AppleWebKit/537.36 (KHTML, like Gecko) OculusBrowser/34.0.0.49 Chrome/122.0.6261.64 VR Safari/537.36",
        client_id: 28,
    },
    InnerTubeClient {
        name: "MWEB",
        client_version: "2.20241126.01.00",
        user_agent: "Mozilla/5.0 (Linux; Android 12; Pixel 6) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Mobile Safari/537.36",
        client_id: 2,
    },
    InnerTubeClient {
        name: "TVHTML5_SIMPLY",
        client_version: "2.0",
        user_agent: "Mozilla/5.0 (ChromiumStylePlatform) Cobalt/Version",
        client_id: 85,
    },
    InnerTubeClient {
        name: "IOS",
        client_version: "19.45.4",
        user_agent: "com.google.ios.youtube/19.45.4 (iPhone16,2; U; CPU iOS 18_1_0 like Mac OS X;)",
        client_id: 5,
    },
];

pub struct YouTubeSource {
    default_client: reqwest::Client,
    route_planner: Option<Arc<RoutePlanner>>,
    /// PO token for bot detection bypass
    po_token: RwLock<Option<String>>,
    /// OAuth2 token for authenticated requests
    oauth_token: RwLock<Option<String>>,
}

impl YouTubeSource {
    pub fn new(route_planner: Option<Arc<RoutePlanner>>) -> Arc<Self> {
        let mut headers = HeaderMap::new();
        headers.insert(ACCEPT, HeaderValue::from_static("application/json"));

        let default_client = crate::config::global_proxy()
            .apply_to_builder(reqwest::Client::builder())
            .default_headers(headers)
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .expect("Failed to build Reqwest client for YouTube");

        let po_token = std::env::var("YOUTUBE_PO_TOKEN").ok();
        let oauth_token = std::env::var("YOUTUBE_OAUTH_TOKEN").ok();

        if po_token.is_some() {
            info!("YouTube: PO token loaded from environment");
        }
        if oauth_token.is_some() {
            info!("YouTube: OAuth2 token loaded from environment");
        }

        Arc::new(Self {
            default_client,
            route_planner,
            po_token: RwLock::new(po_token),
            oauth_token: RwLock::new(oauth_token),
        })
    }

    fn get_http_client(&self) -> (reqwest::Client, Option<std::net::IpAddr>) {
        if let Some(rp) = &self.route_planner {
            rp.get_client()
        } else {
            (self.default_client.clone(), None)
        }
    }

    /// Try InnerTube clients in order until one succeeds
    async fn innertube_request(&self, endpoint: &str, payload: &Value) -> Result<Value, String> {
        let (client, bound_ip) = self.get_http_client();

        for it_client in INNERTUBE_CLIENTS {
            let url = format!(
                "https://www.youtube.com/youtubei/v1/{}?key={}",
                endpoint, INNERTUBE_API_KEY
            );

            let mut req = client
                .post(&url)
                .header(USER_AGENT, it_client.user_agent)
                .header(
                    "X-YouTube-Client-Name",
                    it_client.client_id.to_string(),
                )
                .header(
                    "X-YouTube-Client-Version",
                    it_client.client_version,
                )
                .header(ACCEPT, "application/json")
                .json(payload);

            if let Some(oauth) = self.oauth_token.read().await.as_ref() {
                req = req.bearer_auth(oauth);
            }

            match req.send().await {
                Ok(res) => {
                    let status = res.status();
                    if status == 429 || status == 403 {
                        warn!(
                            "YouTube client {} returned {} — trying next client",
                            it_client.name, status
                        );
                        if let (Some(rp), Some(ip)) = (&self.route_planner, bound_ip) {
                            rp.mark_failed(ip);
                        }
                        continue;
                    }

                    let json: Value = res.json().await.map_err(|e| e.to_string())?;

                    // Check for playability errors
                    if let Some(playability) = json.get("playabilityStatus") {
                        if let Some(status) = playability.get("status") {
                            if status.as_str() == Some("ERROR")
                                || status.as_str() == Some("UNPLAYABLE")
                            {
                                let reason = playability
                                    .get("reason")
                                    .and_then(|r| r.as_str())
                                    .unwrap_or("Unknown error");
                                warn!(
                                    "YouTube client {} returned playability error: {} — trying next",
                                    it_client.name, reason
                                );
                                continue;
                            }
                        }
                    }

                    return Ok(json);
                }
                Err(e) => {
                    warn!(
                        "YouTube client {} request failed: {} — trying next",
                        it_client.name, e
                    );
                    if let (Some(rp), Some(ip)) = (&self.route_planner, bound_ip) {
                        rp.mark_failed(ip);
                    }
                }
            }
        }

        Err("All YouTube InnerTube clients failed".to_string())
    }

    /// Resolve a YouTube URL or video ID using InnerTube API with multi-client fallback
    pub async fn resolve_video(&self, video_id: &str) -> Result<Option<LavalinkTrack>, String> {
        let mut payload = serde_json::json!({
            "context": {
                "client": {
                    "clientName": "WEB",
                    "clientVersion": "2.20241126.01.00",
                    "hl": "en",
                    "gl": "US",
                }
            },
            "videoId": video_id,
        });

        // Add PO token if available
        if let Some(po) = self.po_token.read().await.as_ref() {
            if let Some(obj) = payload.as_object_mut() {
                obj.insert(
                    "serviceIntegrityDimensions".to_string(),
                    serde_json::json!({ "poToken": po }),
                );
            }
        }

        let json = self.innertube_request("player", &payload).await?;

        let details = json
            .get("videoDetails")
            .ok_or("No videoDetails in response")?;

        let title = details
            .get("title")
            .and_then(|t| t.as_str())
            .unwrap_or("Unknown Title")
            .to_string();
        let author = details
            .get("author")
            .and_then(|a| a.as_str())
            .unwrap_or("Unknown Artist")
            .to_string();
        let length_sec: u64 = details
            .get("lengthSeconds")
            .and_then(|l| l.as_str())
            .and_then(|l| l.parse().ok())
            .unwrap_or(0);
        let artwork = details
            .get("thumbnail")
            .and_then(|t| t.get("thumbnails"))
            .and_then(|t| t.as_array())
            .and_then(|t| t.last())
            .and_then(|t| t.get("url"))
            .and_then(|u| u.as_str())
            .map(|s| s.to_string());

        // Find best audio stream (Opus preferred, then any audio)
        let audio_url = json
            .get("streamingData")
            .and_then(|s| s.get("adaptiveFormats"))
            .and_then(|f| f.as_array())
            .and_then(|formats| {
                // Prefer Opus audio
                formats
                    .iter()
                    .find(|f| {
                        f.get("mimeType")
                            .and_then(|m| m.as_str())
                            .map(|m| m.contains("audio/webm") && m.contains("opus"))
                            .unwrap_or(false)
                    })
                    .or_else(|| {
                        formats.iter().find(|f| {
                            f.get("mimeType")
                                .and_then(|m| m.as_str())
                                .map(|m| m.contains("audio/"))
                                .unwrap_or(false)
                        })
                    })
                    .and_then(|f| f.get("url").and_then(|u| u.as_str()))
                    .map(|s| s.to_string())
            });

        let mut track = LavalinkTrack {
            encoded: String::new(),
            info: TrackInfo {
                identifier: video_id.to_string(),
                is_seekable: true,
                author,
                length: length_sec * 1000,
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

        Ok(Some(track))
    }

    /// Resolve direct playable audio stream URL for a YouTube video ID.
    pub async fn resolve_stream_url(&self, video_id: &str) -> Result<String, String> {
        let mut payload = serde_json::json!({
            "context": {
                "client": {
                    "clientName": "WEB",
                    "clientVersion": "2.20241126.01.00",
                    "hl": "en",
                    "gl": "US",
                }
            },
            "videoId": video_id,
        });

        if let Some(po) = self.po_token.read().await.as_ref() {
            if let Some(obj) = payload.as_object_mut() {
                obj.insert(
                    "serviceIntegrityDimensions".to_string(),
                    serde_json::json!({ "poToken": po }),
                );
            }
        }

        let json = self.innertube_request("player", &payload).await?;
        let audio_url = json
            .get("streamingData")
            .and_then(|s| s.get("adaptiveFormats"))
            .and_then(|f| f.as_array())
            .and_then(|formats| {
                formats
                    .iter()
                    .find(|f| {
                        f.get("mimeType")
                            .and_then(|m| m.as_str())
                            .map(|m| m.contains("audio/webm") && m.contains("opus"))
                            .unwrap_or(false)
                    })
                    .or_else(|| {
                        formats.iter().find(|f| {
                            f.get("mimeType")
                                .and_then(|m| m.as_str())
                                .map(|m| m.contains("audio/"))
                                .unwrap_or(false)
                        })
                    })
                    .and_then(|f| f.get("url").and_then(|u| u.as_str()))
                    .map(|s| s.to_string())
            });

        audio_url.ok_or_else(|| "No direct audio stream URL found in YouTube player response".to_string())
    }

    /// Search YouTube using InnerTube with multi-client fallback
    pub async fn search(&self, query: &str, limit: usize) -> Result<Vec<LavalinkTrack>, String> {
        let clean_query = query.trim();
        if clean_query.is_empty() {
            return Ok(vec![]);
        }

        let mut payload = serde_json::json!({
            "context": {
                "client": {
                    "clientName": "WEB",
                    "clientVersion": "2.20241126.01.00",
                    "hl": "en",
                    "gl": "US",
                }
            },
            "query": clean_query,
            "params": "EgIQAQ==",
        });

        if let Some(po) = self.po_token.read().await.as_ref() {
            if let Some(obj) = payload.as_object_mut() {
                obj.insert(
                    "serviceIntegrityDimensions".to_string(),
                    serde_json::json!({ "poToken": po }),
                );
            }
        }

        let json = self.innertube_request("search", &payload).await?;

        let mut tracks = Vec::new();
        if let Some(contents) = json
            .get("contents")
            .and_then(|c| c.get("twoColumnSearchResultsRenderer"))
            .and_then(|c| c.get("primaryContents"))
            .and_then(|c| c.get("sectionListRenderer"))
            .and_then(|c| c.get("contents"))
            .and_then(|c| c.as_array())
        {
            for section in contents {
                if let Some(items) = section
                    .get("itemSectionRenderer")
                    .and_then(|i| i.get("contents"))
                    .and_then(|i| i.as_array())
                {
                    for item in items {
                        if let Some(video) = item.get("videoRenderer") {
                            if let Some(track) = self.parse_video_renderer(video) {
                                tracks.push(track);
                                if tracks.len() >= limit {
                                    break;
                                }
                            }
                        }
                    }
                }
            }
        }

        Ok(tracks)
    }

    fn parse_video_renderer(&self, video: &Value) -> Option<LavalinkTrack> {
        let video_id = video.get("videoId").and_then(|v| v.as_str())?;

        let title = video
            .get("title")
            .and_then(|t| t.get("runs"))
            .and_then(|r| r.as_array())
            .and_then(|a| a.first())
            .and_then(|r| r.get("text"))
            .and_then(|t| t.as_str())
            .unwrap_or("Unknown Title")
            .to_string();

        let author = video
            .get("ownerText")
            .and_then(|o| o.get("runs"))
            .and_then(|r| r.as_array())
            .and_then(|a| a.first())
            .and_then(|r| r.get("text"))
            .and_then(|t| t.as_str())
            .unwrap_or("Unknown Artist")
            .to_string();

        let length_text = video
            .get("lengthText")
            .and_then(|l| l.get("simpleText"))
            .and_then(|l| l.as_str())
            .unwrap_or("0:00");

        let duration_ms = parse_duration_to_ms(length_text);

        let artwork = video
            .get("thumbnail")
            .and_then(|t| t.get("thumbnails"))
            .and_then(|t| t.as_array())
            .and_then(|t| t.last())
            .and_then(|t| t.get("url"))
            .and_then(|u| u.as_str())
            .map(|s| {
                if s.starts_with("//") {
                    format!("https:{}", s)
                } else {
                    s.to_string()
                }
            });

        let mut track = LavalinkTrack {
            encoded: String::new(),
            info: TrackInfo {
                identifier: video_id.to_string(),
                is_seekable: true,
                author,
                length: duration_ms,
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

    pub async fn resolve_playlist(
        &self,
        playlist_id: &str,
    ) -> Result<Option<crate::models::track::PlaylistData>, String> {
        let browse_id = if playlist_id.starts_with("VL") {
            playlist_id.to_string()
        } else {
            format!("VL{}", playlist_id)
        };

        let mut payload = serde_json::json!({
            "context": {
                "client": {
                    "clientName": "WEB",
                    "clientVersion": "2.20241126.01.00",
                    "hl": "en",
                    "gl": "US"
                }
            },
            "browseId": browse_id,
        });

        if let Some(po) = self.po_token.read().await.as_ref() {
            if let Some(obj) = payload.as_object_mut() {
                obj.insert(
                    "serviceIntegrityDimensions".to_string(),
                    serde_json::json!({ "poToken": po }),
                );
            }
        }

        let json = self.innertube_request("browse", &payload).await?;

        let playlist_name = json
            .get("header")
            .and_then(|h| h.get("playlistHeaderRenderer"))
            .and_then(|p| p.get("title"))
            .and_then(|t| {
                t.get("simpleText").and_then(|s| s.as_str()).or_else(|| {
                    t.get("runs")
                        .and_then(|r| r.as_array())
                        .and_then(|a| a.first())
                        .and_then(|r| r.get("text"))
                        .and_then(|s| s.as_str())
                })
            })
            .unwrap_or("YouTube Playlist")
            .to_string();

        let mut tracks = Vec::new();

        if let Some(tabs) = json
            .get("contents")
            .and_then(|c| c.get("twoColumnBrowseResultsRenderer"))
            .and_then(|c| c.get("tabs"))
            .and_then(|t| t.as_array())
        {
            for tab in tabs {
                if let Some(sections) = tab
                    .get("tabRenderer")
                    .and_then(|tr| tr.get("content"))
                    .and_then(|c| c.get("sectionListRenderer"))
                    .and_then(|sl| sl.get("contents"))
                    .and_then(|c| c.as_array())
                {
                    for section in sections {
                        if let Some(items) = section
                            .get("itemSectionRenderer")
                            .and_then(|isr| isr.get("contents"))
                            .and_then(|c| c.as_array())
                        {
                            for item in items {
                                if let Some(video_list) = item
                                    .get("playlistVideoListRenderer")
                                    .and_then(|pvl| pvl.get("contents"))
                                    .and_then(|c| c.as_array())
                                {
                                    for v_item in video_list {
                                        if let Some(video) = v_item.get("playlistVideoRenderer") {
                                            if let Some(track) =
                                                self.parse_playlist_video_renderer(video)
                                            {
                                                tracks.push(track);
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        if tracks.is_empty() {
            return Ok(None);
        }

        Ok(Some(crate::models::track::PlaylistData {
            info: crate::models::track::PlaylistInfo {
                name: playlist_name,
                selected_track: 0,
            },
            plugin_info: serde_json::Value::Null,
            tracks,
        }))
    }

    fn parse_playlist_video_renderer(&self, video: &Value) -> Option<LavalinkTrack> {
        let video_id = video.get("videoId").and_then(|v| v.as_str())?;
        let title = video
            .get("title")
            .and_then(|t| {
                t.get("simpleText").and_then(|s| s.as_str()).or_else(|| {
                    t.get("runs")
                        .and_then(|r| r.as_array())
                        .and_then(|a| a.first())
                        .and_then(|r| r.get("text"))
                        .and_then(|s| s.as_str())
                })
            })
            .unwrap_or("Unknown Title")
            .to_string();

        let author = video
            .get("shortBylineText")
            .and_then(|o| o.get("runs"))
            .and_then(|r| r.as_array())
            .and_then(|a| a.first())
            .and_then(|r| r.get("text"))
            .and_then(|t| t.as_str())
            .unwrap_or("Unknown Artist")
            .to_string();

        let length_sec: u64 = video
            .get("lengthSeconds")
            .and_then(|l| l.as_str())
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);

        let artwork = video
            .get("thumbnail")
            .and_then(|t| t.get("thumbnails"))
            .and_then(|t| t.as_array())
            .and_then(|t| t.last())
            .and_then(|t| t.get("url"))
            .and_then(|u| u.as_str())
            .map(|s| {
                if s.starts_with("//") {
                    format!("https:{}", s)
                } else {
                    s.to_string()
                }
            });

        let mut track = LavalinkTrack {
            encoded: String::new(),
            info: TrackInfo {
                identifier: video_id.to_string(),
                is_seekable: true,
                author,
                length: length_sec * 1000,
                is_stream: false,
                position: 0,
                title,
                uri: Some(format!("https://www.youtube.com/watch?v={}", video_id)),
                artwork_url: artwork,
                isrc: None,
                source_name: "youtube".to_string(),
            },
            plugin_info: serde_json::Value::Null,
            user_data: serde_json::Value::Null,
        };

        if let Ok(enc) = crate::track_encoding::encode_track(&track) {
            track.encoded = enc;
        }

        Some(track)
    }
}

/// Parse "M:SS" or "MM:SS" or "HH:MM:SS" to milliseconds
fn parse_duration_to_ms(duration: &str) -> u64 {
    let parts: Vec<&str> = duration.split(':').collect();
    match parts.len() {
        3 => {
            let h: u64 = parts[0].parse().unwrap_or(0);
            let m: u64 = parts[1].parse().unwrap_or(0);
            let s: u64 = parts[2].parse().unwrap_or(0);
            (h * 3600 + m * 60 + s) * 1000
        }
        2 => {
            let m: u64 = parts[0].parse().unwrap_or(0);
            let s: u64 = parts[1].parse().unwrap_or(0);
            (m * 60 + s) * 1000
        }
        _ => 0,
    }
}
