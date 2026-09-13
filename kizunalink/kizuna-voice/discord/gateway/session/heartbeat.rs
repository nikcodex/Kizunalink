// Copyright (c) 2026 nikcodex (KizunaLink)
// Licensed under the MIT License

use std::sync::{
    Arc,
    atomic::{AtomicI64, AtomicU32, AtomicU64, Ordering},
};

use tokio::sync::mpsc::Sender;
use tokio_tungstenite::tungstenite::protocol::Message;
use tokio_util::sync::CancellationToken;
use tracing::warn;

use super::protocol::{GatewayPayload, OpCode};
use crate::common::utils::now_ms;

pub struct HeartbeatTracker {
    pub last_nonce: Arc<AtomicU64>,
    pub sent_at: Arc<AtomicU64>,
    pub missed_acks: Arc<AtomicU32>,
}

impl Default for HeartbeatTracker {
    fn default() -> Self {
        Self {
            last_nonce: Arc::new(AtomicU64::new(0)),
            sent_at: Arc::new(AtomicU64::new(0)),
            missed_acks: Arc::new(AtomicU32::new(0)),
        }
    }
}

impl HeartbeatTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Validates an acknowledgment against the most recently sent heartbeat.
    ///
    /// A match clears the missed-ack count and returns the round-trip time in
    /// milliseconds. A mismatched nonce leaves the count unchanged and returns
    /// `None`.
    pub fn validate_ack(&self, acked_nonce: u64) -> Option<u64> {
        // Acquire pairs with the Release stores in `spawn`, so the nonce written by
        // the heartbeat task is guaranteed visible to the WS read loop.
        let expected = self.last_nonce.load(Ordering::Acquire);
        if expected != acked_nonce {
            warn!("Heartbeat mismatch: sent={expected} got={acked_nonce}");
            return None;
        }
        // A valid ACK proves the connection is alive — reset the miss counter so
        // recovery after a single dropped heartbeat isn't punished forever.
        self.missed_acks.store(0, Ordering::Release);
        Some(now_ms().saturating_sub(self.sent_at.load(Ordering::Acquire)))
    }

    /// Spawns a task that sends heartbeats at `interval_ms` and tracks their
    /// acknowledgments.
    ///
    /// The task cancels `conn_token` after two consecutive heartbeats remain
    /// unacknowledged, and exits if the message receiver is dropped.
    pub fn spawn(
        &self,
        tx: Sender<Message>,
        seq_ack: Arc<AtomicI64>,
        conn_token: CancellationToken,
        interval_ms: u64,
    ) -> tokio::task::JoinHandle<()> {
        let last_nonce = self.last_nonce.clone();
        let sent_at = self.sent_at.clone();
        let missed_acks = self.missed_acks.clone();

        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(tokio::time::Duration::from_millis(interval_ms));
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

            loop {
                ticker.tick().await;

                let missed = missed_acks.fetch_add(1, Ordering::Relaxed);
                if missed >= 2 {
                    warn!("Heartbeat timeout: {missed} missed ACKs.");
                    conn_token.cancel();
                    break;
                }

                let nonce = now_ms();
                // Release so `validate_ack` (Acquire) on the read-loop task is
                // guaranteed to observe these values.
                last_nonce.store(nonce, Ordering::Release);
                sent_at.store(nonce, Ordering::Release);

                let hb = GatewayPayload {
                    op: OpCode::Heartbeat as u8,
                    seq: None,
                    d: serde_json::json!({
                        "t": nonce,
                        "seq_ack": seq_ack.load(Ordering::Relaxed)
                    }),
                };

                if let Ok(json) = serde_json::to_string(&hb)
                    && tx.try_send(Message::Text(json.into())).is_err()
                {
                    conn_token.cancel();
                    break;
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_heartbeat_tracker_new() {
        let tracker = HeartbeatTracker::new();
        assert_eq!(tracker.last_nonce.load(Ordering::Relaxed), 0);
        assert_eq!(tracker.sent_at.load(Ordering::Relaxed), 0);
        assert_eq!(tracker.missed_acks.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn test_heartbeat_tracker_default() {
        let tracker = HeartbeatTracker::default();
        assert_eq!(tracker.last_nonce.load(Ordering::Relaxed), 0);
        assert_eq!(tracker.sent_at.load(Ordering::Relaxed), 0);
        assert_eq!(tracker.missed_acks.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn test_validate_ack_matching() {
        let tracker = HeartbeatTracker::new();
        let nonce = 12345u64;

        tracker.last_nonce.store(nonce, Ordering::Relaxed);
        tracker.sent_at.store(now_ms(), Ordering::Relaxed);

        let result = tracker.validate_ack(nonce);
        assert!(result.is_some());

        let rtt = result.unwrap();
        assert!(rtt < 1000); // Should be very small for local test
    }

    #[test]
    fn test_validate_ack_mismatch() {
        let tracker = HeartbeatTracker::new();

        tracker.last_nonce.store(12345, Ordering::Relaxed);
        tracker.sent_at.store(now_ms(), Ordering::Relaxed);

        let result = tracker.validate_ack(99999);
        assert!(result.is_none());
    }

    #[test]
    fn test_validate_ack_zero_nonce() {
        let tracker = HeartbeatTracker::new();

        tracker.last_nonce.store(0, Ordering::Relaxed);
        tracker.sent_at.store(now_ms(), Ordering::Relaxed);

        let result = tracker.validate_ack(0);
        assert!(result.is_some());
    }

    #[test]
    fn test_validate_ack_rtt_calculation() {
        let tracker = HeartbeatTracker::new();
        let nonce = 12345u64;

        // Simulate a heartbeat sent 100ms ago
        let sent_time = now_ms().saturating_sub(100);
        tracker.last_nonce.store(nonce, Ordering::Relaxed);
        tracker.sent_at.store(sent_time, Ordering::Relaxed);

        let result = tracker.validate_ack(nonce);
        assert!(result.is_some());

        let rtt = result.unwrap();
        assert!(rtt >= 100);
        assert!(rtt < 200); // Allow some tolerance
    }

    #[test]
    fn test_missed_acks_counter() {
        let tracker = HeartbeatTracker::new();

        assert_eq!(tracker.missed_acks.load(Ordering::Relaxed), 0);

        tracker.missed_acks.fetch_add(1, Ordering::Relaxed);
        assert_eq!(tracker.missed_acks.load(Ordering::Relaxed), 1);

        tracker.missed_acks.fetch_add(1, Ordering::Relaxed);
        assert_eq!(tracker.missed_acks.load(Ordering::Relaxed), 2);

        tracker.missed_acks.store(0, Ordering::Relaxed);
        assert_eq!(tracker.missed_acks.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn test_atomic_updates() {
        let tracker = HeartbeatTracker::new();

        tracker.last_nonce.store(100, Ordering::Relaxed);
        tracker.sent_at.store(200, Ordering::Relaxed);
        tracker.missed_acks.store(3, Ordering::Relaxed);

        assert_eq!(tracker.last_nonce.load(Ordering::Relaxed), 100);
        assert_eq!(tracker.sent_at.load(Ordering::Relaxed), 200);
        assert_eq!(tracker.missed_acks.load(Ordering::Relaxed), 3);
    }

    #[test]
    fn test_validate_ack_resets_missed_acks() {
        let tracker = HeartbeatTracker::new();

        tracker.missed_acks.store(2, Ordering::Relaxed);
        tracker.last_nonce.store(42, Ordering::Relaxed);
        tracker.sent_at.store(now_ms(), Ordering::Relaxed);

        assert!(tracker.validate_ack(42).is_some());
        assert_eq!(tracker.missed_acks.load(Ordering::Relaxed), 0);

        // A mismatched ACK must not reset the counter.
        tracker.missed_acks.store(1, Ordering::Relaxed);
        assert!(tracker.validate_ack(43).is_none());
        assert_eq!(tracker.missed_acks.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn test_validate_ack_multiple_times() {
        let tracker = HeartbeatTracker::new();

        for i in 1..=5 {
            let nonce = i * 1000;
            tracker.last_nonce.store(nonce, Ordering::Relaxed);
            tracker.sent_at.store(now_ms(), Ordering::Relaxed);

            let result = tracker.validate_ack(nonce);
            assert!(result.is_some(), "Iteration {}", i);
        }
    }
}
