// Copyright (c) 2026 nikcodex (KizunaLink)
// Licensed under the MIT License

use thiserror::Error;

/// Errors from the Discord voice gateway connection.
#[derive(Debug, Error)]
pub enum GatewayError {
    #[error("WebSocket error: {0}")]
    WebSocket(String),

    #[error("Connection closed: code={code}, reason={reason}")]
    Closed { code: u16, reason: String },

    #[error("Heartbeat timeout ({0}ms since last ack)")]
    HeartbeatTimeout(u64),

    #[error("Session description error: {0}")]
    SessionDescription(String),

    #[error("UDP bind error: {0}")]
    UdpBind(String),

    #[error("IP discovery error: {0}")]
    IpDiscovery(String),

    #[error("Voice gateway error: {0}")]
    Protocol(String),

    #[error("DAVE error: {0}")]
    Dave(String),

    #[error("{0}")]
    Other(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heartbeat_timeout_display() {
        let err = GatewayError::HeartbeatTimeout(5000);
        assert!(err.to_string().contains("5000"));
    }

    #[test]
    fn closed_display() {
        let err = GatewayError::Closed {
            code: 4014,
            reason: "disconnected".into(),
        };
        assert!(err.to_string().contains("4014"));
        assert!(err.to_string().contains("disconnected"));
    }

    #[test]
    fn websocket_display() {
        let err = GatewayError::WebSocket("connection reset".into());
        assert!(err.to_string().contains("connection reset"));
    }
}
