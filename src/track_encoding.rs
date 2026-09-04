use crate::models::track::{LavalinkTrack, TrackInfo};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use dashmap::DashMap;
use std::io::{Cursor, Read};
use std::sync::LazyLock;

const TRACK_INFO_VERSIONED: i32 = 1;
const TRACK_INFO_VERSION: i32 = 3;

static TRACK_CACHE: LazyLock<DashMap<String, LavalinkTrack>> =
    LazyLock::new(|| DashMap::with_capacity(1024));
const MAX_CACHE_ENTRIES: usize = 4096;

/// Decode Lavalink binary encoded track (supports v1, v2, and v3).
pub fn decode_track(encoded: &str) -> Result<LavalinkTrack, TrackDecodeError> {
    let clean = encoded.trim();
    if let Some(cached) = TRACK_CACHE.get(clean) {
        return Ok(cached.clone());
    }

    let data = STANDARD
        .decode(clean)
        .map_err(|e| TrackDecodeError::Base64Error(e.to_string()))?;

    if data.len() < 4 {
        return Err(TrackDecodeError::IoError(
            "Track data too short".to_string(),
        ));
    }

    let mut cursor = Cursor::new(&data);
    let mut buf4 = [0u8; 4];
    let mut buf1 = [0u8; 1];
    let mut buf8 = [0u8; 8];

    // Read the prefix (4 bytes)
    cursor
        .read_exact(&mut buf4)
        .map_err(|e| TrackDecodeError::IoError(e.to_string()))?;
    let prefix = i32::from_be_bytes(buf4);
    let flags = (prefix >> 30) & 1;

    // Check version
    let version = if flags != 0 {
        cursor
            .read_exact(&mut buf1)
            .map_err(|e| TrackDecodeError::IoError(e.to_string()))?;
        buf1[0] as i32
    } else {
        1
    };

    if !(1..=3).contains(&version) {
        return Err(TrackDecodeError::UnsupportedVersion(version));
    }

    let title = read_utf(&mut cursor)?;
    let author = read_utf(&mut cursor)?;

    cursor
        .read_exact(&mut buf8)
        .map_err(|e| TrackDecodeError::IoError(e.to_string()))?;
    let length = u64::from_be_bytes(buf8);

    let identifier = read_utf(&mut cursor)?;

    cursor
        .read_exact(&mut buf1)
        .map_err(|e| TrackDecodeError::IoError(e.to_string()))?;
    let is_stream = buf1[0] != 0;

    cursor
        .read_exact(&mut buf1)
        .map_err(|e| TrackDecodeError::IoError(e.to_string()))?;
    let has_uri = buf1[0] != 0;

    let uri = if has_uri {
        Some(read_utf(&mut cursor)?)
    } else {
        None
    };

    let (artwork_url, isrc, source_name) = if version >= 3 {
        let artwork = read_nullable_text(&mut cursor)?;
        let isrc_code = read_nullable_text(&mut cursor)?;
        let src = match read_nullable_text(&mut cursor)? {
            Some(s) if !s.is_empty() => s,
            _ => read_utf(&mut cursor).unwrap_or_else(|_| "unknown".to_string()),
        };
        (artwork, isrc_code, src)
    } else {
        let src = read_utf(&mut cursor)?;
        (None, None, src)
    };

    // Source-specific data (for http/local sources, there is probeInfo)
    if source_name == "http" || source_name == "local" {
        // Probe info might be present; read if remaining bytes allow
        let _ = read_nullable_text(&mut cursor).or_else(|_| read_utf(&mut cursor).map(Some));
    }

    let position = if cursor.read_exact(&mut buf8).is_ok() {
        u64::from_be_bytes(buf8)
    } else {
        0
    };

    let mut plugin_info = serde_json::Map::new();
    if let Some(ref code) = isrc {
        plugin_info.insert("isrc".to_string(), serde_json::Value::String(code.clone()));
    }

    let track = LavalinkTrack {
        encoded: encoded.to_string(),
        info: TrackInfo {
            identifier,
            is_seekable: !is_stream,
            author,
            length,
            is_stream,
            position,
            title,
            uri,
            artwork_url,
            isrc,
            source_name,
        },
        plugin_info: serde_json::Value::Object(plugin_info),
        user_data: serde_json::Value::Object(Default::default()),
    };

    if TRACK_CACHE.len() < MAX_CACHE_ENTRIES {
        TRACK_CACHE.insert(clean.to_string(), track.clone());
    }

    Ok(track)
}

