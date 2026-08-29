use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{broadcast, mpsc, Mutex};

#[derive(Debug, Clone)]
pub enum TrackEvent {
    Started,
    Paused,
    Resumed,
    Seeked(Duration),
    Stopped,
    Ended,
    Error(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrackState {
    Idle,
    Loading,
    Playing,
    Paused,
    Stopped,
    Ended,
    Error,
}

pub enum TrackCommand {
    Play,
    Pause,
    Resume,
    Stop,
    Seek(Duration),
    SetVolume(f32),
    GetInfo(tokio::sync::oneshot::Sender<TrackInfo>),
}

#[derive(Debug, Clone)]
pub struct TrackInfo {
    pub state: TrackState,
    pub position: Duration,
    pub duration: Option<Duration>,
    pub volume: f32,
}

#[derive(Clone)]
pub struct KizunaTrackHandle {
    cmd_tx: mpsc::Sender<TrackCommand>,
    event_rx: Arc<Mutex<broadcast::Receiver<TrackEvent>>>,
}

impl KizunaTrackHandle {
    pub fn new(
        cmd_tx: mpsc::Sender<TrackCommand>,
        event_rx: broadcast::Receiver<TrackEvent>,
    ) -> Self {
        Self {
            cmd_tx,
            event_rx: Arc::new(Mutex::new(event_rx)),
        }
    }

    pub async fn play(&self) -> Result<(), String> {
        self.cmd_tx
            .send(TrackCommand::Play)
            .await
            .map_err(|e| e.to_string())
    }

    pub async fn pause(&self) -> Result<(), String> {
        self.cmd_tx
            .send(TrackCommand::Pause)
            .await
            .map_err(|e| e.to_string())
    }

    pub async fn resume(&self) -> Result<(), String> {
        self.cmd_tx
            .send(TrackCommand::Resume)
            .await
            .map_err(|e| e.to_string())
    }

    pub async fn stop(&self) -> Result<(), String> {
        self.cmd_tx
            .send(TrackCommand::Stop)
            .await
            .map_err(|e| e.to_string())
    }

    pub async fn seek(&self, position: Duration) -> Result<(), String> {
        self.cmd_tx
            .send(TrackCommand::Seek(position))
            .await
            .map_err(|e| e.to_string())
    }

    pub async fn set_volume(&self, volume: f32) -> Result<(), String> {
        self.cmd_tx
            .send(TrackCommand::SetVolume(volume))
            .await
            .map_err(|e| e.to_string())
    }

    pub async fn get_info(&self) -> Result<TrackInfo, String> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.cmd_tx
            .send(TrackCommand::GetInfo(tx))
            .await
            .map_err(|e| e.to_string())?;
        rx.await.map_err(|e| e.to_string())
    }

    pub async fn next_event(&self) -> Result<TrackEvent, String> {
        let mut rx = self.event_rx.lock().await;
        rx.recv().await.map_err(|e| e.to_string())
    }

    pub async fn events(&self) -> broadcast::Receiver<TrackEvent> {
        let rx = self.event_rx.lock().await;
        rx.resubscribe()
    }
}
