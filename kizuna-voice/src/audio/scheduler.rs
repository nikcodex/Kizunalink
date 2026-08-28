use super::packet::AudioFrame;
use super::source::AudioSource;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, Mutex};
use tokio::time::{interval, MissedTickBehavior};
use tracing::{debug, error, info};

pub enum SchedulerCommand {
    Pause,
    Resume,
    Stop,
}

pub struct FrameScheduler {
    source: Arc<Mutex<dyn AudioSource>>,
}

impl FrameScheduler {
    pub fn new(source: Arc<Mutex<dyn AudioSource>>) -> Self {
        Self { source }
    }

    pub async fn run<F, Fut>(
        &self,
        mut rx_cmd: mpsc::Receiver<SchedulerCommand>,
        mut send_callback: F,
    ) where
        F: FnMut(AudioFrame) -> Fut,
        Fut: std::future::Future<Output = ()> + Send,
    {
        let mut interval = interval(Duration::from_millis(20));
        interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
        let mut paused = false;

        loop {
            tokio::select! {
                cmd_opt = rx_cmd.recv() => {
                    match cmd_opt {
                        Some(SchedulerCommand::Pause) => {
                            info!("Scheduler paused");
                            paused = true;
                        },
                        Some(SchedulerCommand::Resume) => {
                            info!("Scheduler resumed");
                            paused = false;
                            // reset interval to avoid instant ticks catching up
                            interval.reset_immediately();
                        },
                        Some(SchedulerCommand::Stop) | None => {
                            info!("Scheduler stopped");
                            break;
                        }
                    }
                }
                _ = interval.tick() => {
                    if paused { continue; }

                    let frame_opt = {
                        let mut source = self.source.lock().await;
                        match source.next_frame().await {
                            Ok(Some(frame)) => Some(frame),
                            Ok(None) => {
                                info!("Audio source EOF");
                                break;
                            }
                            Err(e) => {
                                error!("Audio source error: {}", e);
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
