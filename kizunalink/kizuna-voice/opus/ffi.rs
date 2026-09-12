// Copyright (c) 2026 nikcodex (KizunaLink)
// Licensed under the MIT License

//! Raw FFI bindings to C `libopus`.
//!
//! This module provides direct foreign function interface declarations and
//! C constants for the reference Opus codec implementation (`opus.h` and `opus_defines.h`).

use std::os::raw::c_char;

/// Opaque structure representing an Opus encoder state.
#[repr(C)]
pub struct OpusEncoder {
    _private: [u8; 0],
}

/// Opaque structure representing an Opus decoder state.
#[repr(C)]
pub struct OpusDecoder {
    _private: [u8; 0],
}

// Safety: An Opus encoder state can be safely transferred across threads
// when not actively in concurrent use.
unsafe impl Send for OpusEncoder {}

// Safety: An Opus decoder state can be safely transferred across threads
// when not actively in concurrent use.
unsafe impl Send for OpusDecoder {}

// --- Opus Error and Return Codes ---

/// No error occurred.
pub const OPUS_OK: i32 = 0;
/// One or more invalid/out of range arguments.
pub const OPUS_BAD_ARG: i32 = -1;
/// The buffer provided is too small.
pub const OPUS_BUFFER_TOO_SMALL: i32 = -2;
/// An internal error was detected.
pub const OPUS_INTERNAL_ERROR: i32 = -3;
/// The compressed data passed is corrupted.
pub const OPUS_INVALID_PACKET: i32 = -4;
/// Invalid/unsupported request number.
pub const OPUS_UNIMPLEMENTED: i32 = -5;
/// An encoder or decoder structure is invalid or already freed.
pub const OPUS_INVALID_STATE: i32 = -6;
/// Memory allocation has failed.
pub const OPUS_ALLOC_FAIL: i32 = -7;

// --- Generic Configuration Constants ---

/// Auto/default configuration setting.
pub const OPUS_AUTO: i32 = -1000;
/// Maximum allowable bitrate.
pub const OPUS_BITRATE_MAX: i32 = -1;

// --- Application Profile Constants ---

/// Best for most VoIP/videoconference applications where listening quality and intelligibility matter most.
pub const OPUS_APPLICATION_VOIP: i32 = 2048;
/// Best for broadcast/high-fidelity audio applications where the input is considered to be music/audio.
pub const OPUS_APPLICATION_AUDIO: i32 = 2049;
/// Only use when lowest-achievable latency is what matters most.
pub const OPUS_APPLICATION_RESTRICTED_LOWDELAY: i32 = 2051;

// --- Encoder and Decoder CTL Request Constants ---

/// Configures the application profile of the encoder.
pub const OPUS_SET_APPLICATION_REQUEST: i32 = 4000;
/// Gets the current application profile of the encoder.
pub const OPUS_GET_APPLICATION_REQUEST: i32 = 4001;
/// Configures the target bitrate in the encoder.
pub const OPUS_SET_BITRATE_REQUEST: i32 = 4002;
/// Gets the target bitrate of the encoder.
pub const OPUS_GET_BITRATE_REQUEST: i32 = 4003;
/// Enables or disables variable bitrate (VBR) in the encoder.
pub const OPUS_SET_VBR_REQUEST: i32 = 4006;
/// Gets the variable bitrate (VBR) mode of the encoder.
pub const OPUS_GET_VBR_REQUEST: i32 = 4007;
/// Sets the maximum audio bandwidth of the encoder.
pub const OPUS_SET_BANDWIDTH_REQUEST: i32 = 4008;
/// Gets the maximum audio bandwidth of the encoder.
pub const OPUS_GET_BANDWIDTH_REQUEST: i32 = 4009;
/// Configures the encoder's computational complexity (0-10).
pub const OPUS_SET_COMPLEXITY_REQUEST: i32 = 4010;
/// Gets the encoder's computational complexity.
pub const OPUS_GET_COMPLEXITY_REQUEST: i32 = 4011;
/// Enables or disables in-band forward error correction (FEC).
pub const OPUS_SET_INBAND_FEC_REQUEST: i32 = 4012;
/// Gets the in-band forward error correction (FEC) setting of the encoder.
pub const OPUS_GET_INBAND_FEC_REQUEST: i32 = 4013;
/// Configures the encoder's expected packet loss percentage (0-100).
pub const OPUS_SET_PACKET_LOSS_PERC_REQUEST: i32 = 4014;
/// Gets the encoder's expected packet loss percentage.
pub const OPUS_GET_PACKET_LOSS_PERC_REQUEST: i32 = 4015;
/// Enables or disables discontinuous transmission (DTX).
pub const OPUS_SET_DTX_REQUEST: i32 = 4016;
/// Gets the discontinuous transmission (DTX) setting of the encoder.
pub const OPUS_GET_DTX_REQUEST: i32 = 4017;
/// Configures the type of signal being encoded (voice or music).
pub const OPUS_SET_SIGNAL_REQUEST: i32 = 4024;
/// Gets the type of signal configured in the encoder.
pub const OPUS_GET_SIGNAL_REQUEST: i32 = 4025;
/// Gets the total lookahead latency in samples.
pub const OPUS_GET_LOOKAHEAD_REQUEST: i32 = 4027;
/// Resets the codec state to newly initialized.
pub const OPUS_RESET_STATE: i32 = 4028;
/// Gets the sampling rate of the encoder or decoder in Hz.
pub const OPUS_GET_SAMPLE_RATE_REQUEST: i32 = 4029;

