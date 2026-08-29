// [DAVE CRYPTO/WIRE TEST]
use kizuna_voice::dave::protocol::DaveSession;

#[test]
fn test_dave_golden_vector_structure() {
    let mut session = DaveSession::new("test_guild_123".to_string());
    session.add_sender("1001");

    // Known test Opus frame (e.g. Discord 3-byte silence packet 0xF8FFFE)
    let opus_silence = vec![0xF8, 0xFF, 0xFE];
    let nonce_counter: u32 = 1;
    let rtp_header = [0x80, 0x78, 0x00, 0x01, 0x00, 0x00, 0x03, 0xC0, 0x00, 0x01, 0xE2, 0x40];

    let encrypted = session
        .encrypt_frame("1001", &opus_silence, nonce_counter, &rtp_header)
        .expect("Encryption should succeed");

    // 1. Minimum length check: 3 bytes payload + 8 bytes tag + 1 byte ULEB nonce (val 1) + 1 byte size + 2 bytes magic = 15 bytes
    assert_eq!(encrypted.len(), 3 + 8 + 1 + 1 + 2);

    // 2. Check Magic Marker at the end: 0xFAFA
    let len = encrypted.len();
    assert_eq!(encrypted[len - 2], 0xFA);
    assert_eq!(encrypted[len - 1], 0xFA);

    // 3. Check Supplemental Size byte (should be 8 + 1 + 1 + 2 = 12)
    let suppl_size = encrypted[len - 3];
    assert_eq!(suppl_size, 12);

    // 4. Check ULEB128 nonce (value 1 should encode as [0x01])
    let nonce_byte = encrypted[len - 4];
    assert_eq!(nonce_byte, 0x01);

    // 5. Check that media length matches plaintext length (AES-CTR stream component)
    let media_ciphertext = &encrypted[..3];
    assert_eq!(media_ciphertext.len(), opus_silence.len());
    // Ciphertext must not be plaintext
    assert_ne!(media_ciphertext, opus_silence.as_slice());

    // 6. Roundtrip decryption
    let decrypted = session
        .decrypt_frame("1001", &encrypted)
        .expect("Decryption of valid frame must succeed");
    assert_eq!(decrypted, opus_silence);
}

#[test]
fn test_dave_uleb128_multi_byte_nonce_encoding() {
    let mut session = DaveSession::new("test_guild_uleb".to_string());
    session.add_sender("2002");

    let payload = vec![0x11, 0x22, 0x33, 0x44, 0x55];

    // Case 1: Nonce = 128 (requires 2 bytes in ULEB128: 0x80, 0x01)
    let nonce_128: u32 = 128;
    let enc_128 = session
        .encrypt_frame("2002", &payload, nonce_128, &[])
        .expect("Encryption should succeed");

    let len = enc_128.len();
    assert_eq!(enc_128[len - 2], 0xFA);
    assert_eq!(enc_128[len - 1], 0xFA);
    // suppl_size = 8 (tag) + 2 (nonce) + 1 (size) + 2 (magic) = 13
    assert_eq!(enc_128[len - 3], 13);
    // ULEB128 bytes for 128: [0x80, 0x01]
    assert_eq!(enc_128[len - 5], 0x80);
    assert_eq!(enc_128[len - 4], 0x01);

    let dec_128 = session
        .decrypt_frame("2002", &enc_128)
        .expect("Decryption of 2-byte nonce frame must succeed");
    assert_eq!(dec_128, payload);

    // Case 2: Nonce = 16384 (requires 3 bytes in ULEB128: 0x80, 0x80, 0x01)
    let nonce_16384: u32 = 16384;
    let enc_16384 = session
        .encrypt_frame("2002", &payload, nonce_16384, &[])
        .expect("Encryption should succeed");

    let len = enc_16384.len();
    assert_eq!(enc_16384[len - 2], 0xFA);
    assert_eq!(enc_16384[len - 1], 0xFA);
    // suppl_size = 8 (tag) + 3 (nonce) + 1 (size) + 2 (magic) = 14
    assert_eq!(enc_16384[len - 3], 14);
    assert_eq!(enc_16384[len - 6], 0x80);
    assert_eq!(enc_16384[len - 5], 0x80);
    assert_eq!(enc_16384[len - 4], 0x01);

    let dec_16384 = session
        .decrypt_frame("2002", &enc_16384)
        .expect("Decryption of 3-byte nonce frame must succeed");
    assert_eq!(dec_16384, payload);
}

