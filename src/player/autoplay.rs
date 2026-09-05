use crate::models::track::LavalinkTrack;
use crate::sources::jiosaavn::JioSaavnSource;
use crate::sources::spotify::SpotifySource;
use crate::sources::youtube::YouTubeSource;
use std::collections::{HashSet, VecDeque};
use tracing::info;

pub struct AutoplayEngine {
    pub enabled: bool,
    recent_tracks: VecDeque<String>,
    played_ids: HashSet<String>,
    max_history: usize,
    max_played: usize,
}

impl AutoplayEngine {
    pub fn new() -> Self {
        Self {
            enabled: false,
            recent_tracks: VecDeque::new(),
            played_ids: HashSet::new(),
            max_history: 20,
            max_played: 50,
        }
    }
}

impl Default for AutoplayEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl AutoplayEngine {
    pub fn enable(&mut self) {
        self.enabled = true;
    }

    pub fn disable(&mut self) {
        self.enabled = false;
    }

    pub fn toggle(&mut self) -> bool {
        self.enabled = !self.enabled;
        self.enabled
    }

    pub fn record_track(&mut self, track: &LavalinkTrack) {
        let title_lower = format!(
            "{} {}",
            track.info.title.to_lowercase(),
            track.info.author.to_lowercase()
        );
        self.recent_tracks.push_back(title_lower);
        if self.recent_tracks.len() > self.max_history {
            self.recent_tracks.pop_front();
        }
        self.played_ids.insert(track.info.identifier.clone());
        if self.played_ids.len() > self.max_played {
            let oldest: Vec<String> = self.played_ids.iter().take(10).cloned().collect();
            for id in &oldest {
                self.played_ids.remove(id);
            }
        }
    }

    pub fn clear(&mut self) {
        self.recent_tracks.clear();
        self.played_ids.clear();
    }

    pub async fn get_recommendation(
        &self,
        last_track: &LavalinkTrack,
        jiosaavn: &JioSaavnSource,
        youtube: &YouTubeSource,
        spotify: &SpotifySource,
    ) -> Option<LavalinkTrack> {
        let artist = &last_track.info.author;
        let queries = self.build_queries(artist, &last_track.info.title);
        let source = &last_track.info.source_name;
        let duration = last_track.info.length;

        let mut candidates: Vec<(LavalinkTrack, f64)> = Vec::new();

        for query in &queries {
            let tracks = match source.as_str() {
                "spotify" => spotify.search(query, 10).await.unwrap_or_default(),
                "youtube" => youtube.search(query, 10).await.unwrap_or_default(),
                _ => jiosaavn.search(query, 10).await.unwrap_or_default(),
            };

            for track in tracks {
                if self.played_ids.contains(&track.info.identifier) {
                    continue;
                }

                let normalized =
                    normalize_title(&format!("{} {}", track.info.title, track.info.author));
                if self.is_duplicate(&normalized) {
                    continue;
                }

                let score = self.score_track(&track, artist, duration, source);
                if score >= 40.0 {
                    candidates.push((track, score));
                }
            }

            if !candidates.is_empty() {
                break;
            }
        }

        candidates.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        if let Some((best, score)) = candidates.into_iter().next() {
            info!(
                "Autoplay picked: {} by {} (score: {:.1})",
                best.info.title, best.info.author, score
            );
            Some(best)
        } else {
            self.fallback_search(jiosaavn, youtube, spotify, artist)
                .await
        }
    }

    fn build_queries(&self, artist: &str, title: &str) -> Vec<String> {
        let main_artist = get_main_artist(artist);
        let title_words: Vec<&str> = title.split_whitespace().take(3).collect();
        let title_prefix = title_words.join(" ");

        vec![
            main_artist.to_string(),
            format!("{} popular", main_artist),
            format!("{} mix", main_artist),
            title_prefix,
        ]
    }

