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
use tokio::sync::{broadcast, mpsc, Mutex, RwLock};
use tracing::{info, warn};

/// Maximum number of concurrent players allowed.
pub const MAX_PLAYERS: usize = 10000;

/// A player together with the session that owns it.
///
/// Ownership is fixed at creation: a guild/player can only ever be controlled
/// by the session that created it. Ownership never transfers via PATCH.
#[derive(Clone)]
pub struct PlayerEntry {
    pub player: Arc<RwLock<GuildPlayer>>,
    pub session_id: String,
}

/// Errors from player operations that map to distinct HTTP semantics.
#[derive(Debug)]
pub enum PlayerManagerError {
    /// The guild has no player owned by the requesting session (either it does
    /// not exist, or it is owned by a different session).
    NotFound(String),
    /// The global player limit was reached.
    LimitReached(usize),
    /// Any other internal failure.
    Internal(String),
}

impl std::fmt::Display for PlayerManagerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PlayerManagerError::NotFound(g) => write!(f, "Player not found for guild: {}", g),
            PlayerManagerError::LimitReached(n) => {
                write!(f, "Player limit reached: maximum {} players allowed", n)
            }
            PlayerManagerError::Internal(e) => write!(f, "Internal player error: {}", e),
        }
    }
}

impl std::error::Error for PlayerManagerError {}

pub struct PlayerManager {
    /// Shared so that the shallow clone handed back by [`PlayerManager::new`]
    /// and the clone owned by the background track-end task observe exactly the
    /// same map. A plain `DashMap` here would be *copied* by `clone_shallow`,
    /// leaving the track-end task looking at a permanently empty map.
    players: Arc<DashMap<String, PlayerEntry>>,
    /// Serializes player creation so that the player-limit check and the
    /// ownership claim are atomic (no check-then-act races).
    creation_lock: Arc<Mutex<()>>,
    max_players: usize,
    pub bot_user_id: Arc<RwLock<String>>,
    event_tx: broadcast::Sender<String>,
    track_end_tx: mpsc::UnboundedSender<String>,
    jiosaavn: Arc<JioSaavnSource>,
    youtube: Arc<YouTubeSource>,
    spotify: Arc<SpotifySource>,
    soundcloud: Arc<SoundCloudSource>,
    deezer: Arc<DeezerSource>,
    apple_music: Arc<AppleMusicSource>,
    pub queue_max_history: usize,
    sources: crate::config::SourcesConfig,
}

pub struct SourceBundle {
    pub jiosaavn: Arc<JioSaavnSource>,
    pub youtube: Arc<YouTubeSource>,
    pub spotify: Arc<SpotifySource>,
    pub soundcloud: Arc<SoundCloudSource>,
    pub deezer: Arc<DeezerSource>,
    pub apple_music: Arc<AppleMusicSource>,
}

impl PlayerManager {
    pub fn new(
        event_tx: broadcast::Sender<String>,
        sources: SourceBundle,
        queue_max_history: usize,
        max_players: usize,
        sources_config: crate::config::SourcesConfig,
    ) -> Self {
        let (track_end_tx, mut track_end_rx) = mpsc::unbounded_channel::<String>();

        let manager = Self {
            players: Arc::new(DashMap::new()),
            creation_lock: Arc::new(Mutex::new(())),
            max_players,
            bot_user_id: Arc::new(RwLock::new("0".to_string())),
            event_tx,
            track_end_tx,
            jiosaavn: sources.jiosaavn,
            youtube: sources.youtube,
            spotify: sources.spotify,
            soundcloud: sources.soundcloud,
            deezer: sources.deezer,
            apple_music: sources.apple_music,
            queue_max_history,
            sources: sources_config,
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
            creation_lock: self.creation_lock.clone(),
            max_players: self.max_players,
            bot_user_id: self.bot_user_id.clone(),
            event_tx: self.event_tx.clone(),
            track_end_tx: self.track_end_tx.clone(),
            jiosaavn: self.jiosaavn.clone(),
            youtube: self.youtube.clone(),
            spotify: self.spotify.clone(),
            soundcloud: self.soundcloud.clone(),
            deezer: self.deezer.clone(),
            apple_music: self.apple_music.clone(),
            queue_max_history: self.queue_max_history,
            sources: self.sources.clone(),
        }
    }

