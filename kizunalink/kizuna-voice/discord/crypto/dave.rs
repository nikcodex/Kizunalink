// Copyright (c) 2026 nikcodex (KizunaLink)
// Licensed under the MIT License

use std::{
    collections::{HashMap, HashSet},
    num::NonZeroU16,
};

use davey::{DaveSession, ProposalsOperationType};
use tracing::{debug, trace, warn};

use crate::{
    common::types::{AnyError, AnyResult, ChannelId, UserId},
    discord::gateway::{
        constants::{
            DAVE_INITIAL_VERSION, MAX_DAVE_CONTROL_PAYLOAD_BYTES, MAX_PENDING_PROPOSALS,
            SILENCE_FRAME,
        },
        session::types::map_boxed_err,
    },
};

const DAVE_MIN_VERSION: NonZeroU16 = match NonZeroU16::new(DAVE_INITIAL_VERSION) {
    Some(v) => v,
    // B24: give a clear const-panic message if someone sets DAVE_INITIAL_VERSION = 0.
    None => panic!("DAVE_INITIAL_VERSION must be non-zero"),
};

pub struct DaveHandler {
    session: Option<DaveSession>,
    user_id: UserId,
    channel_id: ChannelId,
    protocol_version: u16,
    prepared_protocol_version: u16,
    pending_transitions: HashMap<u16, u16>,
    external_sender_set: bool,
    saved_external_sender: Option<Vec<u8>>,
    pending_proposals: Vec<Vec<u8>>,
    pending_handshake: Vec<(Vec<u8>, bool)>,
    was_ready: bool,
    recognized_users: HashSet<UserId>,
    cached_user_ids: Vec<u64>,
}

impl DaveHandler {
    pub fn new(user_id: UserId, channel_id: ChannelId) -> Self {
        let mut recognized_users = HashSet::new();
        recognized_users.insert(user_id);
        Self {
            session: None,
            user_id,
            channel_id,
            protocol_version: 0,
            prepared_protocol_version: 0,
            pending_transitions: HashMap::new(),
            external_sender_set: false,
            saved_external_sender: None,
            pending_proposals: Vec::new(),
            pending_handshake: Vec::new(),
            was_ready: false,
            recognized_users,
            cached_user_ids: vec![user_id.0],
        }
    }

    pub fn add_users(&mut self, uids: &[u64]) {
        for &uid in uids {
            self.recognized_users.insert(UserId(uid));
        }
        self.update_user_cache();
        debug!("DAVE adding users: {:?}", uids);
    }

    pub fn remove_user(&mut self, uid: u64) {
        if self.recognized_users.remove(&UserId(uid)) {
            self.update_user_cache();
        }
        debug!("DAVE removing user: {}", uid);
    }

    fn update_user_cache(&mut self) {
        self.cached_user_ids.clear();
        self.cached_user_ids
            .extend(self.recognized_users.iter().map(|u| u.0));
        self.cached_user_ids.sort_unstable();
    }

    pub fn protocol_version(&self) -> u16 {
        self.protocol_version
    }

    pub fn setup_session(&mut self, version: u16) -> AnyResult<Vec<u8>> {
        if version == 0 {
            self.reset();
            return Ok(Vec::new());
        }

        let nz_version = NonZeroU16::new(version).unwrap_or(DAVE_MIN_VERSION);

        let reusing_session = self.session.is_some();
        let session = if let Some(s) = &mut self.session {
            if self.protocol_version == 0 {
                s.reinit(nz_version, self.user_id.0, self.channel_id.0, None)
                    .map_err(map_boxed_err)?;
            } else {
                s.reinit_for_transition(nz_version, self.user_id.0, self.channel_id.0, None)
                    .map_err(map_boxed_err)?;
            }
            s
        } else {
            let session = DaveSession::new(nz_version, self.user_id.0, self.channel_id.0, None)
                .map_err(map_boxed_err)?;
            self.session = Some(session);
            self.session.as_mut().unwrap()
        };

        self.prepared_protocol_version = version;
        self.external_sender_set = reusing_session && self.saved_external_sender.is_some();
        self.pending_proposals.clear();
        self.pending_handshake.clear();
        self.was_ready = false;

        debug!("DAVE session setup (v{})", version);
        let key_package = session.create_key_package().map_err(map_boxed_err)?;

        if self.external_sender_set {
            debug!("DAVE retained external sender while preparing new epoch");
        }

        Ok(key_package)
    }

