// Copyright (c) 2026 nikcodex (KizunaLink)
// Licensed under the MIT License

//! Native SponsorBlock integration.
//!
//! Mirrors the behaviour of the `SponsorBlock-Plugin` for Lavalink v4, but with no
//! JVM dependency: we call the public SponsorBlock API directly, cache the segments
//! per video, and expose REST endpoints + WebSocket events with the same wire shape
//! as the plugin, so clients that understand the plugin work unchanged.
//!
//! API: <https://wiki.sponsorblock.dev/>

use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::common::types::AnyResult;

/// The sponsor-segment categories understood by the SponsorBlock API and clients.
pub const KNOWN_CATEGORIES: [&str; 8] = [
    "sponsor",
    "selfpromo",
    "interaction",
    "intro",
    "outro",
    "preview",
    "music_offtopic",
    "filler",
];

pub const DEFAULT_CATEGORIES: [&str; 2] = ["sponsor", "selfpromo"];

/// Default SponsorBlock API origin.
pub const DEFAULT_API_URL: &str = "https://sponsor.ajay.app";

/// A sponsored segment returned by the API. Times are in **seconds**.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Segment {
    pub segment: [f64; 2],
    pub category: String,
    #[serde(rename = "videoDuration")]
    pub video_duration: Option<f64>,
}

impl Segment {
    pub fn start_ms(&self) -> u64 {
        (self.segment[0] * 1000.0).round() as u64
    }

    pub fn end_ms(&self) -> u64 {
        (self.segment[1] * 1000.0).round() as u64
    }
}

/// Build the authoritative list of segments to skip from the raw API results.
///
/// - Filters out categories not requested by the user.
/// - Drops zero-length segments and segments that start after the track ends.
/// - Sorts by start time.
pub fn filter_segments(
    segments: Vec<Segment>,
    categories: &[String],
    duration_ms: Option<u64>,
) -> Vec<Segment> {
    let mut out: Vec<Segment> = segments
        .into_iter()
        .filter(|s| categories.iter().any(|c| c == &s.category))
        .filter(|s| s.end_ms() > s.start_ms())
        .filter(|s| duration_ms.is_none_or(|d| s.start_ms() < d))
        .collect();
    out.sort_by_key(|s| s.start_ms());
    out
}

/// A chapter derived from the SponsorBlock API's chapter data (not currently
/// part of the skip-segments response; kept for wire-compatible `ChaptersLoaded`
/// events used by clients that also speak the SponsorBlock chapter API).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Chapter {
    #[serde(default)]
    pub title: String,
    #[serde(rename = "startTime")]
    pub start_ms: u64,
    #[serde(rename = "endTime")]
    pub end_ms: u64,
}

/// If `pos_ms` falls inside any filtered segment, return the position (ms) to
/// seek to — the end of the *merged* span covering it.
///
/// Overlapping/adjacent segments are merged so we don't seek several times
/// through a long crawl of consecutive categories.
pub fn skip_position_if_in_segment(segments: &[Segment], pos_ms: u64) -> Option<u64> {
    for s in segments {
        if pos_ms >= s.start_ms() && pos_ms < s.end_ms() {
            // Merge any segments that overlap or abut this one.
            let mut end = s.end_ms();
            for o in segments {
                if o.start_ms() <= end && o.end_ms() > end {
                    end = o.end_ms();
                }
            }
            return Some(end);
        }
    }
    None
}

/// Fetch segments from the SponsorBlock API for a YouTube video id.
pub async fn fetch_segments(
    client: &reqwest::Client,
    api_url: &str,
    video_id: &str,
    categories: &[String],
) -> AnyResult<Vec<Segment>> {
    let cat_json = serde_json::to_string(&categories)?;
    let url = format!(
        "{}/api/skipSegments?videoID={}&categories={}",
        api_url.trim_end_matches('/'),
        video_id,
        urlencoding::encode(&cat_json)
    );

    let resp = client
        .get(&url)
        .timeout(Duration::from_secs(8))
        .send()
        .await?;

    match resp.status() {
        // 200: segments found.
        rs if rs.is_success() => {
            let body: Value = resp.json().await?;
            let arr = body.as_array().cloned().unwrap_or_default();
            let mut out = Vec::with_capacity(arr.len());
            for v in arr {
                if let Ok(seg) = serde_json::from_value::<Segment>(v) {
                    out.push(seg);
                }
            }
            Ok(out)
        }
        // 404: no segments for this video (normal — most videos have none).
        reqwest::StatusCode::NOT_FOUND => Ok(Vec::new()),
        other => Err(format!("SponsorBlock API returned {} for video {}", other, video_id).into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(s: f64, e: f64, cat: &str) -> Segment {
        Segment {
            segment: [s, e],
            category: cat.to_string(),
            video_duration: None,
        }
    }

    fn cats() -> Vec<String> {
        vec!["sponsor".to_string()]
    }

    #[test]
    fn filters_by_category() {
        let segments = vec![seg(0.0, 10.0, "sponsor"), seg(20.0, 30.0, "intro")];
        let filtered = filter_segments(segments, &cats(), None);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].category, "sponsor");
        assert_eq!(filtered[0].start_ms(), 0);
        assert_eq!(filtered[0].end_ms(), 10_000);
    }

    #[test]
    fn skip_inside_segment() {
        let segments = filter_segments(
            vec![seg(0.0, 10.0, "sponsor"), seg(20.0, 30.0, "sponsor")],
            &cats(),
            None,
        );
        assert_eq!(skip_position_if_in_segment(&segments, 5_000), Some(10_000));
        assert_eq!(skip_position_if_in_segment(&segments, 25_000), Some(30_000));
        assert_eq!(skip_position_if_in_segment(&segments, 15_000), None);
    }

    #[test]
    fn merges_overlapping_on_skip() {
        let segments = filter_segments(
            vec![seg(0.0, 10.0, "sponsor"), seg(5.0, 20.0, "sponsor")],
            &cats(),
            None,
        );
        // Entering at 6s skips the entire merged span 0..20s → seek to 20s.
        assert_eq!(skip_position_if_in_segment(&segments, 6_000), Some(20_000));
    }

    #[test]
    fn clamps_to_duration() {
        let segments = filter_segments(vec![seg(0.0, 100.0, "sponsor")], &cats(), Some(30_000));
        // Segment ends past duration → still returned (end clamped at play side);
        // skip target is the segment end.
        assert_eq!(skip_position_if_in_segment(&segments, 1_000), Some(100_000));
    }

    #[test]
    fn ignores_zero_or_negative() {
        let segments = filter_segments(
            vec![seg(10.0, 10.0, "sponsor"), seg(20.0, 19.0, "sponsor")],
            &cats(),
            None,
        );
        assert!(segments.is_empty());
    }

    #[test]
    fn segments_after_track_end_excluded() {
        let segments = filter_segments(
            vec![seg(40.0, 50.0, "sponsor"), seg(5.0, 8.0, "sponsor")],
            &cats(),
            Some(30_000),
        );
        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].start_ms(), 5_000);
    }
}
