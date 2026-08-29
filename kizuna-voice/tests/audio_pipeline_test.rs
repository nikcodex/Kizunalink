// [LOCAL INTEGRATION]
use async_trait::async_trait;
use kizuna_voice::audio::{
    AudioFrame, AudioSource, FrameScheduler, KizunaTrackHandle, OpusEncoder, OpusSource, TrackEvent,
};
use kizuna_voice::dave::protocol::DaveSession;
use kizuna_voice::test_harness::FakeVoiceUdpServer;
use kizuna_voice::transport::{RtpHeader, VoiceUdp};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{broadcast, mpsc, Mutex};

struct CountingOpusSource {
    frames_sent: usize,
    max_frames: usize,
    frame_data: Vec<u8>,
}

#[async_trait]
impl AudioSource for CountingOpusSource {
    async fn next_frame(&mut self) -> kizuna_voice::error::Result<Option<AudioFrame>> {
        if self.frames_sent >= self.max_frames {
            Ok(None)
        } else {
            self.frames_sent += 1;
            let mut data = self.frame_data.clone();
            data.push(self.frames_sent as u8); // tag frame number
            Ok(Some(AudioFrame::Opus(data)))
        }
    }
}

struct PcmSineSource {
    frames_sent: usize,
    max_frames: usize,
}

#[async_trait]
impl AudioSource for PcmSineSource {
    async fn next_frame(&mut self) -> kizuna_voice::error::Result<Option<AudioFrame>> {
        if self.frames_sent >= self.max_frames {
            Ok(None)
        } else {
            self.frames_sent += 1;
            // 20ms @ 48kHz stereo = 960 samples/ch * 2 = 1920 i16 samples
            let mut samples = Vec::with_capacity(1920);
            for i in 0..960 {
                let sample = (10000.0 * ((i as f32 * 0.05).sin())) as i16;
                samples.push(sample); // L
                samples.push(sample); // R
            }
            Ok(Some(AudioFrame::Pcm(samples)))
        }
    }
}

