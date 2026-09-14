// Copyright (c) 2026 nikcodex (KizunaLink)
// Licensed under the MIT License

use std::sync::Arc;

use crate::{
    common::HttpClientPool,
    media::sources::plugin::{BoxedSource, BoxedTrack},
};

mod best_match;
mod registration;
mod resolver;

/// Source Manager handles the lifecycle and coordination of all audio sources.
pub struct SourceManager {
    pub sources: Vec<BoxedSource>,
    pub mirrors: Option<crate::config::server::MirrorsConfig>,
    pub youtube_cipher_manager:
        Option<Arc<crate::media::sources::youtube::cipher::YouTubeCipherManager>>,
    pub youtube_stream_ctx: Option<Arc<crate::media::sources::youtube::YoutubeStreamContext>>,
    pub http_pool: Arc<HttpClientPool>,
}

impl SourceManager {
    /// Create a new SourceManager with all available sources configured via AppConfig.
    pub fn new(config: &crate::config::AppConfig) -> Self {
        let http_pool = Arc::new(HttpClientPool::new());
        let mut sources = Vec::new();

        let (youtube_cipher_manager, youtube_stream_ctx) =
            registration::register_all(&mut sources, config, &http_pool);

        Self {
            sources,
            mirrors: config.player.mirrors.clone(),
            youtube_cipher_manager,
            youtube_stream_ctx,
            http_pool,
        }
    }
}

#[cfg(test)]
impl SourceManager {
    /// The name of the first source that claims `identifier`, or `None`.
    fn dispatches_to(&self, identifier: &str) -> Option<String> {
        self.sources
            .iter()
            .find(|s| s.can_handle(identifier))
            .map(|s| s.name().to_owned())
    }
}

impl SourceManager {
    /// Load tracks using the first matching source that can handle the identifier.
    pub async fn load(
        &self,
        identifier: &str,
        routeplanner: Option<Arc<dyn crate::lavalink::routeplanner::RoutePlanner>>,
    ) -> crate::lavalink::protocol::tracks::LoadResult {
        for source in &self.sources {
            if source.can_handle(identifier) {
                tracing::debug!(
                    "SourceManager: Loading '{}' with source: {}",
                    identifier,
                    source.name()
                );
                return source.load(identifier, routeplanner.clone()).await;
            }
        }

        tracing::debug!(
            "SourceManager: No source matched identifier: '{}'",
            identifier
        );
        crate::lavalink::protocol::tracks::LoadResult::Empty {}
    }

    /// Perform a search across available sources.
    pub async fn load_search(
        &self,
        query: &str,
        types: &[String],
        routeplanner: Option<Arc<dyn crate::lavalink::routeplanner::RoutePlanner>>,
    ) -> Option<crate::lavalink::protocol::tracks::SearchResult> {
        for source in &self.sources {
            if source.can_handle(query) {
                tracing::trace!("Loading search '{}' with source: {}", query, source.name());
                return source.load_search(query, types, routeplanner.clone()).await;
            }
        }

        tracing::debug!("No source could handle search query: {}", query);
        None
    }

    pub async fn resolve_track(
        &self,
        track_info: &crate::lavalink::protocol::tracks::TrackInfo,
        routeplanner: Option<Arc<dyn crate::lavalink::routeplanner::RoutePlanner>>,
    ) -> Result<BoxedTrack, String> {
        let identifier = track_info.uri.as_deref().unwrap_or(&track_info.identifier);

        for source in &self.sources {
            if source.can_handle(identifier) {
                tracing::trace!(
                    "Resolving playable track for '{}' with source: {}",
                    identifier,
                    source.name()
                );

                if let Some(track) = source.get_track(identifier, routeplanner.clone()).await {
                    return Ok(track);
                }
                break;
            }
        }

        if let Some(mirrors) = &self.mirrors {
            return resolver::resolve_with_mirrors(
                self,
                track_info,
                identifier,
                mirrors,
                routeplanner,
            )
            .await;
        }

        Err(format!(
            "Failed to resolve playable track for: {}",
            identifier
        ))
    }

    /// Get names of all registered sources.
    pub fn source_names(&self) -> Vec<String> {
        self.sources.iter().map(|s| s.name().to_string()).collect()
    }

