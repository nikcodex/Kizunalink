// Copyright (c) 2026 nikcodex (KizunaLink)
// Licensed under the MIT License

use std::{
    collections::HashSet,
    sync::{Arc, OnceLock},
    time::{Duration, Instant},
};

use regex::Regex;
use tokio::sync::RwLock;
use tracing::{debug, error, info, trace, warn};

use crate::common::types::SharedRw;

const SOUNDCLOUD_URL: &str = "https://soundcloud.com";
const CLIENT_ID_REFRESH_INTERVAL: Duration = Duration::from_secs(3600); // 1 hour
const CLIENT_ID_REQUEST_TIMEOUT: Duration = Duration::from_secs(15);

fn asset_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r#"(?:https?:)?//[^"'\s<>]+(?:sndcdn\.com|soundcloud\.com)[^"'\s<>]*\.js(?:\?[^"'\s<>]*)?"#)
            .expect("valid regex")
    })
}

fn script_src_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r#"<script[^>]+src\s*=\s*["']([^"']+)["']"#).expect("valid regex"))
}

fn client_id_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r#"(?i)(?:client[_-]?id|clientId)\s*[:=]\s*["']?([a-zA-Z0-9_-]{16,})"#)
            .expect("valid regex")
    })
}

pub struct SoundCloudTokenTracker {
    client: Arc<reqwest::Client>,
    client_id: SharedRw<CachedClientId>,
}

struct CachedClientId {
    value: Option<String>,
    last_updated: Option<Instant>,
}

impl CachedClientId {
    fn is_stale(&self) -> bool {
        match self.last_updated {
            None => true,
            Some(t) => t.elapsed() > CLIENT_ID_REFRESH_INTERVAL,
        }
    }
}

impl SoundCloudTokenTracker {
    pub fn new(client: Arc<reqwest::Client>, config: &crate::config::SoundCloudConfig) -> Self {
        // Pre-seed client_id from config if provided
        let cached = CachedClientId {
            value: config.client_id.clone(),
            last_updated: if config.client_id.is_some() {
                Some(Instant::now())
            } else {
                None
            },
        };

        Self {
            client,
            client_id: Arc::new(RwLock::new(cached)),
        }
    }

    pub async fn get_client_id(&self) -> Option<String> {
        {
            let guard = self.client_id.read().await;
            if !guard.is_stale()
                && let Some(id) = &guard.value
            {
                return Some(id.clone());
            }
        }

        // Need to refresh
        self.refresh_client_id().await
    }

    pub async fn refresh_client_id(&self) -> Option<String> {
        debug!("Refreshing SoundCloud client_id...");
        trace!("SoundCloud: Fetching client_id from soundcloud.com...");

        let response = match self
            .client
            .get(SOUNDCLOUD_URL)
            .timeout(CLIENT_ID_REQUEST_TIMEOUT)
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                error!("SoundCloud: Failed to fetch main page: {}", e);
                return None;
            }
        };
        let status = response.status();
        let html = match response.text().await {
            Ok(t) => t,
            Err(e) => {
                error!("SoundCloud: Failed to read main page: {}", e);
                return None;
            }
        };
        if !status.is_success() || html.is_empty() {
            warn!(
                "SoundCloud: homepage returned HTTP {} with {} bytes; client_id discovery may be blocked",
                status,
                html.len()
            );
            return None;
        }
        let html = html.replace("&amp;", "&");

        // Try to find client_id directly in page HTML first
        if let Some(caps) = client_id_re().captures(&html)
            && let Some(m) = caps.get(1)
        {
            let id = m.as_str().to_owned();
            trace!("SoundCloud: Found client_id in main page: {id}");
            self.store_client_id(id.clone()).await;
            info!("Successfully refreshed SoundCloud client_id");
            return Some(id);
        }

        // Otherwise, find all asset JS URLs and probe them
        let mut asset_urls = HashSet::new();
        for caps in script_src_re().captures_iter(&html) {
            if let Some(src) = caps.get(1).map(|m| m.as_str()) {
                let url = if src.starts_with("//") {
                    format!("https:{src}")
                } else if src.starts_with('/') {
                    format!("https://soundcloud.com{src}")
                } else if src.starts_with("http") {
                    src.to_owned()
                } else {
                    continue;
                };
                asset_urls.insert(url);
            }
        }
        for m in asset_re().find_iter(&html) {
            let mut url = m.as_str().to_owned();
            if url.starts_with("//") {
                url = format!("https:{url}");
            }
            asset_urls.insert(url);
        }
        let asset_urls: Vec<String> = asset_urls.into_iter().collect();

        if asset_urls.is_empty() {
            warn!("SoundCloud: No asset JS URLs found in main page");
            return None;
        }

        trace!(
            "SoundCloud: Found {} asset URLs, probing for client_id",
            asset_urls.len()
        );

        // Try the last few asset scripts (the relevant one is usually one of the last)
        for url in asset_urls.iter().rev().take(9) {
            let js = match self
                .client
                .get(url)
                .timeout(CLIENT_ID_REQUEST_TIMEOUT)
                .send()
                .await
            {
                Ok(r) => match r.text().await {
                    Ok(t) => t,
                    Err(_) => continue,
                },
                Err(_) => continue,
            };

            if let Some(caps) = client_id_re().captures(&js)
                && let Some(m) = caps.get(1)
            {
                let id = m.as_str().to_owned();
                trace!("SoundCloud: Found client_id in asset {url}: {id}");
                self.store_client_id(id.clone()).await;
                info!("Successfully refreshed SoundCloud client_id");
                return Some(id);
            }
        }

        warn!("SoundCloud: client_id not found in any asset scripts");
        None
    }

    pub async fn invalidate(&self) {
        let mut guard = self.client_id.write().await;
        guard.last_updated = None;
    }

    async fn store_client_id(&self, id: String) {
        let mut guard = self.client_id.write().await;
        guard.value = Some(id);
        guard.last_updated = Some(Instant::now());
    }

    pub fn init(self: Arc<Self>) {
        let this = self.clone();
        tokio::spawn(async move {
            this.get_client_id().await;
        });
    }
}
