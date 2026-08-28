use super::packet::AudioFrame;
use super::source::AudioSource;
use super::track::{TrackCommand, TrackEvent, TrackInfo, TrackState};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{broadcast, mpsc, Mutex};
use tokio::time::{interval, MissedTickBehavior};
use tracing::{debug, error, info};

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

        let mut state = TrackState::Idle;
        let mut position = Duration::from_secs(0);
        let mut volume = 1.0f32;
        let mut last_tick = Instant::now();

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
                            state = TrackState::Stopped;
                            let _ = event_tx.send(TrackEvent::Stopped);
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
                            volume = vol.clamp(0.0, 1000.0);
                        }
                        Some(TrackCommand::GetInfo(tx)) => {
                            let _ = tx.send(TrackInfo {
                                state: state.clone(),
                                position,
                                duration: None, // Could fetch from source
                                volume,
                            });
                        }
                    }
                }
                _ = interval.tick() => {
                    if state != TrackState::Playing { continue; }

                    let elapsed = last_tick.elapsed();
                    position += elapsed;
                    last_tick = Instant::now();

                    let frame_opt = {
                        let mut source = self.source.lock().await;
                        match source.next_frame().await {
                            Ok(Some(mut frame)) => {
                                // Apply volume if PCM
                                if volume != 1.0 {
                                    if let AudioFrame::Pcm(ref mut samples) = frame {
                                        for sample in samples.iter_mut() {
                                            *sample = (*sample as f32 * volume).clamp(i16::MIN as f32, i16::MAX as f32) as i16;
                                        }
                                    }
                                }
                                Some(frame)
                            }
                            Ok(None) => {
                                state = TrackState::Ended;
                                let _ = event_tx.send(TrackEvent::Ended);
                                break;
                            }
                            Err(e) => {
                                state = TrackState::Error;
                                let _ = event_tx.send(TrackEvent::Error(e.to_string()));
                                break;
                            }
                        }
                    };

                    if let Some(frame) = frame_opt {
                        send_callback(frame).await;
                    }
                }
            }
        }
    }
}
