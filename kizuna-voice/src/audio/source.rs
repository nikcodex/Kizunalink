use super::packet::AudioFrame;
use crate::error::Result;
use async_trait::async_trait;
use std::time::Duration;

#[async_trait]
pub trait AudioSource: Send + Sync {
    /// Provide the next audio frame (usually representing 20ms of audio).
    /// Returns Ok(None) on End of Stream.
    async fn next_frame(&mut self) -> Result<Option<AudioFrame>>;

    /// Optional: the intrinsic length/duration of this source if known.
    fn duration(&self) -> Option<Duration> {
        None
    }

    /// Optional: seek to a specific position if the source supports it.
    async fn seek(&mut self, _position: Duration) -> Result<()> {
        Err(crate::error::Error::Connection("Seek not supported".into()))
    }
}
