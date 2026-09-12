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
