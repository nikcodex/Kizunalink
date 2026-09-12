// Copyright (c) 2026 nikcodex (KizunaLink)
// Licensed under the MIT License

pub mod encoder;
pub mod standard;

pub use encoder::Encoder;
pub use standard::StandardEngine;

use crate::engine::frame::AudioFrame;

pub trait Engine: Send {
    fn push(&mut self, frame: AudioFrame) -> bool;
}

pub type BoxedEngine = Box<dyn Engine>;
