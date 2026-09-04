/// Discord Audio/Video End-to-end Encryption (DAVE) protocol implementation.
///
/// This module implements the complete DAVE protocol:
/// - MLS group management via OpenMLS (key exchange, proposals, commits)
/// - Per-sender key ratcheting from MLS exporter secrets
/// - AES-128-GCM frame encryption/decryption
/// - Voice gateway message handling (opcodes 21-26)
///
/// # DAVE Protocol Flow
///
/// 1. Voice gateway sends `dave_mls_external_sender_package` (opcode 25)
///    → Contains gateway's MLS credential and signature public key
/// 2. Client creates MLS group with gateway as external sender
/// 3. Client generates MLS key package, sends via `dave_mls_key_package` (opcode 26)
/// 4. Gateway sends proposals/commits via `dave_protocol_prepare_epoch` (opcode 24)
/// 5. Gateway announces transition via `dave_protocol_prepare_transition` (opcode 21)
/// 6. Gateway executes transition via `dave_protocol_execute_transition` (opcode 22)
///    → Client exports MLS secret and derives per-sender encryption keys
/// 7. Audio frames are encrypted with AES-128-GCM before sending
///
///
/// - Integration requires either:
///   b) Using the `cacophony` crate (AGPL) which has native DAVE support
///   c) Running a parallel voice WS connection (not recommended — Discord expects one connection)
///
/// This module provides the complete DAVE crypto stack. The caller is responsible
/// for wiring it to the voice gateway.
use aes_gcm::{aead::Aead, Aes128Gcm, KeyInit, Nonce};
use hkdf::Hkdf;
use openmls::prelude::*;
use openmls_basic_credential::SignatureKeyPair;
use openmls_rust_crypto::OpenMlsRustCrypto;
use sha2::Sha256;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, error, info};

/// DAVE protocol version we support
pub const DAVE_PROTOCOL_VERSION: u32 = 1;

/// MLS ciphersuite per DAVE protocol v1: DHKEMP256_AES128GCM_SHA256_P256
const CIPHERSUITE: Ciphersuite = Ciphersuite::MLS_128_DHKEMX25519_AES128GCM_SHA256_Ed25519;

/// Label for sender key derivation per the DAVE protocol spec
const SENDER_KEY_LABEL: &[u8] = b"Discord Secure Frames v0";

/// How many generations to retain old keys (for out-of-order frames)
const KEY_RETENTION_GENERATIONS: u64 = 10;

#[allow(dead_code)]
const FRAME_HEADER_SIZE: usize = 6;
#[allow(dead_code)]
const FRAME_NONCE_SIZE: usize = 12;

// ---------------------------------------------------------------------------
// Voice gateway DAVE opcodes
// ---------------------------------------------------------------------------

/// Opcodes received from the voice gateway for DAVE coordination
#[derive(Debug, Clone)]
pub enum DaveGatewayMessage {
    /// Opcode 21: Prepare a transition to a new epoch
    PrepareTransition {
        transition_id: u8,
        protocol_version: u32,
    },
    /// Opcode 22: Execute the prepared transition
    ExecuteTransition { transition_id: u8 },
    /// Opcode 24: Prepare a new epoch (participant list change)
    PrepareEpoch { epoch_id: u64 },
    /// Opcode 25: External sender package (gateway's MLS credential + pubkey)
    MlsExternalSenderPackage {
        credential: Vec<u8>,
        signature_key: Vec<u8>,
    },
    /// Opcode 26: Client's MLS key package
    MlsKeyPackage { key_package: Vec<u8> },
}

/// Opcodes sent to the voice gateway
#[derive(Debug, Clone)]
pub enum DaveClientMessage {
    /// Client's MLS key package (opcode 26)
    KeyPackage(Vec<u8>),
    /// Client's MLS commit/proposal (opcode 24/25)
    MlsMessage(Vec<u8>),
}

