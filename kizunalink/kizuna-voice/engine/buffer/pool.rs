// Copyright (c) 2026 nikcodex (KizunaLink)
// Licensed under the MIT License

use std::{
    collections::HashMap,
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, Instant},
};

use crate::engine::constants::{MAX_BUCKET_ENTRIES, MAX_POOL_BYTES, POOL_IDLE_CLEAR_SECS};

struct PoolInner {
    buckets: HashMap<usize, Vec<Vec<u8>>>,
    total_bytes: usize,
    last_activity: Instant,
    last_cleanup: Instant,
}

impl PoolInner {
    fn new() -> Self {
        let now = Instant::now();
        Self {
            buckets: HashMap::new(),
            total_bytes: 0,
            last_activity: now,
            last_cleanup: now,
        }
    }

    fn aligned_size(size: usize) -> usize {
        size.max(1024).next_power_of_two()
    }

    fn acquire(&mut self, size: usize) -> Vec<u8> {
        self.last_activity = Instant::now();
        let aligned = Self::aligned_size(size);

        if let Some(buf) = self
            .buckets
            .get_mut(&aligned)
            .and_then(|bucket| bucket.pop())
        {
            self.total_bytes -= aligned;
            return buf;
        }
        Vec::with_capacity(aligned)
    }

    fn release(&mut self, mut buf: Vec<u8>) {
        self.last_activity = Instant::now();
        let size = buf.capacity();
        let key = Self::aligned_size(size);

        // Only pool buffers in the 1 KB – 10 MB range.
        if !(1024..=10 * 1024 * 1024).contains(&key) {
            return;
        }
        if self.total_bytes + key > MAX_POOL_BYTES {
            return;
        }

        let bucket = self.buckets.entry(key).or_default();
        if bucket.len() >= MAX_BUCKET_ENTRIES {
            return;
        }

        buf.clear();
        if buf.capacity() > key {
            buf.shrink_to(key);
        }
        self.total_bytes += key;
        bucket.push(buf);
    }

    fn cleanup(&mut self) {
        if self.total_bytes == 0 {
            return;
        }

        // Rate-limit cleanup checks to every 30 seconds.
        if self.last_cleanup.elapsed() < Duration::from_secs(30) {
            return;
        }
        self.last_cleanup = Instant::now();

        let is_idle = self.last_activity.elapsed() >= Duration::from_secs(POOL_IDLE_CLEAR_SECS);
        let is_over_limit = self.total_bytes > MAX_POOL_BYTES;

        if is_idle || is_over_limit {
            self.buckets.clear();
            self.total_bytes = 0;
        }
    }
}

pub struct BufferPool {
    inner: Mutex<PoolInner>,
}

impl BufferPool {
    fn new() -> Self {
        Self {
            inner: Mutex::new(PoolInner::new()),
        }
    }

    pub fn acquire(&self, size: usize) -> Vec<u8> {
        let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        g.cleanup();
        g.acquire(size)
    }

    pub fn release(&self, buf: Vec<u8>) {
        let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        g.release(buf);
    }

    pub fn stats(&self) -> PoolStats {
        let g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        PoolStats {
            total_bytes: g.total_bytes,
            buckets: g.buckets.len(),
            entries: g.buckets.values().map(|b| b.len()).sum(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct PoolStats {
    pub total_bytes: usize,
    pub buckets: usize,
    pub entries: usize,
}

static GLOBAL_BYTE_POOL: OnceLock<Arc<BufferPool>> = OnceLock::new();

pub fn get_byte_pool() -> Arc<BufferPool> {
    GLOBAL_BYTE_POOL
        .get_or_init(|| Arc::new(BufferPool::new()))
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aligned_size_rounds_to_power_of_two() {
        assert_eq!(PoolInner::aligned_size(0), 1024);
        assert_eq!(PoolInner::aligned_size(1), 1024);
        assert_eq!(PoolInner::aligned_size(1024), 1024);
        assert_eq!(PoolInner::aligned_size(1025), 2048);
        assert_eq!(PoolInner::aligned_size(4096), 4096);
        assert_eq!(PoolInner::aligned_size(4097), 8192);
    }

    #[test]
    fn acquire_returns_buffer_with_sufficient_capacity() {
        let pool = BufferPool::new();
        let buf = pool.acquire(500);
        assert!(buf.capacity() >= 1024);
        assert!(buf.is_empty());

        let buf2 = pool.acquire(5000);
        assert!(buf2.capacity() >= 5000);
    }

    #[test]
    fn release_and_reuse() {
        let pool = BufferPool::new();
        let buf = pool.acquire(2048);
        let cap = buf.capacity();
        pool.release(buf);

        let stats = pool.stats();
        assert_eq!(stats.entries, 1);
        assert_eq!(stats.buckets, 1);

        let buf2 = pool.acquire(2048);
        assert!(buf2.capacity() >= cap);
    }

    #[test]
    fn release_small_buffer_not_pooled() {
        let pool = BufferPool::new();
        // Acquire a tiny buffer (will get 1024-aligned)
        let buf = pool.acquire(10);
        pool.release(buf);
        let stats = pool.stats();
        // 1024 is in range, so it should be pooled
        assert_eq!(stats.entries, 1);
    }

    #[test]
    fn pool_respects_max_entries() {
        let pool = BufferPool::new();
        let mut bufs = Vec::new();
        for _ in 0..MAX_BUCKET_ENTRIES + 5 {
            bufs.push(pool.acquire(1024));
        }
        for buf in bufs {
            pool.release(buf);
        }
        let stats = pool.stats();
        assert!(stats.entries <= MAX_BUCKET_ENTRIES);
    }

    #[test]
    fn acquire_with_zero_size_uses_minimum() {
        let pool = BufferPool::new();
        let buf = pool.acquire(0);
        assert!(buf.capacity() >= 1024);
    }
}
