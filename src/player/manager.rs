use crate::models::protocol::{PlayerResponse, PlayerUpdatePayload};
use crate::player::guild_player::GuildPlayer;
use crate::player::queue::LoopMode;
use crate::sources::jiosaavn::JioSaavnSource;
use crate::sources::soundcloud::SoundCloudSource;
use crate::sources::spotify::SpotifySource;
use crate::sources::youtube::YouTubeSource;
use base64::{engine::general_purpose::STANDARD, Engine as _};
use dashmap::DashMap;
use std::sync::Arc;
use tokio::sync::{broadcast, RwLock};
use tracing::{error, info, warn};

pub struct PlayerManager {
    players: DashMap<String, Arc<RwLock<GuildPlayer>>>,
    pub bot_user_id: Arc<RwLock<String>>,
    event_tx: broadcast::Sender<String>,
    jiosaavn: Arc<JioSaavnSource>,
    youtube: Arc<YouTubeSource>,
    spotify: Arc<SpotifySource>,
    soundcloud: Arc<SoundCloudSource>,
}

impl PlayerManager {
    pub fn new(
        event_tx: broadcast::Sender<String>,
        jiosaavn: Arc<JioSaavnSource>,
        youtube: Arc<YouTubeSource>,
        spotify: Arc<SpotifySource>,
        soundcloud: Arc<SoundCloudSource>,
    ) -> Self {
        Self {
            players: DashMap::new(),
            bot_user_id: Arc::new(RwLock::new("0".to_string())),
            event_tx,
            jiosaavn,
            youtube,
            spotify,
            soundcloud,
        }
    }

    pub async fn get_player(&self, guild_id: &str) -> Option<PlayerResponse> {
        if let Some(player_arc) = self.players.get(guild_id) {
            let player = player_arc.read().await;
            Some(player.to_response())
        } else {
            None
        }
    }

    fn create_guild_player(&self, guild_id: &str) -> Arc<RwLock<GuildPlayer>> {
        let user_id = "0".to_string();
        let player = GuildPlayer::new(
            guild_id.to_string(),
            user_id,
            self.event_tx.clone(),
        );
        Arc::new(RwLock::new(player))
    }

    pub async fn update_player(
        &self,
        guild_id: &str,
        payload: PlayerUpdatePayload,
    ) -> Result<PlayerResponse, String> {
        let player_arc = self
            .players
            .entry(guild_id.to_string())
            .or_insert_with(|| self.create_guild_player(guild_id))
            .clone();

        let mut player = player_arc.write().await;

        if let Some(voice) = payload.voice {
            info!("Received voice credentials for guild: {}", guild_id);
            player.set_voice(voice).await;
        }

        if let Some(volume) = payload.volume {
            player.set_volume(volume);
        }

        if let Some(paused) = payload.paused {
            player.set_paused(paused);
        }

        if let Some(position) = payload.position {
            player.seek(position).await;
        }

        player.end_time = payload.end_time;

        if let Some(filters) = payload.filters {
            player.filters = filters;
            player.apply_filters().await;
        }

        let encoded_value = payload
            .track
            .as_ref()
            .and_then(|t| t.encoded.clone())
            .or(payload.encoded_track.clone());

        if let Some(encoded_opt) = encoded_value {
            if encoded_opt.trim().is_empty() {
                info!("Stopping player for guild: {}", guild_id);
                player.stop();
            } else {
                let decoded_str = STANDARD
                    .decode(&encoded_opt)
                    .ok()
                    .and_then(|b| String::from_utf8(b).ok())
                    .unwrap_or_else(|| encoded_opt.clone());

                let (source, id) = if let Some(idx) = decoded_str.find(':') {
                    (&decoded_str[..idx], &decoded_str[idx + 1..])
                } else {
                    ("jiosaavn", decoded_str.as_str())
                };

                let mut track = self.resolve_track_metadata(source, id).await.unwrap_or_else(|| {
                    crate::models::track::LavalinkTrack {
                        encoded: encoded_opt.clone(),
                        info: crate::models::track::TrackInfo {
                            identifier: id.to_string(),
                            is_seekable: true,
                            author: "Unknown".to_string(),
                            length: 0,
                            is_stream: false,
                            position: 0,
                            title: id.to_string(),
                            uri: None,
                            artwork_url: None,
                            isrc: None,
                            source_name: source.to_string(),
                        },
                        plugin_info: serde_json::json!({}),
                        user_data: serde_json::json!({}),
                    }
                });

                track.encoded = encoded_opt;

                if let Some(stream_url) = self.resolve_stream_url(&track).await {
                    info!("Resolved stream URL for guild {} source={}", guild_id, source);
                    player.play_track(track, stream_url).await;
                } else {
                    warn!("Failed to resolve stream URL for guild {}", guild_id);
                    player.emit_track_load_failed(&track, "Failed to resolve stream URL");
                }
            }
        }

        Ok(player.to_response())
    }

