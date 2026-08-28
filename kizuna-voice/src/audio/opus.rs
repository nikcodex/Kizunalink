use super::packet::AudioFrame;
use crate::error::{Error, Result};

pub enum OpusSource {
    Encoded(Vec<u8>),
    Pcm(Vec<i16>),
}

pub struct OpusEncoder {
    // Wrap real encoder later
}

impl OpusEncoder {
    pub fn new() -> Result<Self> {
        Ok(Self {})
    }

    pub fn encode(&mut self, source: OpusSource) -> Result<AudioFrame> {
        match source {
            OpusSource::Encoded(data) => Ok(AudioFrame::Opus(data)),
            OpusSource::Pcm(_pcm_data) => {
                // Here we would encode PCM to Opus (Phase 5)
                Ok(AudioFrame::Opus(vec![])) // Stub
            }
        }
    }
}
