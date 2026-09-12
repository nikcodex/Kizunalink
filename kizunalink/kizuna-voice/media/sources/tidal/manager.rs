// Copyright (c) 2026 nikcodex (KizunaLink)
// Licensed under the MIT License

use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose};
use regex::Regex;
use serde_json::Value;
use tracing::{debug, warn};

use super::{
    client::HifiClient,
    model::{Manifest, PlaybackData},
    track::TidalTrack,
};
use crate::{
    common::types::AudioFormat,
    lavalink::protocol::tracks::{LoadResult, PlaylistData, PlaylistInfo, Track, TrackInfo},
    media::sources::SourcePlugin,
};

fn url_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"https?://(?:(?:listen|www)\.)?tidal\.com/(?:browse/)?(album|track|playlist|mix|artist)/([a-zA-Z0-9\-]+)(?:/.*)?(?:\?.*)?").unwrap()
    })
}

fn default_quality_order() -> Vec<String> {
    vec!["LOSSLESS".to_string(), "HIGH".to_string(), "LOW".to_string()]
}

fn audio_format(url: &str, mime_type: Option<&str>, quality: &str) -> AudioFormat {
    if let Some(m) = mime_type {
        if m.contains("flac") {
            return AudioFormat::Flac;
        }
        if m.contains("mp4") || m.contains("aac") {
            return AudioFormat::Mp4;
        }
    }
    let by_url = AudioFormat::from_url(url);
    if by_url != AudioFormat::Unknown {
        return by_url;
    }
    match quality {
        "HI_RES_LOSSLESS" | "HI_RES" | "LOSSLESS" => AudioFormat::Flac,
        _ => AudioFormat::Aac,
    }
}

pub struct TidalSource {
    pub client: Arc<HifiClient>,
    playlist_load_limit: usize,
    album_load_limit: usize,
    artist_load_limit: usize,
}

impl TidalSource {
    pub fn new(
        config: Option<crate::config::TidalConfig>,
        http_client: Arc<reqwest::Client>,
    ) -> Result<Self, String> {
        let (country, quality_order, hifi_apis, p_limit, a_limit, art_limit) =
            if let Some(c) = config {
                let quality_order = if c.hifi_qualities.is_empty() {
                    default_quality_order()
                } else {
                    c.hifi_qualities
                };
                (c.country_code, quality_order, c.hifi_apis, c.playlist_load_limit, c.album_load_limit, c.artist_load_limit)
            } else {
                ("US".to_string(), default_quality_order(), vec![], 0, 0, 0)
            };

        let client = Arc::new(HifiClient::new(http_client, hifi_apis, quality_order, country)?);

        Ok(Self {
            client,
            playlist_load_limit: p_limit,
            album_load_limit: a_limit,
            artist_load_limit: art_limit,
        })
    }

