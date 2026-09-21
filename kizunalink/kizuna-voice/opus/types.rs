// Copyright (c) 2026 nikcodex (KizunaLink)
// Licensed under the MIT License

//! Type-safe Rust enumerations for Opus codec configuration.
//!
//! Provides strongly typed representations for sample rates, channels,
//! application profiles, signal hints, bandwidths, and bitrates with
//! conversions to and from raw C libopus representation.

use super::error::Error;
use super::ffi;

/// Supported Opus sampling rates in Hertz.
#[repr(i32)]
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum SampleRate {
    /// 8 kHz narrowband.
    Hz8000 = 8000,
    /// 12 kHz mediumband.
    Hz12000 = 12000,
    /// 16 kHz wideband.
    Hz16000 = 16000,
    /// 24 kHz superwideband.
    Hz24000 = 24000,
    /// 48 kHz fullband.
    Hz48000 = 48000,
}

impl SampleRate {
    /// Returns the sampling rate in Hertz as an `i32`.
    #[must_use]
    pub const fn raw(self) -> i32 {
        self as i32
    }
}

impl TryFrom<i32> for SampleRate {
    type Error = Error;

    fn try_from(value: i32) -> Result<Self, Self::Error> {
        match value {
            8000 => Ok(Self::Hz8000),
            12000 => Ok(Self::Hz12000),
            16000 => Ok(Self::Hz16000),
            24000 => Ok(Self::Hz24000),
            48000 => Ok(Self::Hz48000),
            other => Err(Error::InvalidSampleRate(other)),
        }
    }
}

impl From<SampleRate> for i32 {
    fn from(rate: SampleRate) -> Self {
        rate as Self
    }
}

/// Supported Opus channel configurations.
#[repr(i32)]
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum Channels {
    /// Single-channel mono.
    Mono = 1,
    /// Two-channel stereo.
    Stereo = 2,
}

impl Channels {
    /// Returns the channel count as a `usize` (1 for Mono, 2 for Stereo).
    #[must_use]
    pub fn count(&self) -> usize {
        match self {
            Self::Mono => 1,
            Self::Stereo => 2,
        }
    }

    /// Returns the raw channel count as an `i32`.
    #[must_use]
    pub const fn raw(self) -> i32 {
        self as i32
    }
}

impl TryFrom<i32> for Channels {
    type Error = Error;

    fn try_from(value: i32) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::Mono),
            2 => Ok(Self::Stereo),
            other => Err(Error::InvalidChannels(other)),
        }
    }
}

impl From<Channels> for i32 {
    fn from(channels: Channels) -> Self {
        channels as Self
    }
}

/// Opus coding application mode.
#[repr(i32)]
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum Application {
    /// Best for most VoIP/videoconference applications where listening quality and intelligibility matter most.
    Voip = 2048,
    /// Best for broadcast/high-fidelity audio applications where the input is considered to be music/audio.
    Audio = 2049,
    /// Only use when lowest-achievable latency is what matters most.
    LowDelay = 2051,
}

impl Application {
    /// Returns the raw application code as an `i32`.
    #[must_use]
    pub const fn raw(self) -> i32 {
        self as i32
    }
}

impl TryFrom<i32> for Application {
    type Error = Error;

    fn try_from(value: i32) -> Result<Self, Self::Error> {
        match value {
            2048 => Ok(Self::Voip),
            2049 => Ok(Self::Audio),
            2051 => Ok(Self::LowDelay),
            other => Err(Error::InvalidApplication(other)),
        }
    }
}

impl From<Application> for i32 {
    fn from(app: Application) -> Self {
        app as Self
    }
}

/// Signal type hint for the encoder.
#[repr(i32)]
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum Signal {
    /// Auto-detect signal type.
    Auto = -1000,
    /// Voice speech signal.
    Voice = 3001,
    /// Music audio signal.
    Music = 3002,
}

impl Signal {
    /// Returns the raw signal code as an `i32`.
    #[must_use]
    pub const fn raw(self) -> i32 {
        self as i32
    }
}

impl TryFrom<i32> for Signal {
    type Error = Error;

    fn try_from(value: i32) -> Result<Self, Self::Error> {
        match value {
            -1000 => Ok(Self::Auto),
            3001 => Ok(Self::Voice),
            3002 => Ok(Self::Music),
            other => Err(Error::InvalidSignal(other)),
        }
    }
}

impl From<Signal> for i32 {
    fn from(signal: Signal) -> Self {
        signal as Self
    }
}

/// Audio bandwidth limit configuration.
#[repr(i32)]
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum Bandwidth {
    /// Auto-select bandwidth.
    Auto = -1000,
    /// 4 kHz audio passband (narrowband).
    Narrowband = 1101,
    /// 6 kHz audio passband (mediumband).
    Mediumband = 1102,
    /// 8 kHz audio passband (wideband).
    Wideband = 1103,
    /// 12 kHz audio passband (superwideband).
    Superwideband = 1104,
    /// 20 kHz audio passband (fullband).
    Fullband = 1105,
}

impl Bandwidth {
    /// Returns the raw bandwidth code as an `i32`.
    #[must_use]
    pub const fn raw(self) -> i32 {
        self as i32
    }
}

impl TryFrom<i32> for Bandwidth {
    type Error = Error;

    fn try_from(value: i32) -> Result<Self, Self::Error> {
        match value {
            -1000 => Ok(Self::Auto),
            1101 => Ok(Self::Narrowband),
            1102 => Ok(Self::Mediumband),
            1103 => Ok(Self::Wideband),
            1104 => Ok(Self::Superwideband),
            1105 => Ok(Self::Fullband),
            other => Err(Error::InvalidBandwidth(other)),
        }
    }
}

impl From<Bandwidth> for i32 {
    fn from(bandwidth: Bandwidth) -> Self {
        bandwidth as Self
    }
}

/// Target bitrate setting for the Opus encoder.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum Bitrate {
    /// Automatic bitrate selection based on signal and sample rate.
    Auto,
    /// Maximum allowable bitrate.
    Max,
    /// Target bitrate in bits per second (e.g. 64000 for 64 kbps).
    BitsPerSecond(i32),
}

impl From<Bitrate> for i32 {
    fn from(bitrate: Bitrate) -> Self {
        match bitrate {
            Bitrate::Auto => ffi::OPUS_AUTO,
            Bitrate::Max => ffi::OPUS_BITRATE_MAX,
            Bitrate::BitsPerSecond(bps) => bps,
        }
    }
}

impl TryFrom<i32> for Bitrate {
    type Error = Error;

    fn try_from(value: i32) -> Result<Self, Self::Error> {
        match value {
            ffi::OPUS_AUTO => Ok(Self::Auto),
            ffi::OPUS_BITRATE_MAX => Ok(Self::Max),
            bps if bps > 0 => Ok(Self::BitsPerSecond(bps)),
            other => Err(Error::InvalidBitrate(other)),
        }
    }
}
