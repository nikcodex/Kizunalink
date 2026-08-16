use crate::models::track::{LavalinkTrack, TrackInfo};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, REFERER, USER_AGENT};
use serde_json::Value;
use std::sync::Arc;
use tracing::info;

const JIOSAAVN_API_BASE: &str = "https://www.jiosaavn.com/api.php";
const ANDROID_USER_AGENT: &str = "JioSaavn/9.5.0 (Linux; Android 13; SM-S908B Build/TP1A.220624.014; wv) AppleWebKit/537.36";

#[derive(Clone)]
pub struct JioSaavnSource {
    client: reqwest::Client,
}

impl JioSaavnSource {
    pub fn new() -> Arc<Self> {
        let mut headers = HeaderMap::new();
        headers.insert(USER_AGENT, HeaderValue::from_static(ANDROID_USER_AGENT));
        headers.insert(ACCEPT, HeaderValue::from_static("application/json, text/plain, */*"));
        headers.insert(REFERER, HeaderValue::from_static("https://www.jiosaavn.com/"));

        let client = reqwest::Client::builder()
            .default_headers(headers)
            .build()
            .expect("Failed to build Reqwest client for JioSaavn");

        Arc::new(Self { client })
    }

    pub async fn search(&self, query: &str, limit: usize) -> Result<Vec<LavalinkTrack>, String> {
        let clean_query = query.trim();
        if clean_query.is_empty() {
            return Ok(vec![]);
        }

        let url = format!(
            "{}?__call=autocomplete.get&query={}&_format=json&_marker=0&ctx=android&api_version=4",
            JIOSAAVN_API_BASE,
            urlencoding::encode(clean_query)
        );

        let response = self.client.get(&url).send().await.map_err(|e| e.to_string())?;
        let text = response.text().await.map_err(|e| e.to_string())?;

        let json_str = match text.find('{') {
            Some(idx) => &text[idx..],
            None => return Ok(vec![]),
        };

        let json_val: Value = serde_json::from_str(json_str).map_err(|e| e.to_string())?;
        self.parse_search_results(&json_val)
    }

    fn parse_search_results(&self, json: &Value) -> Result<Vec<LavalinkTrack>, String> {
        let songs = json
            .get("songs")
            .and_then(|s| s.get("data"))
            .and_then(|d| d.as_array())
            .or_else(|| json.get("songs").and_then(|s| s.as_array()));

        let songs_array = match songs {
            Some(arr) => arr,
            None => return Ok(vec![]),
        };

        let mut tracks = Vec::new();

        for song in songs_array.iter() {
            let id = song.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
            if id.is_empty() {
                continue;
            }

            let title = song
                .get("title")
                .or_else(|| song.get("song"))
                .and_then(|v| v.as_str())
                .unwrap_or("Unknown Title")
                .to_string();

            let artist = song
                .get("more_info")
                .and_then(|m| m.get("primary_artists").or_else(|| m.get("singers")))
                .or_else(|| song.get("description"))
                .and_then(|v| v.as_str())
                .unwrap_or("Unknown Artist")
                .to_string();

            let duration_sec = song
                .get("more_info")
                .and_then(|m| m.get("duration"))
                .or_else(|| song.get("duration"))
                .and_then(|d| {
                    if let Some(s) = d.as_str() {
                        s.parse::<u64>().ok()
                    } else {
                        d.as_u64()
                    }
                })
                .unwrap_or(210);

            let artwork = song
                .get("image")
                .and_then(|v| v.as_str())
                .map(|img| img.replace("50x50", "500x500").replace("150x150", "500x500"));

            let url = song
                .get("perma_url")
                .or_else(|| song.get("url"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());

            let clean_title = title.replace("&quot;", "\"").replace("&amp;", "&").replace("&#039;", "'");
            let clean_artist = artist.replace("&quot;", "\"").replace("&amp;", "&").replace("&#039;", "'");

            let encoded = STANDARD.encode(format!("jiosaavn:{}", id));

            tracks.push(LavalinkTrack {
                encoded,
                info: TrackInfo {
                    identifier: id,
                    is_seekable: true,
                    author: clean_artist,
                    length: duration_sec * 1000,
                    is_stream: false,
                    position: 0,
                    title: clean_title,
                    uri: url,
                    artwork_url: artwork,
                    source_name: "jiosaavn".to_string(),
                    bitrate: Some("320kbps".to_string()),
                    stream_url: None,
                },
                plugin_info: Value::Object(Default::default()),
                user_data: Value::Object(Default::default()),
            });
        }

        Ok(tracks)
    }

    pub async fn resolve_stream_url(&self, song_id: &str) -> Result<String, String> {
        let details_url = format!(
            "{}?__call=song.getDetails&pids={}&_format=json&_marker=0&ctx=android&api_version=4",
            JIOSAAVN_API_BASE, song_id
        );

        let details_res = self.client.get(&details_url).send().await.map_err(|e| e.to_string())?;
        let details_text = details_res.text().await.map_err(|e| e.to_string())?;

        let json_str = match details_text.find('{') {
            Some(idx) => &details_text[idx..],
            None => return Err("Invalid song details JSON response".to_string()),
        };

        let details_json: Value = serde_json::from_str(json_str).map_err(|e| e.to_string())?;
        let song_obj = details_json
            .get(song_id)
            .or_else(|| details_json.get("songs").and_then(|s| s.as_array()).and_then(|a| a.first()))
            .ok_or_else(|| format!("Song ID {} not found in details", song_id))?;

        let encrypted_url = song_obj
            .get("encrypted_media_url")
            .or_else(|| song_obj.get("more_info").and_then(|m| m.get("encrypted_media_url")))
            .and_then(|v| v.as_str())
            .ok_or_else(|| "Missing encrypted_media_url in song object".to_string())?;

        let auth_url = format!(
            "{}?__call=song.generateAuthToken&url={}&bitrate=320&_format=json&_marker=0&ctx=android&api_version=4",
            JIOSAAVN_API_BASE,
            urlencoding::encode(encrypted_url)
        );

        let auth_res = self.client.get(&auth_url).send().await.map_err(|e| e.to_string())?;
        let auth_json: Value = auth_res.json().await.map_err(|e| e.to_string())?;

        let stream_url = auth_json
            .get("auth_url")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "Failed to generate signed CDN stream URL".to_string())?;

        info!("⚡ Resolved 320kbps JioSaavn stream for song ID: {}", song_id);
        Ok(stream_url.to_string())
    }