/// Encode a track into Lavalink v3 binary format.
pub fn encode_track(track: &LavalinkTrack) -> Result<String, TrackDecodeError> {
    let mut output = Vec::new();

    // Write placeholder for 4-byte header
    output.extend_from_slice(&[0u8; 4]);

    // Write version byte (3)
    output.push(TRACK_INFO_VERSION as u8);

    // Track metadata
    write_utf(&mut output, &track.info.title)?;
    write_utf(&mut output, &track.info.author)?;
    output.extend_from_slice(&track.info.length.to_be_bytes());
    write_utf(&mut output, &track.info.identifier)?;
    output.push(if track.info.is_stream { 1 } else { 0 });

    // URI
    if let Some(uri) = &track.info.uri {
        output.push(1);
        write_utf(&mut output, uri)?;
    } else {
        output.push(0);
    }

    // Version 3 fields: artworkUrl, isrc, sourceName (all NullableText)
    write_nullable_text(&mut output, track.info.artwork_url.as_deref())?;
    write_nullable_text(&mut output, track.info.isrc.as_deref())?;
    write_nullable_text(&mut output, Some(&track.info.source_name))?;

    // Probe info for http / local
    if track.info.source_name == "http" || track.info.source_name == "local" {
        write_nullable_text(&mut output, None)?;
    }

    // Position
    output.extend_from_slice(&track.info.position.to_be_bytes());

    // Calculate length & header prefix
    let payload_len = (output.len() - 4) as i32;
    let prefix = (payload_len & 0x3FFFFFFF) | (TRACK_INFO_VERSIONED << 30);
    output[0..4].copy_from_slice(&prefix.to_be_bytes());

    Ok(STANDARD.encode(&output))
}

fn read_utf(reader: &mut Cursor<&Vec<u8>>) -> Result<String, TrackDecodeError> {
    let mut len_buf = [0u8; 2];
    reader
        .read_exact(&mut len_buf)
        .map_err(|e| TrackDecodeError::IoError(e.to_string()))?;
    let len = u16::from_be_bytes(len_buf) as usize;

    let mut str_buf = vec![0u8; len];
    reader
        .read_exact(&mut str_buf)
        .map_err(|e| TrackDecodeError::IoError(e.to_string()))?;

    String::from_utf8(str_buf).map_err(|e| TrackDecodeError::Utf8Error(e.to_string()))
}

fn write_utf(writer: &mut Vec<u8>, s: &str) -> Result<(), TrackDecodeError> {
    let bytes = s.as_bytes();
    if bytes.len() > u16::MAX as usize {
        return Err(TrackDecodeError::StringTooLong);
    }
    writer.extend_from_slice(&(bytes.len() as u16).to_be_bytes());
    writer.extend_from_slice(bytes);
    Ok(())
}

fn read_nullable_text(reader: &mut Cursor<&Vec<u8>>) -> Result<Option<String>, TrackDecodeError> {
    let mut len_buf = [0u8; 2];
    reader
        .read_exact(&mut len_buf)
        .map_err(|e| TrackDecodeError::IoError(e.to_string()))?;
    let len = u16::from_be_bytes(len_buf);
    if len == 0xFFFF {
        return Ok(None);
    }
    let mut str_buf = vec![0u8; len as usize];
    reader
        .read_exact(&mut str_buf)
        .map_err(|e| TrackDecodeError::IoError(e.to_string()))?;
    String::from_utf8(str_buf)
        .map(Some)
        .map_err(|e| TrackDecodeError::Utf8Error(e.to_string()))
}

fn write_nullable_text(writer: &mut Vec<u8>, text: Option<&str>) -> Result<(), TrackDecodeError> {
    match text {
        Some(s) if !s.is_empty() => {
            let bytes = s.as_bytes();
            if bytes.len() > 0xFFFE {
                return Err(TrackDecodeError::StringTooLong);
            }
            writer.extend_from_slice(&(bytes.len() as u16).to_be_bytes());
            writer.extend_from_slice(bytes);
        }
        _ => {
            writer.extend_from_slice(&0xFFFFu16.to_be_bytes());
        }
    }
    Ok(())
}

#[derive(Debug)]
pub enum TrackDecodeError {
    Base64Error(String),
    IoError(String),
    UnsupportedVersion(i32),
    Utf8Error(String),
    StringTooLong,
}

impl std::fmt::Display for TrackDecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TrackDecodeError::Base64Error(e) => write!(f, "Base64 decode error: {}", e),
            TrackDecodeError::IoError(e) => write!(f, "IO error: {}", e),
            TrackDecodeError::UnsupportedVersion(v) => {
                write!(f, "Unsupported track version: {}", v)
            }
            TrackDecodeError::Utf8Error(e) => write!(f, "UTF-8 error: {}", e),
            TrackDecodeError::StringTooLong => write!(f, "String too long for UTF encoding"),
        }
    }
}

