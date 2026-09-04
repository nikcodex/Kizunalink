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
                if self.tracks.is_empty() {
                    return self.current.clone();
                }
                // Move current to history first
                if let Some(current) = self.current.take() {
                    self.previous.push_back(current);
                    if self.previous.len() > self.max_history {
                        self.previous.pop_front();
                    }
                }
                if let Some(track) = self.tracks.pop_front() {
                    self.tracks.push_back(track.clone());
                    Some(track)
                } else {
                    // Queue exhausted — loop back from history
                    None
                }
            }
            LoopMode::None => {
                // Move current to history first
                if let Some(current) = self.current.take() {
                    self.previous.push_back(current);
                    if self.previous.len() > self.max_history {
                        self.previous.pop_front();
                    }
                }
                self.tracks.pop_front()
            }
        }
    }

    pub fn previous_track(&mut self) -> Option<LavalinkTrack> {
        self.previous.pop_back()
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
