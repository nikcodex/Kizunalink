use super::state::ConnectionState;

pub struct VoiceSession {
    pub session_id: String,
    pub token: String,
    pub endpoint: String,
    pub state: ConnectionState,
}

impl VoiceSession {
    pub fn new(session_id: String, token: String, endpoint: String) -> Self {
        Self {
            session_id,
            token,
            endpoint,
            state: ConnectionState::default(),
        }
    }
}
