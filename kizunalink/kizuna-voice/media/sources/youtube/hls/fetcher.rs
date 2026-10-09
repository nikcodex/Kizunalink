// Copyright (c) 2026 nikcodex (KizunaLink)
// Licensed under the MIT License

use super::types::Resource;
use crate::{common::types::AnyResult, engine::source::range::validate_content_range};

/// Hard upper bound on a single HLS segment/init body, including full-body fallbacks.
pub const MAX_HLS_SEGMENT_BYTES: usize = 32 * 1024 * 1024;
const MAX_TRANSIENT_RETRIES: usize = 2;

pub async fn fetch_segment_into(
    client: &reqwest::Client,
    resource: &Resource,
    out: &mut Vec<u8>,
) -> AnyResult<()> {
    let range = resource.range.as_ref();
    let end = if let Some(r) = range {
        Some(r.offset.checked_add(r.length.checked_sub(1).ok_or("empty HLS range")?)
            .ok_or("HLS range overflow")?)
    } else {
        None
    };
    if range.is_some_and(|r| r.length > MAX_HLS_SEGMENT_BYTES as u64) {
        return Err("HLS fetch: requested range exceeds segment limit".into());
    }

    for attempt in 0..=MAX_TRANSIENT_RETRIES {
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
                tokio::time::sleep(std::time::Duration::from_millis(100 << attempt)).await;
                continue;
            }
            Err(e) => return Err(e.into()),
        };
        let status = res.status();
        if matches!(status.as_u16(), 429 | 500 | 502 | 503 | 504)
            && attempt < MAX_TRANSIENT_RETRIES
        {
            tokio::time::sleep(std::time::Duration::from_millis(100 << attempt)).await;
            continue;
        }
        if let Some(r) = range {
            let bytes = if status == reqwest::StatusCode::PARTIAL_CONTENT {
                validate_content_range(&res, r.offset, Some(r.length))?;
                let bytes = read_body_capped(res, r.length as usize).await?;
                if bytes.len() as u64 != r.length {
                    return Err(format!("HLS fetch: truncated range body ({} of {} bytes)", bytes.len(), r.length).into());
                }
                bytes
            } else if status == reqwest::StatusCode::OK {
                // A server that ignores Range may be used only if the complete
                // representation is declared, fits the cap, and contains the
                // requested interval. Never mistake byte zero for a seek target.
                let full_len = res.content_length().ok_or("HLS fetch: ignored Range without Content-Length")?;
                let requested_end = r.offset + r.length - 1; // checked above
                if full_len > MAX_HLS_SEGMENT_BYTES as u64 || full_len <= requested_end {
                    return Err("HLS fetch: unsafe full-body Range fallback".into());
                }
                let full = read_body_capped(res, MAX_HLS_SEGMENT_BYTES).await?;
                if full.len() as u64 != full_len {
                    return Err("HLS fetch: truncated full-body Range fallback".into());
                }
                full[r.offset as usize..=requested_end as usize].to_vec()
            } else {
                return Err(format!("HLS fetch failed {status}").into());
            };
            out.extend_from_slice(&bytes);
        } else {
            if status != reqwest::StatusCode::OK {
                return Err(format!("HLS fetch failed {status}").into());
            }
            let bytes = read_body_capped(res, MAX_HLS_SEGMENT_BYTES).await?;
            out.extend_from_slice(&bytes);
        }
        return Ok(());
    }
    Err("HLS fetch: retry budget exhausted".into())
}

/// Read a response body into a temporary buffer; the caller commits it only
/// after validating the complete response (no partial segment exposure).
async fn read_body_capped(mut res: reqwest::Response, max: usize) -> AnyResult<Vec<u8>> {
    if res.content_length().is_some_and(|len| len > max as u64) {
        return Err(format!("HLS fetch: body exceeds {max} bytes").into());
    }
    let mut out = Vec::new();
    while let Some(chunk) = res.chunk().await? {
        if chunk.len() > max.saturating_sub(out.len()) {
            return Err(format!("HLS fetch: body exceeded {max} bytes").into());
        }
        out.extend_from_slice(&chunk);
    }
    Ok(out)
}
