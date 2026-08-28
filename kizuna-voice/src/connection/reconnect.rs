use std::time::Duration;
use tokio::time::sleep;
use tracing::{info, warn};

pub struct ReconnectStrategy {
    pub max_retries: u32,
    pub base_delay: Duration,
}

impl Default for ReconnectStrategy {
    fn default() -> Self {
        Self {
            max_retries: 5,
            base_delay: Duration::from_millis(500),
        }
    }
}

impl ReconnectStrategy {
    pub async fn attempt_reconnect<F, Fut, T, E>(&self, mut action: F) -> Result<T, E>
    where
        F: FnMut() -> Fut,
        Fut: std::future::Future<Output = Result<T, E>>,
        E: std::fmt::Display,
    {
        let mut attempts = 0;
        let mut current_delay = self.base_delay;

        loop {
            match action().await {
                Ok(result) => {
                    if attempts > 0 {
                        info!("Successfully reconnected after {} attempts", attempts);
                    }
                    return Ok(result);
                }
                Err(e) => {
                    attempts += 1;
                    if attempts >= self.max_retries {
                        warn!(
                            "Max reconnect retries ({}) reached. Last error: {}",
                            self.max_retries, e
                        );
                        return Err(e);
                    }
                    warn!(
                        "Reconnect attempt {} failed: {}. Retrying in {:?}",
                        attempts, e, current_delay
                    );
                    sleep(current_delay).await;
                    current_delay *= 2; // Exponential backoff
                }
            }
        }
    }
}
