// Copyright (c) 2026 nikcodex (KizunaLink)
// Licensed under the MIT License

pub mod encoder;
pub mod standard;

pub use encoder::Encoder;
pub use standard::StandardEngine;

use crate::engine::frame::AudioFrame;

/// Transport seam between the decoder thread and the mixer pipeline.
///
/// B15: this trait intentionally survives even though `StandardEngine` is the only
/// production implementation — `AudioProcessor::with_engine` injects an arbitrary
/// `BoxedEngine`, which is what makes the decode loop unit-testable without a live
/// voice pipeline, and leaves room for alternative transports (e.g. a raw-file
/// sink). Removing the indirection would trade the test seam for one virtual call
/// per 20 ms frame, so the indirection stays on purpose.
pub trait Engine: Send {
    /// Forward one frame downstream. Returning `false` means the pipeline is gone
    /// and the caller should stop producing frames.
    fn push(&mut self, frame: AudioFrame) -> bool;
}

pub type BoxedEngine = Box<dyn Engine>;
