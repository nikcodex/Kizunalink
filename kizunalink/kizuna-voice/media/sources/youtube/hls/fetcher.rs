// Copyright (c) 2026 nikcodex (KizunaLink)
// Licensed under the MIT License

use super::types::Resource;
use crate::common::types::AnyResult;

/// Hard upper bound on a single HLS segment/init body. Segments are small
/// (typically tens of KiB); this only guards against a provider or compromised
/// endpoint streaming an unbounded body into memory.
pub const MAX_HLS_SEGMENT_BYTES: usize = 32 * 1024 * 1024; // 32 MiB

pub async fn fetch_segment_into(
    client: &reqwest::Client,
    resource: &Resource,
    out: &mut Vec<u8>,
) -> AnyResult<()> {
    let mut req = client.get(&resource.url).header("Accept", "*/*");

    let wanted: Option<u64> = resource.range.as_ref().map(|r| r.length);
    if let Some(range) = &resource.range {
        let end = range.offset + range.length - 1;
        req = req.header("Range", format!("bytes={}-{}", range.offset, end));
    }

    let res = req.send().await?;
    let status = res.status();

    if wanted.is_some() {
        // A range was requested. `206 Partial Content` is the only correct
        // answer; a bare `200 OK` means the server ignored the Range header and
        // sent the whole resource from byte 0, which would be appended as though
        // it were the requested slice and corrupt the stream.
        if status == reqwest::StatusCode::OK {
            return Err(format!(
                "HLS fetch: server ignored Range request (200 OK) for {}",
                resource.url
            )
            .into());
        }
        if status != reqwest::StatusCode::PARTIAL_CONTENT {
            return Err(format!("HLS fetch failed {}: {}", status, resource.url).into());
        }
    } else if !status.is_success() {
        return Err(format!("HLS fetch failed {}: {}", status, resource.url).into());
    }

    // Never materialize more than the requested range (and never more than the
    // absolute segment cap), even if the response body claims/streams more.
    let cap = wanted
        .map(|w| w.min(MAX_HLS_SEGMENT_BYTES as u64))
        .unwrap_or(MAX_HLS_SEGMENT_BYTES as u64) as usize;

    let bytes = read_body_capped(res, cap).await?;
    out.extend_from_slice(&bytes);

    Ok(())
}

/// Read a response body, rejecting anything larger than `max` bytes. Enforces the
/// cap both up front (when `Content-Length` is known) and while streaming, so a
/// chunked/oversized body cannot grow memory without bound.
async fn read_body_capped(mut res: reqwest::Response, max: usize) -> AnyResult<Vec<u8>> {
    if let Some(len) = res.content_length()
        && len > max as u64
    {
        return Err(format!("HLS fetch: body too large ({len} bytes, limit {max})").into());
    }

    let mut out: Vec<u8> = Vec::new();
    while let Some(chunk) = res.chunk().await? {
        if out.len() + chunk.len() > max {
            return Err(format!("HLS fetch: body exceeded {max} bytes").into());
        }
        out.extend_from_slice(&chunk);
    }
    Ok(out)
}
