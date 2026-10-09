// Copyright (c) 2026 nikcodex (KizunaLink)
// Licensed under the MIT License

use std::io;

/// Validate an inclusive HTTP Content-Range before exposing a 206 body to a reader.
/// The returned length is the number of bytes the body must contain.
pub(crate) fn validate_content_range(
    response: &reqwest::Response,
    offset: u64,
    length: Option<u64>,
) -> io::Result<u64> {
    let invalid = || {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid or mismatched Content-Range",
        )
    };
    if response.status() != reqwest::StatusCode::PARTIAL_CONTENT {
        return Err(invalid());
    }
    let header = response
        .headers()
        .get(reqwest::header::CONTENT_RANGE)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(invalid)?;
    let (unit, interval) = header.split_once(' ').ok_or_else(invalid)?;
    if unit != "bytes" {
        return Err(invalid());
    }
    let (span, total) = interval.split_once('/').ok_or_else(invalid)?;
    let (start, end) = span.split_once('-').ok_or_else(invalid)?;
    let start = start.parse::<u64>().map_err(|_| invalid())?;
    let end = end.parse::<u64>().map_err(|_| invalid())?;
    // RFC 9110 permits an unknown complete length (`bytes 2-3/*`).
    // SegmentedSource separately requires a numeric total for its probe.
    let total = if total == "*" {
        None
    } else {
        Some(total.parse::<u64>().map_err(|_| invalid())?)
    };
    let count = end
        .checked_sub(start)
        .and_then(|n| n.checked_add(1))
        .ok_or_else(invalid)?;
    if start != offset || total.is_some_and(|total| end >= total) || length.is_some_and(|len| len != count) {
        return Err(invalid());
    }
    if response.content_length().is_some_and(|len| len != count) {
        return Err(invalid());
    }
    Ok(count)
}
