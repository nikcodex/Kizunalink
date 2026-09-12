// Copyright (c) 2026 nikcodex (KizunaLink)
// Licensed under the MIT License

use std::sync::Arc;

use tracing::{debug, error};

use crate::{
    engine::{AudioFrame, processor::AudioProcessor, source::HttpSource},
    common::types::AudioFormat,
    media::sources::plugin::{DecoderOutput, PlayableTrack},
};

pub struct TidalTrack {
    pub identifier: String,
    pub stream_url: String,
    pub kind: AudioFormat,
    pub http_client: Arc<reqwest::Client>,
}

impl PlayableTrack for TidalTrack {
    fn start_decoding(&self, config: crate::discord::player::PlayerConfig) -> DecoderOutput {
        let (tx, rx) = flume::bounded::<AudioFrame>((config.buffer_duration_ms / 20) as usize);
        let (cmd_tx, cmd_rx) = flume::bounded(8);
        let (err_tx, err_rx) = flume::bounded(1);

        let identifier = self.identifier.clone();
        let stream_url = self.stream_url.clone();
        let kind = self.kind;
        let http_client = (*self.http_client).clone();

        let err_tx_for_setup = err_tx.clone();
        let identifier_for_setup = identifier.clone();
        tokio::spawn(async move {
            debug!("TidalTrack: starting playback for {}", identifier);

            let setup_res_task = tokio::task::spawn_blocking(move || {
                let client_clone = http_client.clone();
                match HttpSource::new(client_clone, &stream_url) {
                    Ok(reader) => AudioProcessor::new(
                        Box::new(reader),
                        Some(kind),
                        tx,
                        cmd_rx,
                        Some(err_tx_for_setup),
                        config,
                    )
                    .map_err(|e| e.to_string()),
                    Err(e) => {
                        error!("TidalTrack: HttpSource init failed for {}: {}", identifier_for_setup, e);
                        Err(format!("Failed to initialize source: {}", e))
                    }
                }
            })
                .await;
                let setup_res = match setup_res_task {
                    Ok(res) => res,
                    Err(e) => {
                        tracing::error!("spawn_blocking failed: {}", e);
                        let _ = err_tx.send(format!("Failed to spawn task: {e}"));
                        return;
                    }
                };

            match setup_res {
                Ok(mut processor) => {
                    if let Err(e) = std::thread::Builder::new()
                        .name(format!("tidal-decoder-{}", identifier))
                        .spawn(move || {
                            if let Err(e) = processor.run() {
                                error!(
                                    "TidalTrack audio processor error for {}: {}",
                                    identifier, e
                                );
                            }
                        }) {
                            tracing::error!("failed to spawn thread: {e}");
                            let _ = err_tx.send(format!("Failed to spawn decoder thread: {e}"));
                        }
                }
                Err(e) => {
                    error!(
                        "TidalTrack failed to initialize processor for {}: {}",
                        identifier, e
                    );
                    let _ = err_tx.send(format!("Failed to initialize processor: {e}"));
                }
            }
        });

        (rx, cmd_tx, err_rx)
    }
}