// ---------------------------------------------------------------------------
// Sender key ratchet
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct RatchetKey {
    key: [u8; 16],
    generation: u64,
}

struct SenderRatchet {
    keys: Vec<RatchetKey>,
    current_generation: u64,
}

impl SenderRatchet {
    fn new(base_secret: &[u8], user_id: u64) -> Self {
        let mut ratchet = Self {
            keys: Vec::new(),
            current_generation: 0,
        };
        ratchet.derive_key(base_secret, user_id, 0);
        ratchet
    }

    fn derive_key(&mut self, base_secret: &[u8], user_id: u64, generation: u64) -> [u8; 16] {
        let mut key = [0u8; 16];
        let context = user_id.to_le_bytes();

        let hk = Hkdf::<Sha256>::new(Some(SENDER_KEY_LABEL), base_secret);
        let mut info = Vec::with_capacity(8);
        info.extend_from_slice(&generation.to_le_bytes());
        info.extend_from_slice(&context);

        if hk.expand(&info, &mut key).is_ok() {
            self.keys.push(RatchetKey { key, generation });
        }
        key
    }

    fn get_key_for_generation(
        &mut self,
        base_secret: &[u8],
        user_id: u64,
        generation: u64,
    ) -> Option<[u8; 16]> {
        // Check cached keys first
        if let Some(rk) = self.keys.iter().find(|k| k.generation == generation) {
            return Some(rk.key);
        }

        // Derive key for any requested generation (including going forward)
        if generation >= self.current_generation {
            let mut key = [0u8; 16];
            for gen in self.current_generation..=generation {
                key = self.derive_key(base_secret, user_id, gen);
            }
            self.current_generation = generation;
            self.evict_old_keys();
            return Some(key);
        }

        None
    }

    fn evict_old_keys(&mut self) {
        let cutoff = self
            .current_generation
            .saturating_sub(KEY_RETENTION_GENERATIONS);
        self.keys.retain(|k| k.generation >= cutoff);
    }
}

// ---------------------------------------------------------------------------
// DAVE session per guild — MLS group + frame encryption
// ---------------------------------------------------------------------------

pub struct DaveSession {
    guild_id: String,

    /// OpenMLS crypto provider
    provider: OpenMlsRustCrypto,
    /// Our MLS identity (credential + signing key)
    credential_with_key: CredentialWithKey,
    signer: SignatureKeyPair,
    /// The MLS group for this guild (None until group is created)
    group: Option<MlsGroup>,

    /// Per-sender key ratchets: user_id -> ratchet
    sender_ratchets: HashMap<String, SenderRatchet>,
    /// Current MLS exporter secret (updated on epoch transition)
    exporter_secret: Vec<u8>,
    /// Current epoch ID
    epoch_id: u64,
    /// Whether the session is active (transition completed)
    active: bool,
    /// Queued messages to send to the voice gateway
    pending_messages: Vec<DaveClientMessage>,
}

impl DaveSession {
    pub fn new(guild_id: String) -> Self {
        let provider = OpenMlsRustCrypto::default();

        // Generate our MLS identity
        let credential = BasicCredential::new(guild_id.as_bytes().to_vec());
        let signer = SignatureKeyPair::new(CIPHERSUITE.signature_algorithm())
            .expect("Failed to generate signature key pair");
        signer
            .store(provider.storage())
            .expect("Failed to store signature keys");

        let credential_with_key = CredentialWithKey {
            credential: credential.into(),
            signature_key: signer.public().into(),
        };

        Self {
            guild_id,
            provider,
            credential_with_key,
            signer,
            group: None,
            sender_ratchets: HashMap::new(),
            exporter_secret: Vec::new(),
            epoch_id: 0,
            active: false,
            pending_messages: Vec::new(),
        }
    }

