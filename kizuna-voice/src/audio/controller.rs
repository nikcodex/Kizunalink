use super::scheduler::{FrameScheduler, SchedulerCommand};
use super::source::AudioSource;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, Mutex};
use tracing::{info, warn};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackState {
    Playing,
    Paused,
    Stopped,
    Ended,
    Error,
}

pub struct TrackInfo {
    pub state: TrackState,
    pub position: Duration,
    pub duration: Option<Duration>,
    pub started_at: Option<Instant>,
}

pub struct AudioController {
    scheduler_tx: Option<mpsc::Sender<SchedulerCommand>>,
    pub state: TrackState,
}

impl AudioController {
    pub fn new() -> Self {
        Self {
            scheduler_tx: None,
            state: TrackState::Stopped,
        }
    }

    pub fn attach_scheduler(&mut self, tx: mpsc::Sender<SchedulerCommand>) {
        self.scheduler_tx = Some(tx);
        self.state = TrackState::Playing;
    }

    pub async fn pause(&mut self) {
        if self.state == TrackState::Playing {
            if let Some(tx) = &self.scheduler_tx {
                let _ = tx.send(SchedulerCommand::Pause).await;
                self.state = TrackState::Paused;
            }
        }
    }

    pub async fn resume(&mut self) {
        if self.state == TrackState::Paused {
            if let Some(tx) = &self.scheduler_tx {
                let _ = tx.send(SchedulerCommand::Resume).await;
                self.state = TrackState::Playing;
            }
        }
    }

    pub async fn stop(&mut self) {
        if let Some(tx) = &self.scheduler_tx {
            let _ = tx.send(SchedulerCommand::Stop).await;
            self.state = TrackState::Stopped;
        }
    }
}