    async fn resolve_stream_url(&self, track: &crate::models::track::LavalinkTrack) -> Option<String> {
        let source = &track.info.source_name;
        let identifier = &track.info.identifier;

        match source.as_str() {
            "jiosaavn" => self.jiosaavn.resolve_stream_url(identifier).await.ok(),
            "youtube" => {
                if let Some(url) = &track.info.uri {
                    Some(url.clone())
                } else {
                    self.youtube.resolve_video(identifier).await.ok().flatten().and_then(|t| t.info.uri)
                }
            }
            "soundcloud" => self.soundcloud.resolve_stream(identifier).await.ok(),
            "spotify" => self.spotify.resolve_stream(&track.info).await.ok(),
            "http" => track.info.uri.clone(),
            _ => None,
        }
    }

    async fn resolve_track_metadata(&self, source: &str, id: &str) -> Option<crate::models::track::LavalinkTrack> {
        match source {
            "jiosaavn" => {
                self.jiosaavn.search(id, 1).await.ok()?.into_iter().next()
            }
            "youtube" => {
                self.youtube.resolve_video(id).await.ok()?
            }
            "spotify" => {
                self.spotify.resolve_track(id).await.ok()?
            }
            "soundcloud" => {
                self.soundcloud.search(id, 1).await.ok()?.into_iter().next()
            }
            _ => None,
        }
    }

    pub async fn queue_track(&self, guild_id: &str, encoded: &str) -> Result<PlayerResponse, String> {
        let player_arc = self
            .players
            .entry(guild_id.to_string())
            .or_insert_with(|| self.create_guild_player(guild_id))
            .clone();

        let mut player = player_arc.write().await;

        let decoded_str = STANDARD
            .decode(encoded)
            .ok()
            .and_then(|b| String::from_utf8(b).ok())
            .unwrap_or_else(|| encoded.to_string());

        let (source, id) = if let Some(idx) = decoded_str.find(':') {
            (&decoded_str[..idx], &decoded_str[idx + 1..])
        } else {
            ("jiosaavn", decoded_str.as_str())
        };

        let mut track = self.resolve_track_metadata(source, id).await.unwrap_or_else(|| {
            crate::models::track::LavalinkTrack {
                encoded: encoded.to_string(),
                info: crate::models::track::TrackInfo {
                    identifier: id.to_string(),
                    is_seekable: true,
                    author: "Unknown".to_string(),
                    length: 0,
                    is_stream: false,
                    position: 0,
                    title: id.to_string(),
                    uri: None,
                    artwork_url: None,
                    isrc: None,
                    source_name: source.to_string(),
                },
                plugin_info: serde_json::json!({}),
                user_data: serde_json::json!({}),
            }
        });

        track.encoded = encoded.to_string();

        if !player.is_playing() && player.queue.current.is_none() {
            if let Some(stream_url) = self.resolve_stream_url(&track).await {
                player.play_track(track, stream_url).await;
            }
        } else {
            player.add_to_queue(track);
        }

        Ok(player.to_response())
    }