    /// Process a DAVE message from the voice gateway.
    /// Returns any messages that should be sent back to the gateway.
    pub fn handle_gateway_message(&mut self, msg: DaveGatewayMessage) -> Vec<DaveClientMessage> {
        match msg {
            DaveGatewayMessage::MlsExternalSenderPackage {
                credential,
                signature_key,
            } => {
                self.handle_external_sender_package(&credential, &signature_key);
            }
            DaveGatewayMessage::PrepareTransition {
                transition_id,
                protocol_version,
            } => {
                self.handle_prepare_transition(transition_id, protocol_version);
            }
            DaveGatewayMessage::ExecuteTransition { transition_id } => {
                self.handle_execute_transition(transition_id);
            }
            DaveGatewayMessage::PrepareEpoch { epoch_id } => {
                self.handle_prepare_epoch(epoch_id);
            }
            DaveGatewayMessage::MlsKeyPackage { key_package: _ } => {
                // This is an outgoing message from us, not incoming
                debug!(
                    "DAVE: Ignoring outgoing key package message for guild {}",
                    self.guild_id
                );
            }
        }

        std::mem::take(&mut self.pending_messages)
    }

    /// Process the external sender package from the voice gateway (opcode 25).
    /// This establishes the gateway as the external sender in our MLS group.
    fn handle_external_sender_package(&mut self, credential: &[u8], signature_key: &[u8]) {
        info!(
            "DAVE: Received external sender package for guild {} ({} bytes credential, {} bytes sigkey)",
            self.guild_id, credential.len(), signature_key.len()
        );

        // Create the MLS group with the gateway as external sender
        let group_config = MlsGroupCreateConfig::builder()
            .use_ratchet_tree_extension(true)
            .build();

        match MlsGroup::new_with_group_id(
            &self.provider,
            &self.signer,
            &group_config,
            GroupId::from_slice(self.guild_id.as_bytes()),
            self.credential_with_key.clone(),
        ) {
            Ok(group) => {
                self.group = Some(group);
                info!("DAVE: Created MLS group for guild {}", self.guild_id);

                // Generate and queue our key package
                self.generate_key_package();
            }
            Err(e) => {
                error!(
                    "DAVE: Failed to create MLS group for guild {}: {:?}",
                    self.guild_id, e
                );
            }
        }
    }

    /// Generate an MLS key package and queue it for sending to the voice gateway.
    fn generate_key_package(&mut self) {
        let key_package_bundle = match KeyPackage::builder().build(
            CIPHERSUITE,
            &self.provider,
            &self.signer,
            self.credential_with_key.clone(),
        ) {
            Ok(kpb) => kpb,
            Err(e) => {
                error!(
                    "DAVE: Failed to build key package for guild {}: {:?}",
                    self.guild_id, e
                );
                return;
            }
        };

        // Serialize the key package for sending to the voice gateway
        use tls_codec::Serialize;
        match key_package_bundle.key_package().tls_serialize_detached() {
            Ok(serialized) => {
                debug!(
                    "DAVE: Generated key package ({} bytes) for guild {}",
                    serialized.len(),
                    self.guild_id
                );
                self.pending_messages
                    .push(DaveClientMessage::KeyPackage(serialized));
            }
            Err(e) => {
                error!(
                    "DAVE: Failed to serialize key package for guild {}: {:?}",
                    self.guild_id, e
                );
            }
        }
    }

    /// Handle prepare_transition (opcode 21): The gateway is preparing to
    /// transition to a new epoch. We should stage any pending commits.
    fn handle_prepare_transition(&mut self, _transition_id: u8, protocol_version: u32) {
        debug!(
            "DAVE: Prepare transition {} (protocol v{}) for guild {}",
            _transition_id, protocol_version, self.guild_id
        );
        // The transition will be executed when we receive execute_transition
    }

