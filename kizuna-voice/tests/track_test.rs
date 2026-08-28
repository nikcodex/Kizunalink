use async_trait::async_trait;
use kizuna_voice::audio::{
    AudioFrame, AudioSource, FrameScheduler, KizunaTrackHandle, TrackCommand, TrackEvent,
    TrackState,
};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{broadcast, mpsc, Mutex};

struct MockSource {
    frames: usize,
    max_frames: usize,
}

#[async_trait]
impl AudioSource for MockSource {
    async fn next_frame(&mut self) -> kizuna_voice::error::Result<Option<AudioFrame>> {
        if self.frames >= self.max_frames {
            return Ok(None);
        }
        self.frames += 1;
        Ok(Some(AudioFrame::Opus(vec![0; 10])))
    }
}

#[tokio::test]
async fn test_track_lifecycle() {
    let source = Arc::new(Mutex::new(MockSource {
        frames: 0,
        max_frames: 5,
    }));

    let (cmd_tx, cmd_rx) = mpsc::channel(32);
    let (event_tx, event_rx) = broadcast::channel(32);

    let handle = KizunaTrackHandle::new(cmd_tx, event_rx);
    let scheduler = FrameScheduler::new(source);

    tokio::spawn(async move {
        scheduler.run(cmd_rx, event_tx, |_| async {}).await;
    });

    handle.play().await.unwrap();
    let ev1 = handle.next_event().await.unwrap();
    assert!(matches!(ev1, TrackEvent::Started));

    handle.pause().await.unwrap();
    let ev2 = handle.next_event().await.unwrap();
    assert!(matches!(ev2, TrackEvent::Paused));

    handle.resume().await.unwrap();
    let ev3 = handle.next_event().await.unwrap();
    assert!(matches!(ev3, TrackEvent::Resumed));

    // Wait for the track to finish naturally
    let mut ended = false;
    while let Ok(ev) = handle.next_event().await {
        if matches!(ev, TrackEvent::Ended) {
            ended = true;
            break;
        }
    }
    assert!(ended);
}
