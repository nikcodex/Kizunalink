// Copyright (c) 2026 nikcodex (KizunaLink)
// Licensed under the MIT License

use std::net::IpAddr;

use crate::media::sources::{
    http::HttpTrack,
    plugin::{DecoderOutput, PlayableTrack},
};

pub struct VkMusicTrack {
    pub stream_url: String,
    pub local_addr: Option<IpAddr>,
    pub proxy: Option<crate::config::HttpProxyConfig>,
}

impl PlayableTrack for VkMusicTrack {
    fn start_decoding(&self, config: crate::discord::player::PlayerConfig) -> DecoderOutput {
        HttpTrack {
            url: self.stream_url.clone(),
            local_addr: self.local_addr,
            proxy: self.proxy.clone(),
        }
        .start_decoding(config)
    }
}
