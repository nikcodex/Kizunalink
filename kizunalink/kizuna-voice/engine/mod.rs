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

/// Opt out of denormal (subnormal) float processing for this thread and every thread it
/// spawns afterwards (Linux `clone` copies the FPU control state to child threads).
///
/// Denormals take the x87/SSE microcode slow path, so in steady-state DSP loops (mix,
/// resample, filters) a handful of subnormal samples can stall whole frames. FTZ flushes
/// subnormal *outputs* to zero and DAZ treats subnormal *inputs* as zero (P02). Call this
/// from `main` *before* the tokio runtime is constructed so every worker inherits it.
///
/// On non-x86 targets this is a documented no-op: AArch64 needs `FPCR.FZ` via `mrs/msr`,
/// which we currently only ship to x86 where the slow-path penalty is measured.
pub fn disable_denormals() {
    // MXCSR: Flush-To-Zero (bit 15) | Denormals-Are-Zero (bit 6).
    #[cfg(target_arch = "x86_64")]
    unsafe {
        let mut csr: u32 = 0;
        core::arch::asm!(
            "stmxcsr [{0}]",
            in(reg) &mut csr,
            options(nostack)
        );
        csr |= 0x8040;
        core::arch::asm!(
            "ldmxcsr [{0}]",
            in(reg) &csr,
            options(readonly, nostack)
        );
    }
    #[cfg(target_arch = "x86")]
    unsafe {
        let mut csr: u32 = 0;
        core::arch::asm!(
            "stmxcsr [{0}]",
            in(reg) &mut csr,
            options(nostack)
        );
        csr |= 0x8040;
        core::arch::asm!(
            "ldmxcsr [{0}]",
            in(reg) &csr,
            options(readonly, nostack)
        );
    }
}

pub mod buffer;
pub mod codec;
pub mod constants;
pub mod demux;
pub mod effects;
// The public `engine::engine` path predates the lint and is part of the crate API.
#[allow(clippy::module_inception)]
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
