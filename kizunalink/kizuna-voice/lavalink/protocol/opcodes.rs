// Copyright (c) 2026 nikcodex (KizunaLink)
// Licensed under the MIT License

use serde::Deserialize;
use serde_json::Value;

use crate::common::types::GuildId;

/// Incoming WebSocket messages from Lavalink-compatible clients.
#[derive(Deserialize, Debug)]
#[serde(tag = "op", rename_all = "camelCase")]
pub enum IncomingMessage {
    VoiceUpdate {
        guild_id: GuildId,
        session_id: String,
        channel_id: Option<String>,
        event: Value,
    },
    Play {
        guild_id: GuildId,
        track: String,
    },
    Stop {
        guild_id: GuildId,
    },
    Destroy {
        guild_id: GuildId,
    },
}