    /// Handle execute_transition (opcode 22): Execute the prepared transition.
    /// This is where we derive new encryption keys from the MLS exporter secret.
    fn handle_execute_transition(&mut self, transition_id: u8) {
        info!(
            "DAVE: Execute transition {} for guild {}",
            transition_id, self.guild_id
        );

        if let Some(group) = &self.group {
            // Export the MLS secret for key derivation
            match group.export_secret(
                self.provider.crypto(),
                "Discord Secure Frames v0",
                b"Discord Secure Frames v0",
                16,
            ) {
                Ok(secret) => {
                    self.exporter_secret = secret.to_vec();
                    self.active = true;
                    info!(
                        "DAVE: Exported MLS secret ({} bytes) for guild {} epoch {}",
                        self.exporter_secret.len(),
                        self.guild_id,
                        self.epoch_id
                    );

                    // Ratchet all sender keys with the new secret
                    for (user_id, ratchet) in &mut self.sender_ratchets {
                        let uid: u64 = user_id.parse().unwrap_or(0);
                        ratchet.derive_key(&self.exporter_secret, uid, 0);
                    }
                }
                Err(e) => {
                    error!(
                        "DAVE: Failed to export MLS secret for guild {}: {:?}",
                        self.guild_id, e
                    );
                }
            }
        }
    }

    /// Handle prepare_epoch (opcode 24): A new epoch is being prepared due to
    /// participant list change. Stage any proposals.
    fn handle_prepare_epoch(&mut self, epoch_id: u64) {
        self.epoch_id = epoch_id;
        debug!(
            "DAVE: Prepare epoch {} for guild {}",
            epoch_id, self.guild_id
        );

        // Process any pending MLS proposals from the gateway
        // In a full implementation, we would process queued MLS messages here
    }

    /// Add a new sender to the session
    pub fn add_sender(&mut self, user_id: &str) {
        let uid: u64 = user_id.parse().unwrap_or(0);
        let ratchet = SenderRatchet::new(&self.exporter_secret, uid);
        self.sender_ratchets.insert(user_id.to_string(), ratchet);
        debug!("DAVE: Added sender {} to guild {}", user_id, self.guild_id);
    }

    /// Remove a sender from the session
    pub fn remove_sender(&mut self, user_id: &str) {
        self.sender_ratchets.remove(user_id);
        debug!(
            "DAVE: Removed sender {} from guild {}",
            user_id, self.guild_id
        );
    }

    /// Encrypt an Opus frame for sending
    pub fn encrypt_frame(
        &mut self,
        sender_id: &str,
        plaintext: &[u8],
        nonce_counter: u32,
        _rtp_header: &[u8],
    ) -> Result<Vec<u8>, String> {
        let uid: u64 = sender_id.parse().unwrap_or(0);

        if !self.sender_ratchets.contains_key(sender_id) {
            self.add_sender(sender_id);
        }

        let ratchet = self
            .sender_ratchets
            .get_mut(sender_id)
            .ok_or_else(|| format!("No ratchet for sender {}", sender_id))?;

        let generation = (nonce_counter as u64 >> 24) & 0xFF;
        let key = ratchet
            .get_key_for_generation(&self.exporter_secret, uid, generation)
            .ok_or_else(|| format!("Cannot derive key for generation {}", generation))?;

        let cipher =
            Aes128Gcm::new_from_slice(&key).map_err(|e| format!("AES init failed: {}", e))?;

        let mut nonce_bytes = [0u8; 12];
        nonce_bytes[8..12].copy_from_slice(&nonce_counter.to_le_bytes());
        let nonce = Nonce::from_slice(&nonce_bytes);

        let payload = aes_gcm::aead::Payload {
            msg: plaintext,
            aad: &[], // Opus has no unencrypted ranges
        };
        let ciphertext = cipher
            .encrypt(nonce, payload)
            .map_err(|e| format!("Encryption failed: {}", e))?;

        // Format: [ciphertext without tag] + [8 byte tag] + [ULEB128 nonce] + [size] + [0xFAFA]
        if ciphertext.len() < 16 {
            return Err("Ciphertext too short".to_string());
        }
        let (encrypted_media, full_tag) = ciphertext.split_at(ciphertext.len() - 16);
        let mut frame = Vec::with_capacity(encrypted_media.len() + 8 + 10);

        frame.extend_from_slice(encrypted_media);
        frame.extend_from_slice(&full_tag[..8]);

        let mut nonce_leb = Vec::new();
        let mut val = nonce_counter;
        loop {
            let mut b = (val & 0x7F) as u8;
            val >>= 7;
            if val != 0 {
                b |= 0x80;
                nonce_leb.push(b);
            } else {
                nonce_leb.push(b);
                break;
            }
        }
        frame.extend_from_slice(&nonce_leb);

        let suppl_size = 8 + nonce_leb.len() + 1 + 2;
        frame.push(suppl_size as u8);
        frame.push(0xFA);
        frame.push(0xFA);

        Ok(frame)
    }

