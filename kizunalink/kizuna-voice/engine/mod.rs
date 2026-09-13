// Copyright (c) 2026 nikcodex (KizunaLink)
// Licensed under the MIT License

//! # Engine — Audio Processing Pipeline
//!
//! This module implements the core audio pipeline:
//!
//! ```text
//! Source → Decode (Symphonia) → Resample → Filters → Mixer → Opus Encode → UDP Send
//! ```
//!
//! Key components:
//! - [`buffer`]: Ring buffers and a pooled byte allocator for zero-copy audio frames.
//! - [`codec`]: Opus encoder/decoder wrappers.
//! - [`constants`]: Shared audio constants (sample rate, frame size, etc.).
//! - [`demux`]: Container demuxers (WebM/Opus).
//! - [`effects`]: Audio effects (fade, crossfade, volume, tape stop).
//! - [`engine`]: The `StandardEngine` that drives encoding.
//! - [`filters`]: 24 DSP filters (EQ, reverb, karaoke, timescale, etc.).
//! - [`flow`]: Playback flow controller (pause/seek/volume ramps).
//! - [`mix`]: Multi-track mixer with i16 summation.
//! - [`resample`]: Linear, hermite, and sinc resamplers.
//! - [`source`]: Audio source abstraction (HTTP, segmented, local).

pub mod buffer;
pub mod codec;
pub mod constants;
pub mod demux;
pub mod effects;
pub mod engine;
pub mod error;
pub mod filters;
pub mod flow;
pub mod frame;
pub mod mix;
pub mod playback;
pub mod processor;
pub mod resample;
pub mod source;

pub use buffer::{BufferPool, PooledBuffer, RingBuffer, get_byte_pool};
pub use flow::FlowController;
pub use frame::AudioFrame;
pub use mix::{AudioMixer, Mixer};