    fn parse_track(&self, item: &Value) -> Option<TrackInfo> {
        let id = item.get("id")?.as_u64()?.to_string();
        let title = item
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown Title")
            .to_string();

        let author = item
            .get("artists")
            .and_then(|v| v.as_array())
            .filter(|a| !a.is_empty())
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.get("name").and_then(|n| n.as_str()))
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .or_else(|| {
                item.get("artist")
                    .and_then(|a| a.get("name"))
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_owned())
            })
            .unwrap_or_else(|| "Unknown Artist".to_owned());

        let length = item.get("duration").and_then(|v| v.as_u64()).unwrap_or(0) * 1000;

        let isrc = item
            .get("isrc")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_owned());

        let artwork_url = item
            .get("album")
            .and_then(|a| a.get("cover"))
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| {
                format!(
                    "https://resources.tidal.com/images/{}/1280x1280.jpg",
                    s.replace('-', "/")
                )
            });

        let uri = item
            .get("url")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.replace("http://", "https://"));

        Some(TrackInfo {
            title,
            author,
            length,
            identifier: id,
            is_stream: false,
            uri,
            artwork_url,
            isrc,
            source_name: "tidal".to_owned(),
            is_seekable: true,
            position: 0,
        })
    }

    async fn get_track_data(&self, id: &str) -> LoadResult {
        match self.client.get("/info/", &[("id", id)]).await {
            Ok(data) => data
                .get("data")
                .and_then(|d| self.parse_track(d))
                .map(|i| LoadResult::Track(Track::new(i)))
                .unwrap_or(LoadResult::Empty {}),
            Err(_) => LoadResult::Empty {},
        }
    }

    async fn get_album(&self, id: &str) -> LoadResult {
        let limit = self.album_load_limit.clamp(1, 500).to_string();
        let data = match self.client.get("/album/", &[("id", id), ("limit", &limit)]).await {
            Ok(d) => d,
            Err(_) => return LoadResult::Empty {},
        };

        let album = match data.get("data") {
            Some(d) => d,
            None => return LoadResult::Empty {},
        };

        let title = album
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown")
            .to_owned();

        let tracks: Vec<Track> = album
            .get("items")
            .and_then(|v| v.as_array())
            .map(|items| {
                items
                    .iter()
                    .filter_map(|w| self.parse_track(w.get("item").unwrap_or(w)))
                    .map(Track::new)
                    .collect()
            })
            .unwrap_or_default();

        if tracks.is_empty() {
            return LoadResult::Empty {};
        }

        LoadResult::Playlist(PlaylistData {
            info: PlaylistInfo { name: title, selected_track: -1 },
            plugin_info: serde_json::json!({
                "type": "album",
                "url": format!("https://tidal.com/browse/album/{id}"),
                "totalTracks": album.get("numberOfTracks").and_then(|v| v.as_u64()).unwrap_or(tracks.len() as u64)
            }),
            tracks,
        })
    }

    async fn get_playlist(&self, id: &str) -> LoadResult {
        let limit = self.playlist_load_limit.clamp(1, 500).to_string();
        let data = match self.client.get("/playlist/", &[("id", id), ("limit", &limit)]).await {
            Ok(d) => d,
            Err(_) => return LoadResult::Empty {},
        };

        let title = data
            .get("playlist")
            .and_then(|p| p.get("title"))
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown")
            .to_owned();

        let tracks: Vec<Track> = data
            .get("items")
            .and_then(|v| v.as_array())
            .map(|items| {
                items
                    .iter()
                    .filter_map(|w| self.parse_track(w.get("item").unwrap_or(w)))
                    .map(Track::new)
                    .collect()
            })
            .unwrap_or_default();

        if tracks.is_empty() {
            return LoadResult::Empty {};
        }

        LoadResult::Playlist(PlaylistData {
            info: PlaylistInfo { name: title, selected_track: -1 },
            plugin_info: serde_json::json!({
                "type": "playlist",
                "url": format!("https://tidal.com/browse/playlist/{id}"),
                "totalTracks": tracks.len()
            }),
            tracks,
        })
    }

    async fn get_mix(&self, id: &str, name_override: Option<String>) -> LoadResult {
        let data = match self.client.get("/mix/", &[("id", id)]).await {
            Ok(d) => d,
            Err(_) => return LoadResult::Empty {},
        };

        let title = name_override.unwrap_or_else(|| {
            data.get("mix")
                .and_then(|m| m.get("title"))
                .and_then(|v| v.as_str())
                .unwrap_or("Mix")
                .to_owned()
        });

        let tracks: Vec<Track> = data
            .get("items")
            .and_then(|v| v.as_array())
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| self.parse_track(item))
                    .map(Track::new)
                    .collect()
            })
            .unwrap_or_default();

        if tracks.is_empty() {
            return LoadResult::Empty {};
        }

        LoadResult::Playlist(PlaylistData {
            info: PlaylistInfo { name: title, selected_track: -1 },
            plugin_info: serde_json::json!({
                "type": "mix",
                "url": format!("https://tidal.com/browse/mix/{id}"),
                "totalTracks": tracks.len()
            }),
            tracks,
        })
    }

    async fn get_artist(&self, id: &str) -> LoadResult {
        let info = match self.client.get("/artist/", &[("id", id)]).await {
            Ok(d) => d,
            Err(_) => return LoadResult::Empty {},
        };

        let name = info
            .get("artist")
            .and_then(|a| a.get("name"))
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown Artist")
            .to_owned();

        let limit = self.artist_load_limit.clamp(1, 50).to_string();
        let data = match self.client.get("/artist/", &[("f", id), ("limit", &limit)]).await {
            Ok(d) => d,
            Err(_) => return LoadResult::Empty {},
        };

        let tracks: Vec<Track> = data
            .get("tracks")
            .and_then(|v| v.as_array())
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| self.parse_track(item))
                    .map(Track::new)
                    .collect()
            })
            .unwrap_or_default();

        if tracks.is_empty() {
            return LoadResult::Empty {};
        }

        LoadResult::Playlist(PlaylistData {
            info: PlaylistInfo {
                name: format!("{name}'s Top Tracks"),
                selected_track: -1,
            },
            plugin_info: serde_json::json!({
                "type": "artist",
                "url": format!("https://tidal.com/browse/artist/{id}"),
                "totalTracks": tracks.len()
            }),
            tracks,
        })
    }

    async fn search(&self, query: &str) -> LoadResult {
        match self.client.get("/search/", &[("s", query), ("limit", "10")]).await {
            Ok(data) => {
                let tracks: Vec<Track> = data
                    .get("data")
                    .and_then(|d| d.get("items"))
                    .and_then(|v| v.as_array())
                    .map(|items| {
                        items
                            .iter()
                            .filter_map(|item| self.parse_track(item))
                            .map(Track::new)
                            .collect()
                    })
                    .unwrap_or_default();

                if tracks.is_empty() {
                    LoadResult::Empty {}
                } else {
                    LoadResult::Search(tracks)
                }
            }
            Err(_) => LoadResult::Empty {},
        }
    }

    async fn resolve_by_isrc(&self, isrc: &str) -> LoadResult {
        match self.client.get("/search/", &[("isrc", isrc), ("limit", "5")]).await {
            Ok(data) => {
                let items = match data
                    .get("data")
                    .and_then(|d| d.get("items"))
                    .and_then(|v| v.as_array())
                {
                    Some(i) => i,
                    None => return LoadResult::Empty {},
                };

                let item = items
                    .iter()
                    .find(|t| {
                        t.get("isrc")
                            .and_then(|v| v.as_str())
                            .map(|i| i.eq_ignore_ascii_case(isrc))
                            .unwrap_or(false)
                    })
                    .or_else(|| items.first());

                item.and_then(|i| self.parse_track(i))
                    .map(|i| LoadResult::Track(Track::new(i)))
                    .unwrap_or(LoadResult::Empty {})
            }
            Err(_) => LoadResult::Empty {},
        }
    }

    async fn get_recommendations(&self, id: &str) -> LoadResult {
        match self.client.get("/recommendations/", &[("id", id)]).await {
            Ok(data) => {
                let tracks: Vec<Track> = data
                    .get("data")
                    .and_then(|d| d.get("items"))
                    .and_then(|v| v.as_array())
                    .map(|items| {
                        items
                            .iter()
                            .filter_map(|item| {
                                self.parse_track(item.get("track").unwrap_or(item))
                            })
                            .map(Track::new)
                            .collect()
                    })
                    .unwrap_or_default();

                if tracks.is_empty() {
                    LoadResult::Empty {}
                } else {
                    LoadResult::Playlist(PlaylistData {
                        info: PlaylistInfo {
                            name: "Tidal Recommendations".to_owned(),
                            selected_track: -1,
                        },
                        plugin_info: serde_json::json!({ "type": "recommendations", "totalTracks": tracks.len() }),
                        tracks,
                    })
                }
            }
            Err(_) => LoadResult::Empty {},
        }
    }

    async fn resolve_stream_url(&self, track_id: &str) -> Option<(String, AudioFormat)> {
        for quality in &self.client.quality_order {
            let result = self
                .client
                .get("/track/", &[("id", track_id), ("quality", quality.as_str())])
                .await;

            let raw = match result {
                Ok(r) => r,
                Err(e) => {
                    debug!("HiFi /track/ id={} quality={} failed: {}", track_id, quality, e);
                    continue;
                }
            };

            let playback: PlaybackData = match raw
                .get("data")
                .and_then(|d| serde_json::from_value(d.clone()).ok())
            {
                Some(p) => p,
                None => continue,
            };

            if playback.manifest_mime_type == "application/dash+xml" {
                debug!("HiFi /track/ id={} quality={}: skipping DASH", track_id, quality);
                continue;
            }

            let decoded = match general_purpose::STANDARD.decode(&playback.manifest) {
                Ok(d) => d,
                Err(e) => {
                    warn!("HiFi /track/ id={} quality={}: base64 decode failed: {}", track_id, quality, e);
                    continue;
                }
            };

            let manifest: Manifest = match serde_json::from_slice(&decoded) {
                Ok(m) => m,
                Err(e) => {
                    debug!("HiFi /track/ id={} quality={}: manifest parse failed: {}", track_id, quality, e);
                    continue;
                }
            };

            let stream_url = match manifest.urls.into_iter().next() {
                Some(u) => u,
                None => continue,
            };

            let fmt = audio_format(&stream_url, manifest.mime_type.as_deref(), quality);
            debug!("HiFi /track/ id={} quality={} → {:?}", track_id, quality, fmt);
            return Some((stream_url, fmt));
        }

        warn!("HiFi: all qualities exhausted for track {}", track_id);
        None
    }
}

