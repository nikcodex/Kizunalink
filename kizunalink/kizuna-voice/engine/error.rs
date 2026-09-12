// Copyright (c) 2026 nikcodex (KizunaLink)
// Licensed under the MIT License

use thiserror::Error;

#[derive(Debug, Error)]
pub enum AudioError {
    #[error("Decoder finished")]
    DecoderFinished,
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Decoder error: {0}")]
    Decoder(#[from] symphonia::core::errors::Error),
    #[error("Opus error: {0}")]
    Opus(#[from] crate::opus::Error),
    #[error("Resample error: {0}")]
    Resample(String),
    #[error("Seek out of range")]
    SeekOutOfRange,
    #[error("Unsupported format: {0}")]
    UnsupportedFormat(String),
    #[error("Empty stream")]
    EmptyStream,
    #[error("Operation cancelled")]
    Cancelled,
    #[error("{0}")]
    Other(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decoder_finished_display() {
        let err = AudioError::DecoderFinished;
        assert!(err.to_string().contains("Decoder finished"));
    }

    #[test]
    fn empty_stream_display() {
        let err = AudioError::EmptyStream;
        assert!(err.to_string().contains("Empty stream"));
    }

    #[test]
    fn unsupported_format_display() {
        let err = AudioError::UnsupportedFormat("FLAC".into());
        assert!(err.to_string().contains("FLAC"));
    }

    #[test]
    fn resample_display() {
        let err = AudioError::Resample("ratio mismatch".into());
        assert!(err.to_string().contains("ratio mismatch"));
    }

    #[test]
    fn cancelled_display() {
        let err = AudioError::Cancelled;
        assert!(err.to_string().contains("cancelled"));
    }

    #[test]
    fn other_display() {
        let err = AudioError::Other("custom error".into());
        assert!(err.to_string().contains("custom error"));
    }
}
