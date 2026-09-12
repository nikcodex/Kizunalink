// Copyright (c) 2026 nikcodex (KizunaLink)
// Licensed under the MIT License

//! Opus interactive audio codec module.
//!
//! Provides safe, high-level Rust wrappers and raw FFI bindings around C `libopus`, featuring:
//! - [`Encoder`]: Safe Opus audio encoder with comprehensive CTL controls.
//! - [`Decoder`]: Safe Opus audio decoder supporting packet loss concealment (PLC) and FEC.
//! - [`types`]: Strongly typed enums for sample rates, channels, bitrates, bandwidths, and applications.
//! - [`error`]: Unified error reporting mapping directly to Opus return codes.
//! - [`ffi`]: Low-level foreign function declarations and Opus C constants.

pub mod decoder;
pub mod encoder;
pub mod error;
pub mod ffi;
pub mod types;

pub use decoder::Decoder;
pub use encoder::Encoder;
pub use error::{Error, ErrorCode, Result};
pub use types::*;
