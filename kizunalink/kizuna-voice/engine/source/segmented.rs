// Copyright (c) 2026 nikcodex (KizunaLink)
// Licensed under the MIT License

use std::{
    collections::HashMap,
    io::{Read, Seek, SeekFrom},
    sync::Arc,
    time::Duration,
};

use bytes::Bytes;
use parking_lot::{Condvar, Mutex};
use symphonia::core::io::MediaSource;
use tracing::{debug, trace, warn};

use super::AudioSource;
use crate::{
    common::types::AnyResult,
    engine::constants::{
        CHUNK_SIZE, FETCH_WAIT_MS, MAX_CONCURRENT_FETCHES, MAX_FETCH_RETRIES, PREFETCH_CHUNKS,
        PROBE_TIMEOUT_SECS, WORKER_IDLE_MS,
    },
};

#[derive(Clone)]
enum ChunkState {
    Empty(u32),
    Downloading,
    Ready(Bytes),
}

struct ReaderState {
    chunks: HashMap<usize, ChunkState>,
    current_pos: u64,
    total_len: u64,
    is_terminated: bool,
    fatal_error: Option<String>,
}

pub struct SegmentedSource {
    pos: u64,
    len: u64,
    content_type: Option<Arc<str>>,
    shared: Arc<(Mutex<ReaderState>, Condvar)>,
    workers: Vec<tokio::task::JoinHandle<()>>,
}

impl SegmentedSource {
    pub fn new(client: reqwest::Client, url: &str) -> AnyResult<Self> {
        let handle = tokio::runtime::Handle::current();

        let probe = handle.block_on(
            client
                .get(url)
                .header("Range", "bytes=0-0")
                .header("Connection", "close")
                .timeout(Duration::from_secs(PROBE_TIMEOUT_SECS))
                .send(),
        )?;

        // This source requires real byte-range support. A 200 probe or a
        // mismatched 206 must never be used to infer the file length.
        super::range::validate_content_range(&probe, 0, Some(1))?;
        let len = probe
            .headers()
            .get(reqwest::header::CONTENT_RANGE)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.split_once('/'))
            .and_then(|(_, total)| total.parse::<u64>().ok())
            .ok_or_else(|| std::io::Error::other("SegmentedSource: missing total length"))?;

        let content_type: Option<Arc<str>> = probe
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(Arc::from);

        debug!(
            "SegmentedSource opened: len={}, type={:?}",
            len, content_type
        );

        let mut chunks = HashMap::new();
        chunks.insert(0, ChunkState::Empty(0));

        let shared = Arc::new((
            Mutex::new(ReaderState {
                chunks,
                current_pos: 0,
                total_len: len,
                is_terminated: false,
                fatal_error: None,
            }),
            Condvar::new(),
        ));

        let mut workers = Vec::with_capacity(MAX_CONCURRENT_FETCHES);
        for worker_id in 0..MAX_CONCURRENT_FETCHES {
            let shared_clone = shared.clone();
            let client_clone = client.clone();
            let url_str = url.to_string();
            workers.push(tokio::spawn(async move {
                fetch_worker(worker_id, shared_clone, client_clone, url_str).await;
            }));
        }

        Ok(Self {
            pos: 0,
            len,
            content_type,
            shared,
            workers,
        })
    }
}

impl AudioSource for SegmentedSource {
    fn content_type(&self) -> Option<String> {
        self.content_type.as_deref().map(str::to_string)
    }
}

impl Read for SegmentedSource {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.pos >= self.len {
            return Ok(0);
        }

        let (lock, cvar) = &*self.shared;
        let mut state = lock.lock();
        state.current_pos = self.pos;

        loop {
            if let Some(ref err) = state.fatal_error {
                return Err(std::io::Error::other(err.clone()));
            }

            let chunk_idx = (self.pos / CHUNK_SIZE as u64) as usize;
            let offset_in_chunk = (self.pos % CHUNK_SIZE as u64) as usize;

            match state.chunks.get(&chunk_idx) {
                Some(ChunkState::Ready(bytes)) => {
                    let bytes = bytes.clone();
                    let available = bytes.len().saturating_sub(offset_in_chunk);

                    if available == 0 {
                        self.pos = ((chunk_idx + 1) * CHUNK_SIZE) as u64;
                        state.current_pos = self.pos;
                        continue;
                    }

                    let n = buf.len().min(available);
                    buf[..n].copy_from_slice(&bytes[offset_in_chunk..offset_in_chunk + n]);
                    self.pos += n as u64;
                    state.current_pos = self.pos;

                    if chunk_idx > 1 {
                        state.chunks.retain(|&idx, _| idx >= chunk_idx - 1);
                    }

                    return Ok(n);
                }

                Some(ChunkState::Downloading) | Some(ChunkState::Empty(_)) => {
                    cvar.notify_all();
                    trace!("SegmentedSource: waiting for chunk {}", chunk_idx);
                    cvar.wait_for(&mut state, Duration::from_millis(FETCH_WAIT_MS));
                }

                None => {
                    state.chunks.insert(chunk_idx, ChunkState::Empty(0));
                    cvar.notify_all();
                }
            }
        }
    }
}

