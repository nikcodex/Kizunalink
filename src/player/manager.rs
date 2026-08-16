use crate::models::protocol::{PlayerResponse, PlayerUpdatePayload};
use crate::player::guild_player::GuildPlayer;
use crate::sources::jiosaavn::JioSaavnSource;
use base64::{engine::general_purpose::STANDARD, Engine as _};
use dashmap::DashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::info;

pub struct PlayerManager {
    players: DashMap<String, Arc<RwLock<GuildPlayer>>>,
    jiosaavn: Arc<JioSaavnSource>,
    http_client: reqwest::Client,
    pub bot_user_id: Arc<RwLock<String>>,
}

impl PlayerManager {
    pub fn new(jiosaavn: Arc<JioSaavnSource>) -> Arc<Self> {
        Arc::new(Self {
            players: DashMap::new(),
            jiosaavn,
            http_client: reqwest::Client::builder()
                .tcp_keepalive(Some(std::time::Duration::from_secs(30)))
                .build()
                .unwrap_or_default(),
            bot_user_id: Arc::new(RwLock::new("0".to_string())),
        })
    }

    pub async fn get_player(&self, guild_id: &str) -> Option<PlayerResponse> {
        if let Some(player_arc) = self.players.get(guild_id) {
            let player = player_arc.read().await;
            Some(player.to_response())
        } else {
            None
        }
    }

    pub async fn update_player(
        &self,
        guild_id: &str,
        payload: PlayerUpdatePayload,
    ) -> Result<PlayerResponse, String> {
        let user_id = self.bot_user_id.read().await.clone();

        let player_arc = self
            .players
            .entry(guild_id.to_string())
            .or_insert_with(|| Arc::new(RwLock::new(GuildPlayer::new(guild_id.to_string(), user_id))))
            .clone();

        let mut player = player_arc.write().await;

        // 1. Update Voice State if provided
        if let Some(voice) = payload.voice {
            info!("🔌 Received voice credentials for guild: {}", guild_id);
            player.set_voice(voice).await;
        }

        // 2. Update Volume
        if let Some(volume) = payload.volume {
            player.set_volume(volume);
        }

        // 3. Update Paused
        if let Some(paused) = payload.paused {
            player.set_paused(paused);
        }

        // 4. Update Position
        if let Some(position) = payload.position {
            player.seek(position);
        }

        // 5. Update Filters
        if let Some(filters) = payload.filters {
            player.filters = filters;
        }

        // 6. Update Track (Play / Stop)
        if let Some(encoded_opt) = payload.encoded_track {
            if encoded_opt.trim().is_empty() {
                info!("⏹️ Stopped player for guild: {}", guild_id);
                player.stop();
            } else {
                let track_id = if let Ok(decoded) = STANDARD.decode(&encoded_opt) {
                    String::from_utf8_lossy(&decoded).to_string()
                } else {
                    encoded_opt.clone()
                };

                let clean_id = track_id.strip_prefix("jiosaavn:").unwrap_or(&track_id);

                if let Ok(results) = self.jiosaavn.search(clean_id, 1).await {
                    if let Some(mut track) = results.into_iter().next() {
                        if let Ok(stream_url) = self.jiosaavn.resolve_stream_url(&track.info.identifier).await {
                            track.info.stream_url = Some(stream_url.clone());
                            player.play_stream(track, stream_url, self.http_client.clone()).await;
                        }
                    }
                }
            }
        }

        Ok(player.to_response())
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

    pub fn count_players(&self) -> (usize, usize) {
        (self.players.len(), self.players.len())
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
