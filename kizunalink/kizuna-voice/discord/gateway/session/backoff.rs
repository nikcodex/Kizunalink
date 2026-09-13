// Copyright (c) 2026 nikcodex (KizunaLink)
// Licensed under the MIT License

use std::time::Duration;

use crate::discord::gateway::constants::{BACKOFF_BASE_MS, MAX_RECONNECT_ATTEMPTS};

/// Exponential backoff manager for terminal or recoverable retry loops.
#[derive(Debug, Clone, Default)]
pub struct Backoff {
    attempt: u32,
}

impl Backoff {
    /// Creates a fresh backoff state.
    pub const fn new() -> Self {
        Self { attempt: 0 }
    }

    /// Computes and returns the next delay duration, incrementing the attempt counter.
    ///
    /// The exponent is capped at 3, so delays stop growing from the fourth attempt
    /// onward (`8 * base`) while `attempt` keeps counting toward [`Self::is_exhausted`].
    pub fn next_delay(&mut self) -> Duration {
        let exponent = self.attempt.min(3);
        let ms = BACKOFF_BASE_MS * 2u64.pow(exponent);
        self.attempt += 1;
        Duration::from_millis(ms)
    }

    /// Returns `true` once `MAX_RECONNECT_ATTEMPTS` consecutive attempts have been
    /// made. Note this reports the *attempt limit*, not backoff saturation — delays
    /// are already capped at `8 * base` from the fourth attempt on.
    #[inline]
    pub const fn is_exhausted(&self) -> bool {
        self.attempt >= MAX_RECONNECT_ATTEMPTS
    }

    /// Resets the counter to zero.
    #[inline]
    pub fn reset(&mut self) {
        self.attempt = 0;
    }

    /// Current attempt count.
    #[inline]
    pub const fn attempt(&self) -> u32 {
        self.attempt
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_backoff_new() {
        let backoff = Backoff::new();
        assert_eq!(backoff.attempt(), 0);
        assert!(!backoff.is_exhausted());
    }

    #[test]
    fn test_backoff_default() {
        let backoff = Backoff::default();
        assert_eq!(backoff.attempt(), 0);
    }

    #[test]
    fn test_backoff_next_delay_exponential() {
        let mut backoff = Backoff::new();

        // First attempt: 2^0 * base = base
        let delay1 = backoff.next_delay();
        assert_eq!(delay1, Duration::from_millis(BACKOFF_BASE_MS));
        assert_eq!(backoff.attempt(), 1);

        // Second attempt: 2^1 * base
        let delay2 = backoff.next_delay();
        assert_eq!(delay2, Duration::from_millis(BACKOFF_BASE_MS * 2));
        assert_eq!(backoff.attempt(), 2);

        // Third attempt: 2^2 * base
        let delay3 = backoff.next_delay();
        assert_eq!(delay3, Duration::from_millis(BACKOFF_BASE_MS * 4));
        assert_eq!(backoff.attempt(), 3);

        // Fourth attempt: 2^3 * base
        let delay4 = backoff.next_delay();
        assert_eq!(delay4, Duration::from_millis(BACKOFF_BASE_MS * 8));
        assert_eq!(backoff.attempt(), 4);

        // Further attempts should cap at exponent 3
        let delay5 = backoff.next_delay();
        assert_eq!(delay5, Duration::from_millis(BACKOFF_BASE_MS * 8));
    }

    #[test]
    fn test_backoff_is_exhausted() {
        let mut backoff = Backoff::new();

        for _ in 0..MAX_RECONNECT_ATTEMPTS {
            assert!(!backoff.is_exhausted());
            backoff.next_delay();
        }

        assert!(backoff.is_exhausted());
    }

    #[test]
    fn test_backoff_reset() {
        let mut backoff = Backoff::new();

        backoff.next_delay();
        backoff.next_delay();
        assert_eq!(backoff.attempt(), 2);

        backoff.reset();
        assert_eq!(backoff.attempt(), 0);
        assert!(!backoff.is_exhausted());

        // Verify delay starts from beginning after reset
        let delay = backoff.next_delay();
        assert_eq!(delay, Duration::from_millis(BACKOFF_BASE_MS));
    }

    #[test]
    fn test_backoff_clone() {
        let mut backoff = Backoff::new();
        backoff.next_delay();

        let cloned = backoff.clone();
        assert_eq!(cloned.attempt(), backoff.attempt());
    }

    #[test]
    fn test_backoff_edge_cases() {
        let mut backoff = Backoff::new();

        // Exhaust all attempts
        for _ in 0..100 {
            backoff.next_delay();
        }

        assert!(backoff.is_exhausted());
        assert!(backoff.attempt() > MAX_RECONNECT_ATTEMPTS);
    }
}