    pub async fn get_player(&self, guild_id: &str) -> Option<PlayerResponse> {
        let player_arc = self
            .players
            .get(guild_id)
            .map(|r| r.value().player.clone())?;
        let player = player_arc.read().await;
        Some(player.to_response())
    }

    /// Return the player for `guild_id` if and only if it is owned by `session_id`.
    fn get_owned_player(
        &self,
        guild_id: &str,
        session_id: &str,
    ) -> Result<Arc<RwLock<GuildPlayer>>, PlayerManagerError> {
        match self.players.get(guild_id) {
            Some(entry) if entry.value().session_id == session_id => {
                Ok(entry.value().player.clone())
            }
            Some(_) | None => Err(PlayerManagerError::NotFound(guild_id.to_string())),
        }
    }

    /// Atomically claim or reuse the player for `guild_id` on behalf of `session_id`.
    ///
    /// - If a player exists and is owned by `session_id`, it is returned.
    /// - If a player exists but is owned by a different session, `NotFound` is
    ///   returned — ownership NEVER transfers through a PATCH.
    /// - If no player exists, one is created and owned by `session_id`.
    ///
    /// Creation, the ownership claim, and the player-limit check are serialized
    /// by `creation_lock`, so concurrent requests can never exceed `max_players`
    /// and two sessions racing to claim the same guild resolve to a single owner.
    pub async fn get_or_create_player_for_session(
        &self,
        guild_id: &str,
        session_id: &str,
    ) -> Result<Arc<RwLock<GuildPlayer>>, PlayerManagerError> {
        let user_id = self.bot_user_id.read().await.clone();

        let _guard = self.creation_lock.lock().await;

        if let Some(entry) = self.players.get(guild_id) {
            if entry.value().session_id == session_id {
                return Ok(entry.value().player.clone());
            }
            return Err(PlayerManagerError::NotFound(guild_id.to_string()));
        }

        if self.players.len() >= self.max_players {
            return Err(PlayerManagerError::LimitReached(self.max_players));
        }

        let player = GuildPlayer::new(
            guild_id.to_string(),
            user_id,
            self.event_tx.clone(),
            self.track_end_tx.clone(),
            self.queue_max_history,
        );
        let p = Arc::new(RwLock::new(player));
        self.players.insert(
            guild_id.to_string(),
            PlayerEntry {
                player: p.clone(),
                session_id: session_id.to_string(),
            },
        );
        Ok(p)
    }

    /// Legacy creation helper without a session context (internal use only).
    pub async fn get_or_create_player(
        &self,
        guild_id: &str,
    ) -> Result<Arc<RwLock<GuildPlayer>>, String> {
        self.get_or_create_player_for_session(guild_id, "")
            .await
            .map_err(|e| e.to_string())
    }

    /// GET a player only if it belongs to `session_id`.
    pub async fn get_player_for_session(
        &self,
        guild_id: &str,
        session_id: &str,
    ) -> Option<PlayerResponse> {
        let entry = self.players.get(guild_id)?;
        if entry.value().session_id != session_id {
            return None;
        }
        // Clone out of the map guard before awaiting the player lock so we never
        // hold a DashMap shard lock across an await.
        let player_arc = entry.value().player.clone();
        drop(entry);
        let response = player_arc.read().await.to_response();
        Some(response)
    }

    /// List only the players owned by `session_id`.
    pub async fn get_players_for_session(&self, session_id: &str) -> Vec<PlayerResponse> {
        let entries: Vec<PlayerEntry> = self
            .players
            .iter()
            .filter(|e| e.value().session_id == session_id)
            .map(|e| e.value().clone())
            .collect();
        let mut responses = Vec::with_capacity(entries.len());
        for entry in entries {
            responses.push(entry.player.read().await.to_response());
        }
        responses
    }

