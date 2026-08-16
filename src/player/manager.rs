use crate::models::protocol::{PlayerResponse, PlayerUpdatePayload};
use crate::player::guild_player::GuildPlayer;
use crate::sources::jiosaavn::JioSaavnSource;
use base64::{engine::general_purpose::STANDARD, Engine as _};
use dashmap::DashMap;
use std::sync::Arc;
use tracing::info;

pub struct PlayerManager {
    players: DashMap<String, GuildPlayer>,
    jiosaavn: Arc<JioSaavnSource>,
}

impl PlayerManager {
    pub fn new(jiosaavn: Arc<JioSaavnSource>) -> Arc<Self> {
        Arc::new(Self {
            players: DashMap::new(),
            jiosaavn,
        })
    }

    /// Retrieve or initialize a player for a Discord guild
    pub fn get_or_create(&self, guild_id: &str) -> GuildPlayer {
        self.players
            .entry(guild_id.to_string())
            .or_insert_with(|| GuildPlayer::new(guild_id.to_string()))
            .clone()
    }

    /// Retrieve existing player
    pub fn get_player(&self, guild_id: &str) -> Option<PlayerResponse> {
        self.players.get(guild_id).map(|p| p.to_response())
    }

    /// Update player state via Lavalink v4 PATCH payload
    pub async fn update_player(
        &self,
        guild_id: &str,
        payload: PlayerUpdatePayload,
    ) -> Result<PlayerResponse, String> {
        let mut entry = self
            .players
            .entry(guild_id.to_string())
            .or_insert_with(|| GuildPlayer::new(guild_id.to_string()));

        let player = entry.value_mut();

        // 1. Update Voice State if provided
        if let Some(voice) = payload.voice {
            info!("🔌 Updated voice state for guild: {}", guild_id);
            player.set_voice(voice);
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
                // Decode track string or identifier
                let track_id = if let Ok(decoded) = STANDARD.decode(&encoded_opt) {
                    String::from_utf8_lossy(&decoded).to_string()
                } else {
                    encoded_opt.clone()
                };

                let clean_id = track_id.strip_prefix("jiosaavn:").unwrap_or(&track_id);

                // Fetch JioSaavn track info & 320kbps stream URL
                if let Ok(results) = self.jiosaavn.search(clean_id, 1).await {
                    if let Some(mut track) = results.into_iter().next() {
                        // Resolve direct 320kbps CDN stream
                        if let Ok(stream_url) = self.jiosaavn.resolve_stream_url(&track.info.identifier).await {
                            track.info.stream_url = Some(stream_url);
                        }

                        info!("▶️ Started track \"{}\" for guild {}", track.info.title, guild_id);
                        player.set_track(track);
                    }
                }
            }
        }

        Ok(player.to_response())
    }

    /// Destroy and remove a player
    pub fn destroy_player(&self, guild_id: &str) -> bool {
        self.players.remove(guild_id).is_some()
    }

    /// Get all active player counts (total, playing)
    pub fn count_players(&self) -> (usize, usize) {
        let total = self.players.len();
        let playing = self.players.iter().filter(|p| p.current_track.is_some() && !p.paused).count();
        (total, playing)
    }

    /// Get all players as responses
    pub fn get_all_players(&self) -> Vec<PlayerResponse> {
        self.players.iter().map(|p| p.to_response()).collect()
    }
}
