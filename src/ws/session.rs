use dashmap::DashMap;
use std::collections::{HashSet, VecDeque};
use std::sync::Arc;
use std::time::Instant;
use tracing::info;

pub const DEFAULT_MAX_SESSIONS: usize = 10_000;
pub const MAX_SESSION_BUFFER_SIZE: usize = 100;

#[derive(Debug, Clone)]
pub struct SessionState {
    pub resuming: bool,
    pub timeout: u64,
    pub guild_ids: HashSet<String>,
    pub event_buffer: VecDeque<String>,
    pub connected: bool,
    pub user_id: String,
    pub last_active: Instant,
}

impl Default for SessionState {
    fn default() -> Self {
        Self {
            resuming: false,
            timeout: 60,
            guild_ids: HashSet::new(),
            event_buffer: VecDeque::new(),
            connected: true,
            user_id: String::new(),
            last_active: Instant::now(),
        }
    }
}

pub struct SessionManager {
    sessions: DashMap<String, SessionState>,
    max_sessions: usize,
    max_buffer_size: usize,
}

impl Default for SessionManager {
    fn default() -> Self {
        Self::new(DEFAULT_MAX_SESSIONS, MAX_SESSION_BUFFER_SIZE)
    }
}

impl SessionManager {
    pub fn new(max_sessions: usize, max_buffer_size: usize) -> Self {
        Self {
            sessions: DashMap::new(),
            max_sessions,
            max_buffer_size,
        }
    }

    /// Register or resume a session.
    ///
    /// If `resume_id` is provided and a session exists, attempts to resume it.
    /// Returns `(session_id, is_resumed, replay_events)`.
    pub fn handle_connection(
        &self,
        resume_id: Option<String>,
        user_id: String,
    ) -> Result<(String, bool, Vec<String>), String> {
        if let Some(id) = resume_id {
            if let Some(mut entry) = self.sessions.get_mut(&id) {
                let state = entry.value_mut();
                if state.resuming && !state.connected {
                    state.connected = true;
                    state.last_active = Instant::now();
                    if !user_id.is_empty() && user_id != "0" {
                        state.user_id = user_id;
                    }
                    let replay = state.event_buffer.drain(..).collect();
                    return Ok((id, true, replay));
                }
            }
        }

        // Create new session
        if self.sessions.len() >= self.max_sessions {
            return Err(format!(
                "Session limit reached: maximum {} active sessions allowed",
                self.max_sessions
            ));
        }

        let new_id = crate::util::uuid_v4();
        let mut initial_state = SessionState::default();
        initial_state.user_id = user_id;
        initial_state.connected = true;
        initial_state.last_active = Instant::now();

        self.sessions.insert(new_id.clone(), initial_state);
        crate::metrics::Metrics::global().active_sessions.inc();

        Ok((new_id, false, Vec::new()))
    }

    /// Add a guild subscription to a session.
    pub fn add_guild(&self, session_id: &str, guild_id: &str) {
        if let Some(mut entry) = self.sessions.get_mut(session_id) {
            entry.guild_ids.insert(guild_id.to_string());
            entry.last_active = Instant::now();
        }
    }

    /// Remove a guild subscription from a session.
    pub fn remove_guild(&self, session_id: &str, guild_id: &str) {
        if let Some(mut entry) = self.sessions.get_mut(session_id) {
            entry.guild_ids.remove(guild_id);
            entry.last_active = Instant::now();
        }
    }

    /// Check if a session is subscribed to a guild.
    pub fn is_guild_subscribed(&self, session_id: &str, guild_id: &str) -> bool {
        self.sessions
            .get(session_id)
            .map(|s| s.guild_ids.contains(guild_id))
            .unwrap_or(false)
    }

    /// Update session configuration (resuming and timeout).
    pub fn update_session(
        &self,
        session_id: &str,
        resuming: Option<bool>,
        timeout: Option<u64>,
    ) -> (bool, u64) {
        let mut entry = self.sessions.entry(session_id.to_string()).or_default();
        if let Some(r) = resuming {
            entry.resuming = r;
        }
        if let Some(t) = timeout {
            entry.timeout = t;
        }
        entry.last_active = Instant::now();
        (entry.resuming, entry.timeout)
    }

    /// Returns list of all known session IDs.
    pub fn get_session_ids(&self) -> Vec<String> {
        self.sessions.iter().map(|e| e.key().clone()).collect()
    }

    /// Get session state summary.
    pub fn get_session(&self, session_id: &str) -> Option<(bool, u64)> {
        self.sessions.get(session_id).map(|s| (s.resuming, s.timeout))
    }

    /// Buffer an event for disconnected sessions that are subscribed to the guild (or global events).
    pub fn buffer_event(&self, guild_id: Option<&str>, msg: &str) {
        for mut entry in self.sessions.iter_mut() {
            let state = entry.value_mut();
            if !state.connected && state.resuming {
                let should_buffer = match guild_id {
                    Some(gid) => state.guild_ids.contains(gid),
                    None => true,
                };
                if should_buffer {
                    if state.event_buffer.len() >= self.max_buffer_size {
                        state.event_buffer.pop_front();
                    }
                    state.event_buffer.push_back(msg.to_string());
                }
            }
        }
    }

    /// Mark a session as disconnected.
    ///
    /// If resuming is configured and timeout > 0, returns `Some(timeout_seconds)`.
    /// Otherwise, immediately removes the session and returns `None`.
    pub fn mark_disconnected(&self, session_id: &str) -> Option<u64> {
        let (resuming, timeout) = {
            if let Some(mut entry) = self.sessions.get_mut(session_id) {
                entry.connected = false;
                entry.last_active = Instant::now();
                (entry.resuming, entry.timeout)
            } else {
                return None;
            }
        };

        if resuming && timeout > 0 {
            Some(timeout)
        } else {
            self.remove_session(session_id);
            None
        }
    }

    /// Remove a session completely and decrement metrics.
    pub fn remove_session(&self, session_id: &str) -> bool {
        if self.sessions.remove(session_id).is_some() {
            crate::metrics::Metrics::global().active_sessions.dec();
            info!("Session {} cleaned up", session_id);
            true
        } else {
            false
        }
    }

    /// Check if a disconnected session has expired, and remove it if so.
    pub fn expire_if_disconnected(&self, session_id: &str) -> bool {
        let should_remove = self
            .sessions
            .get(session_id)
            .map(|s| !s.connected)
            .unwrap_or(false);

        if should_remove {
            self.remove_session(session_id)
        } else {
            false
        }
    }

    /// Periodic cleanup of stale expired sessions.
    pub fn cleanup_stale(&self) {
        let mut expired = Vec::new();
        for entry in self.sessions.iter() {
            let s = entry.value();
            if !s.connected {
                let ttl = std::time::Duration::from_secs(s.timeout.max(60) * 2);
                if s.last_active.elapsed() > ttl {
                    expired.push(entry.key().clone());
                }
            }
        }
        for id in expired {
            self.remove_session(&id);
        }
    }

    /// Count total registered sessions.
    pub fn count_sessions(&self) -> usize {
        self.sessions.len()
    }
}
