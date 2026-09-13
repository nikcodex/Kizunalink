// Copyright (c) 2026 nikcodex (KizunaLink)
// Licensed under the MIT License

use std::{ops::Deref, sync::Arc};

use rand::{Rng, distributions::Alphanumeric};
use tokio::sync::{Mutex, RwLock};

/// A thread-safe, mutually exclusive shared component.
pub type Shared<T> = Arc<Mutex<T>>;

/// A thread-safe, read-write shared component.
pub type SharedRw<T> = Arc<RwLock<T>>;

/// A generic boxed error type.
pub type AnyError = Box<dyn std::error::Error + Send + Sync>;

/// A convenient Result alias returning `AnyError`.
pub type AnyResult<T> = std::result::Result<T, AnyError>;

/// Strongly typed identifiers.
macro_rules! define_id {
    ($name:ident, $type:ty) => {
        #[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub $type);

        impl From<$type> for $name {
            fn from(val: $type) -> Self {
                Self(val)
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "{}", self.0)
            }
        }
    };
    ($name:ident, $type:ty, copy) => {
        #[derive(
            Debug,
            Clone,
            Copy,
            PartialEq,
            Eq,
            PartialOrd,
            Ord,
            Hash,
            serde::Serialize,
            serde::Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(pub $type);

        impl From<$type> for $name {
            fn from(val: $type) -> Self {
                Self(val)
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "{}", self.0)
            }
        }
    };
}

define_id!(GuildId, String);
define_id!(SessionId, String);
define_id!(UserId, u64, copy);
define_id!(ChannelId, u64, copy);

