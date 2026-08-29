// [UNIT]
// [LOCAL INTEGRATION]
use async_trait::async_trait;
use kizuna_voice::audio::{
    AudioFrame, AudioSource, FrameScheduler, KizunaTrackHandle, TrackEvent, TrackState,
};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{broadcast, mpsc, Mutex};
use tokio::time::sleep;

struct InfiniteSource {
    frames_generated: Arc<AtomicUsize>,
}

#[async_trait]
impl AudioSource for InfiniteSource {
    async fn next_frame(&mut self) -> kizuna_voice::error::Result<Option<AudioFrame>> {
        self.frames_generated.fetch_add(1, Ordering::SeqCst);
        Ok(Some(AudioFrame::Opus(vec![0xAA, 0xBB])))
    }

    async fn seek(&mut self, _pos: Duration) -> kizuna_voice::error::Result<()> {
        Ok(())
    }
}

struct FailingSource;

#[async_trait]
impl AudioSource for FailingSource {
    async fn next_frame(&mut self) -> kizuna_voice::error::Result<Option<AudioFrame>> {
        Err(kizuna_voice::error::Error::Connection("Audio stream decode failure".into()))
    }
}

#[tokio::test]
async fn test_track_control_play_pause_resume_seek_stop() {
    let frames_count = Arc::new(AtomicUsize::new(0));
    let source = Arc::new(Mutex::new(InfiniteSource {
        frames_generated: frames_count.clone(),
    }));

    let (cmd_tx, cmd_rx) = mpsc::channel(32);
    let (event_tx, _) = broadcast::channel(32);
    let handle = KizunaTrackHandle::new(cmd_tx, event_tx.subscribe());
    let mut rx_events = handle.events();

    let scheduler = FrameScheduler::new(source);

    let scheduler_task = tokio::spawn(async move {
        scheduler.run(cmd_rx, event_tx, |_| async {}).await;
    });

    // 1. Initial event on start: Started
    let ev1 = rx_events.recv().await.expect("Started event");
    assert!(matches!(ev1, TrackEvent::Started));

    // 2. Pause
    handle.pause().await.expect("Pause cmd");
    let ev2 = rx_events.recv().await.expect("Paused event");
    assert!(matches!(ev2, TrackEvent::Paused));

    let info = handle.get_info().await.expect("Get info");
    assert_eq!(info.state, TrackState::Paused);

    // Ensure frames are not generating while paused
    let count_before = frames_count.load(Ordering::SeqCst);
    sleep(Duration::from_millis(50)).await;
    let count_after = frames_count.load(Ordering::SeqCst);
    assert_eq!(count_before, count_after, "Frames should not tick while paused");

    // 3. Resume
    handle.resume().await.expect("Resume cmd");
    let ev3 = rx_events.recv().await.expect("Resumed event");
    assert!(matches!(ev3, TrackEvent::Resumed));

    let info = handle.get_info().await.expect("Get info");
    assert_eq!(info.state, TrackState::Playing);

    // 4. Seek
    handle.seek(Duration::from_secs(15)).await.expect("Seek cmd");
    let ev4 = rx_events.recv().await.expect("Seeked event");
    assert!(matches!(ev4, TrackEvent::Seeked(d) if d == Duration::from_secs(15)));

    let info = handle.get_info().await.expect("Get info");
    assert_eq!(info.position, Duration::from_secs(15));

    // 5. Volume
    handle.set_volume(1.75).await.expect("Set volume cmd");
    let info = handle.get_info().await.expect("Get info");
    assert!((info.volume - 1.75).abs() < 0.001);

    // 6. Stop
    handle.stop().await.expect("Stop cmd");
    let ev5 = rx_events.recv().await.expect("Stopped event");
    assert!(matches!(ev5, TrackEvent::Stopped));

    // Scheduler task must terminate immediately on Stop
    let res = tokio::time::timeout(Duration::from_secs(1), scheduler_task).await;
    assert!(res.is_ok(), "Scheduler task must exit on stop without hang");
}

#[tokio::test]
async fn test_failing_source_emits_error_and_terminates() {
    let source = Arc::new(Mutex::new(FailingSource));

    let (cmd_tx, cmd_rx) = mpsc::channel(32);
    let (event_tx, _) = broadcast::channel(32);
    let handle = KizunaTrackHandle::new(cmd_tx, event_tx.subscribe());
    let mut rx_events = handle.events();

    let scheduler = FrameScheduler::new(source);

    let scheduler_task = tokio::spawn(async move {
        scheduler.run(cmd_rx, event_tx, |_| async {}).await;
    });

    let _ev_started = rx_events.recv().await.expect("Started event");
    let ev_err = rx_events.recv().await.expect("Error event");
    assert!(matches!(ev_err, TrackEvent::Error(msg) if msg.contains("decode failure")));

    let res = tokio::time::timeout(Duration::from_secs(1), scheduler_task).await;
    assert!(res.is_ok(), "Scheduler must terminate on source error");
}