    fn score_track(
        &self,
        track: &LavalinkTrack,
        original_artist: &str,
        original_duration: u64,
        original_source: &str,
    ) -> f64 {
        let mut score = 0.0f64;

        if are_artists_related(&track.info.author, original_artist) {
            score += 60.0;
        } else {
            score -= 30.0;
        }

        if original_duration > 0 && track.info.length > 0 {
            let ratio = if track.info.length > original_duration {
                original_duration as f64 / track.info.length as f64
            } else {
                track.info.length as f64 / original_duration as f64
            };
            score += ratio * 20.0;
        }

        if track.info.source_name == original_source {
            score += 15.0;
        }

        score
    }

    fn is_duplicate(&self, normalized: &str) -> bool {
        for recent in &self.recent_tracks {
            if levenshtein_ratio(normalized, recent) > 0.75 {
                return true;
            }
        }
        false
    }

    async fn fallback_search(
        &self,
        jiosaavn: &JioSaavnSource,
        youtube: &YouTubeSource,
        spotify: &SpotifySource,
        artist: &str,
    ) -> Option<LavalinkTrack> {
        let main_artist = get_main_artist(artist);

        let tracks = match jiosaavn.search(&main_artist, 5).await {
            Ok(t) if !t.is_empty() => t,
            _ => match youtube.search(&main_artist, 5).await {
                Ok(t) if !t.is_empty() => t,
                _ => match spotify.search(&main_artist, 5).await {
                    Ok(t) => t,
                    Err(_) => return None,
                },
            },
        };

        for track in tracks {
            if !self.played_ids.contains(&track.info.identifier) {
                let normalized =
                    normalize_title(&format!("{} {}", track.info.title, track.info.author));
                if !self.is_duplicate(&normalized) {
                    return Some(track);
                }
            }
        }

        None
    }
}

fn normalize_title(title: &str) -> String {
    let lower = title.to_lowercase();
    let cleaned: String = lower
        .chars()
        .filter(|c| c.is_alphanumeric() || c.is_whitespace())
        .collect();

    let stops = [
        "official",
        "music video",
        "audio",
        "lyrics",
        "hd",
        "4k",
        "remix",
        "feat",
        "ft",
        "explicit",
        "clean",
    ];
    let mut result = cleaned.clone();
    for stop in &stops {
        result = result.replace(stop, "");
    }
    result.split_whitespace().collect::<Vec<&str>>().join(" ")
}

fn get_main_artist(artist: &str) -> String {
    let separators = [" feat ", " ft ", " & ", " x ", " vs ", " versus "];
    for sep in &separators {
        if let Some(idx) = find_ci_ascii(artist, sep) {
            return artist[..idx].trim().to_string();
        }
    }
    artist.to_string()
}

/// Case-insensitive search for an ASCII `needle`, returning a byte offset in
/// `haystack` that is guaranteed to be a char boundary.
///
/// The previous implementation searched the *lowercased* string and then sliced
/// the original at that byte offset. `str::to_lowercase` can change a string's
/// byte length (e.g. `İ` becomes two chars), so the offset was not necessarily a
/// boundary in the original — slicing there panics, and artist names come from
/// remote track metadata.
fn find_ci_ascii(haystack: &str, needle: &str) -> Option<usize> {
    let needle_chars: Vec<char> = needle.chars().map(|c| c.to_ascii_lowercase()).collect();
    let chars: Vec<(usize, char)> = haystack.char_indices().collect();
    if needle_chars.is_empty() || chars.len() < needle_chars.len() {
        return None;
    }
    for start in 0..=chars.len() - needle_chars.len() {
        let matches = chars[start..start + needle_chars.len()]
            .iter()
            .zip(&needle_chars)
            .all(|((_, hc), nc)| hc.to_ascii_lowercase() == *nc);
        if matches {
            return Some(chars[start].0);
        }
    }
    None
}

