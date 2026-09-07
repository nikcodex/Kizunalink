use aes_gcm::{
    aead::{Aead, KeyInit, Payload},
    Aes256Gcm, Key, Nonce,
};

pub struct TransportCrypto {
    cipher: Aes256Gcm,
    nonce_counter: u32,
}

impl TransportCrypto {
    pub fn new(secret_key: &[u8]) -> Result<Self, String> {
        if secret_key.len() != 32 {
            return Err("Secret key must be exactly 32 bytes for AES-256-GCM".into());
        }
        let key = Key::<Aes256Gcm>::from_slice(secret_key);
        let cipher = Aes256Gcm::new(key);
        Ok(Self {
            cipher,
            nonce_counter: 0,
        })
    }

    pub fn encrypt_rtp_packet(
        &mut self,
        rtp_header: &[u8],
        payload: &[u8],
    ) -> Result<Vec<u8>, String> {
        // Discord's `aead_aes256_gcm_rtpsize` transport mode: the 4-byte
        // incremental nonce is *left aligned* in the 12-byte AES-GCM nonce and
        // the remaining 8 bytes are zero. The very same 4 bytes are then
        // appended to the packet so the SFU can rebuild the nonce.
        let mut nonce_bytes = [0u8; 12];
        nonce_bytes[0..4].copy_from_slice(&self.nonce_counter.to_be_bytes());
        let nonce = Nonce::from_slice(&nonce_bytes);

        // AAD is the rtp_header
        let aead_payload = Payload {
            msg: payload,
            aad: rtp_header,
        };

        // Encrypt (returns ciphertext || 16-byte tag)
        let encrypted = self
            .cipher
            .encrypt(nonce, aead_payload)
            .map_err(|e| format!("Encryption failed: {:?}", e))?;

        // Append 4-byte nonce counter
        let mut final_packet = Vec::with_capacity(rtp_header.len() + encrypted.len() + 4);
        final_packet.extend_from_slice(rtp_header);
        final_packet.extend_from_slice(&encrypted);
        final_packet.extend_from_slice(&self.nonce_counter.to_be_bytes());

        // Increment nonce
        self.nonce_counter = self.nonce_counter.wrapping_add(1);

        Ok(final_packet)
    }