impl std::error::Error for TrackDecodeError {}

#[cfg(test)]
mod tests {
    use super::*;

    // Real encoded track (v2) from Lavalink's MessageSerializerTest.kt
    const REAL_YOUTUBE_ENCODED: &str = "QAAAjQIAJVJpY2sgQXN0bGV5IC0gTmV2ZXIgR29ubmEgR2l2ZSBZb3UgVXAADlJpY2tBc3RsZXlWRVZPAAAAAAADPCAAC2RRdzR3OVdnWGNRAAEAK2h0dHBzOi8vd3d3LnlvdXR1YmUuY29tL3dhdGNoP3Y9ZFF3NHc5V2dYY1EAB3lvdXR1YmUAAAAAAAAAAA==";

    #[test]
    fn test_decode_real_v2_youtube_track() {
        let decoded =
            decode_track(REAL_YOUTUBE_ENCODED).expect("Failed to decode real YouTube v2 track");

        assert_eq!(decoded.info.identifier, "dQw4w9WgXcQ");
        assert_eq!(decoded.info.author, "RickAstleyVEVO");
        assert_eq!(decoded.info.length, 212000);
        assert!(!decoded.info.is_stream);
        assert_eq!(decoded.info.position, 0);
        assert_eq!(decoded.info.title, "Rick Astley - Never Gonna Give You Up");
        assert_eq!(
            decoded.info.uri.as_deref(),
            Some("https://www.youtube.com/watch?v=dQw4w9WgXcQ")
        );
        assert_eq!(decoded.info.source_name, "youtube");
        assert!(decoded.info.is_seekable);
    }

    #[test]
    fn test_v3_roundtrip_with_artwork_and_isrc() {
        let original = LavalinkTrack {
            encoded: String::new(),
            info: TrackInfo {
                identifier: "4cOdK2wGLETKBW3PvgPWqT".to_string(),
                is_seekable: true,
                author: "Rick Astley".to_string(),
                length: 213573,
                is_stream: false,
                position: 15000,
                title: "Never Gonna Give You Up".to_string(),
                uri: Some("https://open.spotify.com/track/4cOdK2wGLETKBW3PvgPWqT".to_string()),
                artwork_url: Some(
                    "https://i.scdn.co/image/ab67616d0000b2735755e164993798e0c9ef7d7a".to_string(),
                ),
                isrc: Some("GBARL8700014".to_string()),
                source_name: "spotify".to_string(),
            },
            plugin_info: serde_json::json!({"isrc": "GBARL8700014"}),
            user_data: serde_json::json!({}),
        };

        let encoded = encode_track(&original).expect("Failed to encode v3 track");
        let decoded = decode_track(&encoded).expect("Failed to decode v3 track");

        assert_eq!(decoded.info.identifier, original.info.identifier);
        assert_eq!(decoded.info.author, original.info.author);
        assert_eq!(decoded.info.length, original.info.length);
        assert_eq!(decoded.info.is_stream, original.info.is_stream);
        assert_eq!(decoded.info.position, original.info.position);
        assert_eq!(decoded.info.title, original.info.title);
        assert_eq!(decoded.info.uri, original.info.uri);
        assert_eq!(decoded.info.artwork_url, original.info.artwork_url);
        assert_eq!(decoded.info.isrc, original.info.isrc);
        assert_eq!(decoded.info.source_name, original.info.source_name);
    }

    #[test]
    fn test_v3_roundtrip_stream_track() {
        let original = LavalinkTrack {
            encoded: String::new(),
            info: TrackInfo {
                identifier: "https://example.com/radio.mp3".to_string(),
                is_seekable: false,
                author: "Radio Host".to_string(),
                length: 0,
                is_stream: true,
                position: 0,
                title: "Live Stream".to_string(),
                uri: Some("https://example.com/radio.mp3".to_string()),
                artwork_url: None,
                isrc: None,
                source_name: "http".to_string(),
            },
            plugin_info: serde_json::json!({}),
            user_data: serde_json::json!({}),
        };

        let encoded = encode_track(&original).expect("Failed to encode track");
        let decoded = decode_track(&encoded).expect("Failed to decode track");

        assert!(decoded.info.is_stream);
        assert!(!decoded.info.is_seekable);
        assert_eq!(decoded.info.source_name, "http");
    }

    #[test]
    fn test_decode_invalid_data() {
        assert!(decode_track("invalid-base64-content!").is_err());
        assert!(decode_track("").is_err());
    }
}
