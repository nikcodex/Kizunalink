// Copyright (c) 2026 nikcodex (KizunaLink)
// Licensed under the MIT License

pub mod decode;
pub mod encode;
pub mod io;

pub use decode::{decode_playlist_info, decode_track};
pub use encode::{encode_playlist_info, encode_track};

#[cfg(test)]
mod tests {
    use super::{decode_playlist_info, decode_track};
    use crate::lavalink::protocol::PlaylistInfo;

    /// Captured from `lavalink-devs/lavalink` protocol round-trip tests (v4.2.2).
    ///
    /// Layout (lavaplayer `MessageOutput` v2, little-endian string framing):
    /// header u32+flags, version byte, title, author, length, identifier,
    /// isStream, uri(v2), artworkUrl(v2), isrc(v2), source name, position.
    const LAVALINK_4_2_2_RICK_ASTLEY: &str = "QAAAjQIAJVJpY2sgQXN0bGV5IC0gTmV2ZXIgR29ubmEgR2l2ZSBZb3UgVXAADlJpY2tBc3RsZXlWRVZPAAAAAAADPCAAC2RRdzR3OVdnWGNRAAEAK2h0dHBzOi8vd3d3LnlvdXR1YmUuY29tL3dhdGNoP3Y9ZFF3NHc5V2dYY1EAB3lvdXR1YmUAAAAAAAAAAA==";

    #[test]
    fn decodes_an_actual_lavalink_4_2_2_track_string() {
        let track =
            decode_track(LAVALINK_4_2_2_RICK_ASTLEY).expect("real lavaplayer v2 track decodes");

        assert_eq!(track.info.title, "Rick Astley - Never Gonna Give You Up");
        assert_eq!(track.info.author, "RickAstleyVEVO");
        assert_eq!(track.info.length, 212_000);
        assert_eq!(track.info.identifier, "dQw4w9WgXcQ");
        assert!(!track.info.is_stream, "track is not a live stream");
        assert!(track.info.is_seekable, "non-stream tracks are seekable");
        assert_eq!(
            track.info.uri.as_deref(),
            Some("https://www.youtube.com/watch?v=dQw4w9WgXcQ")
        );
        assert_eq!(track.info.source_name, "youtube");
        assert_eq!(track.info.position, 0);
    }

    #[test]
    fn round_trips_an_encoded_track() {
        let track = decode_track(LAVALINK_4_2_2_RICK_ASTLEY).expect("decodes first for round-trip");
        let info = track.info.clone();
        let re_encoded = super::encode::encode_track(&info, &track.user_data).expect("re-encodes");
        let reparsed = decode_track(&re_encoded).expect("round-trip decode succeeds");

        assert_eq!(reparsed.info.title, track.info.title);
        assert_eq!(reparsed.info.author, track.info.author);
        assert_eq!(reparsed.info.identifier, track.info.identifier);
        assert_eq!(reparsed.info.uri, track.info.uri);
        assert_eq!(reparsed.info.length, track.info.length);
        assert_eq!(reparsed.info.source_name, track.info.source_name);
    }

    /// `encode_playlist_info` / `decode_playlist_info` round-trip.
    #[test]
    fn round_trips_playlist_info() {
        let info = PlaylistInfo {
            name: "My Playlist".to_string(),
            selected_track: -1,
        };

        let encoded = super::encode::encode_playlist_info(&info).expect("playlist info encodes");
        let reparsed = decode_playlist_info(&encoded).expect("playlist info round-trips");
        assert_eq!(reparsed.name, "My Playlist");
        assert_eq!(reparsed.selected_track, -1);
    }
}