fn are_artists_related(artist1: &str, artist2: &str) -> bool {
    let a1 = get_main_artist(artist1).to_lowercase();
    let a2 = get_main_artist(artist2).to_lowercase();

    if a1 == a2 {
        return true;
    }

    levenshtein_ratio(&a1, &a2) > 0.85
}

fn levenshtein_ratio(a: &str, b: &str) -> f64 {
    let a_chars: Vec<char> = a.chars().collect();
    let b_chars: Vec<char> = b.chars().collect();
    let a_len = a_chars.len();
    let b_len = b_chars.len();

    if a_len == 0 && b_len == 0 {
        return 1.0;
    }
    if a_len == 0 || b_len == 0 {
        return 0.0;
    }

    let mut matrix = vec![vec![0usize; b_len + 1]; a_len + 1];

    for (i, row) in matrix.iter_mut().enumerate() {
        row[0] = i;
    }
    for (j, cell) in matrix[0].iter_mut().enumerate() {
        *cell = j;
    }

    for i in 1..=a_len {
        for j in 1..=b_len {
            let cost = if a_chars[i - 1] == b_chars[j - 1] {
                0
            } else {
                1
            };
            matrix[i][j] = (matrix[i - 1][j] + 1)
                .min(matrix[i][j - 1] + 1)
                .min(matrix[i - 1][j - 1] + cost);
        }
    }

    let max_len = a_len.max(b_len);
    1.0 - (matrix[a_len][b_len] as f64 / max_len as f64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::track::{LavalinkTrack, TrackInfo};

    fn track(identifier: &str, title: &str, author: &str) -> LavalinkTrack {
        LavalinkTrack {
            encoded: format!("encoded-{}", identifier),
            info: TrackInfo {
                identifier: identifier.to_string(),
                is_seekable: true,
                author: author.to_string(),
                length: 200_000,
                is_stream: false,
                position: 0,
                title: title.to_string(),
                uri: Some(format!("https://example.test/{}", identifier)),
                artwork_url: None,
                isrc: None,
                source_name: "youtube".to_string(),
            },
            plugin_info: serde_json::json!({}),
            user_data: serde_json::Value::Null,
        }
    }

    #[test]
    fn test_get_main_artist_strips_feature_separators() {
        // "feat." (with a dot) is not a separator, so the name is left alone.
        let dotted = get_main_artist("Artist feat. Someone");
        assert_eq!(dotted, "Artist feat. Someone");
        assert_eq!(get_main_artist("Artist feat Someone"), "Artist");
        assert_eq!(get_main_artist("Artist FEAT Someone"), "Artist");
        assert_eq!(get_main_artist("Artist & Other"), "Artist");
        assert_eq!(get_main_artist("Artist x Other"), "Artist");
        assert_eq!(get_main_artist("Artist versus Other"), "Artist");
        assert_eq!(get_main_artist("Plain Artist"), "Plain Artist");
    }

    #[test]
    fn test_get_main_artist_survives_multibyte_artist_names() {
        // U+212A (KELVIN SIGN) lowercases to a single ASCII "k", shrinking the
        // lowercased copy by two bytes. The old implementation searched that
        // lowercased copy and then sliced the *original* at the resulting byte
        // offset, which landed inside the 3-byte euro sign below and panicked
        // with "byte index 4 is not a char boundary".
        let kelvin = "\u{212A}\u{20AC} feat Someone";
        assert_eq!(get_main_artist(kelvin), "\u{212A}\u{20AC}");

        // U+0130 lowercases to two chars, growing the lowercased copy: the old
        // offset pointed one byte too far and produced a mangled artist name.
        assert_eq!(get_main_artist("İstanbul feat Someone"), "İstanbul");

        let cjk = "アーティスト feat 誰か";
        assert_eq!(get_main_artist(cjk), "アーティスト");

        let emoji = "🎧 Artist x DJ";
        assert_eq!(get_main_artist(emoji), "🎧 Artist");
    }

    #[test]
    fn test_find_ci_ascii_returns_char_boundaries() {
        assert_eq!(find_ci_ascii("aBc", "b"), Some(1));
        assert_eq!(find_ci_ascii("Artist FT Other", " ft "), Some(6));
        assert_eq!(find_ci_ascii("no match", " ft "), None);
        assert_eq!(find_ci_ascii("", " ft "), None);
        assert_eq!(find_ci_ascii("anything", ""), None);

        // Offset must index the start of a char, never its interior.
        let idx = find_ci_ascii("日本語 FT other", " ft ").expect("match");
        assert_eq!(&"日本語 FT other"[..idx], "日本語");
    }

    #[test]
    fn test_record_track_bounds_history_and_played_ids() {
        let mut engine = AutoplayEngine::new();
        for i in 0..80 {
            let played = track(&format!("id-{}", i), &format!("Song {}", i), "Artist");
            engine.record_track(&played);
        }
        // Both collections stay bounded regardless of how many tracks are played.
        assert!(engine.recent_tracks.len() <= 20);
        assert!(engine.played_ids.len() <= 50);
        assert!(!engine.played_ids.is_empty());
        // The most recent title is always retained in the ring buffer.
        let recent = "song 79 artist".to_string();
        assert!(engine.recent_tracks.contains(&recent));

        engine.clear();
        assert!(engine.recent_tracks.is_empty());
        assert!(engine.played_ids.is_empty());
    }

    #[test]
    fn test_is_duplicate_matches_recent_tracks() {
        let mut engine = AutoplayEngine::new();
        engine.record_track(&track("a", "Song Title (Official Music Video)", "Artist"));

        // One edit away from the recorded title => treated as a duplicate.
        let near_duplicate = "song title (official music video) artis";
        assert!(engine.is_duplicate(near_duplicate));
        // Completely unrelated input is not a duplicate.
        assert!(!engine.is_duplicate("zzz qqq xxx www vvv"));
    }

    #[test]
    fn test_normalize_title_strips_stop_words() {
        assert_eq!(normalize_title("Song  [Official Video] 4K"), "song video");
        assert_eq!(normalize_title("Song (Lyrics)"), "song");
    }

    #[test]
    fn test_levenshtein_ratio_edge_cases() {
        assert_eq!(levenshtein_ratio("", ""), 1.0);
        assert_eq!(levenshtein_ratio("abc", ""), 0.0);
        assert_eq!(levenshtein_ratio("", "abc"), 0.0);
        assert_eq!(levenshtein_ratio("abc", "abc"), 1.0);
        assert_eq!(levenshtein_ratio("abc", "abd"), 1.0 - 1.0 / 3.0);
        // Multi-byte input must be measured in chars, not bytes.
        assert_eq!(levenshtein_ratio("日本語", "日本語"), 1.0);
    }

    #[test]
    fn test_are_artists_related() {
        assert!(are_artists_related("Artist", "artist"));
        assert!(are_artists_related(
            "Artist feat Someone",
            "Artist ft Other"
        ));
        assert!(are_artists_related("Artist", "Artists"));
        assert!(!are_artists_related("Artist", "Completely Different"));
    }

    #[test]
    fn test_score_track_prefers_related_artist_and_matching_source() {
        let engine = AutoplayEngine::new();
        let candidate = track("c", "Song", "Artist");

        let related = engine.score_track(&candidate, "Artist", 200_000, "youtube");
        assert!(related >= 60.0);

        // Unrelated artist is penalised below the 40.0 acceptance threshold.
        let other_artist = "Someone Else";
        let unrelated = engine.score_track(&candidate, other_artist, 200_000, "youtube");
        assert!(unrelated < 40.0);
    }

    #[test]
    fn test_enable_disable_toggle() {
        let mut engine = AutoplayEngine::new();
        assert!(!engine.enabled);
        assert!(engine.toggle());
        engine.enable();
        assert!(engine.enabled);
        engine.disable();
        assert!(!engine.enabled);
        assert!(engine.toggle());
    }
}
