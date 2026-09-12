// Copyright (c) 2026 nikcodex (KizunaLink)
// Licensed under the MIT License

use std::net::IpAddr;

use crate::{
    config::HttpProxyConfig,
    media::sources::{
        http::HttpTrack,
        plugin::{DecoderOutput, PlayableTrack},
    },
};

pub struct NeteaseTrack {
    pub stream_url: String,
    pub local_addr: Option<IpAddr>,
    pub proxy: Option<HttpProxyConfig>,
}

impl PlayableTrack for NeteaseTrack {
    fn start_decoding(&self, config: crate::config::discord::player::PlayerConfig) -> DecoderOutput {
        HttpTrack {
            url: self.stream_url.clone(),
            local_addr: self.local_addr,
            proxy: self.proxy.clone(),
        }
        .start_decoding(config)
    }
}
