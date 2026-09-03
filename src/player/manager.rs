use crate::models::protocol::{PlayerResponse, PlayerUpdatePayload};
use crate::player::guild_player::GuildPlayer;
use crate::player::queue::LoopMode;
use crate::sources::apple_music::AppleMusicSource;
use crate::sources::deezer::DeezerSource;
use crate::sources::jiosaavn::JioSaavnSource;
use crate::sources::soundcloud::SoundCloudSource;
use crate::sources::spotify::SpotifySource;
use crate::sources::youtube::YouTubeSource;
use dashmap::DashMap;
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc, RwLock};
use tracing::{info, warn};

pub struct PlayerManager {
    players: DashMap<String, Arc<RwLock<GuildPlayer>>>,
    pub bot_user_id: Arc<RwLock<String>>,
    event_tx: broadcast::Sender<String>,
    track_end_tx: mpsc::UnboundedSender<String>,
    jiosaavn: Arc<JioSaavnSource>,
    youtube: Arc<YouTubeSource>,
    spotify: Arc<SpotifySource>,
    soundcloud: Arc<SoundCloudSource>,
    deezer: Arc<DeezerSource>,
    apple_music: Arc<AppleMusicSource>,
}

impl PlayerManager {
    pub fn new(
        event_tx: broadcast::Sender<String>,
        jiosaavn: Arc<JioSaavnSource>,
        youtube: Arc<YouTubeSource>,
        spotify: Arc<SpotifySource>,
        soundcloud: Arc<SoundCloudSource>,
        deezer: Arc<DeezerSource>,
        apple_music: Arc<AppleMusicSource>,
    ) -> Self {
        let (track_end_tx, mut track_end_rx) = mpsc::unbounded_channel::<String>();

        let manager = Self {
            players: DashMap::new(),
            bot_user_id: Arc::new(RwLock::new("0".to_string())),
            event_tx,
            track_end_tx,
            jiosaavn,
            youtube,
            spotify,
            soundcloud,
            deezer,
            apple_music,
        };

        let manager_arc = Arc::new(manager);
        let task_manager = manager_arc.clone();

        tokio::spawn(async move {
            while let Some(guild_id) = track_end_rx.recv().await {
                task_manager.handle_track_end(&guild_id).await;
            }
        });

        // We return the unwrapped struct from Arc or deref
        match Arc::try_unwrap(manager_arc) {
            Ok(m) => m,
            Err(arc) => (*arc).clone_shallow(),
        }
    }