    pub async fn update_player(
        &self,
        guild_id: &str,
        payload: PlayerUpdatePayload,
        no_replace: bool,
        session_id: &str,
    ) -> Result<PlayerResponse, PlayerManagerError> {
        let player_arc = self
            .get_or_create_player_for_session(guild_id, session_id)
            .await?;

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
        if let Some(ref tp) = payload.track {
            if tp.encoded.as_deref().unwrap_or("").trim().is_empty()
                && tp.identifier.as_deref().unwrap_or("").trim().is_empty()
            {
                should_stop_track = true;
            }
        }
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
            if let Err(e) = player.set_voice(voice).await {
                warn!("Voice update failed for guild {}: {}", guild_id, e);
            }
        }

        if should_stop_track {
            info!("Stopping player for guild: {}", guild_id);
            player.stop();
        } else if let Some(track) = resolved_track {
            let has_track = player.queue.current.is_some() || player.is_playing();
            if no_replace && has_track {
                info!(
                    "Skipping track replacement for guild {} because noReplace is true and track is already loaded",
                    guild_id
                );
            } else if let Some(stream_url) = resolved_stream_url {
                info!(
                    "Playing track '{}' for guild {}",
                    track.info.title, guild_id
                );
                player.play_track(track, stream_url).await;
            } else {
                warn!("Failed to resolve stream URL for guild {}", guild_id);
                player.emit_track_load_failed(&track, "Failed to resolve playable audio stream");
            }
        }

        if let Some(position) = payload.position {
            player.seek(position).await;
        }

        if let Some(volume) = payload.volume {
            player.set_volume(volume);
        }

        if let Some(paused) = payload.paused {
            player.set_paused(paused).await;
        }

        // `endTime` is only touched when the client actually sent the field.
        // Assigning `payload.end_time` unconditionally silently dropped a
        // previously configured end time on every unrelated PATCH (volume,
        // pause, filters, ...), because an omitted field and an explicit `null`
        // both deserialize to `None`.
        if let Some(end_time) = payload.end_time {
            player.end_time = Some(end_time);
            player.end_time_generation += 1;
            let generation = player.end_time_generation;

            // Enforce it: Lavalink ends the track once playback reaches the
            // requested position. The value used to be stored but never acted
            // on. The watchdog is armed here (rather than on every PATCH) so
            // there is exactly one per requested end time, and the captured
            // generation invalidates the previous one when the end time is
            // re-scheduled or the track changes.
            if player.queue.current.is_some() {
                let position = player.get_position();
                if end_time > position {
                    let delay = std::time::Duration::from_millis(end_time - position);
                    let tx = self.track_end_tx.clone();
                    let gid = guild_id.to_string();
                    let handle = player_arc.clone();
                    tokio::spawn(async move {
                        tokio::time::sleep(delay).await;
                        let due = {
                            let p = handle.read().await;
                            p.end_time == Some(end_time)
                                && p.end_time_generation == generation
                                && p.get_position() >= end_time
                        };
                        if due {
                            let _ = tx.send(gid);
                        }
                    });
                } else {
                    // Already at or past the requested end position.
                    let _ = self.track_end_tx.send(guild_id.to_string());
                }
            }
        }

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

