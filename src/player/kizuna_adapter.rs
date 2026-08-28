use kizuna_voice::audio::{
    AudioFrame, AudioSource, FrameScheduler, KizunaTrackHandle, OpusEncoder, OpusSource,
};
use kizuna_voice::connection::session::VoiceSession;
use kizuna_voice::dave::protocol::DaveSession;
use kizuna_voice::gateway::{connection::GatewayEvent, VoiceGatewayClient};
use kizuna_voice::transport::{RtpHeader, VoiceUdp};
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc, Mutex};
use tracing::{error, info};

pub struct KizunaVoiceAdapter {
    session: VoiceSession,
    udp: Option<Arc<VoiceUdp>>,
    dave: Arc<Mutex<DaveSession>>,
    ssrc: u32,
    sequence: u16,
    timestamp: u32,
}

impl KizunaVoiceAdapter {
    pub fn new(session_id: String, token: String, endpoint: String, guild_id: String) -> Self {
        Self {
            session: VoiceSession::new(session_id, token, endpoint),
            udp: None,
            dave: Arc::new(Mutex::new(DaveSession::new(guild_id))),
            ssrc: 0,
            sequence: 0,
            timestamp: 0,
        }
    }

    pub async fn connect(&mut self, server_id: String, user_id: String) -> Result<(), String> {
        info!("Connecting using KizunaVoice adapter...");

        let mut gw = VoiceGatewayClient::connect(&self.session.endpoint)
            .await
            .map_err(|e| e.to_string())?;

        gw.send_identify(
            &server_id,
            &user_id,
            &self.session.session_id,
            &self.session.token,
        )
        .await
        .map_err(|e| e.to_string())?;

        let dave_clone = self.dave.clone();

        let mut ready_data = None;
        while let Ok(event) = gw.receive_event().await {
            match event {
                GatewayEvent::Ready(ready) => {
                    ready_data = Some(ready);
                    break;
                }
                GatewayEvent::DaveMessage(dave_msg) => {
                    let mut dave = dave_clone.lock().await;
                    let client_messages = dave.handle_gateway_message(dave_msg);
                    for msg in client_messages {
                        let _ = gw.send_dave_message(msg).await;
                    }
                }
                _ => {}
            }
        }

        let ready = ready_data.ok_or("Did not receive Ready event")?;
        self.ssrc = ready.ssrc;

        let (udp, _external_ip, _external_port) =
            VoiceUdp::bind_and_discover(&ready.ip, ready.port, ready.ssrc)
                .await
                .map_err(|e| e.to_string())?;

        let udp_arc = Arc::new(udp);
        self.udp = Some(udp_arc.clone());

        gw.send_select_protocol(
            "udp",
            &_external_ip,
            _external_port,
            "aead_aes256_gcm_rtpsize",
        )
        .await
        .map_err(|e| e.to_string())?;

        tokio::spawn(async move {
            loop {
                match gw.receive_event().await {
                    Ok(GatewayEvent::DaveMessage(dave_msg)) => {
                        let mut dave = dave_clone.lock().await;
                        let client_messages = dave.handle_gateway_message(dave_msg);
                        for msg in client_messages {
                            let _ = gw.send_dave_message(msg).await;
                        }
                    }
                    Ok(GatewayEvent::SessionDescription(_sd)) => {
                        info!("Received Session Description. Handshake complete.");
                    }
                    Err(e) => {
                        error!("Gateway disconnected: {}", e);
                        break;
                    }
                    _ => {}
                }
            }
        });

        Ok(())
    }

    pub fn play_source(
        &mut self,
        source: Arc<Mutex<dyn AudioSource>>,
        sender_id: String,
    ) -> KizunaTrackHandle {
        let (cmd_tx, cmd_rx) = mpsc::channel(32);
        let (event_tx, event_rx) = broadcast::channel(32);

        let handle = KizunaTrackHandle::new(cmd_tx, event_rx);

        let scheduler = FrameScheduler::new(source);
        let udp = self.udp.clone().expect("UDP not connected");
        let dave = self.dave.clone();
        let ssrc = self.ssrc;
        let mut sequence = self.sequence;
        let mut timestamp = self.timestamp;

        tokio::spawn(async move {
            let encoder = std::sync::Arc::new(tokio::sync::Mutex::new(OpusEncoder::new().unwrap()));
            scheduler
                .run(cmd_rx, event_tx, |frame| {
                    let udp = udp.clone();
                    let dave = dave.clone();
                    let sender_id_clone = sender_id.clone();
                    let enc_clone = encoder.clone();

                    async move {
                        sequence = sequence.wrapping_add(1);
                        timestamp = timestamp.wrapping_add(960);

                        let opus_data = match frame {
                            AudioFrame::Opus(data) => data,
                            AudioFrame::Pcm(pcm) => {
                                let mut enc = enc_clone.try_lock().unwrap();
                                let encoded = enc.encode(OpusSource::Pcm(pcm)).unwrap();
                                if let AudioFrame::Opus(data) = encoded {
                                    data
                                } else {
                                    vec![]
                                }
                            }
                        };

                        let header = RtpHeader::new(sequence, timestamp, ssrc);
                        let mut header_buf = Vec::new();
                        header.write_to(&mut header_buf).unwrap();

                        let mut dave_guard = dave.lock().await;
                        if dave_guard.is_active() {
                            if let Ok(encrypted) = dave_guard.encrypt_frame(
                                &sender_id_clone,
                                &opus_data,
                                sequence as u32,
                                &header_buf,
                            ) {
                                let mut packet = header_buf;
                                packet.extend(encrypted);
                                let _ = udp.send_packet(&packet).await;
                            }
                        } else {
                            let mut packet = header_buf;
                            packet.extend(opus_data);
                            let _ = udp.send_packet(&packet).await;
                        }
                    }
                })
                .await;
        });

        self.sequence = sequence;
        self.timestamp = timestamp;

        handle
    }
}

use async_trait::async_trait;
use kizuna_voice::error::Result as KzResult;
use std::io::Read;

pub struct PcmSourceWrapper<R: Read + Send + Sync> {
    pub reader: R,
}

#[async_trait]
impl<R: Read + Send + Sync> AudioSource for PcmSourceWrapper<R> {
    async fn next_frame(&mut self) -> KzResult<Option<AudioFrame>> {
        // Read 1920 f32 samples (7680 bytes)
        let mut buf = [0u8; 7680];
        let mut total_read = 0;

        while total_read < buf.len() {
            match self.reader.read(&mut buf[total_read..]) {
                Ok(0) => break, // EOF
                Ok(n) => total_read += n,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => continue,
                Err(e) => return Err(kizuna_voice::error::Error::Connection(e.to_string())),
            }
        }

        if total_read == 0 {
            return Ok(None);
        }

        let num_samples = total_read / 4;
        let mut samples = Vec::with_capacity(num_samples);
        for chunk in buf[..total_read].chunks_exact(4) {
            let f = f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
            // Convert f32 to i16
            let s = (f * i16::MAX as f32).clamp(i16::MIN as f32, i16::MAX as f32) as i16;
            samples.push(s);
        }

        Ok(Some(AudioFrame::Pcm(samples)))
    }
}
