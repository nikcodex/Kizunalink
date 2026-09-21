// Copyright (c) 2026 nikcodex (KizunaLink)
// Licensed under the MIT License

use std::{collections::HashSet, sync::Arc};

use super::{
    parser::parse_m3u8,
    types::{M3u8Playlist, Resource},
};
use crate::{common::types::AnyResult, media::sources::youtube::cipher::YouTubeCipherManager};

/// Maximum master-playlist nesting we will follow (variant → audio rendition → …).
/// A well-formed provider uses one or two levels; anything deeper is treated as
/// malformed rather than recursed into without bound.
const MAX_PLAYLIST_DEPTH: usize = 8;

/// Boxed future returned by [`resolve_playlist_inner`]; the recursion has to be
/// boxed so the compiler can name its type.
type PlaylistFuture<'a> = std::pin::Pin<
    Box<dyn std::future::Future<Output = AnyResult<(Vec<Resource>, Option<Resource>)>> + 'a>,
>;

pub async fn resolve_playlist(
    client: &reqwest::Client,
    url: &str,
) -> AnyResult<(Vec<Resource>, Option<Resource>)> {
    let mut visited = HashSet::new();
    resolve_playlist_inner(client, url, 0, &mut visited).await
}

/// Recursive implementation of [`resolve_playlist`]. `depth` bounds nesting and
/// `visited` rejects cycles (a playlist that references itself, directly or
/// through a variant/rendition chain).
fn resolve_playlist_inner<'a>(
    client: &'a reqwest::Client,
    url: &'a str,
    depth: usize,
    visited: &'a mut HashSet<String>,
) -> PlaylistFuture<'a> {
    Box::pin(async move {
        if depth > MAX_PLAYLIST_DEPTH {
            return Err(format!(
                "HLS playlist nesting exceeded {MAX_PLAYLIST_DEPTH} levels at {url}"
            )
            .into());
        }
        if !visited.insert(url.to_string()) {
            return Err(format!("HLS playlist cycle detected at {url}").into());
        }

        let text = fetch_text(client, url).await?;
        let playlist = parse_m3u8(&text, url);

        match playlist {
            M3u8Playlist::Master {
                variants,
                audio_groups,
            } => {
                let best = variants
                    .iter()
                    .filter(|v| v.is_audio_only)
                    .max_by_key(|v| v.bandwidth)
                    .or_else(|| {
                        variants
                            .iter()
                            .filter(|v| v.audio_group.is_some())
                            .max_by_key(|v| v.bandwidth)
                    })
                    .or_else(|| variants.iter().max_by_key(|v| v.bandwidth));

                match best {
                    Some(v) => {
                        // If the variant has an audio group, try to find a rendition URI.
                        if let Some(group_id) = &v.audio_group
                            && let Some(group) = audio_groups.get(group_id)
                        {
                            let rendition = group
                                .iter()
                                .find(|m| m.is_default)
                                .or_else(|| group.iter().find(|m| m.uri.is_some()))
                                .and_then(|m| m.uri.as_ref());

                            if let Some(uri) = rendition {
                                tracing::debug!(
                                    "HLS: selected audio group {} -> {}",
                                    group_id,
                                    uri
                                );
                                return resolve_playlist_inner(client, uri, depth + 1, visited)
                                    .await;
                            }
                        }

                        tracing::debug!(
                            "HLS: selected variant bw={} codecs={:?} audio_only={} audio_group={:?} url={}",
                            v.bandwidth,
                            v.codecs,
                            v.is_audio_only,
                            v.audio_group,
                            v.url
                        );
                        resolve_playlist_inner(client, &v.url, depth + 1, visited).await
                    }
                    None => Err("HLS master playlist has no variants".into()),
                }
            }
            M3u8Playlist::Media { segments, map } => Ok((segments, map)),
        }
    })
}

pub async fn fetch_text(client: &reqwest::Client, url: &str) -> AnyResult<String> {
    let res = client
        .get(url)
        .header("Accept", "application/x-mpegURL, */*")
        .send()
        .await?;

    if !res.status().is_success() {
        return Err(format!("HLS playlist fetch failed {}: {}", res.status(), url).into());
    }

    let text = res.text().await?;
    Ok(text)
}

pub fn resolve_url_string(
    url: &str,
    cipher_manager: &Option<Arc<YouTubeCipherManager>>,
    player_url: &Option<String>,
) -> AnyResult<String> {
    let (cipher, p_url) = match (cipher_manager, player_url) {
        (Some(c), Some(p)) => (c, p),
        _ => return Ok(url.to_string()),
    };

    let n_token = if let Some(pos) = url.find("/n/") {
        let rest = &url[pos + 3..];
        rest.split('/').next()
    } else {
        url.split("&n=")
            .nth(1)
            .or_else(|| url.split("?n=").nth(1))
            .and_then(|s| s.split('&').next())
    };

    if let Some(n) = n_token {
        let handle = tokio::runtime::Handle::current();
        let cipher = cipher.clone();
        let url_str = url.to_string();
        let p_url_str = p_url.clone();
        let n_str = n.to_string();

        Ok(handle.block_on(async move {
            cipher
                .resolve_url(&url_str, &p_url_str, Some(&n_str), None)
                .await
        })?)
    } else {
        Ok(url.to_string())
    }
}
