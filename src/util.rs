use crate::models::track::{LavalinkTrack, TrackInfo};
use std::time::{SystemTime, UNIX_EPOCH};

pub fn current_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// Constant-time string comparison to prevent timing attacks.
pub fn constant_time_eq(a: &str, b: &str) -> bool {
    let a_bytes = a.as_bytes();
    let b_bytes = b.as_bytes();

    if a_bytes.len() != b_bytes.len() {
        return false;
    }

    let mut diff = 0u8;
    for (x, y) in a_bytes.iter().zip(b_bytes.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

pub fn decode_track(encoded: &str) -> Result<LavalinkTrack, String> {
    crate::track_encoding::decode_track(encoded).map_err(|e| e.to_string())
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

    let mut track = LavalinkTrack {
        encoded: String::new(),
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
        plugin_info: serde_json::Value::Object(Default::default()),
        user_data: serde_json::Value::Object(Default::default()),
    };

    if let Ok(enc) = crate::track_encoding::encode_track(&track) {
        track.encoded = enc;
    } else {
        track.encoded = url.to_string();
    }

    track
}

pub fn uuid_v4() -> String {
    let rand_part: u64 = rand::random();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{:016x}{:016x}", nanos as u64, rand_part)
}
