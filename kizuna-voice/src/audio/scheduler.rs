use super::packet::AudioFrame;
use super::source::AudioSource;
use super::track::{TrackCommand, TrackEvent, TrackInfo, TrackState};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{broadcast, mpsc, Mutex};
use tokio::time::{interval, MissedTickBehavior};

pub struct FrameScheduler {
    source: Arc<Mutex<dyn AudioSource>>,
}

impl FrameScheduler {
    pub fn new(source: Arc<Mutex<dyn AudioSource>>) -> Self {
        Self { source }
    }

    pub async fn run<F, Fut>(
        &self,
        mut rx_cmd: mpsc::Receiver<TrackCommand>,
        event_tx: broadcast::Sender<TrackEvent>,
        mut send_callback: F,
    ) where
        F: FnMut(AudioFrame) -> Fut,
        Fut: std::future::Future<Output = ()> + Send,
    {
        let mut interval = interval(Duration::from_millis(20));
        interval.set_missed_tick_behavior(MissedTickBehavior::Skip);

        let mut state = TrackState::Playing;
        let _ = event_tx.send(TrackEvent::Started);
        let mut position = Duration::from_secs(0);
        let mut volume = 1.0f32;
        let mut last_tick = Instant::now();

        // Decode on a separate task. The source may block on network input, but
        // command handling stays in this loop so Stop/Pause/Resume are not held
        // hostage by a stalled stream.
        let (request_tx, mut request_rx) = mpsc::channel::<()>(1);
        let (frame_tx, mut frame_rx) = mpsc::channel(1);
        let source = self.source.clone();
        let source_task = tokio::spawn(async move {
            while request_rx.recv().await.is_some() {
                let result = {
                    let mut source = source.lock().await;
                    source.next_frame().await
                };
                if frame_tx.send(result).await.is_err() {
                    break;
                }
            }
        });
        let mut frame_pending = false;

        loop {
            tokio::select! {
                cmd_opt = rx_cmd.recv() => {
                    match cmd_opt {
                        Some(TrackCommand::Play) => {
                            if state != TrackState::Playing {
                                state = TrackState::Playing;
                                let _ = event_tx.send(TrackEvent::Started);
                                interval.reset_immediately();
                                last_tick = Instant::now();
                            }
                        }
                        Some(TrackCommand::Pause) => {
                            if state == TrackState::Playing {
                                state = TrackState::Paused;
                                let _ = event_tx.send(TrackEvent::Paused);
                            }
                        }
                        Some(TrackCommand::Resume) => {
                            if state == TrackState::Paused {
                                state = TrackState::Playing;
                                let _ = event_tx.send(TrackEvent::Resumed);
                                interval.reset_immediately();
                                last_tick = Instant::now();
                            }
                        }
                        Some(TrackCommand::Stop) | None => {
                            let _ = event_tx.send(TrackEvent::Stopped);
                            source_task.abort();
                            break;
                        }
                        Some(TrackCommand::Seek(pos)) => {
                            let mut src = self.source.lock().await;
                            match src.seek(pos).await {
                                Ok(_) => {
                                    position = pos;
                                    let _ = event_tx.send(TrackEvent::Seeked(pos));
                                }
                                Err(e) => {
                                    let _ = event_tx.send(TrackEvent::Error(format!("Seek failed: {}", e)));
                                }
                            }
                        }
                        Some(TrackCommand::SetVolume(vol)) => {
                            // The KizunaLink player applies volume in its DSP
                            // chain. Keep this value for TrackInfo compatibility,
                            // but do not multiply PCM a second time here.
                            volume = vol.clamp(0.0, 1000.0);
                        }
                        Some(TrackCommand::GetInfo(tx)) => {
                            let _ = tx.send(TrackInfo {
                                state: state.clone(),
                                position,
                                duration: None,
                                volume,
                            });
                        }
                    }
                }
                _ = interval.tick() => {
                    if state == TrackState::Playing && !frame_pending {
                        let elapsed = last_tick.elapsed();
                        position += elapsed;
                        last_tick = Instant::now();
                        if request_tx.send(()).await.is_ok() {
                            frame_pending = true;
                        }
                    }
                }
                frame_result = frame_rx.recv(), if frame_pending => {
                    frame_pending = false;
                    match frame_result {
                        Some(Ok(Some(frame))) => {
                            // A frame requested before Pause may arrive after the
                            // state change; discard it instead of emitting audio
                            // while paused.
                            if state == TrackState::Playing {
                                send_callback(frame).await;
                            }
                        }
                        Some(Ok(None)) => {
                            let _ = event_tx.send(TrackEvent::Ended);
                            source_task.abort();
                            break;
                        }
                        Some(Err(e)) => {
                            let _ = event_tx.send(TrackEvent::Error(e.to_string()));
                            source_task.abort();
                            break;
                        }
                        None => {
                            let _ = event_tx.send(TrackEvent::Error("audio source task stopped".into()));
                            break;
                        }
                    }
                }
            }
        }
    }
}