    fn clone_shallow(&self) -> Self {
        Self {
            players: self.players.clone(),
            bot_user_id: self.bot_user_id.clone(),
            event_tx: self.event_tx.clone(),
            track_end_tx: self.track_end_tx.clone(),
            jiosaavn: self.jiosaavn.clone(),
            youtube: self.youtube.clone(),
            spotify: self.spotify.clone(),
            soundcloud: self.soundcloud.clone(),
            deezer: self.deezer.clone(),
            apple_music: self.apple_music.clone(),
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

    async fn create_guild_player(&self, guild_id: &str) -> Arc<RwLock<GuildPlayer>> {
        let user_id = self.bot_user_id.read().await.clone();
        let player = GuildPlayer::new(
            guild_id.to_string(),
            user_id,
            self.event_tx.clone(),
            self.track_end_tx.clone(),
        );
        Arc::new(RwLock::new(player))
    }

    pub async fn update_player(
        &self,
        guild_id: &str,
        payload: PlayerUpdatePayload,
        no_replace: bool,
    ) -> Result<PlayerResponse, String> {
        let player_arc = if let Some(p) = self.players.get(guild_id) {
            p.clone()
        } else {
            let p = self.create_guild_player(guild_id).await;
            self.players.insert(guild_id.to_string(), p.clone());
            p
        };

        // Determine if track update is requested
        let encoded_value = payload
            .track
            .as_ref()
            .and_then(|t| t.encoded.clone())
            .or(payload.encoded_track.clone());

        let track_identifier = payload
            .track
            .as_ref()
            .and_then(|t| t.identifier.clone())
            .or(payload.identifier.clone());

        let mut should_stop_track = false;
        let mut resolved_track = None;
        let mut resolved_stream_url = None;

        let user_data_override = payload.track.as_ref().and_then(|t| t.user_data.clone());

        if let Some(ref enc) = encoded_value {
            if enc.trim().is_empty() {
                should_stop_track = true;
            } else {
                // Decode track
                match crate::track_encoding::decode_track(enc) {
                    Ok(mut track) => {
                        track.encoded = enc.clone();
                        if let Some(ref ud) = user_data_override {
                            track.user_data = ud.clone();
                        }
                        let stream_url = self.resolve_stream_url(&track).await;
                        resolved_track = Some(track);
                        resolved_stream_url = stream_url;
                    }
                    Err(e) => {
                        warn!("Track decode error for guild {}: {}", guild_id, e);
                    }
                }
            }
        } else if let Some(ref id) = track_identifier {
            if id.trim().is_empty() {
                should_stop_track = true;
            } else {
                // Resolve identifier
                if let Some(mut track) = self.resolve_identifier(id).await {
                    if let Some(ref ud) = user_data_override {
                        track.user_data = ud.clone();
                    }
                    let stream_url = self.resolve_stream_url(&track).await;
                    resolved_track = Some(track);
                    resolved_stream_url = stream_url;
                }
            }
        }

        // Lock player briefly to apply all state updates
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

        if let Some(autoplay) = payload.autoplay {
            player.autoplay.enabled = autoplay;
            info!("Set autoplay to {} for guild: {}", autoplay, guild_id);
        }

        if let Some(ref l_mode) = payload.loop_mode {
            let mode = match l_mode.to_lowercase().as_str() {
                "track" => crate::player::queue::LoopMode::Track,
                "queue" => crate::player::queue::LoopMode::Queue,
                _ => crate::player::queue::LoopMode::None,
            };
            player.set_loop(mode);
            info!("Set loop mode to {:?} for guild: {}", mode, guild_id);
        }

        if should_stop_track {
            info!("Stopping player for guild: {}", guild_id);
            player.stop();
        } else if let Some(track) = resolved_track {
            let is_currently_playing = player.is_playing();
            if !(no_replace && is_currently_playing) {
                if let Some(stream_url) = resolved_stream_url {
                    info!(
                        "Playing track '{}' for guild {}",
                        track.info.title, guild_id
                    );
                    player.play_track(track, stream_url).await;
                } else {
                    warn!("Failed to resolve stream URL for guild {}", guild_id);
                    player
                        .emit_track_load_failed(&track, "Failed to resolve playable audio stream");
                }
            }
        }

        Ok(player.to_response())
    }

    async fn resolve_identifier(&self, id: &str) -> Option<crate::models::track::LavalinkTrack> {
        let clean = id.trim();
        if clean.starts_with("http://") || clean.starts_with("https://") {
            if clean.contains("spotify.com") {
                if let Some(track_id) = clean
                    .split("/track/")
                    .nth(1)
                    .and_then(|s| s.split('?').next())
                {
                    return self.spotify.resolve_track(track_id).await.ok().flatten();
                }
            } else if clean.contains("youtube.com") || clean.contains("youtu.be") {
                let vid =
                    if let Some(v) = clean.split("v=").nth(1).and_then(|s| s.split('&').next()) {
                        v
                    } else if let Some(v) = clean
                        .split("youtu.be/")
                        .nth(1)
                        .and_then(|s| s.split('?').next())
                    {
                        v
                    } else {
                        clean
                    };
                return self.youtube.resolve_video(vid).await.ok().flatten();
            } else if clean.contains("deezer.com/") {
                if let Some(track_id) = clean
                    .split("/track/")
                    .nth(1)
                    .and_then(|s| s.split('?').next())
                {
                    return self.deezer.resolve_track(track_id).await.ok().flatten();
                }
            } else if clean.contains("music.apple.com/") {
                let track_id = if let Some(i_param) =
                    clean.split("i=").nth(1).and_then(|s| s.split('&').next())
                {
                    i_param.to_string()
                } else {
                    clean.rsplit('/').next()?.split('?').next()?.to_string()
                };
                return self
                    .apple_music
                    .resolve_track(&track_id)
                    .await
                    .ok()
                    .flatten();
            }
            // SSRF protection: validate URL before creating HTTP track
            if let Err(e) = crate::security::validate_url(clean) {
                warn!("SSRF blocked in resolve_identifier for '{}': {}", clean, e);
                return None;
            }
            return Some(crate::util::create_http_track(clean));
        }

        self.jiosaavn
            .search(clean, 1)
            .await
            .ok()?
            .into_iter()
            .next()
            .or_else(|| None)
    }

    pub async fn resolve_stream_url(
        &self,
        track: &crate::models::track::LavalinkTrack,
    ) -> Option<String> {
        let source = &track.info.source_name;
        let identifier = &track.info.identifier;

        match source.as_str() {
            "jiosaavn" => self.jiosaavn.resolve_stream_url(identifier).await.ok(),
            "youtube" => {
                if let Some(url) = &track.info.uri {
                    if url.starts_with("http")
                        && !url.contains("youtube.com")
                        && !url.contains("youtu.be")
                    {
                        return Some(url.clone());
                    }
                }
                let yt_url = self.youtube
                    .resolve_video(identifier)
                    .await
                    .ok()
                    .flatten()
                    .and_then(|t| t.info.uri);

                if let Some(ref u) = yt_url {
                    if u.starts_with("http") && !u.contains("youtube.com") && !u.contains("youtu.be") {
                        return Some(u.clone());
                    }
                }

                // Fallback: Mirror to JioSaavn for 320kbps audio if YouTube stream is ciphered/blocked
                let query = format!("{} {}", track.info.title, track.info.author);
                if let Ok(js_tracks) = self.jiosaavn.search(&query, 1).await {
                    if let Some(first) = js_tracks.into_iter().next() {
                        if let Ok(js_url) = self.jiosaavn.resolve_stream_url(&first.info.identifier).await {
                            info!("⚡ YouTube fallback -> JioSaavn 320kbps for '{}'", track.info.title);
                            return Some(js_url);
                        }
                    }
                }
                None
            }
            "soundcloud" => self.soundcloud.resolve_stream(identifier).await.ok(),
            "spotify" => self.resolve_spotify_mirror(track).await,
            "deezer" => self.resolve_deezer_mirror(track).await,
            "applemusic" => self.resolve_apple_music_mirror(track).await,
            "http" => track.info.uri.clone(),
            _ => track.info.uri.clone(),
        }
    }

    /// Mirror Spotify track to JioSaavn (320kbps) or YouTube via ISRC/title matching.
    async fn resolve_spotify_mirror(
        &self,
        track: &crate::models::track::LavalinkTrack,
    ) -> Option<String> {
        let query = format!("{} {}", track.info.title, track.info.author);

        // 1. Try ISRC-based YouTube search for exact match
        if let Some(isrc) = &track.info.isrc {
            let isrc_query = format!("isrc:{}", isrc);
            if let Ok(yt_tracks) = self.youtube.search(&isrc_query, 1).await {
                if let Some(yt_track) = yt_tracks.into_iter().next() {
                    if let Ok(Some(yt_res)) =
                        self.youtube.resolve_video(&yt_track.info.identifier).await
                    {
                        if let Some(url) = &yt_res.info.uri {
                            info!(
                                "⚡ Spotify->YouTube via ISRC: '{}' for '{}'",
                                isrc, track.info.title
                            );
                            return Some(url.clone());
                        }
                    }
                }
            }
        }

        // 2. Try JioSaavn for 320kbps
        if let Ok(js_tracks) = self.jiosaavn.search(&query, 1).await {
            if let Some(first_js) = js_tracks.into_iter().next() {
                if let Ok(url) = self
                    .jiosaavn
                    .resolve_stream_url(&first_js.info.identifier)
                    .await
                {
                    info!("⚡ Spotify->JioSaavn 320kbps for '{}'", track.info.title);
                    return Some(url);
                }
            }
        }

        // 3. Fallback to YouTube title/artist search
        if let Ok(yt_tracks) = self.youtube.search(&query, 1).await {
            if let Some(first_yt) = yt_tracks.into_iter().next() {
                if let Ok(Some(yt_res)) =
                    self.youtube.resolve_video(&first_yt.info.identifier).await
                {
                    if let Some(url) = yt_res.info.uri {
                        info!("⚡ Spotify->YouTube for '{}'", track.info.title);
                        return Some(url);
                    }
                }
            }
        }

        None
    }

    /// Mirror Deezer track to YouTube via ISRC/title matching (full track, not 30s preview).
    async fn resolve_deezer_mirror(
        &self,
        track: &crate::models::track::LavalinkTrack,
    ) -> Option<String> {
        let query = format!("{} {}", track.info.title, track.info.author);

        // 1. Try ISRC-based YouTube search for exact match
        if let Some(isrc) = &track.info.isrc {
            let isrc_query = format!("isrc:{}", isrc);
            if let Ok(yt_tracks) = self.youtube.search(&isrc_query, 1).await {
                if let Some(yt_track) = yt_tracks.into_iter().next() {
                    if let Ok(Some(yt_res)) =
                        self.youtube.resolve_video(&yt_track.info.identifier).await
                    {
                        if let Some(url) = &yt_res.info.uri {
                            info!("⚡ Deezer->YouTube via ISRC for '{}'", track.info.title);
                            return Some(url.clone());
                        }
                    }
                }
            }
        }

        // 2. Try JioSaavn
        if let Ok(js_tracks) = self.jiosaavn.search(&query, 1).await {
            if let Some(first_js) = js_tracks.into_iter().next() {
                if let Ok(url) = self
                    .jiosaavn
                    .resolve_stream_url(&first_js.info.identifier)
                    .await
                {
                    info!("⚡ Deezer->JioSaavn for '{}'", track.info.title);
                    return Some(url);
                }
            }
        }

        // 3. Fallback to YouTube title/artist search
        if let Ok(yt_tracks) = self.youtube.search(&query, 1).await {
            if let Some(first_yt) = yt_tracks.into_iter().next() {
                if let Ok(Some(yt_res)) =
                    self.youtube.resolve_video(&first_yt.info.identifier).await
                {
                    if let Some(url) = yt_res.info.uri {
                        info!("⚡ Deezer->YouTube for '{}'", track.info.title);
                        return Some(url);
                    }
                }
            }
        }

        // 4. Last resort: use Deezer preview URL (30 seconds)
        if let Some(url) = &track.info.uri {
            if url.contains("preview") {
                warn!(
                    "Using Deezer 30s preview for '{}' (full track unavailable)",
                    track.info.title
                );
                return Some(url.clone());
            }
        }

        None
    }

    /// Mirror Apple Music track to YouTube via ISRC/title matching.
    async fn resolve_apple_music_mirror(
        &self,
        track: &crate::models::track::LavalinkTrack,
    ) -> Option<String> {
        let query = format!("{} {}", track.info.title, track.info.author);

        // 1. Try ISRC-based YouTube search for exact match
        if let Some(isrc) = &track.info.isrc {
            let isrc_query = format!("isrc:{}", isrc);
            if let Ok(yt_tracks) = self.youtube.search(&isrc_query, 1).await {
                if let Some(yt_track) = yt_tracks.into_iter().next() {
                    if let Ok(Some(yt_res)) =
                        self.youtube.resolve_video(&yt_track.info.identifier).await
                    {
                        if let Some(url) = &yt_res.info.uri {
                            info!("⚡ AppleMusic->YouTube via ISRC for '{}'", track.info.title);
                            return Some(url.clone());
                        }
                    }
                }
            }
        }

        // 2. Try JioSaavn
        if let Ok(js_tracks) = self.jiosaavn.search(&query, 1).await {
            if let Some(first_js) = js_tracks.into_iter().next() {
                if let Ok(url) = self
                    .jiosaavn
                    .resolve_stream_url(&first_js.info.identifier)
                    .await
                {
                    info!("⚡ AppleMusic->JioSaavn for '{}'", track.info.title);
                    return Some(url);
                }
            }
        }

        // 3. Fallback to YouTube title/artist search
        if let Ok(yt_tracks) = self.youtube.search(&query, 1).await {
            if let Some(first_yt) = yt_tracks.into_iter().next() {
                if let Ok(Some(yt_res)) =
                    self.youtube.resolve_video(&first_yt.info.identifier).await
                {
                    if let Some(url) = yt_res.info.uri {
                        info!("⚡ AppleMusic->YouTube for '{}'", track.info.title);
                        return Some(url);
                    }
                }
            }
        }

        // 4. Last resort: use Apple Music preview URL
        if let Some(url) = &track.info.uri {
            if url.contains("preview") {
                warn!(
                    "Using Apple Music preview for '{}' (full track unavailable)",
                    track.info.title
                );
                return Some(url.clone());
            }
        }

        None
    }

    pub async fn queue_track(
        &self,
        guild_id: &str,
        encoded: &str,
    ) -> Result<PlayerResponse, String> {
        let player_arc = if let Some(p) = self.players.get(guild_id) {
            p.clone()
        } else {
            let p = self.create_guild_player(guild_id).await;
            self.players.insert(guild_id.to_string(), p.clone());
            p
        };

        let track = crate::track_encoding::decode_track(encoded)
            .map_err(|e| format!("Invalid encoded track: {}", e))?;

        let stream_url = self.resolve_stream_url(&track).await;

        let mut player = player_arc.write().await;

        if !player.is_playing() && player.queue.current.is_none() {
            if let Some(url) = stream_url {
                player.play_track(track, url).await;
            } else {
                player.add_to_queue(track);
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
                drop(player);
                if let Some(url) = self.resolve_stream_url(&track).await {
                    let mut player = player_arc.write().await;
                    player.play_track(track, url).await;
                    return Ok(player.to_response());
                }
            }

            let player = player_arc.read().await;
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
                drop(player);
                if let Some(url) = self.resolve_stream_url(&track).await {
                    let mut player = player_arc.write().await;
                    player.play_track(track, url).await;
                    return Ok(player.to_response());
                }
            }

            let player = player_arc.read().await;
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
        let player_arc = match self.players.get(guild_id) {
            Some(p) => p.clone(),
            None => return,
        };

        // Emit finished event
        let (next_track, last_track, is_autoplay) = {
            let mut player = player_arc.write().await;
            let finished_track = player.queue.current.clone();

            if let Some(ref track) = finished_track {
                player.emit_event(
                    "TrackEndEvent",
                    serde_json::json!({
                        "track": track,
                        "reason": "finished",
                    }),
                );
            }

            let next = player.get_next_track_for_autoplay();
            let last = player.queue.current.clone();
            let autoplay = player.autoplay.enabled;
            (next, last, autoplay)
        };

        if let Some(track) = next_track {
            if let Some(stream_url) = self.resolve_stream_url(&track).await {
                let mut player = player_arc.write().await;
                player.play_track(track, stream_url).await;
                return;
            }
        }

        if is_autoplay {
            if let Some(ref track) = last_track {
                let recommendation = {
                    let player = player_arc.read().await;
                    player
                        .autoplay
                        .get_recommendation(track, &self.jiosaavn, &self.youtube, &self.spotify)
                        .await
                };

                if let Some(rec) = recommendation {
                    if let Some(stream_url) = self.resolve_stream_url(&rec).await {
                        let mut player = player_arc.write().await;
                        player.play_track(rec, stream_url).await;
                        return;
                    }
                }
            }
        }

        let mut player = player_arc.write().await;
        player.stop();
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