// --- Signal Type Constants ---

/// Signal is voice speech.
pub const OPUS_SIGNAL_VOICE: i32 = 3001;
/// Signal is music.
pub const OPUS_SIGNAL_MUSIC: i32 = 3002;

// --- Bandwidth Constants ---

/// Narrowband audio (4 kHz bandpass).
pub const OPUS_BANDWIDTH_NARROWBAND: i32 = 1101;
/// Mediumband audio (6 kHz bandpass).
pub const OPUS_BANDWIDTH_MEDIUMBAND: i32 = 1102;
/// Wideband audio (8 kHz bandpass).
pub const OPUS_BANDWIDTH_WIDEBAND: i32 = 1103;
/// Superwideband audio (12 kHz bandpass).
pub const OPUS_BANDWIDTH_SUPERWIDEBAND: i32 = 1104;
/// Fullband audio (20 kHz bandpass).
pub const OPUS_BANDWIDTH_FULLBAND: i32 = 1105;

// --- C Foreign Function Declarations ---

#[link(name = "opus")]
#[allow(non_snake_case)]
unsafe extern "C" {
    /// Allocates and initializes an `OpusEncoder` state.
    ///
    /// # Parameters
    /// - `Fs`: Sampling rate to encode with in Hz. Must be 8000, 12000, 16000, 24000, or 48000.
    /// - `channels`: Number of channels (1 or 2).
    /// - `application`: Coding mode (`OPUS_APPLICATION_VOIP`, `OPUS_APPLICATION_AUDIO`, or `OPUS_APPLICATION_RESTRICTED_LOWDELAY`).
    /// - `error`: Returns `OPUS_OK` on success, or an error code on failure.
    pub fn opus_encoder_create(
        Fs: i32,
        channels: i32,
        application: i32,
        error: *mut i32,
    ) -> *mut OpusEncoder;

    /// Frees an `OpusEncoder` allocated by `opus_encoder_create`.
    pub fn opus_encoder_destroy(st: *mut OpusEncoder);

    /// Encodes an Opus frame from 16-bit linear PCM audio.
    ///
    /// # Returns
    /// Number of bytes written on success, or a negative error code.
    pub fn opus_encode(
        st: *mut OpusEncoder,
        pcm: *const i16,
        frame_size: i32,
        data: *mut u8,
        max_data_bytes: i32,
    ) -> i32;

    /// Encodes an Opus frame from floating point linear PCM audio.
    ///
    /// # Returns
    /// Number of bytes written on success, or a negative error code.
    pub fn opus_encode_float(
        st: *mut OpusEncoder,
        pcm: *const f32,
        frame_size: i32,
        data: *mut u8,
        max_data_bytes: i32,
    ) -> i32;

    /// Performs a CTL control command on an Opus encoder.
    pub fn opus_encoder_ctl(st: *mut OpusEncoder, request: i32, ...) -> i32;

    /// Allocates and initializes an `OpusDecoder` state.
    ///
    /// # Parameters
    /// - `Fs`: Sampling rate to decode with in Hz. Must be 8000, 12000, 16000, 24000, or 48000.
    /// - `channels`: Number of channels (1 or 2).
    /// - `error`: Returns `OPUS_OK` on success, or an error code on failure.
    pub fn opus_decoder_create(
        Fs: i32,
        channels: i32,
        error: *mut i32,
    ) -> *mut OpusDecoder;

    /// Frees an `OpusDecoder` allocated by `opus_decoder_create`.
    pub fn opus_decoder_destroy(st: *mut OpusDecoder);

    /// Decodes an Opus packet into 16-bit linear PCM audio.
    ///
    /// # Returns
    /// Number of decoded samples on success, or a negative error code.
    pub fn opus_decode(
        st: *mut OpusDecoder,
        data: *const u8,
        len: i32,
        pcm: *mut i16,
        frame_size: i32,
        decode_fec: i32,
    ) -> i32;

    /// Decodes an Opus packet into floating point linear PCM audio.
    ///
    /// # Returns
    /// Number of decoded samples on success, or a negative error code.
    pub fn opus_decode_float(
        st: *mut OpusDecoder,
        data: *const u8,
        len: i32,
        pcm: *mut f32,
        frame_size: i32,
        decode_fec: i32,
    ) -> i32;

    /// Performs a CTL control command on an Opus decoder.
    pub fn opus_decoder_ctl(st: *mut OpusDecoder, request: i32, ...) -> i32;

    /// Returns a static string describing the given Opus error code.
    pub fn opus_strerror(error: i32) -> *const c_char;

    /// Returns the libopus version string.
    pub fn opus_get_version_string() -> *const c_char;

    /// Gets the bandwidth of an Opus packet.
    pub fn opus_packet_get_bandwidth(data: *const u8) -> i32;

    /// Gets the number of channels from an Opus packet.
    pub fn opus_packet_get_nb_channels(data: *const u8) -> i32;

    /// Gets the number of frames in an Opus packet.
    pub fn opus_packet_get_nb_frames(packet: *const u8, len: i32) -> i32;

    /// Gets the number of samples per frame from an Opus packet.
    pub fn opus_packet_get_samples_per_frame(data: *const u8, Fs: i32) -> i32;

    /// Gets the number of samples of an Opus packet in the decoder.
    pub fn opus_decoder_get_nb_samples(
        dec: *const OpusDecoder,
        packet: *const u8,
        len: i32,
    ) -> i32;
}
