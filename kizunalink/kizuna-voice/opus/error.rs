// Copyright (c) 2026 nikcodex (KizunaLink)
// Licensed under the MIT License

//! Error types and error-handling utilities for the Opus codec.
//!
//! Provides the raw [`ErrorCode`] returned by C `libopus`, the unified [`Error`] enum
//! covering both codec and configuration failures, and the [`check_opus_error`] helper.

use std::fmt;
use super::ffi;

/// Standard Opus C library error codes.
#[repr(i32)]
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum ErrorCode {
    /// One or more invalid/out of range arguments (`OPUS_BAD_ARG`).
    BadArgument = -1,
    /// The buffer provided is too small (`OPUS_BUFFER_TOO_SMALL`).
    BufferTooSmall = -2,
    /// An internal error was detected (`OPUS_INTERNAL_ERROR`).
    InternalError = -3,
    /// The compressed data passed is corrupted (`OPUS_INVALID_PACKET`).
    InvalidPacket = -4,
    /// Invalid/unsupported request number (`OPUS_UNIMPLEMENTED`).
    Unimplemented = -5,
    /// An encoder or decoder structure is invalid or already freed (`OPUS_INVALID_STATE`).
    InvalidState = -6,
    /// Memory allocation has failed (`OPUS_ALLOC_FAIL`).
    AllocFail = -7,
}

impl ErrorCode {
    /// Returns the raw integer error code.
    #[must_use]
    pub const fn as_i32(self) -> i32 {
        self as i32
    }

    /// Returns a human-readable description of the error code.
    #[must_use]
    pub const fn description(self) -> &'static str {
        match self {
            Self::BadArgument => "one or more invalid/out of range arguments (OPUS_BAD_ARG)",
            Self::BufferTooSmall => "the buffer provided is too small (OPUS_BUFFER_TOO_SMALL)",
            Self::InternalError => "an internal error was detected (OPUS_INTERNAL_ERROR)",
            Self::InvalidPacket => "the compressed data passed is corrupted (OPUS_INVALID_PACKET)",
            Self::Unimplemented => "invalid/unsupported request number (OPUS_UNIMPLEMENTED)",
            Self::InvalidState => {
                "an encoder or decoder structure is invalid or already freed (OPUS_INVALID_STATE)"
            }
            Self::AllocFail => "memory allocation has failed (OPUS_ALLOC_FAIL)",
        }
    }
}

impl TryFrom<i32> for ErrorCode {
    type Error = i32;

    fn try_from(code: i32) -> std::result::Result<Self, Self::Error> {
        match code {
            -1 => Ok(Self::BadArgument),
            -2 => Ok(Self::BufferTooSmall),
            -3 => Ok(Self::InternalError),
            -4 => Ok(Self::InvalidPacket),
            -5 => Ok(Self::Unimplemented),
            -6 => Ok(Self::InvalidState),
            -7 => Ok(Self::AllocFail),
            other => Err(other),
        }
    }
}

impl From<ErrorCode> for i32 {
    fn from(code: ErrorCode) -> Self {
        code as Self
    }
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.description())
    }
}

impl std::error::Error for ErrorCode {}

/// Unified error type for Opus encoding, decoding, and parameter validation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// An error returned directly by the underlying C `libopus`.
    Opus(ErrorCode),
    /// The specified sample rate is not supported by Opus.
    InvalidSampleRate(i32),
    /// The specified channel count is invalid (only mono and stereo are supported).
    InvalidChannels(i32),
    /// The specified application mode is invalid.
    InvalidApplication(i32),
    /// The specified signal type is invalid.
    InvalidSignal(i32),
    /// The specified audio bandwidth is invalid.
    InvalidBandwidth(i32),
    /// The specified bitrate is invalid.
    InvalidBitrate(i32),
    /// Failed to allocate or initialize the Opus encoder.
    EncoderCreateFailed,
    /// Failed to allocate or initialize the Opus decoder.
    DecoderCreateFailed,
    /// A null pointer was encountered during an FFI operation.
    NullPointer,
}

impl From<ErrorCode> for Error {
    fn from(code: ErrorCode) -> Self {
        Self::Opus(code)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Opus(code) => write!(f, "Opus error: {code}"),
            Self::InvalidSampleRate(rate) => write!(
                f,
                "invalid sample rate: {rate} Hz (expected 8000, 12000, 16000, 24000, or 48000)"
            ),
            Self::InvalidChannels(channels) => {
                write!(f, "invalid channel count: {channels} (expected 1 or 2)")
            }
            Self::InvalidApplication(app) => {
                write!(f, "invalid application identifier: {app}")
            }
            Self::InvalidSignal(sig) => {
                write!(f, "invalid signal type identifier: {sig}")
            }
            Self::InvalidBandwidth(bw) => {
                write!(f, "invalid bandwidth identifier: {bw}")
            }
            Self::InvalidBitrate(br) => {
                write!(f, "invalid bitrate: {br}")
            }
            Self::EncoderCreateFailed => {
                write!(f, "failed to create Opus encoder")
            }
            Self::DecoderCreateFailed => {
                write!(f, "failed to create Opus decoder")
            }
            Self::NullPointer => {
                write!(f, "null pointer encountered in Opus operation")
            }
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Opus(code) => Some(code),
            _ => None,
        }
    }
}

/// A specialized [`Result`](std::result::Result) type for Opus operations.
pub type Result<T> = std::result::Result<T, Error>;

/// Validates an Opus C API return code.
///
/// Returns `Ok(code)` if `code >= 0` (indicating success or a non-negative byte/sample count),
/// or `Err(Error::Opus(ErrorCode))` if `code < 0`.
/// If the negative error code is unrecognized, it falls back to [`ErrorCode::InternalError`].
pub fn check_opus_error(code: i32) -> Result<i32> {
    if code >= 0 {
        Ok(code)
    } else {
        let err_code = ErrorCode::try_from(code).unwrap_or(ErrorCode::InternalError);
        Err(Error::Opus(err_code))
    }
}
