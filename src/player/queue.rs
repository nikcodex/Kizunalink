use crate::models::track::LavalinkTrack;
use std::collections::VecDeque;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoopMode {
    None,
    Track,
    Queue,
}

pub struct TrackQueue {
    pub tracks: VecDeque<LavalinkTrack>,
    pub current: Option<LavalinkTrack>,
    pub previous: VecDeque<LavalinkTrack>,
    pub loop_mode: LoopMode,
    max_history: usize,
}

impl Default for TrackQueue {
    fn default() -> Self {
        Self::new(50)
    }
}

impl TrackQueue {
    pub fn new(max_history: usize) -> Self {
        Self {
            tracks: VecDeque::new(),
            current: None,
            previous: VecDeque::new(),
            loop_mode: LoopMode::None,
            max_history,
        }
    }

    pub fn add(&mut self, track: LavalinkTrack) {
        self.tracks.push_back(track);
    }

    pub fn add_at(&mut self, index: usize, track: LavalinkTrack) {
        if index >= self.tracks.len() {
            self.tracks.push_back(track);
        } else {
            self.tracks.insert(index, track);
        }
    }

    pub fn next_track(&mut self) -> Option<LavalinkTrack> {
        // Check loop mode BEFORE moving current to history
        match self.loop_mode {
            LoopMode::Track => {
                // In track-loop mode, replay the current track without touching history
                if let Some(current) = &self.current {
                    return Some(current.clone());
                }
                // No current track yet — fall through to pop from queue
                self.tracks.pop_front()
            }
            LoopMode::Queue => {
                if let Some(current) = self.current.take() {
                    self.previous.push_back(current.clone());
                    if self.previous.len() > self.max_history {
                        self.previous.pop_front();
                    }
                    self.tracks.push_back(current);
                }
                self.tracks.pop_front()
            }
            LoopMode::None => {
                // Move current to history first
                if let Some(current) = self.current.take() {
                    self.push_history(current);
                }
                self.tracks.pop_front()
            }
        }
    }

    pub fn previous_track(&mut self) -> Option<LavalinkTrack> {
        self.previous.pop_back()
    }

    /// Record a finished/skipped track in the bounded played-history stack.
    pub fn push_history(&mut self, track: LavalinkTrack) {
        self.previous.push_back(track);
        if self.previous.len() > self.max_history {
            self.previous.pop_front();
        }
    }

    pub fn clear(&mut self) {
        self.tracks.clear();
        self.current = None;
        self.previous.clear();
        self.loop_mode = LoopMode::None;
    }

    pub fn remove(&mut self, index: usize) -> Option<LavalinkTrack> {
        if index < self.tracks.len() {
            self.tracks.remove(index)
        } else {
            None
        }
    }

    pub fn shuffle(&mut self) {
        let mut vec: Vec<LavalinkTrack> = self.tracks.drain(..).collect();
        use rand::seq::SliceRandom;
        let mut rng = rand::thread_rng();
        vec.shuffle(&mut rng);
        self.tracks = vec.into();
    }

    pub fn is_empty(&self) -> bool {
        self.tracks.is_empty() && self.current.is_none()
    }

    pub fn len(&self) -> usize {
        self.tracks.len() + if self.current.is_some() { 1 } else { 0 }
    }

    pub fn queue(&self) -> Vec<&LavalinkTrack> {
        self.tracks.iter().collect()
    }

    pub fn set_loop(&mut self, mode: LoopMode) {
        self.loop_mode = mode;
    }

    pub fn toggle_loop(&mut self) -> LoopMode {
        self.loop_mode = match self.loop_mode {
            LoopMode::None => LoopMode::Track,
            LoopMode::Track => LoopMode::Queue,
            LoopMode::Queue => LoopMode::None,
        };
        self.loop_mode
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::track::TrackInfo;

    fn make_test_track(id: &str) -> LavalinkTrack {
        LavalinkTrack {
            encoded: id.to_string(),
            info: TrackInfo {
                identifier: id.to_string(),
                is_seekable: true,
                author: "Test Author".to_string(),
                length: 1000,
                is_stream: false,
                position: 0,
                title: format!("Title {}", id),
                uri: Some(format!("https://example.com/{}", id)),
                artwork_url: None,
                isrc: None,
                source_name: "test".to_string(),
            },
            plugin_info: serde_json::json!({}),
            user_data: serde_json::json!({}),
        }
    }

    #[test]
    fn test_queue_loop_mode_rotates_in_order() {
        let mut queue = TrackQueue::new(10);
        queue.set_loop(LoopMode::Queue);

        let t1 = make_test_track("1");
        let t2 = make_test_track("2");
        let t3 = make_test_track("3");

        queue.add(t1);
        queue.add(t2);
        queue.add(t3);

        // First track popped from queue
        let track1 = queue.next_track().unwrap();
        assert_eq!(track1.info.identifier, "1");
        queue.current = Some(track1);

        // When track 1 finishes, track 2 should play, track 1 is requeued
        let track2 = queue.next_track().unwrap();
        assert_eq!(track2.info.identifier, "2");
        queue.current = Some(track2);

        // When track 2 finishes, track 3 should play, track 2 is requeued
        let track3 = queue.next_track().unwrap();
        assert_eq!(track3.info.identifier, "3");
        queue.current = Some(track3);

        // When track 3 finishes, track 1 should play again!
        let track1_again = queue.next_track().unwrap();
        assert_eq!(track1_again.info.identifier, "1");
        queue.current = Some(track1_again);

        // Followed by track 2 again
        let track2_again = queue.next_track().unwrap();
        assert_eq!(track2_again.info.identifier, "2");
    }

    #[test]
    fn test_push_history_is_bounded_and_lifo() {
        let mut queue = TrackQueue::new(2);
        queue.push_history(make_test_track("a"));
        queue.push_history(make_test_track("b"));
        queue.push_history(make_test_track("c"));

        // Newest first, and the oldest entry is evicted once max_history is hit.
        assert_eq!(queue.previous_track().unwrap().info.identifier, "c");
        assert_eq!(queue.previous_track().unwrap().info.identifier, "b");
        assert!(queue.previous_track().is_none());
    }
}