impl Seek for SegmentedSource {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        let new_pos = match pos {
            SeekFrom::Start(p) => p,
            SeekFrom::Current(delta) => self.pos.saturating_add_signed(delta),
            SeekFrom::End(delta) => self.len.saturating_add_signed(delta),
        };

        self.pos = new_pos.min(self.len);
        debug!("SegmentedSource: seek → {}", self.pos);

        let (lock, cvar) = &*self.shared;
        let mut state = lock.lock();
        state.current_pos = self.pos;
        cvar.notify_all();

        Ok(self.pos)
    }
}

impl MediaSource for SegmentedSource {
    fn is_seekable(&self) -> bool {
        true
    }

    fn byte_len(&self) -> Option<u64> {
        Some(self.len)
    }
}

impl Drop for SegmentedSource {
    fn drop(&mut self) {
        let (lock, cvar) = &*self.shared;
        let mut state = lock.lock();
        state.is_terminated = true;
        cvar.notify_all();
        // Tokio abort cancels in-flight HTTP requests and idle timers. A
        // detached worker used to retain the client/URL after the source died.
        for worker in &self.workers {
            worker.abort();
        }
    }
}

pub(crate) async fn fetch_chunk(
    client: &reqwest::Client,
    url: &str,
    offset: u64,
    size: u64,
) -> AnyResult<Bytes> {
    let range = format!("bytes={}-{}", offset, offset + size - 1);
    let res = client
        .get(url)
        .header("Range", range)
        .header("Accept", "*/*")
        .send()
        .await?;

    let status = res.status();
    if status == reqwest::StatusCode::OK {
        // The server ignored the Range header and is about to stream the whole
        // resource from byte 0. Appending it would corrupt chunk `offset`.
        return Err(format!(
            "fetch_chunk: server ignored Range request (200 OK) for offset {offset}"
        )
        .into());
    }
    if status != reqwest::StatusCode::PARTIAL_CONTENT {
        return Err(format!("fetch_chunk: HTTP {status}").into());
    }
    super::range::validate_content_range(&res, offset, Some(size))?;

    // Never materialize more than the requested chunk, even if the response
    // claims/streams a larger body.
    if let Some(len) = res.content_length()
        && len > size
    {
        return Err(format!("fetch_chunk: body {len} exceeds requested {size}").into());
    }

    let mut res = res;
    let mut out: Vec<u8> = Vec::with_capacity(size as usize);
    while let Some(chunk) = res.chunk().await? {
        if out.len() as u64 + chunk.len() as u64 > size {
            return Err(format!("fetch_chunk: body exceeded requested {size} bytes").into());
        }
        out.extend_from_slice(&chunk);
    }
    if out.len() as u64 != size {
        return Err(format!(
            "fetch_chunk: truncated body ({} of {size} bytes)",
            out.len()
        )
        .into());
    }
    Ok(Bytes::from(out))
}

