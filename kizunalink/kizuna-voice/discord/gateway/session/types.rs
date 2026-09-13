// Copyright (c) 2026 nikcodex (KizunaLink)
// Licensed under the MIT License

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum GatewayError {
    #[error("WebSocket error: {0}")]
    WebSocket(Box<tokio_tungstenite::tungstenite::Error>),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Discovery failed: {0}")]
    Discovery(String),

    #[error("Encoding error: {0}")]
    Encoding(String),

    #[error("Encryption error: {0}")]
    Encryption(String),

    #[error("Internal error: {0}")]
    Internal(String),

    #[error("Other error: {0}")]
    Other(#[from] Box<dyn std::error::Error + Send + Sync>),
}

impl From<tokio_tungstenite::tungstenite::Error> for GatewayError {
    fn from(error: tokio_tungstenite::tungstenite::Error) -> Self {
        Self::WebSocket(Box::new(error))
    }
}

pub fn map_boxed_err<E: std::fmt::Display>(e: E) -> crate::common::types::AnyError {
    Box::new(std::io::Error::other(e.to_string()))
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionOutcome {
    Reconnect,
    Identify,
    Shutdown,
}

#[derive(Default, Debug)]
pub struct PersistentSessionState {
    pub ssrc: u32,
    pub udp_addr: Option<std::net::SocketAddr>,
    pub session_key: Option<[u8; 32]>,
    pub rtp_state: Option<crate::discord::gateway::udp_link::RtpState>,
    pub selected_mode: Option<String>,
}