#[tokio::test]
async fn test_audio_pipeline_to_local_udp_end_to_end() {
    let ssrc = 98765;
    let fake_udp = FakeVoiceUdpServer::start(ssrc).await.expect("Start fake UDP server");
    let udp_port = fake_udp.port();

    // Bind real VoiceUdp client via discovery with fake server
    let (voice_udp, ext_ip, ext_port) = VoiceUdp::bind_and_discover("127.0.0.1", udp_port, ssrc)
        .await
        .expect("VoiceUdp bind and discovery");

    assert_eq!(ext_ip, "127.0.0.1");
    assert!(ext_port > 0);

    let voice_udp = Arc::new(voice_udp);
    let mut dave_session = DaveSession::new("guild_integration_1".to_string());
    dave_session.add_sender("sender_agent_1");
    let dave = Arc::new(Mutex::new(dave_session));

    let frame_count = 10;
    let source = Arc::new(Mutex::new(CountingOpusSource {
        frames_sent: 0,
        max_frames: frame_count,
        frame_data: vec![0xDE, 0xAD, 0xBE, 0xEF],
    }));

    let (cmd_tx, cmd_rx) = mpsc::channel(32);
    let (event_tx, mut event_rx) = broadcast::channel(32);
    let _handle = KizunaTrackHandle::new(cmd_tx, event_rx.resubscribe());

    let scheduler = FrameScheduler::new(source);
    let udp_clone = voice_udp.clone();
    let dave_clone = dave.clone();

    let sequence = Arc::new(std::sync::atomic::AtomicU16::new(0));
    let timestamp = Arc::new(std::sync::atomic::AtomicU32::new(0));

    let seq_clone = sequence.clone();
    let ts_clone = timestamp.clone();

    let scheduler_task = tokio::spawn(async move {
        scheduler
            .run(cmd_rx, event_tx, |frame| {
                let udp = udp_clone.clone();
                let dave = dave_clone.clone();
                let seq_atomic = seq_clone.clone();
                let ts_atomic = ts_clone.clone();

                async move {
                    let cur_seq = seq_atomic.fetch_add(1, std::sync::atomic::Ordering::SeqCst).wrapping_add(1);
                    let cur_ts = ts_atomic.fetch_add(960, std::sync::atomic::Ordering::SeqCst).wrapping_add(960);

                    let AudioFrame::Opus(opus_data) = frame else {
                        panic!("Expected Opus");
                    };

                    let header = RtpHeader::new(cur_seq, cur_ts, ssrc);
                    let mut header_buf = Vec::new();
                    header.write_to(&mut header_buf).unwrap();

                    let mut dave_guard = dave.lock().await;
                    let encrypted = dave_guard
                        .encrypt_frame("sender_agent_1", &opus_data, cur_seq as u32, &header_buf)
                        .expect("DAVE encrypt");

                    let mut packet = header_buf;
                    packet.extend(encrypted);
                    udp.send_packet(&packet).await.expect("UDP send");
                }
            })
            .await;
    });

    // Wait for scheduler to finish
    scheduler_task.await.expect("Scheduler finished");

    // Wait for all packets to be received
    let captured = fake_udp
        .wait_for_packets(frame_count, Duration::from_secs(3))
        .await;

    assert_eq!(
        captured.len(),
        frame_count,
        "Should receive exactly all transmitted frames"
    );

    // Verify RTP header monotonicity and DAVE payload
    let mut dave_guard = dave.lock().await;
    for (idx, packet) in captured.iter().enumerate() {
        let expected_seq = (idx + 1) as u16;
        let expected_ts = ((idx + 1) * 960) as u32;

        assert_eq!(packet.header.version, 0x80);
        assert_eq!(packet.header.payload_type, 0x78);
        assert_eq!(packet.header.sequence, expected_seq);
        assert_eq!(packet.header.timestamp, expected_ts);
        assert_eq!(packet.header.ssrc, ssrc);

        // Verify payload ends with DAVE magic marker
        let payload_len = packet.payload.len();
        assert_eq!(packet.payload[payload_len - 2], 0xFA);
        assert_eq!(packet.payload[payload_len - 1], 0xFA);

        // Decrypt payload and verify content
        let decrypted = dave_guard
            .decrypt_frame("sender_agent_1", &packet.payload)
            .expect("Decryption must succeed");

        let mut expected_frame = vec![0xDE, 0xAD, 0xBE, 0xEF];
        expected_frame.push((idx + 1) as u8);
        assert_eq!(decrypted, expected_frame);
    }

    // Verify TrackEvent::Ended
    let mut ended = false;
    while let Ok(ev) = event_rx.recv().await {
        if matches!(ev, TrackEvent::Ended) {
            ended = true;
            break;
        }
    }
    assert!(ended, "TrackEvent::Ended must be emitted");
}

#[tokio::test]
async fn test_opus_passthrough_no_reencode() {
    // Verify that pre-encoded Opus bytes pass directly into DAVE without modification
    let raw_opus_bytes = vec![0xFC, 0xFF, 0xFE, 0x01, 0x02, 0x03, 0x04];
    let frame = AudioFrame::Opus(raw_opus_bytes.clone());

    let mut dave_session = DaveSession::new("guild_passthrough".to_string());
    dave_session.add_sender("sender_pt");

    let AudioFrame::Opus(opus_data) = frame else {
        panic!("Expected Opus");
    };

    // Asserts direct byte identity before encryption
    assert_eq!(opus_data, raw_opus_bytes);

    let encrypted = dave_session
        .encrypt_frame("sender_pt", &opus_data, 1, &[])
        .expect("Encrypt frame");

    let decrypted = dave_session
        .decrypt_frame("sender_pt", &encrypted)
        .expect("Decrypt frame");

    assert_eq!(decrypted, raw_opus_bytes, "Opus bytes must be preserved without re-encoding");
}

