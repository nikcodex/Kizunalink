// Copyright (c) 2026 nikcodex (KizunaLink)
// Licensed under the MIT License

use std::sync::Arc;

use tokio::sync::RwLock;

use crate::{
    common::types::GuildId,
    lavalink::{
        protocol::{self, events::KizunaLinkEvent, tracks::Track},
        sponsorblock::{self, Segment, filter_segments, skip_position_if_in_segment},
    },
};

/// Holds the SponsorBlock segments for the current track, plus a guard so the
/// monitor only performs one skip transition at a time.
#[derive(Debug, Default)]
pub struct SponsorBlockState {
    pub segments: RwLock<Vec<Segment>>,
    pub loaded_once: std::sync::atomic::AtomicBool,
    pub last_skip_ms: std::sync::atomic::AtomicU64,
}

impl SponsorBlockState {
    pub fn new() -> Self {
        Self::default()
    }
}

/// Asynchronously fetch segments for a YouTube `video_id` when SponsorBlock is
/// enabled, emit `SegmentsLoaded`, and store them for the monitor to consult.
pub async fn load_segments(
    state: &Arc<SponsorBlockState>,
    config: &crate::config::player::SponsorBlockConfig,
    guild_id: &GuildId,
    track: &Track,
    session: &dyn crate::common::server_hooks::SessionContext,
) {
    // Only YouTube videos have sponsor segments.
    let Some(video_id) = extract_youtube_video_id(track) else {
        return;
    };

    let categories = config.categories.clone();
    let api_url = config.api_url.clone();

    let duration_ms = (track.info.length > 0).then_some(track.info.length);
    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(4))
        .timeout(std::time::Duration::from_secs(8))
        .build()
        .unwrap_or_default();
    let segments =
        match sponsorblock::fetch_segments(&client, &api_url, &video_id, &categories).await {
            Ok(segs) => filter_segments(segs, &categories, duration_ms),
            Err(e) => {
                tracing::warn!("SponsorBlock fetch failed for {}: {}", video_id, e);
                return;
            }
        };

    {
        let mut guard = state.segments.write().await;
        *guard = segments.clone();
    }
    state
        .loaded_once
        .store(true, std::sync::atomic::Ordering::Release);

    session.send_message(&protocol::OutgoingMessage::Event {
        event: Box::new(KizunaLinkEvent::SegmentsLoaded {
            guild_id: guild_id.clone(),
            track: track.clone(),
            segments,
        }),
    });
}

/// Check the current playback position against SponsorBlock segments; if it's
/// inside a segment, emit `SegmentSkipped` and return the seek target (or None).
/// The monitor performs the actual seek.
pub async fn check_for_skip(
    state: &SponsorBlockState,
    guild_id: &GuildId,
    pos_ms: u64,
    session: &dyn crate::common::server_hooks::SessionContext,
) -> Option<u64> {
    let segments = state.segments.read().await.clone();
    // Avoid re-seeking right after we already moved (the monitor position may
    // still read the pre-seek value for one tick).
    let last_seek = state
        .last_skip_ms
        .load(std::sync::atomic::Ordering::Relaxed);
    if pos_ms < last_seek.saturating_sub(50) {
        return None;
    }

    if let Some(target_ms) = skip_position_if_in_segment(&segments, pos_ms) {
        state
            .last_skip_ms
            .store(target_ms, std::sync::atomic::Ordering::Relaxed);

        let segment = segments
            .into_iter()
            .find(|s| pos_ms >= s.start_ms() && pos_ms < s.end_ms());

        if let Some(seg) = segment {
            session.send_message(&protocol::OutgoingMessage::Event {
                event: Box::new(KizunaLinkEvent::SegmentSkipped {
                    guild_id: guild_id.clone(),
                    segment: seg,
                }),
            });
        }
        Some(target_ms)
    } else {
        None
    }
}

/// Extract `{videoId}` from a track's identifier or URI if it's a YouTube link.
pub fn extract_youtube_video_id(track: &Track) -> Option<String> {
    let haystack = [
        track.info.uri.as_deref(),
        Some(track.info.identifier.as_str()),
    ]
    .into_iter()
    .flatten();

    for s in haystack {
        // youtube.com/watch?v=ID  | youtu.be/ID  | shorts/ID
        if let Some(idx) = s.find("v=") {
            let id_candidate = &s[idx + 2..];
            let id = id_candidate
                .split(['&', '/', '?', '#'])
                .next()
                .unwrap_or("");
            if is_video_id(id) {
                return Some(id.to_string());
            }
        }
        for prefix in ["youtu.be/", "youtube.com/shorts/", "youtube.com/embed/"] {
            if let Some(stripped) = s.split_once(prefix).map(|(_, rest)| rest) {
                let id = stripped.split(['&', '/', '?', '#']).next().unwrap_or("");
                if is_video_id(id) {
                    return Some(id.to_string());
                }
            }
        }
    }
    None
}

fn is_video_id(id: &str) -> bool {
    id.len() == 11
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track_with_uri(uri: Option<String>) -> Track {
        Track {
            encoded: String::new(),
            info: crate::lavalink::protocol::tracks::TrackInfo {
                identifier: "abc".to_string(),
                is_seekable: true,
                author: String::new(),
                length: 120_000,
                is_stream: false,
                position: 0,
                title: String::new(),
                uri,
                artwork_url: None,
                isrc: None,
                source_name: "youtube".to_string(),
            },
            plugin_info: serde_json::json!({}),
            user_data: serde_json::json!({}),
        }
    }

    #[test]
    fn extracts_youtube_id_from_uri() {
        let t = track_with_uri(Some("https://www.youtube.com/watch?v=dQw4w9WgXcQ".into()));
        assert_eq!(extract_youtube_video_id(&t), Some("dQw4w9WgXcQ".into()));
    }

    #[test]
    fn extracts_youtu_be_id() {
        let t = track_with_uri(Some("https://youtu.be/dQw4w9WgXcQ?t=30".into()));
        assert_eq!(extract_youtube_video_id(&t), Some("dQw4w9WgXcQ".into()));
    }

    #[test]
    fn no_id_for_non_youtube() {
        let t = track_with_uri(Some("https://open.spotify.com/track/xyz".into()));
        assert_eq!(extract_youtube_video_id(&t), None);
    }

    #[test]
    fn no_id_for_raw_search_identifier() {
        let t = track_with_uri(None); // no uri
        assert_eq!(extract_youtube_video_id(&t), None);
    }
}