    /// Retrieves proxy configuration for a specific source by name.
    pub fn get_proxy_config(
        &self,
        source_name: &str,
    ) -> Option<crate::config::sources::HttpProxyConfig> {
        self.sources
            .iter()
            .find(|s| s.name() == source_name)
            .and_then(|s| s.get_proxy_config())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a SourceManager from a TOML snippet that only enables the sources
    /// under test. Registration must not touch the network.
    fn manager_with(sources_toml: &str) -> SourceManager {
        let toml = format!(
            "\
[server]
address = '127.0.0.1'
port = 2333
authorization = 'test'
{sources_toml}"
        );
        let config: crate::config::AppConfig = toml::from_str(&toml).expect("test TOML parses");
        SourceManager::new(&config)
    }

    #[tokio::test]
    async fn youtube_search_prefix_and_urls_dispatch_to_youtube() {
        let m = manager_with("[sources.youtube]\nenabled = true\n");
        assert_eq!(
            m.dispatches_to("ytsearch:hello world").as_deref(),
            Some("youtube")
        );
        assert_eq!(
            m.dispatches_to("ytmsearch:hello world").as_deref(),
            Some("youtube")
        );
        assert_eq!(
            m.dispatches_to("https://www.youtube.com/watch?v=dQw4w9WgXcQ")
                .as_deref(),
            Some("youtube")
        );
    }

    #[tokio::test]
    async fn spotify_search_prefix_and_urls_dispatch_to_spotify() {
        let m = manager_with("[sources.spotify]\nenabled = true\n");
        assert_eq!(
            m.dispatches_to("spsearch:hello").as_deref(),
            Some("spotify")
        );
        assert_eq!(m.dispatches_to("sprec:123").as_deref(), Some("spotify"));
        assert_eq!(
            m.dispatches_to("https://open.spotify.com/track/abc123")
                .as_deref(),
            Some("spotify")
        );
    }

    #[tokio::test]
    async fn soundcloud_search_prefix_and_urls_dispatch_to_soundcloud() {
        let m = manager_with("[sources.soundcloud]\nenabled = true\n");
        assert_eq!(
            m.dispatches_to("scsearch:hello").as_deref(),
            Some("soundcloud")
        );
        assert_eq!(
            m.dispatches_to("https://soundcloud.com/artist/track")
                .as_deref(),
            Some("soundcloud")
        );
    }

    #[tokio::test]
    async fn jiosaavn_and_gaana_prefixes_dispatch_correctly() {
        let m =
            manager_with("[sources.jiosaavn]\nenabled = true\n[sources.gaana]\nenabled = true\n");
        assert_eq!(
            m.dispatches_to("jssearch:hello").as_deref(),
            Some("jiosaavn")
        );
        assert_eq!(m.dispatches_to("gnsearch:hello").as_deref(), Some("gaana"));
    }

    #[tokio::test]
    async fn local_file_identifier_dispatches_to_local_source() {
        let m = manager_with("[sources.local]\nenabled = true\n");
        let file = std::env::temp_dir().join("kizuna-can-handle-test.wav");
        std::fs::write(&file, []).unwrap();
        let uri = format!("file://{}", file.display());
        assert_eq!(m.dispatches_to(&uri).as_deref(), Some("local"));
        std::fs::remove_file(&file).ok();
    }

    #[tokio::test]
    async fn unknown_or_garbage_identifier_matches_nothing() {
        let m =
            manager_with("[sources.youtube]\nenabled = true\n[sources.spotify]\nenabled = true\n");
        assert_eq!(m.dispatches_to("totally-unknown-prefix:x"), None);
        assert_eq!(m.dispatches_to("not a url or prefix"), None);
    }

    #[tokio::test]
    async fn disabled_sources_are_not_registered() {
        // Bandcamp/Shazam are disabled by default in config.example.toml because
        // they are known-broken; ensure a caller turning them off gets nothing.
        let m = manager_with(
            "[sources.bandcamp]\nenabled = false\n[sources.shazam]\nenabled = false\n",
        );
        assert_eq!(m.dispatches_to("https://x.bandcamp.com/track/slug"), None);
        assert_eq!(m.dispatches_to("shsearch:hello"), None);
    }
}