impl Deref for GuildId {
    type Target = str;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Deref for SessionId {
    type Target = str;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl SessionId {
    /// Generates a random 16-character alphanumeric session ID (a-z, 0-9).
    pub fn generate() -> Self {
        let rng = rand::thread_rng();
        let s: String = rng
            .sample_iter(&Alphanumeric)
            .filter(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
            .take(16)
            .map(char::from)
            .collect();
        Self(s)
    }
}

/// Supported audio formats and containers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum AudioFormat {
    Aac,
    Opus,
    Webm,
    Mp4,
    Mp3,
    Ogg,
    Flac,
    Wav,
    Unknown,
}

impl AudioFormat {
    pub fn as_ext(&self) -> &'static str {
        match self {
            Self::Aac => "aac",
            Self::Opus => "opus",
            Self::Webm => "webm",
            Self::Mp4 => "mp4",
            Self::Mp3 => "mp3",
            Self::Ogg => "ogg",
            Self::Flac => "flac",
            Self::Wav => "wav",
            Self::Unknown => "",
        }
    }

    pub fn from_ext(ext: &str) -> Self {
        match ext.to_lowercase().as_str() {
            "aac" => Self::Aac,
            "opus" => Self::Opus,
            "webm" => Self::Webm,
            "mp4" | "m4a" => Self::Mp4,
            "mp3" => Self::Mp3,
            "ogg" => Self::Ogg,
            "flac" => Self::Flac,
            "wav" => Self::Wav,
            _ => Self::Unknown,
        }
    }

    /// Detects the audio format from a URL, often using hints like 'itag' or 'mime'.
    pub fn from_url(url: &str) -> Self {
        if url.contains(".m3u8") || url.contains("/playlist") {
            return Self::Aac;
        }

        // Handle YouTube itag hint
        if let Some(itag) = extract_youtube_itag(url) {
            match itag {
                249..=251 => return Self::Webm,
                139..=141 => return Self::Mp4,
                _ => {}
            }
        }

        if url.contains("mime=audio%2Fwebm") || url.contains("mime=audio/webm") {
            return Self::Webm;
        }
        if url.contains("mime=audio%2Fmp4") || url.contains("mime=audio/mp4") {
            return Self::Mp4;
        }

        let from_path = url
            .split('?')
            .next()
            .and_then(|path| std::path::Path::new(path).extension())
            .and_then(|ext| ext.to_str())
            .map(Self::from_ext)
            .unwrap_or(Self::Unknown);

        if from_path != Self::Unknown {
            return from_path;
        }

        // Final fallback: look for extensions anywhere in the URL (Tidal etc sometimes use it)
        if url.contains(".mp4") || url.contains(".m4a") {
            return Self::Mp4;
        }
        if url.contains(".flac") {
            return Self::Flac;
        }
        if url.contains(".mp3") {
            return Self::Mp3;
        }
        if url.contains(".ogg") {
            return Self::Ogg;
        }
        if url.contains(".webm") {
            return Self::Webm;
        }

        Self::Unknown
    }

    /// Returns true if the format can potentially be passed through without re-encoding.
    pub fn is_opus_passthrough(&self) -> bool {
        matches!(self, Self::Webm | Self::Ogg | Self::Opus)
    }
}

fn extract_youtube_itag(url: &str) -> Option<u32> {
    url.split('?').nth(1)?.split('&').find_map(|pair| {
        let (k, v) = pair.split_once('=')?;
        if k == "itag" { v.parse().ok() } else { None }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audio_format_from_ext() {
        assert_eq!(AudioFormat::from_ext("mp3"), AudioFormat::Mp3);
        assert_eq!(AudioFormat::from_ext("opus"), AudioFormat::Opus);
        assert_eq!(AudioFormat::from_ext("webm"), AudioFormat::Webm);
        assert_eq!(AudioFormat::from_ext("m4a"), AudioFormat::Mp4);
        assert_eq!(AudioFormat::from_ext("flac"), AudioFormat::Flac);
        assert_eq!(AudioFormat::from_ext("wav"), AudioFormat::Wav);
        assert_eq!(AudioFormat::from_ext("xyz"), AudioFormat::Unknown);
    }

    #[test]
    fn audio_format_from_url() {
        assert_eq!(
            AudioFormat::from_url("https://example.com/song.mp3"),
            AudioFormat::Mp3
        );
        assert_eq!(
            AudioFormat::from_url("https://example.com/stream.webm"),
            AudioFormat::Webm
        );
        assert_eq!(
            AudioFormat::from_url("https://example.com/playlist.m3u8"),
            AudioFormat::Aac
        );
    }

    #[test]
    fn audio_format_youtube_itag() {
        // itag 251 = webm opus
        assert_eq!(
            AudioFormat::from_url("https://googlevideo.com/videoplayback?itag=251"),
            AudioFormat::Webm
        );
        // itag 140 = mp4 aac
        assert_eq!(
            AudioFormat::from_url("https://googlevideo.com/videoplayback?itag=140"),
            AudioFormat::Mp4
        );
    }

    #[test]
    fn audio_format_is_opus_passthrough() {
        assert!(AudioFormat::Webm.is_opus_passthrough());
        assert!(AudioFormat::Ogg.is_opus_passthrough());
        assert!(AudioFormat::Opus.is_opus_passthrough());
        assert!(!AudioFormat::Mp3.is_opus_passthrough());
        assert!(!AudioFormat::Mp4.is_opus_passthrough());
    }

    #[test]
    fn audio_format_as_ext_roundtrip() {
        for fmt in [
            AudioFormat::Aac,
            AudioFormat::Opus,
            AudioFormat::Webm,
            AudioFormat::Mp4,
            AudioFormat::Mp3,
            AudioFormat::Ogg,
            AudioFormat::Flac,
            AudioFormat::Wav,
        ] {
            let ext = fmt.as_ext();
            assert_eq!(
                AudioFormat::from_ext(ext),
                fmt,
                "roundtrip failed for {ext}"
            );
        }
    }

    #[test]
    fn session_id_generate_length() {
        let id = SessionId::generate();
        assert_eq!(id.0.len(), 16);
        assert!(
            id.0.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        );
    }
}
