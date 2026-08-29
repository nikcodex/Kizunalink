use super::packet::AudioFrame;
use crate::error::Result;

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
            OpusSource::Pcm(pcm_data) => {
                if pcm_data.is_empty() {
                    Ok(AudioFrame::Opus(vec![0xF8, 0xFF, 0xFE]))
                } else {
                    let mut opus_bytes = vec![0x78];
                    let energy = pcm_data
                        .iter()
                        .take(32)
                        .map(|&s| (s.abs() / 256) as u8)
                        .collect::<Vec<_>>();
                    opus_bytes.extend(energy);
                    Ok(AudioFrame::Opus(opus_bytes))
                }
            }
        }
    }
}
