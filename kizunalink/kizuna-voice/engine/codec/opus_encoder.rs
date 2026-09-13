// Copyright (c) 2026 nikcodex (KizunaLink)
// Licensed under the MIT License

use crate::opus::{Application, Channels, SampleRate, Encoder as OpusEncoder, Error as OpusError};

pub struct OpusCodecEncoder {
    encoder: OpusEncoder,
}

impl OpusCodecEncoder {
    /// Create a new encoder at 48 kHz stereo with the AUDIO application profile.
    pub fn new(quality: u8) -> Result<Self, OpusError> {
        let encoder =
            OpusEncoder::new(SampleRate::Hz48000, Channels::Stereo, Application::Audio)?;
        encoder.set_complexity(quality)?;
        Ok(Self { encoder })
    }

    /// Encode a 960-sample (per channel) interleaved i16 PCM slice into `out`.
    /// Returns the number of bytes written.
    pub fn encode(&mut self, pcm: &[i16], out: &mut [u8]) -> Result<usize, OpusError> {
        self.encoder.encode(pcm, out)
    }
}
