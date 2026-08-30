// [UNIT]
// [FAKE DISCORD PROTOCOL TEST]
use async_trait::async_trait;
use kizuna_voice::audio::{AudioFrame, AudioSource};
use kizuna_voice::dave::protocol::DaveSession;
use kizuna_voice::transport::RtpHeader;
use std::time::Duration;

struct DummyAudioSource;

#[async_trait]
impl AudioSource for DummyAudioSource {
    async fn next_frame(&mut self) -> kizuna_voice::error::Result<Option<AudioFrame>> {
        Ok(None)
    }
}

#[test]
fn test_malformed_rtp_header_parsing() {
    // 1. Completely empty slice
    assert!(RtpHeader::read_from(&[]).is_err());

    // 2. Truncated slice (5 bytes instead of 12)
    assert!(RtpHeader::read_from(&[0x80, 0x78, 0x00, 0x01, 0x02]).is_err());

    // 3. 11 bytes (1 byte short)
    let eleven_bytes = [0u8; 11];
    assert!(RtpHeader::read_from(&eleven_bytes).is_err());
}

#[test]
fn test_malformed_dave_frame_handling() {
    let mut session = DaveSession::new("guild_malformed".to_string());
    session.add_sender("sender_malformed");

    // 1. Frame too short (< 12 bytes)
    let short_frame = vec![0xFA, 0xFA];
    let err = session.decrypt_frame("sender_malformed", &short_frame);
    assert!(err.is_err());
    assert_eq!(err.unwrap_err(), "Frame too short");

    // 2. Invalid magic marker
    let mut bad_magic = vec![0u8; 20];
    bad_magic[17] = 12; // suppl size
    bad_magic[18] = 0xDE;
    bad_magic[19] = 0xAD; // not 0xFAFA
    let err = session.decrypt_frame("sender_malformed", &bad_magic);
    assert!(err.is_err());
    assert_eq!(err.unwrap_err(), "Invalid DAVE magic marker");

    // 3. Overflowing supplemental size
    let mut overflow_size = vec![0u8; 20];
    overflow_size[17] = 200; // suppl size > total frame length
    overflow_size[18] = 0xFA;
    overflow_size[19] = 0xFA;
    let err = session.decrypt_frame("sender_malformed", &overflow_size);
    assert!(err.is_err());
    assert!(err.unwrap_err().contains("Invalid supplemental data size"));

    // 4. Missing sender ratchet
    let valid_frame = session
        .encrypt_frame("sender_malformed", b"test", 1, &[])
        .expect("Encrypt frame");
    let err = session.decrypt_frame("unknown_sender_999", &valid_frame);
    assert!(err.is_err());
    assert!(err.unwrap_err().contains("No ratchet for sender"));
}

#[tokio::test]
async fn test_unsupported_seek_on_default_audio_source() {
    let mut source = DummyAudioSource;
    let res = source.seek(Duration::from_secs(10)).await;
    assert!(res.is_err());
    let err_msg = res.unwrap_err().to_string();
    assert!(err_msg.contains("Seek not supported"));
}
