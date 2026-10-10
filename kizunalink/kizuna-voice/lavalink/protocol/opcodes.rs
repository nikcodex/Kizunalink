// Copyright (c) 2026 nikcodex (KizunaLink)
// Licensed under the MIT License

use serde::Deserialize;
use serde_json::Value;

use crate::common::types::GuildId;

/// Incoming WebSocket messages from Lavalink-compatible clients.
///
/// Lavalink's op payload uses camelCase field names (`guildId`, `sessionId`,
/// `channelId`). `rename_all` only affects the *variant* names when combined with
/// an internal `tag`, so each field needs an explicit `rename` — otherwise real
/// clients' frames fail to deserialize (`missing field 'guild_id'`) and are
/// silently dropped.
#[derive(Deserialize, Debug)]
#[serde(tag = "op", rename_all = "camelCase")]
pub enum IncomingMessage {
    VoiceUpdate {
        #[serde(rename = "guildId")]
        guild_id: GuildId,
        #[serde(rename = "sessionId")]
        session_id: String,
        #[serde(rename = "channelId")]
        channel_id: Option<String>,
        event: Value,
    },
    Play {
        #[serde(rename = "guildId")]
        guild_id: GuildId,
        track: String,
    },
    Stop {
        #[serde(rename = "guildId")]
        guild_id: GuildId,
    },
    Destroy {
        #[serde(rename = "guildId")]
        guild_id: GuildId,
    },
    ConfigureResuming {
        #[serde(default)]
        key: Option<String>,
        timeout: u64,
    },
}

#[cfg(test)]
mod tests {
    use super::IncomingMessage;

    fn parse(s: &str) -> IncomingMessage {
        serde_json::from_str(s).expect("payload must deserialize")
    }

    #[test]
    fn parses_lavalink_camel_case_client_ops() {
        // Exact shapes emitted by Lavalink v4 clients (wavelink, lavalink-client, poru).
        let voice = parse(
            r#"{"op":"voiceUpdate","guildId":"123","sessionId":"abc","channelId":"456",
                "event":{"token":"t","endpoint":"e","guild_id":"123"}}"#,
        );
        match voice {
            IncomingMessage::VoiceUpdate {
                guild_id,
                session_id,
                channel_id,
                ..
            } => {
                assert_eq!(guild_id.0, "123");
                assert_eq!(session_id, "abc");
                assert_eq!(channel_id.as_deref(), Some("456"));
            }
            other => panic!("expected VoiceUpdate, got {other:?}"),
        }

        match parse(r#"{"op":"play","guildId":"123","track":"ENC"}"#) {
            IncomingMessage::Play { guild_id, track } => {
                assert_eq!(guild_id.0, "123");
                assert_eq!(track, "ENC");
            }
            other => panic!("expected Play, got {other:?}"),
        }

        match parse(r#"{"op":"stop","guildId":"123"}"#) {
            IncomingMessage::Stop { guild_id } => assert_eq!(guild_id.0, "123"),
            other => panic!("expected Stop, got {other:?}"),
        }

        match parse(r#"{"op":"destroy","guildId":"123"}"#) {
            IncomingMessage::Destroy { guild_id } => assert_eq!(guild_id.0, "123"),
            other => panic!("expected Destroy, got {other:?}"),
        }

        // `channelId: null` is Discord's leave-voice signal and must still parse.
        match parse(
            r#"{"op":"voiceUpdate","guildId":"1","sessionId":"s","channelId":null,"event":{}}"#,
        ) {
            IncomingMessage::VoiceUpdate { channel_id, .. } => assert!(channel_id.is_none()),
            other => panic!("expected VoiceUpdate, got {other:?}"),
        }
    }

    #[test]
    fn parses_configure_resuming_with_and_without_key() {
        match parse(r#"{"op":"configureResuming","key":"k","timeout":45}"#) {
            IncomingMessage::ConfigureResuming { key, timeout } => {
                assert_eq!(key.as_deref(), Some("k"));
                assert_eq!(timeout, 45);
            }
            other => panic!("expected ConfigureResuming, got {other:?}"),
        }
        match parse(r#"{"op":"configureResuming","timeout":0}"#) {
            IncomingMessage::ConfigureResuming { key, timeout } => {
                assert!(key.is_none());
                assert_eq!(timeout, 0);
            }
            other => panic!("expected ConfigureResuming, got {other:?}"),
        }
    }
}
