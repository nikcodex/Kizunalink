// Copyright (c) 2026 nikcodex (KizunaLink)
// Licensed under the MIT License

//! Per-client token-bucket rate limiting for the public API surface (S06).
//!
//! One bucket per source IP: capacity `limit_per_minute` tokens, refilled
//! continuously at `limit_per_minute / 60` tokens per second. A request costs
//! one token; a request that arrives at an empty bucket is rejected with
//! `429 Too Many Requests` and a `Retry-After` header.

use std::{
    net::{IpAddr, SocketAddr},
    sync::Arc,
    time::{Duration, Instant},
};

use axum::{
    extract::{ConnectInfo, Request, State},
    http::{HeaderName, HeaderValue, StatusCode},
    middleware::Next,
    response::Response,
};
use dashmap::DashMap;
use tracing::warn;

use crate::server::AppState;

#[derive(Debug)]
struct Bucket {
    tokens: f64,
    updated_at: Instant,
}

/// IP-keyed token bucket. `limit_per_minute == 0` disables limiting entirely.
pub struct RateLimiter {
    limit_per_minute: u32,
    buckets: DashMap<IpAddr, Bucket>,
}

impl RateLimiter {
    pub fn new(limit_per_minute: u32) -> Self {
        Self {
            limit_per_minute,
            buckets: DashMap::new(),
        }
    }

    pub fn disabled(&self) -> bool {
        self.limit_per_minute == 0
    }

    fn refill_rate(&self) -> f64 {
        f64::from(self.limit_per_minute) / 60.0
    }

    /// Consume one token for `ip`, refilling from elapsed time first.
    /// Returns `Some(retry_after)` when the caller must be throttled.
    ///
    /// `now` is injected so the logic is deterministic and unit-testable.
    pub fn check(&self, ip: IpAddr, now: Instant) -> Option<Duration> {
        if self.disabled() {
            return None;
        }

        let capacity = f64::from(self.limit_per_minute);
        let mut entry = self.buckets.entry(ip).or_insert(Bucket {
            tokens: capacity,
            updated_at: now,
        });

        let elapsed = now
            .saturating_duration_since(entry.updated_at)
            .as_secs_f64();
        entry.tokens = (entry.tokens + elapsed * self.refill_rate()).min(capacity);
        entry.updated_at = now;

        if entry.tokens >= 1.0 {
            entry.tokens -= 1.0;
            None
        } else {
            let wait_secs = (1.0 - entry.tokens) / self.refill_rate();
            Some(Duration::from_secs_f64(wait_secs).max(Duration::from_secs(1)))
        }
    }

    /// Drop buckets idle for longer than `idle_ttl` so memory stays bounded under IP churn.
    pub fn cleanup(&self, now: Instant, idle_ttl: Duration) {
        self.buckets
            .retain(|_, bucket| now.saturating_duration_since(bucket.updated_at) <= idle_ttl);
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.buckets.len()
    }
}

/// Axum middleware: throttles by the *connected* peer address.
///
/// When deployed behind a reverse proxy the socket IP is the proxy itself; run the
/// proxy-to-client IP extraction (e.g. `tower_http`'s forwarded header layers) in
/// front of this if you need per-client accounting in that topology.
pub async fn rate_limit(State(state): State<Arc<AppState>>, req: Request, next: Next) -> Response {
    let limiter = state.rate_limiter.as_ref();
    if limiter.disabled() {
        return next.run(req).await;
    }

    let peer = req
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ci| ci.0.ip());

    let Some(ip) = peer else {
        // No socket info (e.g. synthetic service calls) — never block on missing data.
        return next.run(req).await;
    };

    match limiter.check(ip, Instant::now()) {
        None => next.run(req).await,
        Some(retry_after) => {
            warn!("rate-limit: throttling {ip} for {retry_after:?}");
            let mut resp = Response::new("Too Many Requests".into());
            *resp.status_mut() = StatusCode::TOO_MANY_REQUESTS;
            if let Ok(v) = HeaderValue::from_str(&retry_after.as_secs().to_string()) {
                resp.headers_mut()
                    .insert(HeaderName::from_static("retry-after"), v);
            }
            resp
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    fn ip(n: u8) -> IpAddr {
        IpAddr::V4(Ipv4Addr::new(10, 0, 0, n))
    }

    #[test]
    fn disabled_limiter_never_blocks() {
        let l = RateLimiter::new(0);
        for _ in 0..1000 {
            assert!(l.check(ip(1), Instant::now()).is_none());
        }
    }

    #[test]
    fn burst_over_capacity_is_rejected_with_retry_after() {
        let l = RateLimiter::new(60); // 60/min -> 1 token/s, capacity 60
        let t0 = Instant::now();
        for _ in 0..60 {
            assert!(l.check(ip(1), t0).is_none());
        }
        let over = l.check(ip(1), t0).expect("61st request must be throttled");
        assert!(!over.is_zero());

        // Different IPs have independent buckets.
        assert!(l.check(ip(2), t0).is_none());
    }

    #[test]
    fn buckets_refill_over_time() {
        let l = RateLimiter::new(60); // 1 token per second
        let t0 = Instant::now();
        for _ in 0..60 {
            l.check(ip(1), t0);
        }
        assert!(l.check(ip(1), t0).is_some());
        // After 3 seconds exactly 3 tokens (1/s refill rate) are back: three pass, the
        // fourth is throttled again.
        let t3 = t0 + Duration::from_secs(3);
        assert!(l.check(ip(1), t3).is_none());
        assert!(l.check(ip(1), t3).is_none());
        assert!(l.check(ip(1), t3).is_none());
        assert!(l.check(ip(1), t3).is_some());
    }

    #[test]
    fn cleanup_drops_only_idle_buckets() {
        let l = RateLimiter::new(10);
        let t0 = Instant::now();
        l.check(ip(1), t0);
        l.check(ip(2), t0 + Duration::from_secs(5));
        let now = t0 + Duration::from_secs(10);
        l.cleanup(now, Duration::from_secs(8));
        assert_eq!(l.len(), 1, "only the recently-seen bucket survives");
    }
}
