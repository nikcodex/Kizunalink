use dashmap::DashMap;
use std::collections::{HashSet, VecDeque};
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
        let initial_state = SessionState {
            user_id,
            connected: true,
            last_active: Instant::now(),
            ..Default::default()
        };

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

    /// Get all subscribed guild IDs for a session.
    pub fn get_session_guilds(&self, session_id: &str) -> HashSet<String> {
        self.sessions
            .get(session_id)
            .map(|s| s.guild_ids.clone())
            .unwrap_or_default()
    }

    /// Update session configuration (resuming and timeout).
    ///
    /// Returns `None` if the session does not exist — this method NEVER creates a
    /// session implicitly. Only the WebSocket connection flow may create sessions.
    pub fn update_session(
        &self,
        session_id: &str,
        resuming: Option<bool>,
        timeout: Option<u64>,
    ) -> Option<(bool, u64)> {
        let mut entry = self.sessions.get_mut(session_id)?;
        if let Some(r) = resuming {
            entry.resuming = r;
        }
        if let Some(t) = timeout {
            entry.timeout = t;
        }
        entry.last_active = Instant::now();
        Some((entry.resuming, entry.timeout))
    }

    /// Returns list of all known session IDs.
    pub fn get_session_ids(&self) -> Vec<String> {
        self.sessions.iter().map(|e| e.key().clone()).collect()
    }

    /// Get session state summary.
    pub fn get_session(&self, session_id: &str) -> Option<(bool, u64)> {
        self.sessions
            .get(session_id)
            .map(|s| (s.resuming, s.timeout))
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
            let mut entry = self.sessions.get_mut(session_id)?;
            entry.connected = false;
            entry.last_active = Instant::now();
            (entry.resuming, entry.timeout)
        };

        if resuming && timeout > 0 {
            Some(timeout)
        } else {
            self.remove_session(session_id);
            None
        }
    }

    /// Remove a session completely and decrement metrics. Returns the removed session state if found.
    pub fn remove_session(&self, session_id: &str) -> Option<SessionState> {
        if let Some((_, state)) = self.sessions.remove(session_id) {
            crate::metrics::Metrics::global().active_sessions.dec();
            info!("Session {} cleaned up", session_id);
            Some(state)
        } else {
            None
        }
    }

    /// Check if a disconnected session has expired, and remove it if so.
    /// Returns the session's subscribed guild IDs if expired.
    pub fn expire_if_disconnected(&self, session_id: &str) -> Option<HashSet<String>> {
        let should_remove = self
            .sessions
            .get(session_id)
            .map(|s| !s.connected)
            .unwrap_or(false);

        if should_remove {
            self.remove_session(session_id).map(|s| s.guild_ids)
        } else {
            None
        }
    }

    /// Deterministic expiration check for a disconnected session.
    ///
    /// A disconnected session expires exactly `timeout` seconds after its
    /// `last_active` timestamp (which is set at disconnect time). Sessions that
    /// cannot resume (`resuming == false`) or that configured `timeout == 0` are
    /// considered immediately expired; `mark_disconnected` removes them eagerly,
    /// so this is the defensive backstop for interrupted cleanup paths.
    /// Active/connected sessions are never considered expired.
    pub fn is_session_expired(state: &SessionState, now: Instant) -> bool {
        if state.connected {
            return false;
        }
        if !state.resuming || state.timeout == 0 {
            return true;
        }
        now.saturating_duration_since(state.last_active)
            > std::time::Duration::from_secs(state.timeout)
    }

    /// Periodic cleanup of stale expired sessions.
    /// Returns all guild IDs belonging to the expired sessions.
    pub fn cleanup_stale(&self) -> Vec<String> {
        let now = Instant::now();
        let mut expired = Vec::new();
        for entry in self.sessions.iter() {
            let s = entry.value();
            if !s.connected && Self::is_session_expired(s, now) {
                expired.push(entry.key().clone());
            }
        }
        let mut expired_guilds = Vec::new();
        for id in expired {
            if let Some(state) = self.remove_session(&id) {
                expired_guilds.extend(state.guild_ids);
            }
        }
        expired_guilds
    }

    /// Count total registered sessions.
    pub fn count_sessions(&self) -> usize {
        self.sessions.len()
    }
}
