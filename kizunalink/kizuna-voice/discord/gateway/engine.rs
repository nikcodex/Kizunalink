// Copyright (c) 2026 nikcodex (KizunaLink)
// Licensed under the MIT License

use tokio::sync::Mutex;

use crate::{engine::Mixer, common::types::Shared, discord::gateway::constants::DEFAULT_SAMPLE_RATE};

pub struct VoiceEngine {
    pub mixer: Shared<Mixer>,
    pub dave: Option<Shared<crate::discord::crypto::DaveHandler>>,
}

impl VoiceEngine {
    pub fn new() -> Self {
        Self {
            mixer: Shared::new(Mutex::new(Mixer::new(DEFAULT_SAMPLE_RATE))),
            dave: None,
        }
    }
}

impl Default for VoiceEngine {
    fn default() -> Self {
        Self::new()
    }
}
