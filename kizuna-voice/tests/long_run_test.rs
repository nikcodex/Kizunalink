// [LOCAL INTEGRATION]
use async_trait::async_trait;
use kizuna_voice::audio::{AudioFrame, AudioSource, FrameScheduler};
use kizuna_voice::dave::protocol::DaveSession;
use kizuna_voice::test_harness::FakeVoiceUdpServer;
use kizuna_voice::transport::{RtpHeader, VoiceUdp};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{broadcast, mpsc, Mutex};

struct LongRunSource {
    sent: usize,
    total: usize,
}

#[async_trait]
impl AudioSource for LongRunSource {
    async fn next_frame(&mut self) -> kizuna_voice::error::Result<Option<AudioFrame>> {
        if self.sent >= self.total {
            Ok(None)
        } else {
            self.sent += 1;
            // Generate dummy Opus frame with counter byte
            let mut data = vec![0x78, 0x00];
            data.extend_from_slice(&(self.sent as u32).to_le_bytes());
            Ok(Some(AudioFrame::Opus(data)))
        }
    }
}

#[tokio::test]
async fn test_long_run_stability_and_rollover() {
    let ssrc = 0xAABBCCDD;
    let fake_udp = FakeVoiceUdpServer::start(ssrc).await.expect("Start fake UDP");
    let udp_port = fake_udp.port();

    let (voice_udp, _, _) = VoiceUdp::bind_and_discover("127.0.0.1", udp_port, ssrc)
        .await
        .expect("VoiceUdp bind");
    let voice_udp = Arc::new(voice_udp);

    let mut dave_session = DaveSession::new("guild_long_run".to_string());
    dave_session.add_sender("sender_long_run");
    let dave = Arc::new(Mutex::new(dave_session));

    let frame_count = 150;
    let source = Arc::new(Mutex::new(LongRunSource {
        sent: 0,
        total: frame_count,
    }));

    let (_cmd_tx, cmd_rx) = mpsc::channel(32);
    let (event_tx, _) = broadcast::channel(32);

    let scheduler = FrameScheduler::new(source);
    let udp_clone = voice_udp.clone();
    let dave_clone = dave.clone();

    let sequence = Arc::new(std::sync::atomic::AtomicU16::new(u16::MAX - 50));
    let timestamp = Arc::new(std::sync::atomic::AtomicU32::new(u32::MAX - 5000));

    let initial_seq = sequence.load(std::sync::atomic::Ordering::SeqCst);
    let initial_ts = timestamp.load(std::sync::atomic::Ordering::SeqCst);

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
                        panic!("Expected Opus frame");
                    };

                    let header = RtpHeader::new(cur_seq, cur_ts, ssrc);
                    let mut header_buf = Vec::new();
                    header.write_to(&mut header_buf).unwrap();

                    let mut dave_guard = dave.lock().await;
                    let encrypted = dave_guard
                        .encrypt_frame("sender_long_run", &opus_data, cur_seq as u32, &header_buf)
                        .expect("DAVE encrypt");

                    let mut packet = header_buf;
                    packet.extend(encrypted);
                    udp.send_packet(&packet).await.expect("Send UDP packet");
                }
            })
            .await;
    });

    scheduler_task.await.expect("Scheduler finished");

    let captured = fake_udp
        .wait_for_packets(frame_count, Duration::from_secs(5))
        .await;

    assert_eq!(
        captured.len(),
        frame_count,
        "All frames must be received across rollover"
    );

    let mut cur_seq = initial_seq;
    let mut cur_ts = initial_ts;
    let mut dave_guard = dave.lock().await;

    for (i, pkt) in captured.iter().enumerate() {
        cur_seq = cur_seq.wrapping_add(1);
        cur_ts = cur_ts.wrapping_add(960);

        assert_eq!(pkt.header.sequence, cur_seq, "Sequence rollover mismatch at index {}", i);
        assert_eq!(pkt.header.timestamp, cur_ts, "Timestamp rollover mismatch at index {}", i);
        assert_eq!(pkt.header.ssrc, ssrc);

        // Verify decryption succeeds across boundary
        let decrypted = dave_guard
            .decrypt_frame("sender_long_run", &pkt.payload)
            .expect("Decryption must succeed");

        let mut expected = vec![0x78, 0x00];
        expected.extend_from_slice(&((i + 1) as u32).to_le_bytes());
        assert_eq!(decrypted, expected);
    }
}
