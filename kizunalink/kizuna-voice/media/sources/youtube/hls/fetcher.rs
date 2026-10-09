// Copyright (c) 2026 nikcodex (KizunaLink)
// Licensed under the MIT License

use super::types::Resource;
use crate::{common::types::AnyResult, engine::source::range::validate_content_range};

/// Hard upper bound on a single HLS segment/init body, including full-body fallbacks.
pub const MAX_HLS_SEGMENT_BYTES: usize = 32 * 1024 * 1024;
const MAX_TRANSIENT_RETRIES: usize = 2;

#[derive(Debug, thiserror::Error)]
enum BodyReadError {
    #[error("HLS fetch: body exceeds {0} bytes")]
    TooLarge(usize),
    #[error("HLS fetch: body read failed: {0}")]
    Transport(#[from] reqwest::Error),
}

impl BodyReadError {
    fn is_transient(&self) -> bool {
        match self {
            // reqwest classifies a dropped connection during streaming as a
            // body error. Decode errors (e.g. invalid compression) are not
            // transport failures and must not be retried.
            Self::Transport(e) => {
                e.is_timeout() || e.is_connect() || (e.is_body() && !e.is_decode())
            }
            Self::TooLarge(_) => false,
        }
    }
}

async fn retry_pause(attempt: usize) {
    // Dropping the fetch future cancels this wait as well as any in-flight
    // reqwest request. The retry budget covers the whole request, not each phase.
    tokio::time::sleep(std::time::Duration::from_millis(100 << attempt)).await;
}

pub async fn fetch_segment_into(
    client: &reqwest::Client,
    resource: &Resource,
    out: &mut Vec<u8>,
) -> AnyResult<()> {
    let range = resource.range.as_ref();
    let end = if let Some(r) = range {
        Some(
            r.offset
                .checked_add(r.length.checked_sub(1).ok_or("empty HLS range")?)
                .ok_or("HLS range overflow")?,
        )
    } else {
        None
    };
    if range.is_some_and(|r| r.length > MAX_HLS_SEGMENT_BYTES as u64) {
        return Err("HLS fetch: requested range exceeds segment limit".into());
    }

    for attempt in 0..=MAX_TRANSIENT_RETRIES {
        // Each attempt creates a new request/response and a fresh temporary
        // body. Never resume from, or append, a partially read response.
        let mut req = client
            .get(&resource.url)
            .header("Accept", "*/*")
            .header("Accept-Encoding", "identity");
        if let (Some(r), Some(end)) = (range, end) {
            req = req.header("Range", format!("bytes={}-{end}", r.offset));
        }
        let res = match req.send().await {
            Ok(res) => res,
            Err(e) if attempt < MAX_TRANSIENT_RETRIES && (e.is_timeout() || e.is_connect()) => {
                retry_pause(attempt).await;
                continue;
            }
            Err(e) => return Err(e.into()),
        };
        let status = res.status();
        if matches!(status.as_u16(), 429 | 500 | 502 | 503 | 504) && attempt < MAX_TRANSIENT_RETRIES
        {
            retry_pause(attempt).await;
            continue;
        }

        // Validate status and byte interval BEFORE any body is read. expected
        // length is checked after streaming, including for chunked responses.
        let (cap, expected_len, slice) = if let Some(r) = range {
            if status == reqwest::StatusCode::PARTIAL_CONTENT {
                validate_content_range(&res, r.offset, Some(r.length))?;
                (r.length as usize, Some(r.length), None)
            } else if status == reqwest::StatusCode::OK {
                // An ignored Range is safe only when the complete resource is
                // declared, fits the cap, and contains the requested interval.
                let full_len = res
                    .content_length()
                    .ok_or("HLS fetch: ignored Range without Content-Length")?;
                let requested_end = r.offset + r.length - 1; // checked above
                if full_len > MAX_HLS_SEGMENT_BYTES as u64 || full_len <= requested_end {
                    return Err("HLS fetch: unsafe full-body Range fallback".into());
                }
                (
                    MAX_HLS_SEGMENT_BYTES,
                    Some(full_len),
                    Some((r.offset as usize, requested_end as usize)),
                )
            } else {
                return Err(format!("HLS fetch failed {status}").into());
            }
        } else {
            if status != reqwest::StatusCode::OK {
                return Err(format!("HLS fetch failed {status}").into());
            }
            (MAX_HLS_SEGMENT_BYTES, res.content_length(), None)
        };

        // All body bytes remain local to this attempt until fully verified.
        // Only a transport interruption or a short response gets a bounded
        // whole-request retry. Too-large bodies, malformed headers and status
        // errors are terminal, never accepted as a successful segment.
        let bytes = match read_body_capped(res, cap).await {
            Ok(bytes) => bytes,
            Err(e) if attempt < MAX_TRANSIENT_RETRIES && e.is_transient() => {
                retry_pause(attempt).await;
                continue;
            }
            Err(e) => return Err(e.into()),
        };
        if let Some(expected_len) = expected_len
            && bytes.len() as u64 != expected_len
        {
            if bytes.len() as u64 < expected_len && attempt < MAX_TRANSIENT_RETRIES {
                retry_pause(attempt).await;
                continue;
            }
            return Err(format!(
                "HLS fetch: incomplete or inconsistent body ({} of {expected_len} bytes)",
                bytes.len()
            )
            .into());
        }

        if let Some((start, end)) = slice {
            out.extend_from_slice(&bytes[start..=end]);
        } else {
            out.extend_from_slice(&bytes);
        }
        return Ok(());
    }
    Err("HLS fetch: retry budget exhausted".into())
}

/// Read a response body into a temporary buffer; the caller commits it only
/// after validating the complete response (no partial segment exposure).
async fn read_body_capped(mut res: reqwest::Response, max: usize) -> Result<Vec<u8>, BodyReadError> {
    if res.content_length().is_some_and(|len| len > max as u64) {
        return Err(BodyReadError::TooLarge(max));
    }
    let mut out = Vec::new();
    while let Some(chunk) = res.chunk().await? {
        if chunk.len() > max.saturating_sub(out.len()) {
            return Err(BodyReadError::TooLarge(max));
        }
        out.extend_from_slice(&chunk);
    }
    Ok(out)
}
