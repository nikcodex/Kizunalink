use super::packet::AudioFrame;
use crate::error::Result;
use audiopus::coder::Encoder;
use audiopus::{Application, Channels, SampleRate};

/// Discord voice requires:
/// - 48kHz sample rate
/// - Stereo (2 channels)
/// - 20ms frames = 960 samples per channel = 1920 total samples
const SAMPLE_RATE: SampleRate = SampleRate::Hz48000;
const CHANNELS: Channels = Channels::Stereo;
const FRAME_SIZE: usize = 960; // samples per channel per 20ms frame
/// Maximum Opus packet size (Discord typically sees ~100-300 bytes, but spec allows up to ~4000)
const MAX_PACKET_SIZE: usize = 4000;

pub enum OpusSource {
    Encoded(Vec<u8>),
    Pcm(Vec<i16>),
}

pub struct OpusEncoder {
    encoder: Encoder,
}

impl OpusEncoder {
    pub fn new() -> Result<Self> {
        let encoder = Encoder::new(SAMPLE_RATE, CHANNELS, Application::Audio)
            .map_err(|e| crate::error::Error::Transport(format!("Opus encoder init failed: {}", e)))?;

        Ok(Self { encoder })
    }

    pub fn encode(&mut self, source: OpusSource) -> Result<AudioFrame> {
        match source {
            // Pre-encoded Opus data passes through directly
            OpusSource::Encoded(data) => Ok(AudioFrame::Opus(data)),
            OpusSource::Pcm(pcm_data) => {
                if pcm_data.is_empty() {
                    // Encode silence — a proper silent Opus frame
                    let silence = vec![0i16; FRAME_SIZE * 2]; // stereo
                    let mut output = vec![0u8; MAX_PACKET_SIZE];
                    let len = self.encoder.encode(&silence, &mut output)
                        .map_err(|e| crate::error::Error::Transport(format!("Opus encode silence failed: {}", e)))?;
                    output.truncate(len);
                    Ok(AudioFrame::Opus(output))
                } else {
                    let mut output = vec![0u8; MAX_PACKET_SIZE];
                    let len = self.encoder.encode(&pcm_data, &mut output)
                        .map_err(|e| crate::error::Error::Transport(format!("Opus encode failed: {}", e)))?;
                    output.truncate(len);
                    Ok(AudioFrame::Opus(output))
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_silence() {
        let mut encoder = OpusEncoder::new().expect("create encoder");
        let silence = vec![0i16; FRAME_SIZE * 2]; // 20ms stereo
        let frame = encoder.encode(OpusSource::Pcm(silence)).expect("encode silence");
        match frame {
            AudioFrame::Opus(data) => {
                assert!(!data.is_empty(), "Opus output must not be empty");
                // A valid Opus silence frame is typically very small (3-6 bytes)
                assert!(data.len() < 100, "Silence frame should be small, got {} bytes", data.len());
            }
            _ => panic!("Expected Opus frame"),
        }
    }

    #[test]
    fn test_encode_pcm_tone() {
        let mut encoder = OpusEncoder::new().expect("create encoder");
        // Generate a 440Hz sine wave, 20ms, stereo, 48kHz
        let mut pcm = vec![0i16; FRAME_SIZE * 2];
        for i in 0..FRAME_SIZE {
            let sample = (2.0 * std::f64::consts::PI * 440.0 * i as f64 / 48000.0).sin();
            let s = (sample * 16000.0) as i16;
            pcm[i * 2] = s;     // left
            pcm[i * 2 + 1] = s; // right
        }
        let frame = encoder.encode(OpusSource::Pcm(pcm)).expect("encode tone");
        match frame {
            AudioFrame::Opus(data) => {
                assert!(!data.is_empty(), "Opus output must not be empty");
                // A 440Hz tone encodes to roughly 50-300 bytes
                assert!(data.len() > 3, "Tone frame should be larger than silence");
            }
            _ => panic!("Expected Opus frame"),
        }
    }

    #[test]
    fn test_encode_empty_pcm_produces_silence() {
        let mut encoder = OpusEncoder::new().expect("create encoder");
        let frame = encoder.encode(OpusSource::Pcm(vec![])).expect("encode empty");
        match frame {
            AudioFrame::Opus(data) => {
                assert!(!data.is_empty(), "Even empty PCM should produce valid Opus");
            }
            _ => panic!("Expected Opus frame"),
        }
    }

    #[test]
    fn test_passthrough_encoded() {
        let mut encoder = OpusEncoder::new().expect("create encoder");
        let fake_opus = vec![0xFC, 0xFF, 0xFE]; // pre-encoded data
        let frame = encoder.encode(OpusSource::Encoded(fake_opus.clone())).expect("passthrough");
        match frame {
            AudioFrame::Opus(data) => assert_eq!(data, fake_opus),
            _ => panic!("Expected Opus frame"),
        }
    }

    #[test]
    fn test_multiple_frames_sequential() {
        let mut encoder = OpusEncoder::new().expect("create encoder");
        // Encode multiple frames to verify the encoder state doesn't corrupt
        for _ in 0..10 {
            let pcm = vec![0i16; FRAME_SIZE * 2];
            let frame = encoder.encode(OpusSource::Pcm(pcm)).expect("encode sequential");
            assert!(matches!(frame, AudioFrame::Opus(_)));
        }
    }
}