    pub async fn skip_track(&self, guild_id: &str) -> Result<PlayerResponse, String> {
        if let Some(player_arc) = self.players.get(guild_id) {
            let mut player = player_arc.write().await;
            let next = player.skip_to_next();

            if let Some(track) = next {
                if let Some(url) = self.resolve_stream_url(&track).await {
                    player.play_track(track, url).await;
                }
            }

            Ok(player.to_response())
        } else {
            Err("Player not found".to_string())
        }
    }

    pub async fn previous_track(&self, guild_id: &str) -> Result<PlayerResponse, String> {
        if let Some(player_arc) = self.players.get(guild_id) {
            let mut player = player_arc.write().await;
            let prev = player.skip_to_previous();

            if let Some(track) = prev {
                if let Some(url) = self.resolve_stream_url(&track).await {
                    player.play_track(track, url).await;
                }
            }

            Ok(player.to_response())
        } else {
            Err("Player not found".to_string())
        }
    }

    pub async fn toggle_autoplay(&self, guild_id: &str) -> Result<bool, String> {
        if let Some(player_arc) = self.players.get(guild_id) {
            let mut player = player_arc.write().await;
            let enabled = player.autoplay.toggle();
            Ok(enabled)
        } else {
            Err("Player not found".to_string())
        }
    }

    pub async fn set_loop_mode(&self, guild_id: &str, mode: LoopMode) -> Result<LoopMode, String> {
        if let Some(player_arc) = self.players.get(guild_id) {
            let mut player = player_arc.write().await;
            player.set_loop(mode);
            Ok(player.queue.loop_mode)
        } else {
            Err("Player not found".to_string())
        }
    }

    pub async fn shuffle_queue(&self, guild_id: &str) -> Result<PlayerResponse, String> {
        if let Some(player_arc) = self.players.get(guild_id) {
            let mut player = player_arc.write().await;
            player.shuffle_queue();
            Ok(player.to_response())
        } else {
            Err("Player not found".to_string())
        }
    }

    pub async fn clear_queue(&self, guild_id: &str) -> Result<PlayerResponse, String> {
        if let Some(player_arc) = self.players.get(guild_id) {
            let mut player = player_arc.write().await;
            player.clear_queue();
            Ok(player.to_response())
        } else {
            Err("Player not found".to_string())
        }
    }

    pub fn destroy_player(&self, guild_id: &str) -> bool {
        if let Some((_, player_arc)) = self.players.remove(guild_id) {
            tokio::spawn(async move {
                let mut player = player_arc.write().await;
                player.stop();
            });
            true
        } else {
            false
        }
    }

    pub async fn handle_track_end(&self, guild_id: &str) {
        if let Some(player_arc) = self.players.get(guild_id) {
            let mut player = player_arc.write().await;
            let next_track = player.get_next_track_for_autoplay();

            if let Some(track) = next_track {
                let url = self.resolve_stream_url(&track).await;
                if let Some(stream_url) = url {
                    player.play_track(track, stream_url).await;
                    return;
                }
            }

            if player.autoplay.enabled {
                if let Some(last_track) = &player.queue.current {
                    let autoplay_track = player.autoplay.get_recommendation(
                        last_track,
                        &self.jiosaavn,
                        &self.youtube,
                        &self.spotify,
                    ).await;

                    if let Some(track) = autoplay_track {
                        if let Some(stream_url) = self.resolve_stream_url(&track).await {
                            player.play_track(track, stream_url).await;
                            return;
                        }
                    }
                }
            }

            player.stop();
        }
    }

    pub async fn count_players(&self) -> (usize, usize) {
        let total = self.players.len();
        let mut playing = 0;
        for item in self.players.iter() {
            if item.value().read().await.is_actively_playing() {
                playing += 1;
            }
        }
        (total, playing)
    }

    pub async fn get_all_players(&self) -> Vec<PlayerResponse> {
        let mut responses = Vec::new();
        for item in self.players.iter() {
            let player = item.value().read().await;
            responses.push(player.to_response());
        }
        responses
    }
}
