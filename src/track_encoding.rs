use crate::models::track::{LavalinkTrack, TrackInfo};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use std::io::{Cursor, Read};

const TRACK_INFO_VERSIONED: i32 = 1;
const TRACK_INFO_VERSION: i32 = 2;

pub fn decode_track(encoded: &str) -> Result<LavalinkTrack, TrackDecodeError> {
    let data = STANDARD
        .decode(encoded)
        .map_err(|e| TrackDecodeError::Base64Error(e.to_string()))?;

    let mut cursor = Cursor::new(&data);
    let mut buf4 = [0u8; 4];
    let mut buf1 = [0u8; 1];
    let mut buf8 = [0u8; 8];

    // Read the prefix (4 bytes)
    cursor.read_exact(&mut buf4)
        .map_err(|e| TrackDecodeError::IoError(e.to_string()))?;
    let prefix = i32::from_be_bytes(buf4);
    let flags = prefix >> 30;

    // Check if versioned
    let version = if flags & TRACK_INFO_VERSIONED != 0 {
        cursor.read_exact(&mut buf1)
            .map_err(|e| TrackDecodeError::IoError(e.to_string()))?;
        buf1[0] as i32
    } else {
        1
    };

    if version != 2 {
        return Err(TrackDecodeError::UnsupportedVersion(version));
    }

    // Version 2 decoding
    let title = read_utf(&mut cursor)?;
    let author = read_utf(&mut cursor)?;

    cursor.read_exact(&mut buf8)
        .map_err(|e| TrackDecodeError::IoError(e.to_string()))?;
    let length = u64::from_be_bytes(buf8);

    let identifier = read_utf(&mut cursor)?;

    cursor.read_exact(&mut buf1)
        .map_err(|e| TrackDecodeError::IoError(e.to_string()))?;
    let is_stream = buf1[0] != 0;

    cursor.read_exact(&mut buf1)
        .map_err(|e| TrackDecodeError::IoError(e.to_string()))?;
    let has_uri = buf1[0] != 0;

    let uri = if has_uri {
        Some(read_utf(&mut cursor)?)
    } else {
        None
    };

    let source_name = read_utf(&mut cursor)?;

    // Source-specific data (for http/local sources, there's probeInfo)
    let artwork_url = if source_name == "http" || source_name == "local" {
        // Read probeInfo (UTF string)
        match read_utf(&mut cursor) {
            Ok(_) => None, // We don't use probeInfo for artwork
            Err(_) => None,
        }
    } else {
        None
    };

    cursor.read_exact(&mut buf8)
        .map_err(|e| TrackDecodeError::IoError(e.to_string()))?;
    let position = u64::from_be_bytes(buf8);

    Ok(LavalinkTrack {
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
            isrc: None,
            source_name,
        },
        plugin_info: serde_json::json!({}),
        user_data: serde_json::json!({}),
    })
}

pub fn encode_track(track: &LavalinkTrack) -> Result<String, TrackDecodeError> {
    let mut output = Vec::new();

    // Write placeholder for prefix (4 bytes)
    output.extend_from_slice(&[0u8; 4]);

    // Write version byte
    output.push(TRACK_INFO_VERSION as u8);

    // Write track info
    write_utf(&mut output, &track.info.title)?;
    write_utf(&mut output, &track.info.author)?;
    output.extend_from_slice(&track.info.length.to_be_bytes());
    write_utf(&mut output, &track.info.identifier)?;
    output.push(if track.info.is_stream { 1 } else { 0 });

    // URI handling
    if let Some(uri) = &track.info.uri {
        output.push(1); // has_uri = true
        write_utf(&mut output, uri)?;
    } else {
        output.push(0); // has_uri = false
    }

    write_utf(&mut output, &track.info.source_name)?;

    // Source-specific data for http/local
    if track.info.source_name == "http" || track.info.source_name == "local" {
        write_utf(&mut output, "<no probe info provided>")?;
    }

    output.extend_from_slice(&track.info.position.to_be_bytes());

    // Now write the prefix (length - 4) | (TRACK_INFO_VERSIONED << 30)
    let data_len = (output.len() - 4) as i32;
    let prefix = data_len | (TRACK_INFO_VERSIONED << 30);
    let prefix_bytes = prefix.to_be_bytes();
    output[0..4].copy_from_slice(&prefix_bytes);

    Ok(STANDARD.encode(&output))
}

fn read_utf(reader: &mut Cursor<&Vec<u8>>) -> Result<String, TrackDecodeError> {
    let mut len_buf = [0u8; 2];
    reader.read_exact(&mut len_buf)
        .map_err(|e| TrackDecodeError::IoError(e.to_string()))?;
    let len = u16::from_be_bytes(len_buf) as usize;

    let mut str_buf = vec![0u8; len];
    reader.read_exact(&mut str_buf)
        .map_err(|e| TrackDecodeError::IoError(e.to_string()))?;

    String::from_utf8(str_buf)
        .map_err(|e| TrackDecodeError::Utf8Error(e.to_string()))
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
            TrackDecodeError::UnsupportedVersion(v) => write!(f, "Unsupported track version: {}", v),
            TrackDecodeError::Utf8Error(e) => write!(f, "UTF-8 error: {}", e),
            TrackDecodeError::StringTooLong => write!(f, "String too long for UTF encoding"),
        }
    }
}

