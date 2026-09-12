// Copyright (c) 2026 nikcodex (KizunaLink)
// Licensed under the MIT License

use super::super::context::PlayerContext;
use crate::{
    lavalink::protocol::{
        self,
        events::{KizunaLinkEvent, TrackEndReason, TrackException},
    },
    common::server_hooks::Session,
};

/// Emit `TrackException` followed by `TrackEnd: LoadFailed`.
pub async fn send_load_failed(player: &PlayerContext, session: &Session, message: String) {
    let Some(track) = player.to_player_response().await.track else {
        return;
    };
    let guild_id = player.guild_id.clone();

    session.send_message(&protocol::OutgoingMessage::Event {
        event: Box::new(KizunaLinkEvent::TrackException {
            guild_id: guild_id.clone(),
            track: track.clone(),
            exception: TrackException {
                message: Some(message.clone()),
                severity: crate::common::Severity::Common,
                cause: message.clone(),
                cause_stack_trace: Some(message),
            },
        }),
    });

    session.send_message(&protocol::OutgoingMessage::Event {
        event: Box::new(KizunaLinkEvent::TrackEnd {
            guild_id,
            track,
            reason: TrackEndReason::LoadFailed,
        }),
    });
}