    pub fn reset(&mut self) {
        self.protocol_version = 0;
        self.prepared_protocol_version = 0;
        self.pending_transitions.clear();
        self.external_sender_set = false;
        self.saved_external_sender = None;
        self.pending_proposals.clear();
        self.pending_handshake.clear();
        self.was_ready = false;
        self.session = None;
        debug!("DAVE session reset to plaintext");
    }

    pub fn prepare_transition(&mut self, transition_id: u16, protocol_version: u16) -> bool {
        self.pending_transitions
            .insert(transition_id, protocol_version);

        if transition_id == 0 {
            self.execute_transition(0);
            return false;
        }
        true
    }

    pub fn execute_transition(&mut self, transition_id: u16) {
        if let Some(next_version) = self.pending_transitions.remove(&transition_id) {
            if let Some(session) = &mut self.session {
                if next_version == 0 {
                    session.discard_transition(transition_id);
                } else {
                    session.execute_transition(transition_id);
                }
            }
            self.protocol_version = next_version;
            trace!(
                "DAVE transition {} executed (v{})",
                transition_id, next_version
            );
        }
    }

    pub fn prepare_epoch(&mut self, epoch: u64, protocol_version: u16) -> Option<Vec<u8>> {
        if epoch == 1 {
            match self.setup_session(protocol_version) {
                Ok(kp) => return Some(kp),
                Err(e) => warn!("DAVE prepare_epoch setup failed: {e}"),
            }
        }
        None
    }

    pub fn process_external_sender(&mut self, data: &[u8]) -> AnyResult<Vec<Vec<u8>>> {
        if data.len() > MAX_DAVE_CONTROL_PAYLOAD_BYTES {
            return Err(map_boxed_err(format!(
                "DAVE external sender payload exceeds {MAX_DAVE_CONTROL_PAYLOAD_BYTES} bytes"
            )));
        }

        let mut responses = Vec::new();

        if self.external_sender_set && self.saved_external_sender.as_deref() == Some(data) {
            trace!("DAVE ignoring unchanged external sender package");
            return Ok(responses);
        }

        if let Some(session) = &mut self.session {
            session.set_external_sender(data).map_err(map_boxed_err)?;
            self.external_sender_set = true;
            self.saved_external_sender = Some(data.to_vec());

            if !self.pending_proposals.is_empty() {
                debug!(
                    "DAVE processing {} buffered proposals",
                    self.pending_proposals.len()
                );
                for prop_data in std::mem::take(&mut self.pending_proposals) {
                    if let Ok(Some(res)) =
                        Self::do_process_proposals(session, &prop_data, &self.cached_user_ids)
                    {
                        responses.push(res);
                    }
                }
            }

            if !self.pending_handshake.is_empty() {
                debug!(
                    "DAVE processing {} buffered handshake messages",
                    self.pending_handshake.len()
                );
                for (handshake_data, is_welcome) in std::mem::take(&mut self.pending_handshake) {
                    if let Err(e) = self.do_process_handshake(&handshake_data, is_welcome) {
                        warn!("DAVE buffered handshake processing failed: {e}");
                    }
                }
            }
        }
        Ok(responses)
    }

    pub fn process_welcome(&mut self, data: &[u8]) -> AnyResult<u16> {
        self.process_handshake_message(data, true)
    }

