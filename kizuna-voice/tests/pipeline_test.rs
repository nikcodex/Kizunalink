use async_trait::async_trait;
use kizuna_voice::audio::{AudioFrame, AudioSource, FrameScheduler, SchedulerCommand};
use kizuna_voice::dave::protocol::DaveSession;
use kizuna_voice::transport::RtpHeader;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};

struct TestSource {
    frames_sent: usize,
    max_frames: usize,
}

#[async_trait]
impl AudioSource for TestSource {
    async fn next_frame(&mut self) -> kizuna_voice::error::Result<Option<AudioFrame>> {
        if self.frames_sent >= self.max_frames {
            Ok(None)
        } else {
            self.frames_sent += 1;
            Ok(Some(AudioFrame::Opus(vec![0x42, 0x43]))) // Dummy opus frame
        }
    }
}

#[tokio::test]
async fn test_full_pipeline_mock() {
    let source = Arc::new(Mutex::new(TestSource {
        frames_sent: 0,
        max_frames: 5,
    }));
    let scheduler = FrameScheduler::new(source);
    let (tx, rx) = mpsc::channel(10);

    // We will use DaveSession in its default state (unactivated) to prove wiring
    let dave = Arc::new(Mutex::new(DaveSession::new("test_guild".into())));

    let (udp_tx, mut udp_rx) = mpsc::channel(10);

    let mut sequence = 0;
    let mut timestamp = 0;

    scheduler
        .run(rx, |frame| {
            let dave = dave.clone();
            let udp_tx = udp_tx.clone();

            async move {
                sequence += 1;
                timestamp += 960;

                let AudioFrame::Opus(opus_data) = frame else {
                    panic!("Expected Opus")
                };
                let header = RtpHeader::new(sequence, timestamp, 12345);
                let mut packet = Vec::new();
                header.write_to(&mut packet).unwrap();

                let mut dave_guard = dave.lock().await;
                if dave_guard.is_active() {
                    let encrypted = dave_guard
                        .encrypt_frame("sender1", &opus_data, sequence as u32)
                        .unwrap();
                    packet.extend(encrypted);
                } else {
                    packet.extend(opus_data);
                }

                udp_tx.send(packet).await.unwrap();
            }
        })
        .await;

    let mut packets_received = 0;
    while let Some(_) = udp_rx.recv().await {
        packets_received += 1;
    }

    assert_eq!(
        packets_received, 5,
        "Should have received exactly 5 RTP packets via UDP mock"
    );
}
