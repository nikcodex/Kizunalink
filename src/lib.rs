pub mod config;
pub mod dave;
pub mod dsp;
pub mod metrics;
pub mod models;
pub mod player;
pub mod plugins;
pub mod ratelimit;
pub mod rest;
pub mod security;
pub mod sources;
pub mod stats;
pub mod track_encoding;
pub mod util;
pub mod ws;

use std::sync::Arc;
use tokio::sync::broadcast;
use player::manager::PlayerManager;
use ratelimit::RateLimiter;
use sources::{
    apple_music::AppleMusicSource, bandcamp::BandcampSource, deezer::DeezerSource,
    jiosaavn::JioSaavnSource, niconico::NicoNicoSource, route_planner::RoutePlanner,
    soundcloud::SoundCloudSource, spotify::SpotifySource, twitch::TwitchSource, vimeo::VimeoSource,
    youtube::YouTubeSource,
};

#[derive(Clone)]
pub struct AppState {
    pub player_manager: Arc<PlayerManager>,
    pub jiosaavn: Arc<JioSaavnSource>,
    pub youtube: Arc<YouTubeSource>,
    pub spotify: Arc<SpotifySource>,
    pub soundcloud: Arc<SoundCloudSource>,
    pub bandcamp: Arc<BandcampSource>,
    pub twitch: Arc<TwitchSource>,
    pub vimeo: Arc<VimeoSource>,
    pub niconico: Arc<NicoNicoSource>,
    pub apple_music: Arc<AppleMusicSource>,
    pub deezer: Arc<DeezerSource>,
    pub plugin_manager: Arc<plugins::PluginManager>,
    pub dave_manager: dave::DaveManager,
    pub route_planner: Option<Arc<RoutePlanner>>,
    pub rate_limiter: Arc<RateLimiter>,
    pub password: String,
    pub start_time: std::time::Instant,
    pub event_tx: broadcast::Sender<String>,
}