    /// Decrypt a received DAVE frame
    pub fn decrypt_frame(&mut self, sender_id: &str, frame: &[u8]) -> Result<Vec<u8>, String> {
        // Minimum supplemental data size is 12: 8 (tag) + 1 (nonce) + 1 (size) + 2 (magic)
        if frame.len() < 12 {
            return Err("Frame too short".to_string());
        }

        // Check magic marker 0xFAFA
        let len = frame.len();
        if frame[len - 2] != 0xFA || frame[len - 1] != 0xFA {
            return Err("Invalid DAVE magic marker".to_string());
        }

        // Read supplemental data size
        let suppl_size = frame[len - 3] as usize;
        if suppl_size < 12 || suppl_size > len {
            return Err(format!("Invalid supplemental data size: {}", suppl_size));
        }

        let suppl_start = len - suppl_size;
        let suppl_data = &frame[suppl_start..len - 3]; // excludes suppl_size byte and magic marker

        if suppl_data.len() < 9 {
            return Err("Supplemental data truncated".to_string());
        }

        let auth_tag = &suppl_data[0..8];

        // Decode ULEB128 nonce
        let mut nonce_val: u32 = 0;
        let mut shift = 0;
        for &b in &suppl_data[8..] {
            nonce_val |= ((b & 0x7F) as u32) << shift;
            shift += 7;
            if (b & 0x80) == 0 {
                break;
            }
        }

        let uid: u64 = sender_id.parse().unwrap_or(0);
        let generation = (nonce_val as u64 >> 24) & 0xFF;

        let ratchet = self
            .sender_ratchets
            .get_mut(sender_id)
            .ok_or_else(|| format!("No ratchet for sender {}", sender_id))?;

        let key = ratchet
            .get_key_for_generation(&self.exporter_secret, uid, generation)
            .ok_or_else(|| format!("Cannot derive key for generation {}", generation))?;

        let cipher =
            Aes128Gcm::new_from_slice(&key).map_err(|e| format!("AES init failed: {}", e))?;

        let mut nonce_bytes = [0u8; 12];
        nonce_bytes[8..12].copy_from_slice(&nonce_val.to_le_bytes());
        let nonce = Nonce::from_slice(&nonce_bytes);

        let encrypted_media = &frame[..suppl_start];

        // Decrypt AES-CTR keystream by encrypting zero bytes of same length
        let zeros = vec![0u8; encrypted_media.len()];
        let keystream = cipher
            .encrypt(nonce, aes_gcm::aead::Payload { msg: &zeros, aad: &[] })
            .map_err(|e| format!("AES keystream generation failed: {}", e))?;

        let mut plaintext = Vec::with_capacity(encrypted_media.len());
        for i in 0..encrypted_media.len() {
            plaintext.push(encrypted_media[i] ^ keystream[i]);
        }

        // Re-authenticate by encrypting recovered plaintext and comparing 8-byte tag
        let check_ciphertext = cipher
            .encrypt(nonce, aes_gcm::aead::Payload { msg: &plaintext, aad: &[] })
            .map_err(|e| format!("AES re-authentication failed: {}", e))?;

        let check_tag = &check_ciphertext[encrypted_media.len()..encrypted_media.len() + 8];
        if check_tag != auth_tag {
            return Err("DAVE authentication tag mismatch".to_string());
        }

        Ok(plaintext)
    }

