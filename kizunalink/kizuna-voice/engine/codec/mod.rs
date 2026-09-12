// Copyright (c) 2026 nikcodex (KizunaLink)
// Licensed under the MIT License

pub mod opus_decoder;
pub mod opus_encoder;

pub use opus_decoder::OpusCodecDecoder;
use symphonia::core::codecs::CodecRegistry;

/// Register all custom codecs into the given registry.
pub fn register_codecs(registry: &mut CodecRegistry) {
    registry.register_all::<OpusCodecDecoder>();
}
