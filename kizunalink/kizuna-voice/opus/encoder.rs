// Copyright (c) 2026 nikcodex (KizunaLink)
// Licensed under the MIT License

//! Safe Opus audio encoder wrapper.
//!
//! Provides the [`Encoder`] struct which wraps a native C `OpusEncoder` pointer,
//! offering safe RAII resource management and type-safe audio encoding for both
//! 16-bit integer and floating-point linear PCM samples.

use super::error::*;
use super::ffi;
use super::types::*;

/// A safe, RAII-managed Opus audio encoder.
///
/// Encodes raw PCM audio (either 16-bit signed integer or 32-bit floating-point)
/// into compressed Opus packets. Wrapping the C `OpusEncoder` state, it provides
/// comprehensive CTL configuration methods for adjusting bitrate, bandwidth,
/// complexity, forward error correction (FEC), discontinuous transmission (DTX),
/// and other encoder parameters.
pub struct Encoder {
    ptr: *mut ffi::OpusEncoder,
    channels: Channels,
}

// Safety: An Opus encoder state can be safely transferred across threads
// when not actively in concurrent use.
unsafe impl Send for Encoder {}

impl Encoder {
    /// Creates and initializes a new Opus encoder with the given configuration.
    ///
    /// # Arguments
    ///
    /// * `sample_rate` - The input sampling rate in Hz (8 kHz, 12 kHz, 16 kHz, 24 kHz, or 48 kHz).
    /// * `channels` - Channel layout (Mono or Stereo).
    /// * `application` - Target coding mode (`Voip`, `Audio`, or `LowDelay`).
    ///
    /// # Errors
    ///
    /// Returns [`Error::Opus`] if libopus fails to initialize the encoder, or
    /// [`Error::EncoderCreateFailed`] if the returned pointer is null.
    pub fn new(
        sample_rate: SampleRate,
        channels: Channels,
        application: Application,
    ) -> Result<Self> {
        let mut error: i32 = 0;
        let ptr = unsafe {
            ffi::opus_encoder_create(
                sample_rate.raw(),
                channels.raw(),
                application.raw(),
                &mut error,
            )
        };

        check_opus_error(error)?;

        if ptr.is_null() {
            return Err(Error::EncoderCreateFailed);
        }

        Ok(Self { ptr, channels })
    }

    /// Encodes an interleaved 16-bit linear PCM audio frame into an Opus packet.
    ///
    /// The frame length in samples per channel is calculated as `pcm.len() / channels.count()`.
    ///
    /// # Arguments
    ///
    /// * `pcm` - Slice of interleaved 16-bit signed PCM audio samples.
    /// * `output` - Output buffer where the compressed Opus packet will be written.
    ///
    /// # Returns
    ///
    /// Returns the number of bytes written to `output` on success.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Opus`] if encoding fails or arguments are invalid.
    pub fn encode(&self, pcm: &[i16], output: &mut [u8]) -> Result<usize> {
        let frame_size = i32::try_from(pcm.len() / self.channels.count())
            .map_err(|_| Error::Opus(ErrorCode::BadArgument))?;
        let max_data_bytes = i32::try_from(output.len())
            .map_err(|_| Error::Opus(ErrorCode::BadArgument))?;

        let ret = unsafe {
            ffi::opus_encode(
                self.ptr,
                pcm.as_ptr(),
                frame_size,
                output.as_mut_ptr(),
                max_data_bytes,
            )
        };

        let written = check_opus_error(ret)?;
        Ok(written as usize)
    }

    /// Encodes an interleaved floating-point linear PCM audio frame into an Opus packet.
    ///
    /// The frame length in samples per channel is calculated as `pcm.len() / channels.count()`.
    ///
    /// # Arguments
    ///
    /// * `pcm` - Slice of interleaved 32-bit float PCM audio samples (normalized to `[-1.0, 1.0]`).
    /// * `output` - Output buffer where the compressed Opus packet will be written.
    ///
    /// # Returns
    ///
    /// Returns the number of bytes written to `output` on success.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Opus`] if encoding fails or arguments are invalid.
    pub fn encode_float(&self, pcm: &[f32], output: &mut [u8]) -> Result<usize> {
        let frame_size = i32::try_from(pcm.len() / self.channels.count())
            .map_err(|_| Error::Opus(ErrorCode::BadArgument))?;
        let max_data_bytes = i32::try_from(output.len())
            .map_err(|_| Error::Opus(ErrorCode::BadArgument))?;

        let ret = unsafe {
            ffi::opus_encode_float(
                self.ptr,
                pcm.as_ptr(),
                frame_size,
                output.as_mut_ptr(),
                max_data_bytes,
            )
        };

        let written = check_opus_error(ret)?;
        Ok(written as usize)
    }

    /// Configures the target bitrate for the encoder.
    ///
    /// # Arguments
    ///
    /// * `bitrate` - Target [`Bitrate`] setting (`Auto`, `Max`, or explicit bits per second).
    pub fn set_bitrate(&self, bitrate: Bitrate) -> Result<()> {
        self.encoder_ctl_set(ffi::OPUS_SET_BITRATE_REQUEST, i32::from(bitrate))
    }

    /// Gets the current target bitrate of the encoder in bits per second.
    pub fn bitrate(&self) -> Result<i32> {
        self.encoder_ctl_get(ffi::OPUS_GET_BITRATE_REQUEST)
    }