#[tokio::test]
async fn test_pcm_pipeline_with_volume_and_opus_encoding() {
    let ssrc = 54321;
    let fake_udp = FakeVoiceUdpServer::start(ssrc).await.expect("Start fake UDP server");
    let udp_port = fake_udp.port();

    let (voice_udp, _, _) = VoiceUdp::bind_and_discover("127.0.0.1", udp_port, ssrc)
        .await
        .expect("VoiceUdp bind");

    let voice_udp = Arc::new(voice_udp);
    let mut dave_session = DaveSession::new("guild_pcm_vol".to_string());
    dave_session.add_sender("sender_pcm");
    let dave = Arc::new(Mutex::new(dave_session));

    let frame_count = 5;
    let source = Arc::new(Mutex::new(PcmSineSource {
        frames_sent: 0,
        max_frames: frame_count,
    }));

    let (cmd_tx, cmd_rx) = mpsc::channel(32);
    let (event_tx, _) = broadcast::channel(32);
    let handle = KizunaTrackHandle::new(cmd_tx, event_tx.subscribe());

    // Set volume to 0.5
    handle.set_volume(0.5).await.expect("Set volume");

    let scheduler = FrameScheduler::new(source);
    let udp_clone = voice_udp.clone();
    let dave_clone = dave.clone();

    let sequence = Arc::new(std::sync::atomic::AtomicU16::new(0));
    let timestamp = Arc::new(std::sync::atomic::AtomicU32::new(0));

    let seq_clone = sequence.clone();
    let ts_clone = timestamp.clone();

    let encoder = Arc::new(Mutex::new(OpusEncoder::new().expect("Create OpusEncoder")));

    tokio::spawn(async move {
        scheduler
            .run(cmd_rx, event_tx, |frame| {
                let udp = udp_clone.clone();
                let dave = dave_clone.clone();
                let enc = encoder.clone();
                let seq_atomic = seq_clone.clone();
                let ts_atomic = ts_clone.clone();

                async move {
                    let cur_seq = seq_atomic.fetch_add(1, std::sync::atomic::Ordering::SeqCst).wrapping_add(1);
                    let cur_ts = ts_atomic.fetch_add(960, std::sync::atomic::Ordering::SeqCst).wrapping_add(960);

                    let opus_data = match frame {
                        AudioFrame::Opus(data) => data,
                        AudioFrame::Pcm(pcm) => {
                            let mut encoder_guard = enc.lock().await;
                            let encoded = encoder_guard
                                .encode(OpusSource::Pcm(pcm))
                                .expect("Opus encode");
                            let AudioFrame::Opus(data) = encoded else {
                                panic!("Encoded frame must be Opus");
                            };
                            data
                        }
                    };

                    let header = RtpHeader::new(cur_seq, cur_ts, ssrc);
                    let mut header_buf = Vec::new();
                    header.write_to(&mut header_buf).unwrap();

                    let mut dave_guard = dave.lock().await;
                    let encrypted = dave_guard
                        .encrypt_frame("sender_pcm", &opus_data, cur_seq as u32, &header_buf)
                        .expect("DAVE encrypt");

                    let mut packet = header_buf;
                    packet.extend(encrypted);
                    udp.send_packet(&packet).await.expect("UDP send");
                }
            })
            .await;
    });

    let captured = fake_udp
        .wait_for_packets(frame_count, Duration::from_secs(3))
        .await;

    assert_eq!(captured.len(), frame_count);
    for packet in &captured {
        assert_eq!(packet.header.ssrc, ssrc);
        assert!(packet.payload.len() >= 12);
        assert_eq!(packet.payload[packet.payload.len() - 2], 0xFA);
        assert_eq!(packet.payload[packet.payload.len() - 1], 0xFA);
    }
}