    pub fn decrypt_rtp_packet(&self, packet: &[u8]) -> Result<(Vec<u8>, Vec<u8>), String> {
        // packet: [rtp_header(12)] [encrypted_payload + tag(16)] [nonce_counter(4)]
        if packet.len() < 12 + 16 + 4 {
            return Err("Packet too short".into());
        }

        let len = packet.len();
        let rtp_header = &packet[..12];
        let encrypted_with_tag = &packet[12..len - 4];
        let nonce_suffix = &packet[len - 4..];

        let mut nonce_bytes = [0u8; 12];
        nonce_bytes[0..4].copy_from_slice(nonce_suffix);
        let nonce = Nonce::from_slice(&nonce_bytes);

        let aead_payload = Payload {
            msg: encrypted_with_tag,
            aad: rtp_header,
        };

        let decrypted = self
            .cipher
            .decrypt(nonce, aead_payload)
            .map_err(|e| format!("Decryption failed: {:?}", e))?;

        Ok((rtp_header.to_vec(), decrypted))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encrypt_decrypt() {
        let key = [42u8; 32];
        let mut crypto = TransportCrypto::new(&key).unwrap();

        let header = [1u8; 12];
        let payload = [2u8; 50];

        let encrypted = crypto.encrypt_rtp_packet(&header, &payload).unwrap();
        assert_eq!(crypto.nonce_counter, 1);

        let (dec_header, dec_payload) = crypto.decrypt_rtp_packet(&encrypted).unwrap();
        assert_eq!(dec_header, header);
        assert_eq!(dec_payload, payload);
    }

    #[test]
    fn test_wrong_key() {
        let key1 = [42u8; 32];
        let key2 = [43u8; 32];
        let mut crypto1 = TransportCrypto::new(&key1).unwrap();
        let crypto2 = TransportCrypto::new(&key2).unwrap();

        let header = [1u8; 12];
        let payload = [2u8; 50];

        let encrypted = crypto1.encrypt_rtp_packet(&header, &payload).unwrap();
        assert!(crypto2.decrypt_rtp_packet(&encrypted).is_err());
    }

    #[test]
    fn test_tampered_ciphertext() {
        let key = [42u8; 32];
        let mut crypto = TransportCrypto::new(&key).unwrap();

        let header = [1u8; 12];
        let payload = [2u8; 50];

        let mut encrypted = crypto.encrypt_rtp_packet(&header, &payload).unwrap();

        // Tamper with ciphertext
        encrypted[20] ^= 1;
        assert!(crypto.decrypt_rtp_packet(&encrypted).is_err());
    }

    #[test]
    fn test_tampered_aad() {
        let key = [42u8; 32];
        let mut crypto = TransportCrypto::new(&key).unwrap();

        let header = [1u8; 12];
        let payload = [2u8; 50];

        let mut encrypted = crypto.encrypt_rtp_packet(&header, &payload).unwrap();

        // Tamper with RTP header (AAD)
        encrypted[0] ^= 1;
        assert!(crypto.decrypt_rtp_packet(&encrypted).is_err());
    }

    #[test]
    fn test_consecutive_packets() {
        let key = [42u8; 32];
        let mut crypto = TransportCrypto::new(&key).unwrap();

        let header = [1u8; 12];
        let payload = [2u8; 50];

        let enc1 = crypto.encrypt_rtp_packet(&header, &payload).unwrap();
        let enc2 = crypto.encrypt_rtp_packet(&header, &payload).unwrap();

        assert_eq!(crypto.nonce_counter, 2);

        // Nonce is the last 4 bytes
        assert_eq!(&enc1[enc1.len() - 4..], &[0, 0, 0, 0]);
        assert_eq!(&enc2[enc2.len() - 4..], &[0, 0, 0, 1]);
    }

    /// Wire-format check against Discord's documented `aead_aes256_gcm_rtpsize`
    /// layout: a receiver only knows the RTP header, the ciphertext and the
    /// 4-byte nonce suffix appended to the packet. It rebuilds the 12-byte
    /// AES-GCM nonce by left-aligning those 4 bytes in a zeroed buffer and uses
    /// the RTP header as AAD. If our packet cannot be opened that way, Discord
    /// cannot decrypt it either.
    #[test]
    fn test_encrypted_packet_matches_discord_rtpsize_wire_format() {
        let key = [7u8; 32];
        let mut crypto = TransportCrypto::new(&key).unwrap();

        let mut header = [0u8; 12];
        header[0] = 0x80; // version 2, no padding/extension/csrc
        header[1] = 0x78; // Opus payload type
        header[2..4].copy_from_slice(&1u16.to_be_bytes()); // sequence
        header[4..8].copy_from_slice(&960u32.to_be_bytes()); // timestamp
        header[8..12].copy_from_slice(&12345u32.to_be_bytes()); // ssrc
        let payload = b"opus-frame-bytes".to_vec();

        let packet = crypto.encrypt_rtp_packet(&header, &payload).unwrap();

        // Structure: [12-byte RTP header][ciphertext || 16-byte tag][4-byte nonce]
        assert_eq!(&packet[..12], &header[..]);
        assert!(packet.len() > 12 + 16 + 4);

        let len = packet.len();
        let nonce_suffix = &packet[len - 4..];
        let ciphertext_with_tag = &packet[12..len - 4];

        let mut nonce_bytes = [0u8; 12];
        nonce_bytes[..4].copy_from_slice(nonce_suffix);

        // Decrypt exactly the way Discord's SFU does: rebuild the nonce from the
        // 4-byte suffix, use the RTP header as AAD.
        let cipher = Aes256Gcm::new_from_slice(&key).expect("valid key");
        let nonce = Nonce::from_slice(&nonce_bytes);
        let aead = Payload {
            msg: ciphertext_with_tag,
            aad: &header[..],
        };
        let decrypted = cipher.decrypt(nonce, aead).expect("packet opens");

        assert_eq!(decrypted, payload);
    }

    /// The 4-byte nonce suffix on the wire must be byte-identical to the first
    /// 4 bytes of the AES-GCM nonce we used, otherwise the SFU derives a
    /// different nonce and every packet fails authentication.
    #[test]
    fn test_nonce_suffix_is_left_aligned_in_gcm_nonce() {
        let key = [9u8; 32];
        let mut crypto = TransportCrypto::new(&key).unwrap();
        let header = [0x80u8; 12];

        for expected in [0u32, 1, 255, 0x01020304, u32::MAX - 1] {
            crypto.nonce_counter = expected;
            let packet = crypto.encrypt_rtp_packet(&header, b"abc").unwrap();
            let len = packet.len();
            assert_eq!(&packet[len - 4..], &expected.to_be_bytes());
        }
    }

    #[test]
    fn test_rejects_short_secret_key() {
        assert!(TransportCrypto::new(&[1u8; 31]).is_err());
        assert!(TransportCrypto::new(&[1u8; 33]).is_err());
        assert!(TransportCrypto::new(&[1u8; 32]).is_ok());
    }
}