impl std::error::Error for TrackDecodeError {}

#[cfg(test)]
mod tests {
    use super::*;

    // Real encoded track from Lavalink's MessageSerializerTest.kt
    // YouTube: Rick Astley - Never Gonna Give You Up
    const REAL_YOUTUBE_ENCODED: &str = "QAAAjQIAJVJpY2sgQXN0bGV5IC0gTmV2ZXIgR29ubmEgR2l2ZSBZb3UgVXAADlJpY2tBc3RsZXlWRVZPAAAAAAADPCAAC2RRdzR3OVdnWGNRAAEAK2h0dHBzOi8vd3d3LnlvdXR1YmUuY29tL3dhdGNoP3Y9ZFF3NHc5V2dYY1EAB3lvdXR1YmUAAAAAAAAAAA==";

    #[test]
    fn test_decode_real_youtube_track() {
        let decoded = decode_track(REAL_YOUTUBE_ENCODED).expect("Failed to decode real YouTube track");

        assert_eq!(decoded.info.identifier, "dQw4w9WgXcQ");
        assert_eq!(decoded.info.author, "RickAstleyVEVO");
        assert_eq!(decoded.info.length, 212000);
        assert_eq!(decoded.info.is_stream, false);
        assert_eq!(decoded.info.position, 0);
        assert_eq!(decoded.info.title, "Rick Astley - Never Gonna Give You Up");
        assert_eq!(decoded.info.uri.as_deref(), Some("https://www.youtube.com/watch?v=dQw4w9WgXcQ"));
        assert_eq!(decoded.info.source_name, "youtube");
        assert_eq!(decoded.info.is_seekable, true);
    }

    #[test]
    fn test_roundtrip_preserves_all_fields() {
        let original = LavalinkTrack {
            encoded: String::new(),
            info: TrackInfo {
                identifier: "test123".to_string(),
                is_seekable: true,
                author: "TestArtist".to_string(),
                length: 180000,
                is_stream: false,
                position: 30000,
                title: "Test Song".to_string(),
                uri: Some("https://example.com/test".to_string()),
                artwork_url: None,
                isrc: None,
                source_name: "spotify".to_string(),
            },
            plugin_info: serde_json::json!({}),
            user_data: serde_json::json!({}),
        };

        let encoded = encode_track(&original).expect("Failed to encode track");
        let decoded = decode_track(&encoded).expect("Failed to decode track");

        assert_eq!(decoded.info.identifier, original.info.identifier);
        assert_eq!(decoded.info.author, original.info.author);
        assert_eq!(decoded.info.length, original.info.length);
        assert_eq!(decoded.info.is_stream, original.info.is_stream);
        assert_eq!(decoded.info.position, original.info.position);
        assert_eq!(decoded.info.title, original.info.title);
        assert_eq!(decoded.info.uri, original.info.uri);
        assert_eq!(decoded.info.source_name, original.info.source_name);
    }

    #[test]
    fn test_roundtrip_stream_track() {
        let original = LavalinkTrack {
            encoded: String::new(),
            info: TrackInfo {
                identifier: "http://example.com/stream.mp3".to_string(),
                is_seekable: false,
                author: "HTTP".to_string(),
                length: 0,
                is_stream: true,
                position: 0,
                title: "Direct Audio Stream".to_string(),
                uri: Some("http://example.com/stream.mp3".to_string()),
                artwork_url: None,
                isrc: None,
                source_name: "http".to_string(),
            },
            plugin_info: serde_json::json!({}),
            user_data: serde_json::json!({}),
        };

        let encoded = encode_track(&original).expect("Failed to encode track");
        let decoded = decode_track(&encoded).expect("Failed to decode track");

        assert_eq!(decoded.info.is_stream, true);
        assert_eq!(decoded.info.is_seekable, false);
        assert_eq!(decoded.info.length, 0);
        assert_eq!(decoded.info.source_name, "http");
    }

    #[test]
    fn test_roundtrip_no_uri() {
        let original = LavalinkTrack {
            encoded: String::new(),
            info: TrackInfo {
                identifier: "test_no_uri".to_string(),
                is_seekable: true,
                author: "Artist".to_string(),
                length: 120000,
                is_stream: false,
                position: 0,
                title: "No URI Track".to_string(),
                uri: None,
                artwork_url: None,
                isrc: None,
                source_name: "jiosaavn".to_string(),
            },
            plugin_info: serde_json::json!({}),
            user_data: serde_json::json!({}),
        };

        let encoded = encode_track(&original).expect("Failed to encode track");
        let decoded = decode_track(&encoded).expect("Failed to decode track");

        assert_eq!(decoded.info.uri, None);
        assert_eq!(decoded.info.source_name, "jiosaavn");
    }

    #[test]
    fn test_decode_invalid_base64() {
        let result = decode_track("not-valid-base64!!!");
        assert!(result.is_err());
    }

    #[test]
    fn test_decode_empty_input() {
        let result = decode_track("");
        assert!(result.is_err());
    }
}