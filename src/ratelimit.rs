use dashmap::DashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

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

/// Default rate limits per audio source (source name, max requests per window).
pub const DEFAULT_SOURCE_LIMITS: [(&str, u32); 6] = [
    ("spotify", 30),
    ("youtube", 30),
    ("soundcloud", 20),
    ("deezer", 20),
    ("applemusic", 20),
    ("jiosaavn", 30),
];

impl Default for RateLimitConfig {
    fn default() -> Self {
        let source_limits = DEFAULT_SOURCE_LIMITS
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect();

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

    /// Window duration in seconds.
    pub fn window_secs(&self) -> u64 {
        self.config.window.as_secs()
    }

    /// Check if a request from the given IP is allowed.
    pub fn check(&self, ip: &str) -> bool {
        self.check_rate_limit(ip)
    }

    /// Check if a request from the given IP is allowed (atomic token bucket check).
    pub fn check_rate_limit(&self, ip: &str) -> bool {
        let max = self.config.max_tokens();
        let rate = self.config.refill_rate();

        let mut allowed = false;
        self.buckets
            .entry(ip.to_string())
            .and_modify(|bucket| {
                allowed = bucket.try_consume(1.0);
            })
            .or_insert_with(|| {
                let mut bucket = TokenBucket::new(max, rate);
                allowed = bucket.try_consume(1.0);
                bucket
            });

        allowed
    }

    /// Check if a request for a specific source is allowed.
    pub fn check_source(&self, ip: &str, source: &str) -> bool {
        if !self.check_rate_limit(ip) {
            return false;
        }

        let limit = self.config.source_limits.get(source).copied();

        if let Some(limit) = limit {
            let key = format!("{}:{}", ip, source);
            let secs = self.config.window.as_secs().max(1) as f64;
            let rate = limit as f64 / secs;
            let mut allowed = false;
            self.buckets
                .entry(key)
                .and_modify(|bucket| {
                    allowed = bucket.try_consume(1.0);
                })
                .or_insert_with(|| {
                    let mut bucket = TokenBucket::new(limit as f64, rate);
                    allowed = bucket.try_consume(1.0);
                    bucket
                });
            allowed
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
        let ttl = self.config.window * 2;
        self.buckets
            .retain(|_, bucket| bucket.last_refill.elapsed() <= ttl);
    }
}

impl RateLimitConfig {
    pub fn window_secs(&self) -> u64 {
        self.window.as_secs()
    }

    fn max_tokens(&self) -> f64 {
        (self.max_requests + self.burst) as f64
    }

    fn refill_rate(&self) -> f64 {
        let secs = self.window.as_secs().max(1) as f64;
        self.max_requests as f64 / secs
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