        Ok(player.to_response())
    }

    async fn resolve_identifier(&self, id: &str) -> Option<crate::models::track::LavalinkTrack> {
        let clean = id.trim();
        if clean.starts_with("http://") || clean.starts_with("https://") {
            if clean.contains("spotify.com") && self.sources.spotify {
                if let Some(track_id) = clean
                    .split("/track/")
                    .nth(1)
                    .and_then(|s| s.split('?').next())
                {
                    return self.spotify.resolve_track(track_id).await.ok().flatten();
                }
            } else if (clean.contains("youtube.com") || clean.contains("youtu.be"))
                && self.sources.youtube
            {
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
            } else if clean.contains("deezer.com/") && self.sources.deezer {
                if let Some(track_id) = clean
                    .split("/track/")
                    .nth(1)
                    .and_then(|s| s.split('?').next())
                {
                    return self.deezer.resolve_track(track_id).await.ok().flatten();
                }
            } else if clean.contains("music.apple.com/") && self.sources.applemusic {
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
                let safe = crate::security::sanitize_for_log(clean);
                warn!("SSRF blocked in resolve_identifier for '{}': {}", safe, e);
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
                if let Ok(stream_url) = self.youtube.resolve_stream_url(identifier).await {
                    return Some(stream_url);
                }

                // Fallback: Mirror to JioSaavn for 320kbps audio if YouTube stream is ciphered/blocked
                let query = format!("{} {}", track.info.title, track.info.author);
                if let Ok(js_tracks) = self.jiosaavn.search(&query, 1).await {
                    if let Some(first) = js_tracks.into_iter().next() {
                        if let Ok(js_url) = self
                            .jiosaavn
                            .resolve_stream_url(&first.info.identifier)
                            .await
                        {
                            info!(
                                "⚡ YouTube fallback -> JioSaavn 320kbps for '{}'",
                                track.info.title
                            );
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
            _ => {
                if let Some(stream_url) =
                    track.plugin_info.get("streamUrl").and_then(|u| u.as_str())
                {
                    Some(stream_url.to_string())
                } else {
                    track.info.uri.clone()
                }
            }
        }
    }

    /// Mirror Spotify track to JioSaavn (320kbps) or YouTube via ISRC/title matching.
    async fn resolve_spotify_mirror(
        &self,
        track: &crate::models::track::LavalinkTrack,
    ) -> Option<String> {
        let query = format!("{} {}", track.info.title, track.info.author);

        // 1. Try JioSaavn for 320kbps high quality audio
        if let Ok(js_tracks) = self.jiosaavn.search(&query, 1).await {
            if let Some(first_js) = js_tracks.into_iter().next() {
                if let Ok(url) = self
                    .jiosaavn
                    .resolve_stream_url(&first_js.info.identifier)
                    .await
                {
                    let safe_title = crate::security::sanitize_for_log(&track.info.title);
                    info!("⚡ Spotify->JioSaavn 320kbps for '{}'", safe_title);
                    return Some(url);
                }
            }
        }

        // 2. Try ISRC-based YouTube search for exact match
        if let Some(isrc) = &track.info.isrc {
            let isrc_query = format!("isrc:{}", isrc);
            if let Ok(yt_tracks) = self.youtube.search(&isrc_query, 1).await {
                if let Some(yt_track) = yt_tracks.into_iter().next() {
                    if let Ok(url) = self
                        .youtube
                        .resolve_stream_url(&yt_track.info.identifier)
                        .await
                    {
                        info!(
                            "⚡ Spotify->YouTube via ISRC: '{}' for '{}'",
                            isrc, track.info.title
                        );
                        return Some(url);
                    }
                }
            }
        }

        // 3. Fallback to YouTube title/artist search
        if let Ok(yt_tracks) = self.youtube.search(&query, 1).await {
            if let Some(first_yt) = yt_tracks.into_iter().next() {
                if let Ok(url) = self
                    .youtube
                    .resolve_stream_url(&first_yt.info.identifier)
                    .await
                {
                    let safe_title = crate::security::sanitize_for_log(&track.info.title);
                    info!("⚡ Spotify->YouTube for '{}'", safe_title);
                    return Some(url);
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

        // 1. Try JioSaavn for 320kbps
        if let Ok(js_tracks) = self.jiosaavn.search(&query, 1).await {
            if let Some(first_js) = js_tracks.into_iter().next() {
                if let Ok(url) = self
                    .jiosaavn
                    .resolve_stream_url(&first_js.info.identifier)
                    .await
                {
                    let safe_title = crate::security::sanitize_for_log(&track.info.title);
                    info!("⚡ Deezer->JioSaavn for '{}'", safe_title);
                    return Some(url);
                }
            }
        }

        // 2. Try ISRC-based YouTube search for exact match
        if let Some(isrc) = &track.info.isrc {
            let isrc_query = format!("isrc:{}", isrc);
            if let Ok(yt_tracks) = self.youtube.search(&isrc_query, 1).await {
                if let Some(yt_track) = yt_tracks.into_iter().next() {
                    if let Ok(url) = self
                        .youtube
                        .resolve_stream_url(&yt_track.info.identifier)
                        .await
                    {
                        let safe_title = crate::security::sanitize_for_log(&track.info.title);
                        info!("⚡ Deezer->YouTube via ISRC for '{}'", safe_title);
                        return Some(url);
                    }
                }
            }
        }

        // 3. Fallback to YouTube title/artist search
        if let Ok(yt_tracks) = self.youtube.search(&query, 1).await {
            if let Some(first_yt) = yt_tracks.into_iter().next() {
                if let Ok(url) = self
                    .youtube
                    .resolve_stream_url(&first_yt.info.identifier)
                    .await
                {
                    let safe_title = crate::security::sanitize_for_log(&track.info.title);
                    info!("⚡ Deezer->YouTube for '{}'", safe_title);
                    return Some(url);
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

        // 1. Try JioSaavn
        if let Ok(js_tracks) = self.jiosaavn.search(&query, 1).await {
            if let Some(first_js) = js_tracks.into_iter().next() {
                if let Ok(url) = self
                    .jiosaavn
                    .resolve_stream_url(&first_js.info.identifier)
                    .await
                {
                    let safe_title = crate::security::sanitize_for_log(&track.info.title);
                    info!("⚡ AppleMusic->JioSaavn for '{}'", safe_title);
                    return Some(url);
                }
            }
        }

        // 2. Try ISRC-based YouTube search for exact match
        if let Some(isrc) = &track.info.isrc {
            let isrc_query = format!("isrc:{}", isrc);
            if let Ok(yt_tracks) = self.youtube.search(&isrc_query, 1).await {
                if let Some(yt_track) = yt_tracks.into_iter().next() {
                    if let Ok(url) = self
                        .youtube
                        .resolve_stream_url(&yt_track.info.identifier)
                        .await
                    {
                        let safe_title = crate::security::sanitize_for_log(&track.info.title);
                        info!("⚡ AppleMusic->YouTube via ISRC for '{}'", safe_title);
                        return Some(url);
                    }
                }
            }
        }

        // 3. Fallback to YouTube title/artist search
        if let Ok(yt_tracks) = self.youtube.search(&query, 1).await {
            if let Some(first_yt) = yt_tracks.into_iter().next() {
                if let Ok(url) = self
                    .youtube
                    .resolve_stream_url(&first_yt.info.identifier)
                    .await
                {
                    let safe_title = crate::security::sanitize_for_log(&track.info.title);
                    info!("⚡ AppleMusic->YouTube for '{}'", safe_title);
                    return Some(url);
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
        session_id: &str,
    ) -> Result<PlayerResponse, PlayerManagerError> {
        let player_arc = self
            .get_or_create_player_for_session(guild_id, session_id)
            .await?;

        let track = crate::track_encoding::decode_track(encoded)
            .map_err(|e| PlayerManagerError::Internal(format!("Invalid encoded track: {}", e)))?;

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

    pub async fn skip_track(
        &self,
        guild_id: &str,
        session_id: &str,
    ) -> Result<PlayerResponse, PlayerManagerError> {
        let player_arc = self.get_owned_player(guild_id, session_id)?;

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
    }

    pub async fn previous_track(
        &self,
        guild_id: &str,
        session_id: &str,
    ) -> Result<PlayerResponse, PlayerManagerError> {
        let player_arc = self.get_owned_player(guild_id, session_id)?;

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
    }

    pub async fn toggle_autoplay(
        &self,
        guild_id: &str,
        session_id: &str,
    ) -> Result<bool, PlayerManagerError> {
        let player_arc = self.get_owned_player(guild_id, session_id)?;
        let mut player = player_arc.write().await;
        let enabled = player.autoplay.toggle();
        Ok(enabled)
    }

    pub async fn set_loop_mode(
        &self,
        guild_id: &str,
        mode: LoopMode,
        session_id: &str,
    ) -> Result<LoopMode, PlayerManagerError> {
        let player_arc = self.get_owned_player(guild_id, session_id)?;
        let mut player = player_arc.write().await;
        player.set_loop(mode);
        Ok(player.queue.loop_mode)
    }

    pub async fn shuffle_queue(
        &self,
        guild_id: &str,
        session_id: &str,
    ) -> Result<PlayerResponse, PlayerManagerError> {
        let player_arc = self.get_owned_player(guild_id, session_id)?;
        let mut player = player_arc.write().await;
        player.shuffle_queue();
        Ok(player.to_response())
    }

    pub async fn clear_queue(
        &self,
        guild_id: &str,
        session_id: &str,
    ) -> Result<PlayerResponse, PlayerManagerError> {
        let player_arc = self.get_owned_player(guild_id, session_id)?;
        let mut player = player_arc.write().await;
        player.clear_queue();
        Ok(player.to_response())
    }

    /// Destroy the player for `guild_id` only if it is owned by `session_id`.
    pub fn destroy_player_for_session(
        &self,
        guild_id: &str,
        session_id: &str,
    ) -> Result<(), PlayerManagerError> {
        // Resolve ownership into a plain `bool` so the `Ref` returned by `get` —
        // and with it the DashMap shard read lock — is released before we touch
        // the map again. `destroy_player` calls `players.remove`, which needs the
        // shard *write* lock; taking it while this thread still holds the shard
        // read guard deadlocks the caller, because parking_lot's `RwLock` is not
        // reentrant and the pending writer can never outlive its own reader.
        let owned_by_session = self
            .players
            .get(guild_id)
            .map(|entry| entry.value().session_id == session_id);

        match owned_by_session {
            Some(true) => {
                self.destroy_player(guild_id);
                Ok(())
            }
            Some(false) | None => Err(PlayerManagerError::NotFound(guild_id.to_string())),
        }
    }

    /// Force-destroy a player regardless of ownership (used by session expiry/cleanup).
    pub fn destroy_player(&self, guild_id: &str) -> bool {
        if let Some((_, entry)) = self.players.remove(guild_id) {
            tokio::spawn(async move {
                let mut player = entry.player.write().await;
                player.stop();
            });
            true
        } else {
            false
        }
    }

    pub async fn handle_track_end(&self, guild_id: &str) {
        let player_arc = match self.players.get(guild_id).map(|r| r.value().player.clone()) {
            Some(p) => p,
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

            // `get_next_track_for_autoplay` consumes `queue.current`, so re-reading
            // it afterwards always yields `None` and the autoplay branch below
            // could never run. Seed the recommendation from the track that just
            // finished instead.
            let next = player.get_next_track_for_autoplay();
            let last = finished_track;
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
        let players: Vec<Arc<RwLock<GuildPlayer>>> = self
            .players
            .iter()
            .map(|r| r.value().player.clone())
            .collect();
        let total = players.len();
        let mut playing = 0;
        for p in players {
            if p.read().await.is_actively_playing() {
                playing += 1;
            }
        }
        (total, playing)
    }

    pub async fn get_all_players(&self) -> Vec<PlayerResponse> {
        let players: Vec<Arc<RwLock<GuildPlayer>>> = self
            .players
            .iter()
            .map(|r| r.value().player.clone())
            .collect();
        let mut responses = Vec::with_capacity(players.len());
        for p in players {
            let player = p.read().await;
            responses.push(player.to_response());
        }
        responses
    }

    pub async fn is_actively_playing(&self, guild_id: &str) -> bool {
        let player_arc = self.players.get(guild_id).map(|r| r.value().player.clone());
        if let Some(player) = player_arc {
            player.read().await.is_actively_playing()
        } else {
            false
        }
    }
}
