// Copyright (c) 2026 nikcodex (KizunaLink)
// Licensed under the MIT License

//! Safe Opus audio decoder wrapper.
//!
//! Provides the [`Decoder`] struct which wraps a native C `OpusDecoder` pointer,
//! offering safe RAII resource management, packet loss concealment (PLC),
//! and in-band forward error correction (FEC) decoding for both 16-bit integer
//! and floating-point linear PCM samples.

use super::error::*;
use super::ffi;
use super::types::*;

/// CTL request code for retrieving the duration of the last decoded packet.
const OPUS_GET_LAST_PACKET_DURATION_REQUEST: i32 = 4039;

/// A safe, RAII-managed Opus audio decoder.
///
/// Decodes compressed Opus packets into raw PCM audio (either 16-bit signed integer
/// or 32-bit floating-point). Wrapping the C `OpusDecoder` state, it supports
/// forward error correction (FEC) decoding and packet loss concealment (PLC)
/// when incoming packets are dropped.
pub struct Decoder {
    ptr: *mut ffi::OpusDecoder,
    channels: Channels,
}

// Safety: An Opus decoder state can be safely transferred across threads
// when not actively in concurrent use.
unsafe impl Send for Decoder {}

impl Decoder {
    /// Creates and initializes a new Opus decoder with the given configuration.
    ///
    /// # Arguments
    ///
    /// * `sample_rate` - The output sampling rate in Hz (8 kHz, 12 kHz, 16 kHz, 24 kHz, or 48 kHz).
    /// * `channels` - Channel layout (Mono or Stereo).
    ///
    /// # Errors
    ///
    /// Returns [`Error::Opus`] if libopus fails to initialize the decoder, or
    /// [`Error::DecoderCreateFailed`] if the returned pointer is null.
    pub fn new(sample_rate: SampleRate, channels: Channels) -> Result<Self> {
        let mut error: i32 = 0;
        let ptr =
            unsafe { ffi::opus_decoder_create(sample_rate.raw(), channels.raw(), &mut error) };

        check_opus_error(error)?;

        if ptr.is_null() {
            return Err(Error::DecoderCreateFailed);
        }

        Ok(Self { ptr, channels })
    }

    /// Decodes an Opus packet into 16-bit linear PCM audio.
    ///
    /// If `packet` is `None`, packet loss concealment (PLC) is performed using a null
    /// pointer and length 0.
    ///
    /// # Arguments
    ///
    /// * `packet` - Optional compressed Opus packet bytes. Pass `None` to indicate a lost packet.
    /// * `output` - Destination buffer for interleaved 16-bit signed PCM samples.
    /// * `fec` - If `true`, request that in-band forward error correction data be decoded.
    ///
    /// # Returns
    ///
    /// Returns the number of decoded samples per channel written to `output`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Opus`] if decoding fails or arguments are invalid.
    pub fn decode(
        &mut self,
        packet: Option<&[u8]>,
        output: &mut [i16],
        fec: bool,
    ) -> Result<usize> {
        let (data_ptr, data_len) = match packet {
            Some(data) => {
                let len =
                    i32::try_from(data.len()).map_err(|_| Error::Opus(ErrorCode::BadArgument))?;
                (data.as_ptr(), len)
            }
            None => (std::ptr::null(), 0),
        };

        let frame_size = i32::try_from(output.len() / self.channels.count())
            .map_err(|_| Error::Opus(ErrorCode::BadArgument))?;
        let fec_flag = if fec { 1 } else { 0 };

        let ret = unsafe {
            ffi::opus_decode(
                self.ptr,
                data_ptr,
                data_len,
                output.as_mut_ptr(),
                frame_size,
                fec_flag,
            )
        };

        let samples = check_opus_error(ret)?;
        Ok(samples as usize)
    }