#[async_trait]
impl SourcePlugin for TidalSource {
    fn name(&self) -> &str {
        "tidal"
    }

    fn can_handle(&self, identifier: &str) -> bool {
        self.search_prefixes().iter().any(|p| identifier.starts_with(p))
            || self.isrc_prefixes().iter().any(|p| identifier.starts_with(p))
            || self.rec_prefixes().iter().any(|p| identifier.starts_with(p))
            || url_regex().is_match(identifier)
    }

    fn search_prefixes(&self) -> Vec<&str> {
        vec!["tdsearch:"]
    }
    fn isrc_prefixes(&self) -> Vec<&str> {
        vec!["tdisrc:"]
    }
    fn rec_prefixes(&self) -> Vec<&str> {
        vec!["tdrec:"]
    }
    fn is_mirror(&self) -> bool {
        false
    }

    async fn load(
        &self,
        identifier: &str,
        _: Option<Arc<dyn crate::lavalink::routeplanner::RoutePlanner>>,
    ) -> LoadResult {
        if let Some(prefix) = self.search_prefixes().iter().find(|p| identifier.starts_with(**p)) {
            return self.search(&identifier[prefix.len()..]).await;
        }

        if let Some(prefix) = self.isrc_prefixes().iter().find(|p| identifier.starts_with(**p)) {
            return self.resolve_by_isrc(&identifier[prefix.len()..]).await;
        }

        if let Some(prefix) = self.rec_prefixes().iter().find(|p| identifier.starts_with(**p)) {
            return self.get_recommendations(&identifier[prefix.len()..]).await;
        }

        if let Some(caps) = url_regex().captures(identifier) {
            let type_str = caps.get(1).map_or("", |m| m.as_str());
            let id = caps.get(2).map_or("", |m| m.as_str());

            return match type_str {
                "track" => self.get_track_data(id).await,
                "album" => self.get_album(id).await,
                "playlist" => self.get_playlist(id).await,
                "mix" => self.get_mix(id, None).await,
                "artist" => self.get_artist(id).await,
                _ => LoadResult::Empty {},
            };
        }

        LoadResult::Empty {}
    }

    async fn get_track(
        &self,
        identifier: &str,
        _: Option<Arc<dyn crate::lavalink::routeplanner::RoutePlanner>>,
    ) -> Option<crate::media::sources::plugin::BoxedTrack> {
        let id = if let Some(caps) = url_regex().captures(identifier) {
            if caps.get(1).map_or("", |m| m.as_str()) != "track" {
                return None;
            }
            caps.get(2).map_or("", |m| m.as_str()).to_owned()
        } else {
            identifier.to_owned()
        };

        let (stream_url, kind) = self.resolve_stream_url(&id).await?;

        Some(Box::new(TidalTrack {
            identifier: id,
            stream_url,
            kind,
            http_client: self.client.inner.clone(),
        }))
    }
}