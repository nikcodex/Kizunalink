import re

with open("kizuna-voice/src/dave/protocol.rs", "r") as f:
    code = f.read()

new_encrypt = """    /// Encrypt an Opus frame for sending
    pub fn encrypt_frame(
        &mut self,
        sender_id: &str,
        plaintext: &[u8],
        nonce_counter: u32,
        _rtp_header: &[u8],
    ) -> Result<Vec<u8>, String> {
        let uid: u64 = sender_id.parse().unwrap_or(0);

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
    }"""

new_decrypt = """    /// Decrypt a received DAVE frame
    pub fn decrypt_frame(&mut self, _sender_id: &str, _frame: &[u8]) -> Result<Vec<u8>, String> {
        Err("Receiving DAVE frames not yet fully implemented".to_string())
    }"""

code = re.sub(r'    /// Encrypt an Opus frame for sending.*?Ok\(frame\)\n    }', new_encrypt, code, flags=re.DOTALL)
code = re.sub(r'    /// Decrypt a received DAVE frame.*?map_err\(\|e\| format\!\("Decryption failed: \{\}", e\)\)\n    \}', new_decrypt, code, flags=re.DOTALL)

with open("kizuna-voice/src/dave/protocol.rs", "w") as f:
    f.write(code)