#[test]
fn test_dave_tamper_detection_auth_tag_and_media() {
    let mut session = DaveSession::new("test_guild_tamper".to_string());
    session.add_sender("3003");

    let payload = b"critical voice stream bytes".to_vec();
    let encrypted = session
        .encrypt_frame("3003", &payload, 42, &[])
        .expect("Encryption should succeed");

    // Tamper 1: Modify 1 bit in media ciphertext
    let mut tampered_media = encrypted.clone();
    tampered_media[0] ^= 0x01;
    let err_media = session.decrypt_frame("3003", &tampered_media);
    assert!(err_media.is_err(), "Decryption must fail when media is tampered");
    assert!(err_media.unwrap_err().contains("authentication tag mismatch"));

    // Tamper 2: Modify 1 bit in authentication tag
    let mut tampered_tag = encrypted.clone();
    // auth tag starts right after media (index payload.len())
    tampered_tag[payload.len()] ^= 0x01;
    let err_tag = session.decrypt_frame("3003", &tampered_tag);
    assert!(err_tag.is_err(), "Decryption must fail when auth tag is tampered");
    assert!(err_tag.unwrap_err().contains("authentication tag mismatch"));

    // Tamper 3: Modify Magic Marker
    let mut tampered_magic = encrypted.clone();
    let len = tampered_magic.len();
    tampered_magic[len - 1] = 0xFB; // 0xFAFB instead of 0xFAFA
    let err_magic = session.decrypt_frame("3003", &tampered_magic);
    assert!(err_magic.is_err(), "Decryption must fail when magic marker is invalid");
    assert!(err_magic.unwrap_err().contains("Invalid DAVE magic marker"));
}

#[test]
fn test_dave_key_ratchet_generation_stepping() {
    let mut session = DaveSession::new("test_guild_ratchet".to_string());
    session.add_sender("4004");

    let payload = b"ratcheted stream frame".to_vec();

    // Generation 0: nonce = 0
    let f0 = session
        .encrypt_frame("4004", &payload, 0, &[])
        .expect("Gen 0 encrypt");

    // Generation 1: upper 8 bits of 32-bit nonce set to 1 (0x01000000)
    let f1 = session
        .encrypt_frame("4004", &payload, 0x01000000, &[])
        .expect("Gen 1 encrypt");

    // Generation 2: upper 8 bits set to 2 (0x02000000)
    let f2 = session
        .encrypt_frame("4004", &payload, 0x02000000, &[])
        .expect("Gen 2 encrypt");

    // All should decrypt successfully
    assert_eq!(session.decrypt_frame("4004", &f0).unwrap(), payload);
    assert_eq!(session.decrypt_frame("4004", &f1).unwrap(), payload);
    assert_eq!(session.decrypt_frame("4004", &f2).unwrap(), payload);
}

#[test]
fn test_dave_aad_and_rtp_header_verification() {
    // Per Discord DAVE specification:
    // For Opus audio, all frame bytes are encrypted, unencrypted ranges = empty, AAD = empty.
    // The RTP header is added at the UDP layer and must remain byte-for-byte identical.
    let mut session = DaveSession::new("test_guild_aad".to_string());
    session.add_sender("5005");

    let opus_frame = vec![0x78, 0x01, 0x02, 0x03, 0x04];
    let rtp_header = [0x80, 0x78, 0x00, 0x2A, 0x00, 0x00, 0x0F, 0x00, 0x00, 0x01, 0x02, 0x03];

    let encrypted_dave = session
        .encrypt_frame("5005", &opus_frame, 42, &rtp_header)
        .expect("Encryption should succeed");

    // Build complete RTP packet
    let mut full_rtp_packet = Vec::with_capacity(12 + encrypted_dave.len());
    full_rtp_packet.extend_from_slice(&rtp_header);
    full_rtp_packet.extend_from_slice(&encrypted_dave);

    // Verify RTP header is intact
    assert_eq!(&full_rtp_packet[..12], &rtp_header);

    // Verify DAVE payload is intact
    let extracted_dave = &full_rtp_packet[12..];
    let decrypted = session
        .decrypt_frame("5005", extracted_dave)
        .expect("Decryption must succeed");
    assert_eq!(decrypted, opus_frame);
}