    /// Configures the computational complexity of the encoder.
    ///
    /// # Arguments
    ///
    /// * `complexity` - Value in the range `0..=10`, where 0 is lowest CPU usage and 10 is highest audio quality.
    ///
    /// # Panics
    ///
    /// Panics if `complexity > 10`.
    pub fn set_complexity(&self, complexity: u8) -> Result<()> {
        assert!(
            complexity <= 10,
            "Opus complexity must be between 0 and 10 (got {complexity})"
        );
        self.encoder_ctl_set(ffi::OPUS_SET_COMPLEXITY_REQUEST, i32::from(complexity))
    }

    /// Configures the signal type hint (voice or music).
    ///
    /// # Arguments
    ///
    /// * `signal` - Signal hint (`Auto`, `Voice`, or `Music`).
    pub fn set_signal(&self, signal: Signal) -> Result<()> {
        self.encoder_ctl_set(ffi::OPUS_SET_SIGNAL_REQUEST, signal.raw())
    }

    /// Enables or disables in-band forward error correction (FEC).
    ///
    /// FEC allows the decoder to recover lost packets when packet loss occurs.
    pub fn set_inband_fec(&self, enabled: bool) -> Result<()> {
        self.encoder_ctl_set(ffi::OPUS_SET_INBAND_FEC_REQUEST, if enabled { 1 } else { 0 })
    }

    /// Configures the expected packet loss percentage for FEC optimization.
    ///
    /// # Arguments
    ///
    /// * `percentage` - Expected loss in the range `0..=100`.
    ///
    /// # Panics
    ///
    /// Panics if `percentage > 100`.
    pub fn set_packet_loss_perc(&self, percentage: u8) -> Result<()> {
        assert!(
            percentage <= 100,
            "Packet loss percentage must be between 0 and 100 (got {percentage})"
        );
        self.encoder_ctl_set(
            ffi::OPUS_SET_PACKET_LOSS_PERC_REQUEST,
            i32::from(percentage),
        )
    }

    /// Enables or disables variable bitrate (VBR) encoding.
    pub fn set_vbr(&self, enabled: bool) -> Result<()> {
        self.encoder_ctl_set(ffi::OPUS_SET_VBR_REQUEST, if enabled { 1 } else { 0 })
    }

    /// Configures the maximum audio bandwidth of the encoder.
    ///
    /// # Arguments
    ///
    /// * `bandwidth` - Maximum audio passband limit.
    pub fn set_bandwidth(&self, bandwidth: Bandwidth) -> Result<()> {
        self.encoder_ctl_set(ffi::OPUS_SET_BANDWIDTH_REQUEST, bandwidth.raw())
    }

    /// Enables or disables discontinuous transmission (DTX).
    ///
    /// When DTX is enabled, the encoder reduces output during silence.
    pub fn set_dtx(&self, enabled: bool) -> Result<()> {
        self.encoder_ctl_set(ffi::OPUS_SET_DTX_REQUEST, if enabled { 1 } else { 0 })
    }

    /// Configures the application coding mode.
    ///
    /// # Arguments
    ///
    /// * `app` - Target application mode (`Voip`, `Audio`, or `LowDelay`).
    pub fn set_application(&self, app: Application) -> Result<()> {
        self.encoder_ctl_set(ffi::OPUS_SET_APPLICATION_REQUEST, app.raw())
    }

    /// Resets the internal encoder state, clearing all historical predictors.
    pub fn reset_state(&mut self) -> Result<()> {
        let ret = unsafe { ffi::opus_encoder_ctl(self.ptr, ffi::OPUS_RESET_STATE) };
        check_opus_error(ret)?;
        Ok(())
    }

    /// Gets the total lookahead latency of the encoder in samples.
    pub fn lookahead(&self) -> Result<u32> {
        let val = self.encoder_ctl_get(ffi::OPUS_GET_LOOKAHEAD_REQUEST)?;
        Ok(val as u32)
    }

    /// Gets the sampling rate configured in the encoder.
    pub fn sample_rate(&self) -> Result<SampleRate> {
        let rate = self.encoder_ctl_get(ffi::OPUS_GET_SAMPLE_RATE_REQUEST)?;
        SampleRate::try_from(rate)
    }

    /// Returns the channel configuration of the encoder.
    #[must_use]
    pub fn channels(&self) -> Channels {
        self.channels
    }

    /// Helper for setting an integer CTL property on the encoder.
    fn encoder_ctl_set(&self, request: i32, value: i32) -> Result<()> {
        let ret = unsafe { ffi::opus_encoder_ctl(self.ptr, request, value) };
        check_opus_error(ret)?;
        Ok(())
    }

    /// Helper for querying an integer CTL property from the encoder.
    fn encoder_ctl_get(&self, request: i32) -> Result<i32> {
        let mut val: i32 = 0;
        let val_ptr: *mut i32 = &mut val;
        let ret = unsafe { ffi::opus_encoder_ctl(self.ptr, request, val_ptr) };
        check_opus_error(ret)?;
        Ok(val)
    }
}

impl Drop for Encoder {
    fn drop(&mut self) {
        if !self.ptr.is_null() {
            unsafe {
                ffi::opus_encoder_destroy(self.ptr);
            }
            self.ptr = std::ptr::null_mut();
        }
    }
}
