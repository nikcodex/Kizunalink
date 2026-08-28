//! Async-safe exponential backoff with jitter for rate-limited HTTP sources.
//!
//! Used by JioSaavn (and any other source) to gracefully handle 429s and transient
//! network errors without blocking the Tokio runtime.

use rand::Rng;
use std::time::Duration;
use tracing::{error, info, warn};

/// Configuration for exponential backoff behavior.
#[derive(Debug, Clone)]
pub struct BackoffConfig {
    /// Initial delay before the first retry.
    pub base_delay: Duration,
    /// Maximum delay cap.
    pub max_delay: Duration,
    /// Maximum number of retries before giving up.
    pub max_retries: u32,
    /// Multiplier for subsequent retries (typically 2.0).
    pub multiplier: f64,
    /// Jitter factor as a fraction of delay (0.0 to 1.0, e.g. 0.25 for ±25%).
    pub jitter_factor: f64,
}

impl Default for BackoffConfig {
    fn default() -> Self {
        Self {
            base_delay: Duration::from_millis(500),
            max_delay: Duration::from_secs(10),
            max_retries: 4,
            multiplier: 2.0,
            jitter_factor: 0.25,
        }
    }
}

/// Check if an HTTP status code is retryable (429, 5xx, or 0/connection errors).
pub fn is_retryable_status(status: u16) -> bool {
    status == 429 || (500..=599).contains(&status) || status == 0
}

/// Execute `op` with exponential backoff + full jitter.
///
/// Returns `Ok(T)` on the first successful call, or the last `Err(E)`
/// after exhausting all retries. Sleeps are executed via `tokio::time::sleep`,
/// ensuring zero blocking of the async executor threads.
pub async fn with_backoff<T, E, F, Fut>(
    config: &BackoffConfig,
    label: &str,
    mut op: F,
) -> Result<T, E>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, E>>,
    E: std::fmt::Display,
{
    let mut last_err: Option<E> = None;

    for attempt in 0..=config.max_retries {
        match op().await {
            Ok(val) => {
                if attempt > 0 {
                    info!(
                        "⚡ [{}] Request succeeded after {} retry attempt(s)",
                        label, attempt
                    );
                }
                return Ok(val);
            }
            Err(e) => {
                if attempt >= config.max_retries {
                    error!(
                        "❌ [{}] Request permanently failed after {} retries: {}",
                        label, config.max_retries, e
                    );
                    return Err(e);
                }

                // Compute exponential delay: base * multiplier^attempt
                let factor = config.multiplier.powi(attempt as i32);
                let raw_delay_ms = (config.base_delay.as_millis() as f64 * factor) as u64;
                let capped_delay_ms = raw_delay_ms.min(config.max_delay.as_millis() as u64);

                // Add random jitter: ±(jitter_factor * delay)
                let jitter_range = (capped_delay_ms as f64 * config.jitter_factor) as u64;
                let final_delay_ms = if jitter_range > 0 {
                    let mut rng = rand::thread_rng();
                    let jitter = rng.gen_range(0..=(jitter_range * 2));
                    capped_delay_ms
                        .saturating_sub(jitter_range)
                        .saturating_add(jitter)
                } else {
                    capped_delay_ms
                };

                let wait_duration = Duration::from_millis(final_delay_ms.max(10));

                warn!(
                    "⏳ [{}] Attempt {}/{} failed (error: {}). Retrying in {:.2?} (backoff + jitter)...",
                    label,
                    attempt + 1,
                    config.max_retries,
                    e,
                    wait_duration
                );

                last_err = Some(e);
                tokio::time::sleep(wait_duration).await;
            }
        }
    }

    Err(last_err.expect("At least one error occurred"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    #[tokio::test]
    async fn test_succeeds_first_try() {
        let cfg = BackoffConfig::default();
        let result = with_backoff(&cfg, "test_first_try", || async { Ok::<i32, String>(42) }).await;
        assert_eq!(result.unwrap(), 42);
    }

    #[tokio::test]
    async fn test_succeeds_after_retries() {
        let cfg = BackoffConfig {
            base_delay: Duration::from_millis(5),
            max_delay: Duration::from_millis(20),
            max_retries: 3,
            multiplier: 1.5,
            jitter_factor: 0.1,
        };
        let counter = AtomicU32::new(0);
        let result = with_backoff(&cfg, "test_retry", || {
            let attempt = counter.fetch_add(1, Ordering::SeqCst);
            async move {
                if attempt < 2 {
                    Err("HTTP 429 Too Many Requests".to_string())
                } else {
                    Ok(100)
                }
            }
        })
        .await;

        assert_eq!(result.unwrap(), 100);
        assert_eq!(counter.load(Ordering::SeqCst), 3); // Attempts 0, 1 (failed), 2 (succeeded)
    }

    #[tokio::test]
    async fn test_exhausts_retries_and_fails() {
        let cfg = BackoffConfig {
            base_delay: Duration::from_millis(2),
            max_delay: Duration::from_millis(10),
            max_retries: 2,
            multiplier: 2.0,
            jitter_factor: 0.0,
        };
        let counter = AtomicU32::new(0);
        let result = with_backoff(&cfg, "test_exhaust", || {
            counter.fetch_add(1, Ordering::SeqCst);
            async move { Err::<(), _>("HTTP 500 Internal Error".to_string()) }
        })
        .await;

        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "HTTP 500 Internal Error");
        assert_eq!(counter.load(Ordering::SeqCst), 3); // 1 initial + 2 retries
    }

    #[test]
    fn test_retryable_status() {
        assert!(is_retryable_status(429));
        assert!(is_retryable_status(500));
        assert!(is_retryable_status(502));
        assert!(is_retryable_status(503));
        assert!(is_retryable_status(504));
        assert!(is_retryable_status(0));

        assert!(!is_retryable_status(200));
        assert!(!is_retryable_status(400));
        assert!(!is_retryable_status(401));
        assert!(!is_retryable_status(404));
    }
}