    pub fn process_commit(&mut self, data: &[u8]) -> AnyResult<u16> {
        self.process_handshake_message(data, false)
    }

    fn process_handshake_message(&mut self, data: &[u8], is_welcome: bool) -> AnyResult<u16> {
        let tag = if is_welcome { "welcome" } else { "commit" };
        if data.len() < 2 {
            return Err(short_payload_err(&format!("DAVE {tag}")));
        }
        if data.len() > MAX_DAVE_CONTROL_PAYLOAD_BYTES {
            return Err(map_boxed_err(format!(
                "DAVE {tag} payload exceeds {MAX_DAVE_CONTROL_PAYLOAD_BYTES} bytes"
            )));
        }

        let transition_id = u16::from_be_bytes([data[0], data[1]]);

        if !self.external_sender_set {
            if self.pending_handshake.len() < MAX_PENDING_PROPOSALS {
                debug!("DAVE buffering {tag} — external sender not set");
                self.pending_handshake.push((data.to_vec(), is_welcome));
            } else {
                warn!("DAVE handshake buffer full, dropping {tag}");
            }
            return Ok(transition_id);
        }

        self.do_process_handshake(data, is_welcome)?;

        Ok(transition_id)
    }

    fn do_process_handshake(&mut self, data: &[u8], is_welcome: bool) -> AnyResult<()> {
        let transition_id = u16::from_be_bytes([data[0], data[1]]);
        if let Some(session) = &mut self.session {
            if transition_id == 0 {
                if is_welcome {
                    session.process_welcome(&data[2..]).map_err(map_boxed_err)?;
                } else {
                    session.process_commit(&data[2..]).map_err(map_boxed_err)?;
                }
                self.protocol_version = self.prepared_protocol_version;
            } else if is_welcome {
                session
                    .process_welcome_for_transition(&data[2..], transition_id)
                    .map_err(map_boxed_err)?;
            } else {
                session
                    .process_commit_for_transition(&data[2..], transition_id)
                    .map_err(map_boxed_err)?;
            }

            if transition_id != 0 {
                self.pending_transitions
                    .insert(transition_id, self.prepared_protocol_version);
            }
            debug!(
                "DAVE {} processed (tid {})",
                if is_welcome { "welcome" } else { "commit" },
                transition_id
            );
        }
        Ok(())
    }

    pub fn process_proposals(&mut self, data: &[u8]) -> AnyResult<Option<Vec<u8>>> {
        if data.is_empty() {
            return Err(short_payload_err("DAVE proposals"));
        }
        if data.len() > MAX_DAVE_CONTROL_PAYLOAD_BYTES {
            return Err(map_boxed_err(format!(
                "DAVE proposals payload exceeds {MAX_DAVE_CONTROL_PAYLOAD_BYTES} bytes"
            )));
        }

        if !self.external_sender_set {
            if self.pending_proposals.len() < MAX_PENDING_PROPOSALS {
                debug!("DAVE buffering proposal — external sender not set");
                self.pending_proposals.push(data.to_vec());
            } else {
                warn!("DAVE proposal buffer full, dropping proposal");
            }
            return Ok(None);
        }

        let session = match &mut self.session {
            Some(s) => s,
            None => return Ok(None),
        };
        Self::do_process_proposals(session, data, &self.cached_user_ids)
    }

    fn do_process_proposals(
        session: &mut DaveSession,
        data: &[u8],
        user_ids: &[u64],
    ) -> AnyResult<Option<Vec<u8>>> {
        let op_type = match data[0] {
            0 => ProposalsOperationType::APPEND,
            1 => ProposalsOperationType::REVOKE,
            raw => return Err(map_boxed_err(format!("Unknown DAVE proposals op: {raw}"))),
        };

        let result = session
            .process_proposals(op_type, &data[1..], Some(user_ids))
            .map_err(map_boxed_err)?;

        if let Some(cw) = result {
            let mut out = cw.commit;
            if let Some(w) = cw.welcome {
                out.extend_from_slice(&w);
            }
            return Ok(Some(out));
        }
        Ok(None)
    }

