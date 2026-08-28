use kizuna_voice::audio::{
    AudioController, AudioFrame, AudioSource, FrameScheduler, OpusEncoder, OpusSource,
    SchedulerCommand, TrackState,
};
use kizuna_voice::connection::session::VoiceSession;
use kizuna_voice::dave::protocol::{DaveClientMessage, DaveGatewayMessage, DaveSession};
use kizuna_voice::gateway::{connection::GatewayEvent, VoiceGatewayClient};
use kizuna_voice::transport::{RtpHeader, RtpPacket, VoiceUdp};
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};
use tokio::time::Instant;
use tracing::{error, info, warn};

pub struct KizunaVoiceAdapter {
    session: VoiceSession,
    udp: Option<Arc<VoiceUdp>>,
    dave: Arc<Mutex<DaveSession>>,
    pub controller: AudioController,
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
            controller: AudioController::new(),
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

        // Wait for Ready event to get IP/Port/SSRC
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

        // Setup UDP
        let (udp, _external_ip, _external_port) =
            VoiceUdp::bind_and_discover(&ready.ip, ready.port, ready.ssrc)
                .await
                .map_err(|e| e.to_string())?;

        let udp_arc = Arc::new(udp);
        self.udp = Some(udp_arc.clone());

        // Protocol Select
        gw.send_select_protocol(
            "udp",
            &_external_ip,
            _external_port,
            "aead_aes256_gcm_rtpsize",
        )
        .await
        .map_err(|e| e.to_string())?;

        // Background loop
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

    pub fn play_source(&mut self, source: Arc<Mutex<dyn AudioSource>>, sender_id: String) {
        let (tx, rx) = mpsc::channel(10);
        self.controller.attach_scheduler(tx);

        let scheduler = FrameScheduler::new(source);
        let udp = self.udp.clone().expect("UDP not connected");
        let dave = self.dave.clone();
        let ssrc = self.ssrc;
        let mut sequence = self.sequence;
        let mut timestamp = self.timestamp;

        tokio::spawn(async move {
            let mut encoder = OpusEncoder::new().unwrap();

            scheduler
                .run(rx, |frame| {
                    let udp = udp.clone();
                    let dave = dave.clone();
                    let sender_id_clone = sender_id.clone();

                    async move {
                        sequence = sequence.wrapping_add(1);
                        timestamp = timestamp.wrapping_add(960);

                        let opus_data = match frame {
                            AudioFrame::Opus(data) => data,
                            AudioFrame::Pcm(pcm) => {
                                let encoded = encoder.encode(OpusSource::Pcm(pcm)).unwrap();
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
    }

    pub async fn stop(&mut self) {
        self.controller.stop().await;
    }

    pub async fn pause(&mut self) {
        self.controller.pause().await;
    }

    pub async fn resume(&mut self) {
        self.controller.resume().await;
    }
}
