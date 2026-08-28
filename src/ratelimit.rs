use dashmap::DashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tracing::warn;

/// Token bucket rate limiter per IP address.
pub struct RateLimiter {
    buckets: DashMap<String, TokenBucket>,
    config: RateLimitConfig,
}

#[derive(Clone)]
pub struct RateLimitConfig {
    /// Max requests per window
    pub max_requests: u32,
    /// Window duration
    pub window: Duration,
    /// Burst allowance (tokens above max_requests)
    pub burst: u32,
    /// Per-source limits (source_name -> max_requests)
    pub source_limits: std::collections::HashMap<String, u32>,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        let mut source_limits = std::collections::HashMap::new();
        source_limits.insert("spotify".to_string(), 30);
        source_limits.insert("youtube".to_string(), 30);
        source_limits.insert("soundcloud".to_string(), 20);
        source_limits.insert("deezer".to_string(), 20);
        source_limits.insert("applemusic".to_string(), 20);
        source_limits.insert("jiosaavn".to_string(), 30);

        Self {
            max_requests: 60,
            window: Duration::from_secs(60),
            burst: 10,
            source_limits,
        }
    }
}

struct TokenBucket {
    tokens: f64,
    last_refill: Instant,
    max_tokens: f64,
    refill_rate: f64, // tokens per second
}

impl TokenBucket {
    fn new(max_tokens: f64, refill_rate: f64) -> Self {
        Self {
            tokens: max_tokens,
            last_refill: Instant::now(),
            max_tokens,
            refill_rate,
        }
    }

    fn try_consume(&mut self, tokens: f64) -> bool {
        self.refill();
        if self.tokens >= tokens {
            self.tokens -= tokens;
            true
        } else {
            false
        }
    }

    fn refill(&mut self) {
        let elapsed = self.last_refill.elapsed().as_secs_f64();
        self.tokens = (self.tokens + elapsed * self.refill_rate).min(self.max_tokens);
        self.last_refill = Instant::now();
    }
}

impl RateLimiter {
    pub fn new(config: RateLimitConfig) -> Arc<Self> {
        Arc::new(Self {
            buckets: DashMap::new(),
            config,
        })
    }

    /// Check if a request from the given IP is allowed.
    pub fn check(&self, ip: &str) -> bool {
        let max = self.config.max_tokens();
        let rate = self.config.refill_rate();

        let mut bucket = self
            .buckets
            .entry(ip.to_string())
            .or_insert_with(|| TokenBucket::new(max, rate));

        bucket.try_consume(1.0)
    }

    /// Check if a request for a specific source is allowed.
    pub fn check_source(&self, ip: &str, source: &str) -> bool {
        if !self.check(ip) {
            return false;
        }

        if let Some(&limit) = self.config.source_limits.get(source) {
            let key = format!("{}:{}", ip, source);
            let rate = limit as f64 / self.config.window.as_secs() as f64;
            let mut bucket = self.buckets.entry(key).or_insert_with(|| {
                TokenBucket::new(limit as f64, rate)
            });
            bucket.try_consume(1.0)
        } else {
            true
        }
    }

    /// Get the number of remaining tokens for an IP.
    pub fn remaining(&self, ip: &str) -> u32 {
        self.buckets
            .get(ip)
            .map(|b| b.tokens.max(0.0) as u32)
            .unwrap_or(self.config.max_requests)
    }

    /// Cleanup old buckets periodically.
    pub async fn cleanup(&self) {
        let mut to_remove = Vec::new();
        for entry in self.buckets.iter() {
            if entry.value().last_refill.elapsed() > self.config.window * 2 {
                to_remove.push(entry.key().clone());
            }
        }
        for key in to_remove {
            self.buckets.remove(&key);
        }
    }
}

impl RateLimitConfig {
    fn max_tokens(&self) -> f64 {
        (self.max_requests + self.burst) as f64
    }

    fn refill_rate(&self) -> f64 {
        self.max_requests as f64 / self.window.as_secs() as f64
    }
}

/// Extract client IP from request headers or peer address.
pub fn extract_ip(headers: &axum::http::HeaderMap, fallback: &str) -> String {
    // Check X-Forwarded-For first (for reverse proxies)
    if let Some(forwarded) = headers.get("x-forwarded-for") {
        if let Ok(val) = forwarded.to_str() {
            if let Some(first) = val.split(',').next() {
                let ip = first.trim().to_string();
                if !ip.is_empty() {
                    return ip;
                }
            }
        }
    }

    // Check X-Real-IP
    if let Some(real_ip) = headers.get("x-real-ip") {
        if let Ok(val) = real_ip.to_str() {
            let ip = val.trim().to_string();
            if !ip.is_empty() {
                return ip;
            }
        }
    }

    fallback.to_string()
}