    pub fn encrypt_opus(&mut self, packet: &[u8]) -> AnyResult<Vec<u8>> {
        if packet == SILENCE_FRAME || self.protocol_version == 0 {
            return Ok(packet.to_vec());
        }

        if let Some(session) = &mut self.session {
            let is_ready = session.is_ready();

            if is_ready != self.was_ready {
                if is_ready {
                    debug!("DAVE session (v{}) is READY", self.protocol_version);
                } else {
                    warn!("DAVE session (v{}) LOST readiness", self.protocol_version);
                }
                self.was_ready = is_ready;
            }

            if is_ready {
                return session
                    .encrypt_opus(packet)
                    .map(|c| c.into_owned())
                    .map_err(map_boxed_err);
            }
        }

        Ok(packet.to_vec())
    }

    pub fn voice_privacy_code(&self) -> Option<String> {
        self.session
            .as_ref()
            .and_then(|s| s.voice_privacy_code().map(|c| c.to_string()))
    }
}

#[inline]
fn short_payload_err(context: &str) -> AnyError {
    map_boxed_err(format!("Invalid {context} payload: too short"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::types::{ChannelId, UserId};

    #[test]
    fn test_handshake_buffering_logic() {
        let mut handler = DaveHandler::new(UserId(1), ChannelId(1));

        // Buffering should happen if external_sender_set is false
        let welcome_data = vec![0, 42, 1, 2, 3]; // tid 42
        let res = handler.process_welcome(&welcome_data);
        assert!(res.is_ok());
        assert_eq!(res.unwrap(), 42);
        assert_eq!(handler.pending_handshake.len(), 1);

        let commit_data = vec![0, 43, 4, 5, 6]; // tid 43
        let res = handler.process_commit(&commit_data);
        assert!(res.is_ok());
        assert_eq!(res.unwrap(), 43);
        assert_eq!(handler.pending_handshake.len(), 2);

        // setup_session should clear buffers
        handler.setup_session(1).unwrap();
        assert_eq!(handler.pending_handshake.len(), 0);
        assert!(!handler.external_sender_set);

        // Buffering again after setup
        handler.process_welcome(&welcome_data).unwrap();
        assert_eq!(handler.pending_handshake.len(), 1);

        // reset should clear buffers
        handler.reset();
        assert_eq!(handler.pending_handshake.len(), 0);
    }

    #[test]
    fn protocol_version_changes_only_when_transition_executes() {
        let mut handler = DaveHandler::new(UserId(1), ChannelId(1));

        handler.setup_session(1).unwrap();
        assert_eq!(handler.protocol_version(), 0);

        assert!(handler.prepare_transition(42, 1));
        assert_eq!(handler.protocol_version(), 0);

        handler.execute_transition(42);
        assert_eq!(handler.protocol_version(), 1);
    }

    #[test]
    fn preparing_replacement_epoch_keeps_active_protocol_version() {
        let mut handler = DaveHandler::new(UserId(1), ChannelId(1));
        handler.protocol_version = 1;
        assert_eq!(handler.protocol_version(), 1);

        handler.setup_session(1).unwrap();
        assert_eq!(handler.protocol_version(), 1);
        assert_eq!(handler.prepared_protocol_version, 1);
    }

    #[test]
    fn rejects_oversized_control_payloads_before_buffering() {
        let mut handler = DaveHandler::new(UserId(1), ChannelId(1));
        let oversized = vec![0; MAX_DAVE_CONTROL_PAYLOAD_BYTES + 1];

        assert!(handler.process_external_sender(&oversized).is_err());
        assert!(handler.process_proposals(&oversized).is_err());
        assert!(handler.process_welcome(&oversized).is_err());
        assert!(handler.pending_handshake.is_empty());
        assert!(handler.pending_proposals.is_empty());
    }
}
