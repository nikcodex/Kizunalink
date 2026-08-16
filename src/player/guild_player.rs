use crate::models::{
    protocol::{PlayerResponse, PlayerState, VoiceStateUpdate},
    track::LavalinkTrack,
};
use songbird::driver::Driver;
use songbird::id::{GuildId, UserId};
use songbird::input::HttpRequest;
use songbird::ConnectionInfo;
use songbird::tracks::TrackHandle;
use std::num::NonZeroU64;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::Mutex;
use tracing::{error, info, warn};

pub struct GuildPlayer {
    pub guild_id: String,
    pub user_id: String,
    pub current_track: Option<LavalinkTrack>,
    pub volume: u32,
    pub paused: bool,
    pub position: u64,
    pub voice: Option<VoiceStateUpdate>,
    pub filters: serde_json::Value,
    pub last_update: u64,
    pub driver: Arc<Mutex<Driver>>,
    pub track_handle: Option<TrackHandle>,
}

impl GuildPlayer {
    pub fn new(guild_id: String, user_id: String) -> Self {
        let driver = Driver::new(Default::default());
        Self {
            guild_id,
            user_id,
            current_track: None,
            volume: 100,
            paused: false,
            position: 0,
            voice: None,
            filters: serde_json::json!({}),
            last_update: current_timestamp(),
            driver: Arc::new(Mutex::new(driver)),
            track_handle: None,
        }
    }

    pub async fn set_voice(&mut self, voice: VoiceStateUpdate) {
        let endpoint_raw = voice.endpoint.clone();
        let endpoint = endpoint_raw
            .trim_start_matches("wss://")
            .trim_start_matches("ws://")
            .split(':')
            .next()
            .unwrap_or(&endpoint_raw)
            .to_string();

        let guild_num = self.guild_id.parse::<u64>().unwrap_or(0);
        let user_num = self.user_id.parse::<u64>().unwrap_or(0);

        let guild_nz = NonZeroU64::new(guild_num).unwrap_or(NonZeroU64::new(1).unwrap());
        let user_nz = NonZeroU64::new(user_num).unwrap_or(NonZeroU64::new(1).unwrap());

        let info = ConnectionInfo {
            endpoint,
            guild_id: GuildId::from(guild_nz),
            channel_id: None,
            session_id: voice.session_id.clone(),
            token: voice.token.clone(),
            user_id: UserId::from(user_nz),
        };

        let mut driver_lock = self.driver.lock().await;
        if let Err(e) = driver_lock.connect(info).await {
            error!("Voice connection failed for guild {}: {:?}", self.guild_id, e);
        } else {
            info!("Voice driver connected for guild: {}", self.guild_id);
        }

        self.voice = Some(voice);
        self.last_update = current_timestamp();
    }

    pub async fn play_stream(&mut self, track: LavalinkTrack, stream_url: String, http_client: reqwest::Client) {
        let input = HttpRequest::new(http_client, stream_url).into();

        let mut driver_lock = self.driver.lock().await;
        let handle = driver_lock.play(input);
        if let Err(e) = handle.set_volume(self.volume as f32 / 100.0) {
            warn!("Failed to set volume on handle: {:?}", e);
        }

        self.track_handle = Some(handle);
        self.current_track = Some(track);
        self.position = 0;
        self.paused = false;
        self.last_update = current_timestamp();
    }

    pub fn stop(&mut self) {
        if let Some(handle) = &self.track_handle {
            let _ = handle.stop();
        }
        self.current_track = None;
        self.position = 0;
        self.paused = false;
        self.last_update = current_timestamp();
    }

    pub fn set_paused(&mut self, paused: bool) {
        if let Some(handle) = &self.track_handle {
            if paused {
                let _ = handle.pause();
            } else {
                let _ = handle.play();
            }
        }
        self.paused = paused;
        self.last_update = current_timestamp();
    }

    pub fn set_volume(&mut self, volume: u32) {
        self.volume = volume.clamp(0, 1000);
        if let Some(handle) = &self.track_handle {
            let _ = handle.set_volume(self.volume as f32 / 100.0);
        }
        self.last_update = current_timestamp();
    }

    pub fn seek(&mut self, position: u64) {
        self.position = position;
        self.last_update = current_timestamp();
    }

    pub fn to_response(&self) -> PlayerResponse {
        let is_connected = self.voice.is_some();
        PlayerResponse {
            guild_id: self.guild_id.clone(),
            track: self.current_track.clone(),
            volume: self.volume,
            paused: self.paused,
            state: PlayerState {
                time: current_timestamp(),
                position: self.position,
                connected: is_connected,
                ping: if is_connected { 12 } else { -1 },
            },
            voice: self.voice.clone().unwrap_or(VoiceStateUpdate {
                token: "".to_string(),
                endpoint: "".to_string(),
                session_id: "".to_string(),
            }),
            filters: self.filters.clone(),
        }
    }
}

fn current_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
