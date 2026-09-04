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
    let max_len = a_bytes.len().max(b_bytes.len());

    let mut diff = if a_bytes.len() == b_bytes.len() { 0u8 } else { 1u8 };
    for i in 0..max_len {
        let x = a_bytes.get(i).copied().unwrap_or(0);
        let y = b_bytes.get(i).copied().unwrap_or(0);
        diff |= x ^ y;
    }
    diff == 0
}

pub fn decode_track_safe(encoded: &str) -> Result<LavalinkTrack, String> {
    crate::track_encoding::decode_track(encoded).map_err(|e| e.to_string())
}

pub fn create_http_track(url: &str) -> LavalinkTrack {
    let title = url
        .split('/')
        .next_back()
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

/// Generates a UUID v4 string using cryptographically secure randomness from `rand::random()`.
/// With 122 bits of random entropy, collision probability is astronomically low (1 in 2^122)
/// and collision is practically impossible.
pub fn uuid_v4() -> String {
    let mut bytes: [u8; 16] = rand::random();
    // Set version to 4 (0100)
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    // Set variant to RFC 4122 (10xx)
    bytes[8] = (bytes[8] & 0x3f) | 0x80;

    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0], bytes[1], bytes[2], bytes[3],
        bytes[4], bytes[5],
        bytes[6], bytes[7],
        bytes[8], bytes[9],
        bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15]
    )
}