async fn fetch_worker(
    worker_id: usize,
    shared: Arc<(Mutex<ReaderState>, Condvar)>,
    client: reqwest::Client,
    url: String,
) {
    let (lock, cvar) = &*shared;

    loop {
        let target = {
            let mut state = lock.lock();

            if state.is_terminated {
                break;
            }

            let current_chunk = (state.current_pos / CHUNK_SIZE as u64) as usize;
            let total_len = state.total_len;

            let claimed = try_claim_chunk(&mut state, current_chunk, total_len);

            if claimed.is_none() {
                let cursor_ready =
                    matches!(state.chunks.get(&current_chunk), Some(ChunkState::Ready(_)));
                let window = if cursor_ready { PREFETCH_CHUNKS } else { 2 };

                let mut found = None;
                for j in 1..window {
                    let idx = current_chunk + j;
                    if (idx * CHUNK_SIZE) as u64 >= total_len {
                        break;
                    }
                    if let Some(c) = try_claim_chunk(&mut state, idx, total_len) {
                        found = Some(c);
                        break;
                    }
                }
                found
            } else {
                claimed
            }
            .map(|(idx, retries)| {
                debug!(
                    "Worker {}: claiming chunk {} (retry={})",
                    worker_id, idx, retries
                );
                state.chunks.insert(idx, ChunkState::Downloading);
                (idx, retries, total_len)
            })
        };

        let (idx, prior_retries, total_len) = match target {
            Some(t) => t,
            None => {
                tokio::time::sleep(Duration::from_millis(WORKER_IDLE_MS)).await;
                continue;
            }
        };

        let offset = (idx * CHUNK_SIZE) as u64;
        let size = CHUNK_SIZE.min((total_len - offset) as usize) as u64;

        trace!(
            "Worker {}: requesting chunk {} (offset={}, size={})",
            worker_id, idx, offset, size
        );

        match fetch_chunk(&client, &url, offset, size).await {
            Ok(bytes) => {
                let actual = bytes.len() as u64;
                if actual != size {
                    warn!(
                        "Worker {}: partial fetch for chunk {} (got {}/{} bytes)",
                        worker_id, idx, actual, size
                    );
                    requeue_or_fatal(
                        lock,
                        cvar,
                        idx,
                        prior_retries,
                        &format!("partial fetch: {}/{} bytes", actual, size),
                    );
                    tokio::time::sleep(Duration::from_millis(FETCH_WAIT_MS)).await;
                    continue;
                }

                let mut state = lock.lock();
                state.chunks.insert(idx, ChunkState::Ready(bytes));
                trace!(
                    "Worker {}: filled chunk {} ({} bytes)",
                    worker_id, idx, actual
                );
                cvar.notify_all();
            }
            Err(e) => {
                warn!(
                    "Worker {}: fetch failed for chunk {}: {}",
                    worker_id, idx, e
                );
                requeue_or_fatal(lock, cvar, idx, prior_retries, &e.to_string());
                tokio::time::sleep(Duration::from_millis(FETCH_WAIT_MS)).await;
            }
        }
    }
}

#[inline]
fn try_claim_chunk(state: &mut ReaderState, idx: usize, total_len: u64) -> Option<(usize, u32)> {
    if (idx * CHUNK_SIZE) as u64 >= total_len {
        return None;
    }
    match state.chunks.get(&idx) {
        Some(ChunkState::Empty(r)) => Some((idx, *r)),
        None => Some((idx, 0)),
        _ => None,
    }
}

#[inline]
fn requeue_or_fatal(
    lock: &Mutex<ReaderState>,
    cvar: &Condvar,
    idx: usize,
    prior_retries: u32,
    error: &str,
) {
    let mut state = lock.lock();
    if prior_retries >= MAX_FETCH_RETRIES {
        let msg = format!(
            "Chunk {}: permanently failed after {} retries: {}",
            idx, prior_retries, error
        );
        warn!("SegmentedSource: fatal error - {}", msg);
        state.fatal_error = Some(msg);
    } else {
        state
            .chunks
            .insert(idx, ChunkState::Empty(prior_retries + 1));
    }
    cvar.notify_all();
}

#[cfg(test)]
mod lifecycle_tests {
    use super::*;
    use tokio::sync::oneshot;

    struct ExitSignal(Option<oneshot::Sender<()>>);
    impl Drop for ExitSignal {
        fn drop(&mut self) {
            if let Some(tx) = self.0.take() {
                let _ = tx.send(());
            }
        }
    }

    #[tokio::test]
    async fn dropping_source_cancels_owned_workers_on_repeated_cycles() {
        // A parked task models a fetch stalled inside reqwest; no external
        // provider or network timeout is needed to test ownership/cancellation.
        for _ in 0..3 {
            let shared = Arc::new((
                Mutex::new(ReaderState {
                    chunks: HashMap::new(),
                    current_pos: 0,
                    total_len: 1,
                    is_terminated: false,
                    fatal_error: None,
                }),
                Condvar::new(),
            ));
            let mut workers = Vec::new();
            let mut stopped = Vec::new();
            for _ in 0..MAX_CONCURRENT_FETCHES {
                let (started_tx, started_rx) = oneshot::channel();
                let (stopped_tx, stopped_rx) = oneshot::channel();
                workers.push(tokio::spawn(async move {
                    let _exit = ExitSignal(Some(stopped_tx));
                    let _ = started_tx.send(());
                    std::future::pending::<()>().await;
                }));
                started_rx.await.expect("worker started");
                stopped.push(stopped_rx);
            }
            let source = SegmentedSource {
                pos: 0,
                len: 1,
                content_type: None,
                shared: Arc::clone(&shared),
                workers,
            };
            drop(source);
            assert!(shared.0.lock().is_terminated);
            for stopped_rx in stopped {
                tokio::time::timeout(Duration::from_secs(2), stopped_rx)
                    .await
                    .expect("worker stopped promptly")
                    .expect("worker exit signal delivered");
            }
        }
    }
}
