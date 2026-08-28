use super::packet::AudioFrame;
use super::source::AudioSource;
use crate::error::{Error, Result};
use async_trait::async_trait;
use std::time::Duration;
use symphonia::core::codecs::CODEC_TYPE_OPUS;
use symphonia::core::formats::FormatOptions;
use symphonia::core::formats::FormatReader;
use symphonia::core::io::{MediaSource, MediaSourceStream};
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

pub struct OpusDemuxer {
    format_reader: Box<dyn FormatReader>,
    track_id: u32,
}

impl OpusDemuxer {
    pub fn new(source: Box<dyn MediaSource>, hint_ext: Option<&str>) -> Result<Self> {
        let mut hint = Hint::new();
        if let Some(ext) = hint_ext {
            hint.with_extension(ext);
        }

        let mss = MediaSourceStream::new(source, Default::default());
        let format_opts = FormatOptions {
            enable_gapless: true,
            ..Default::default()
        };
        let meta_opts = MetadataOptions::default();

        let probed = symphonia::default::get_probe()
            .format(&hint, mss, &format_opts, &meta_opts)
            .map_err(|e| Error::Connection(format!("Probe failed: {}", e)))?;

        let format_reader = probed.format;

        // Find Opus track
        let track = format_reader
            .tracks()
            .iter()
            .find(|t| t.codec_params.codec == CODEC_TYPE_OPUS)
            .ok_or_else(|| Error::Connection("No Opus track found".into()))?;

        let track_id = track.id;

        Ok(Self {
            format_reader,
            track_id,
        })
    }
}

#[async_trait]
impl AudioSource for OpusDemuxer {
    async fn next_frame(&mut self) -> Result<Option<AudioFrame>> {
        loop {
            match self.format_reader.next_packet() {
                Ok(packet) => {
                    if packet.track_id() == self.track_id {
                        let data = packet.data.to_vec();
                        return Ok(Some(AudioFrame::Opus(data)));
                    }
                }
                Err(symphonia::core::errors::Error::IoError(e))
                    if e.kind() == std::io::ErrorKind::UnexpectedEof =>
                {
                    return Ok(None);
                }
                Err(e) => return Err(Error::Connection(format!("Demux error: {}", e))),
            }
        }
    }
}
