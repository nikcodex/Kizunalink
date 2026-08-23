use crate::models::track::{LavalinkTrack, TrackInfo};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use std::time::{SystemTime, UNIX_EPOCH};

pub fn current_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

pub fn decode_track(encoded: &str) -> LavalinkTrack {
    match crate::track_encoding::decode_track(encoded) {
        Ok(track) => track,
        Err(_) => {
            // Fallback for non-standard encoded tracks
            let decoded_str = STANDARD
                .decode(encoded)
                .ok()
                .and_then(|b| String::from_utf8(b).ok())
                .unwrap_or_else(|| encoded.to_string());

            let (source, id) = if let Some(idx) = decoded_str.find(':') {
                (&decoded_str[..idx], &decoded_str[idx + 1..])
            } else {
                ("jiosaavn", decoded_str.as_str())
            };

            LavalinkTrack {
                encoded: encoded.to_string(),
                info: TrackInfo {
                    identifier: id.to_string(),
                    is_seekable: true,
                    author: "Unknown".to_string(),
                    length: 210000,
                    is_stream: false,
                    position: 0,
                    title: id.to_string(),
                    uri: None,
                    artwork_url: None,
                    isrc: None,
                    source_name: source.to_string(),
                },
                plugin_info: serde_json::json!({}),
                user_data: serde_json::json!({}),
            }
        }
    }
}

pub fn create_http_track(url: &str) -> LavalinkTrack {
    let title = url
        .split('/')
        .last()
        .unwrap_or("Direct Audio Stream")
        .split('?')
        .next()
        .unwrap_or("Direct Audio Stream")
        .to_string();

    let encoded = STANDARD.encode(format!("http:{}", url));

    LavalinkTrack {
        encoded,
        info: TrackInfo {
            identifier: url.to_string(),
            is_seekable: true,
            author: "HTTP Stream".to_string(),
            length: 0,
            is_stream: true,
            position: 0,
            title,
            uri: Some(url.to_string()),
            artwork_url: None,
            isrc: None,
            source_name: "http".to_string(),
        },
        plugin_info: serde_json::json!({}),
        user_data: serde_json::json!({}),
    }
}

pub fn uuid_v4() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{:016x}", nanos)
}
