// Copyright (c) 2026 nikcodex (KizunaLink)
// Licensed under the MIT License

use flume::Sender;
use tracing::warn;

use super::Engine;
use crate::engine::AudioFrame;

/// Default [`Engine`] that forwards decoded frames into the mixer pipeline.
///
/// The frame channel is bounded (backpressure by design): `send` blocks while
/// the mixer lags instead of unbounded-queuing latency. A `send` failure can
/// only mean the receiving half is gone, so that case is logged and surfaced
/// as a `false` return, which the `AudioProcessor` uses to end its loop
/// cleanly rather than silently draining frames into a dead channel (B14).
pub struct StandardEngine {
    frame_tx: Sender<AudioFrame>,
}

impl StandardEngine {
    pub fn new(frame_tx: Sender<AudioFrame>) -> Self {
        Self { frame_tx }
    }
}

impl Engine for StandardEngine {
    fn push(&mut self, frame: AudioFrame) -> bool {
        match self.frame_tx.send(frame) {
            Ok(()) => true,
            Err(e) => {
                warn!("AudioEngine: mixer channel closed while pushing frame ({e}); stopping pipeline");
                false
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_reports_disconnect_as_failure() {
        let (tx, rx) = flume::bounded(2);
        let mut engine = StandardEngine::new(tx);
        drop(rx);
        assert!(
            !engine.push(AudioFrame::Pcm(vec![0i16; 8])),
            "push to a dropped channel must return false"
        );
    }

    #[test]
    fn push_delivers_frames() {
        let (tx, rx) = flume::bounded(2);
        let mut engine = StandardEngine::new(tx);
        assert!(engine.push(AudioFrame::Pcm(vec![1i16, 2, 3])));
        match rx.recv().expect("frame should arrive") {
            AudioFrame::Pcm(v) => assert_eq!(v, vec![1, 2, 3]),
            AudioFrame::Opus(_) => panic!("expected Pcm frame"),
        }
    }
}