    pub async fn get_lyrics(&self, song_id: &str) -> Result<Option<String>, String> {
        let lyrics_url = format!(
            "{}?__call=lyrics.getLyrics&lyrics_id={}&_format=json&_marker=0&ctx=web6dot0&api_version=4",
            JIOSAAVN_API_BASE, song_id
        );

        let res = self.client.get(&lyrics_url).send().await.map_err(|e| e.to_string())?;
        let json: Value = res.json().await.map_err(|e| e.to_string())?;

        if let Some(lyrics_raw) = json.get("lyrics").and_then(|l| l.as_str()) {
            let clean = lyrics_raw.replace("<br>", "\n").replace("<br/>", "\n").replace("<br />", "\n");
            return Ok(Some(clean));
        }

        Ok(None)
    }

    pub async fn get_recommendations(&self, query: &str) -> Result<Vec<LavalinkTrack>, String> {
        let song_id = if query.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '-') && query.len() < 20 {
            query.to_string()
        } else {
            let search_results = self.search(query, 1).await?;
            if let Some(first) = search_results.into_iter().next() {
                first.info.identifier
            } else {
                return Ok(Vec::new());
            }
        };

        let station_url = format!(
            "{}?__call=webradio.createEntityStation&api_version=4&ctx=android&entity_id=[\"{}\"]&entity_type=queue&_format=json",
            JIOSAAVN_API_BASE, song_id
        );

        if let Ok(res) = self.client.get(&station_url).send().await {
            if let Ok(json) = res.json::<Value>().await {
                if let Some(station_id) = json.get("stationid").and_then(|v| v.as_str()) {
                    let songs_url = format!(
                        "{}?__call=webradio.getSong&api_version=4&ctx=android&stationid={}&k=20&_format=json",
                        JIOSAAVN_API_BASE, urlencoding::encode(station_id)
                    );

                    if let Ok(songs_res) = self.client.get(&songs_url).send().await {
                        if let Ok(songs_json) = songs_res.json::<Value>().await {
                            if let Ok(tracks) = self.parse_search_results(&songs_json) {
                                if !tracks.is_empty() {
                                    return Ok(tracks);
                                }
                            }
                        }
                    }
                }
            }
        }

        self.search(&format!("{} mix", query), 15).await
    }
}
