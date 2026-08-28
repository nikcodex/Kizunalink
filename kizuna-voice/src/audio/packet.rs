#[derive(Debug, Clone)]
pub enum AudioFrame {
    /// 20ms of PCM data (e.g., 960 frames * 2 channels = 1920 samples)
    Pcm(Vec<i16>),
    /// Already encoded Opus data
    Opus(Vec<u8>),
}

impl AudioFrame {
    pub fn is_opus(&self) -> bool {
        matches!(self, AudioFrame::Opus(_))
    }
}