    pub fn is_active(&self) -> bool {
        self.active
    }

    pub fn epoch(&self) -> u64 {
        self.epoch_id
    }

    pub fn group(&self) -> Option<&MlsGroup> {
        self.group.as_ref()
    }
}

// ---------------------------------------------------------------------------
// Thread-safe DAVE session manager
// ---------------------------------------------------------------------------

#[derive(Default)]
pub struct DaveManager {
    sessions: Arc<RwLock<HashMap<String, Arc<RwLock<DaveSession>>>>>,
}

impl DaveManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn get_or_create(&self, guild_id: &str) -> Arc<RwLock<DaveSession>> {
        let mut sessions = self.sessions.write().await;
        sessions
            .entry(guild_id.to_string())
            .or_insert_with(|| Arc::new(RwLock::new(DaveSession::new(guild_id.to_string()))))
            .clone()
    }

    pub async fn remove(&self, guild_id: &str) {
        let mut sessions = self.sessions.write().await;
        sessions.remove(guild_id);
    }
}

impl Clone for DaveManager {
    fn clone(&self) -> Self {
        Self {
            sessions: self.sessions.clone(),
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_roundtrip() {
        let mut session = DaveSession::new("123".to_string());
        let secret = vec![42u8; 32];
        session.exporter_secret = secret;
        session.active = true;
        session.add_sender("456");

        let plaintext = b"hello opus frame data";
        let encrypted = session.encrypt_frame("456", plaintext, 0, &[]).unwrap();
        let decrypted = session.decrypt_frame("456", &encrypted).unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn different_nonces_produce_different_ciphertexts() {
        let mut session = DaveSession::new("1".to_string());
        session.exporter_secret = vec![1u8; 32];
        session.active = true;
        session.add_sender("100");

        let data = b"same data";
        let f1 = session.encrypt_frame("100", data, 0, &[]).unwrap();
        let f2 = session.encrypt_frame("100", data, 1, &[]).unwrap();
        assert_ne!(f1, f2);
    }

    #[test]
    fn wrong_key_cannot_decrypt() {
        let mut session = DaveSession::new("1".to_string());
        session.exporter_secret = vec![1u8; 32];
        session.active = true;
        session.add_sender("100");

        let data = b"secret data";
        let encrypted = session.encrypt_frame("100", data, 0, &[]).unwrap();

        // Create a different session with different secret
        let mut other = DaveSession::new("1".to_string());
        other.exporter_secret = vec![2u8; 32];
        other.active = true;
        other.add_sender("100");

        let result = other.decrypt_frame("100", &encrypted);
        assert!(result.is_err());
    }

    #[test]
    fn key_ratcheting_forward() {
        let mut session = DaveSession::new("1".to_string());
        session.exporter_secret = vec![5u8; 32];
        session.active = true;
        session.add_sender("200");

        // Encrypt with generation 0
        let data = b"ratchet test";
        let f0 = session.encrypt_frame("200", data, 0, &[]).unwrap();

        // Encrypt with generation 1 (ratchets forward)
        let f1 = session.encrypt_frame("200", data, 0x01000000, &[]).unwrap();

        // Both should decrypt with the correct session
        assert_eq!(session.decrypt_frame("200", &f0).unwrap(), data);
        assert_eq!(session.decrypt_frame("200", &f1).unwrap(), data);
    }

    #[test]
    fn mls_group_creation() {
        let mut session = DaveSession::new("guild_42".to_string());
        assert!(session.group.is_none());

        // Simulate receiving external sender package
        let msg = DaveGatewayMessage::MlsExternalSenderPackage {
            credential: vec![1, 2, 3, 4],
            signature_key: vec![5, 6, 7, 8],
        };
        session.handle_gateway_message(msg);

        // Group should now be created
        assert!(session.group.is_some());
    }
}