    /// Decodes an Opus packet into floating-point linear PCM audio.
    ///
    /// If `packet` is `None`, packet loss concealment (PLC) is performed using a null
    /// pointer and length 0.
    ///
    /// # Arguments
    ///
    /// * `packet` - Optional compressed Opus packet bytes. Pass `None` to indicate a lost packet.
    /// * `output` - Destination buffer for interleaved 32-bit float PCM samples (`[-1.0, 1.0]`).
    /// * `fec` - If `true`, request that in-band forward error correction data be decoded.
    ///
    /// # Returns
    ///
    /// Returns the number of decoded samples per channel written to `output`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Opus`] if decoding fails or arguments are invalid.
    pub fn decode_float(
        &mut self,
        packet: Option<&[u8]>,
        output: &mut [f32],
        fec: bool,
    ) -> Result<usize> {
        let (data_ptr, data_len) = match packet {
            Some(data) => {
                let len =
                    i32::try_from(data.len()).map_err(|_| Error::Opus(ErrorCode::BadArgument))?;
                (data.as_ptr(), len)
            }
            None => (std::ptr::null(), 0),
        };

        let frame_size = i32::try_from(output.len() / self.channels.count())
            .map_err(|_| Error::Opus(ErrorCode::BadArgument))?;
        let fec_flag = if fec { 1 } else { 0 };

        let ret = unsafe {
            ffi::opus_decode_float(
                self.ptr,
                data_ptr,
                data_len,
                output.as_mut_ptr(),
                frame_size,
                fec_flag,
            )
        };

        let samples = check_opus_error(ret)?;
        Ok(samples as usize)
    }

    /// Resets the internal decoder state, clearing all historical predictors.
    pub fn reset_state(&mut self) -> Result<()> {
        let ret = unsafe { ffi::opus_decoder_ctl(self.ptr, ffi::OPUS_RESET_STATE) };
        check_opus_error(ret)?;
        Ok(())
    }

    /// Gets the sampling rate configured in the decoder.
    pub fn sample_rate(&self) -> Result<SampleRate> {
        let rate = self.decoder_ctl_get(ffi::OPUS_GET_SAMPLE_RATE_REQUEST)?;
        SampleRate::try_from(rate)
    }

    /// Returns the channel configuration of the decoder.
    #[must_use]
    pub fn channels(&self) -> Channels {
        self.channels
    }

    /// Calculates the number of samples per channel contained in an Opus packet.
    ///
    /// # Arguments
    ///
    /// * `packet` - Slice containing an Opus packet.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Opus`] if the packet is corrupted or invalid.
    pub fn nb_samples(&self, packet: &[u8]) -> Result<usize> {
        let len = i32::try_from(packet.len()).map_err(|_| Error::Opus(ErrorCode::BadArgument))?;

        let ret = unsafe {
            ffi::opus_decoder_get_nb_samples(
                self.ptr as *const ffi::OpusDecoder,
                packet.as_ptr(),
                len,
            )
        };

        let samples = check_opus_error(ret)?;
        Ok(samples as usize)
    }

    /// Gets the duration of the last decoded packet in samples per channel (CTL request 4039).
    pub fn last_packet_duration(&self) -> Result<u32> {
        let duration = self.decoder_ctl_get(OPUS_GET_LAST_PACKET_DURATION_REQUEST)?;
        Ok(duration as u32)
    }

    /// Helper for setting an integer CTL property on the decoder.
    #[allow(dead_code)]
    fn decoder_ctl_set(&self, request: i32, value: i32) -> Result<()> {
        let ret = unsafe { ffi::opus_decoder_ctl(self.ptr, request, value) };
        check_opus_error(ret)?;
        Ok(())
    }

    /// Helper for querying an integer CTL property from the decoder.
    fn decoder_ctl_get(&self, request: i32) -> Result<i32> {
        let mut val: i32 = 0;
        let val_ptr: *mut i32 = &mut val;
        let ret = unsafe { ffi::opus_decoder_ctl(self.ptr, request, val_ptr) };
        check_opus_error(ret)?;
        Ok(val)
    }
}

impl Drop for Decoder {
    fn drop(&mut self) {
        if !self.ptr.is_null() {
            unsafe {
                ffi::opus_decoder_destroy(self.ptr);
            }
            self.ptr = std::ptr::null_mut();
        }
    }
}
