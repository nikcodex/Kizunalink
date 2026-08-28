#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionState {
    Disconnected,
    Connecting,
    Connected,
    Reconnecting,
    Failed,
}

impl Default for ConnectionState {
    fn default() -> Self {
        Self::Disconnected
    }
}
