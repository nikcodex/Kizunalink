// Copyright (c) 2026 nikcodex (KizunaLink)
// Licensed under the MIT License

use thiserror::Error;

#[derive(Debug, Error)]
pub enum AudioError {
    #[error("Decoder finished")]
    DecoderFinished,
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    // Add other variants as needed
}
