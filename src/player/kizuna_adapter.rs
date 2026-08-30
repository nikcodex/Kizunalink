use kizuna_voice::audio::{
    AudioFrame, AudioSource, FrameScheduler, KizunaTrackHandle, OpusEncoder, OpusSource,
};
use kizuna_voice::connection::session::VoiceSession;
use kizuna_voice::dave::protocol::DaveSession;
use kizuna_voice::gateway::{connection::GatewayEvent, VoiceGatewayClient};
use kizuna_voice::transport::{RtpHeader, TransportCrypto, VoiceUdp};
use kizuna_voice::connection::manager::{VoiceConnectionManager, VoiceCredentials};
use kizuna_voice::connection::state::ConnectionState;
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc, Mutex, RwLock};
use tracing::{error, info};

pub struct KizunaVoiceAdapter {
    session: VoiceSession,
    udp: Arc<RwLock<Option<Arc<VoiceUdp>>>>,
    dave: Arc<Mutex<DaveSession>>,
    transport_crypto: Arc<Mutex<Option<TransportCrypto>>>,
    ssrc: Arc<std::sync::atomic::AtomicU32>,
    sequence: Arc<std::sync::atomic::AtomicU16>,
    timestamp: Arc<std::sync::atomic::AtomicU32>,
    manager: Option<Arc<VoiceConnectionManager>>,
}

impl KizunaVoiceAdapter {
    pub fn new(session_id: String, token: String, endpoint: String, guild_id: String) -> Self {
        Self {
            session: VoiceSession::new(session_id, token, endpoint),
            udp: Arc::new(RwLock::new(None)),
            dave: Arc::new(Mutex::new(DaveSession::new(guild_id))),
            transport_crypto: Arc::new(Mutex::new(None)),
            ssrc: Arc::new(std::sync::atomic::AtomicU32::new(0)),
            sequence: Arc::new(std::sync::atomic::AtomicU16::new(0)),
            timestamp: Arc::new(std::sync::atomic::AtomicU32::new(0)),
            manager: None,
        }
    }

    pub async fn connect(&mut self, server_id: String, user_id: String) -> Result<(), String> {
        info!("Connecting using KizunaVoice adapter with reconnect support...");

        let credentials = VoiceCredentials {
            endpoint: self.session.endpoint.clone(),
            server_id: server_id.clone(),
            user_id: user_id.clone(),
            session_id: self.session.session_id.clone(),
            token: self.session.token.clone(),
        };

        let manager = Arc::new(VoiceConnectionManager::new(
            credentials,
            self.dave.clone(),
            self.transport_crypto.clone(),
        ));

        let udp_ref = self.udp.clone();
        let ssrc_ref = self.ssrc.clone();

        manager.set_on_fresh_identify(move |new_ssrc, new_udp| {
            info!("Adapter: Session established, new SSRC={}, UDP socket created", new_ssrc);
            ssrc_ref.store(new_ssrc, std::sync::atomic::Ordering::SeqCst);
            let udp_ref = udp_ref.clone();
            tokio::spawn(async move {
                let mut guard = udp_ref.write().await;
                *guard = Some(new_udp);
            });
        }).await;

        self.manager = Some(manager.clone());
        let mut rx = manager.state_receiver();

        // Spawn manager background task
        let manager_clone = manager.clone();
        tokio::spawn(async move {
            if let Err(e) = manager_clone.run_gateway_loop().await {
                error!("VoiceConnectionManager stopped with error: {}", e);
            }
        });

        // Wait until Connected or Failed
        loop {
            // we must wait for state change
            let _ = rx.changed().await;
            let current = *rx.borrow();
            if current == ConnectionState::Connected {
                info!("KizunaVoice Adapter fully connected!");
                return Ok(());
            } else if current == ConnectionState::Failed {
                return Err("Failed to connect to Discord Voice Gateway".into());
            }
        }
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
        let udp_ref = self.udp.clone();
        let dave = self.dave.clone();
        let transport_crypto = self.transport_crypto.clone();
        let ssrc_ref = self.ssrc.clone();
        let sequence = self.sequence.clone();
        let timestamp = self.timestamp.clone();

        tokio::spawn(async move {
            let encoder = std::sync::Arc::new(tokio::sync::Mutex::new(OpusEncoder::new().unwrap()));
            scheduler
                .run(cmd_rx, event_tx, |frame| {
                    let udp_ref = udp_ref.clone();
                    let dave = dave.clone();
                    let transport_crypto = transport_crypto.clone();
                    let sender_id_clone = sender_id.clone();
                    let enc_clone = encoder.clone();
                    let seq_atomic = sequence.clone();
                    let ts_atomic = timestamp.clone();
                    let current_ssrc = ssrc_ref.load(std::sync::atomic::Ordering::SeqCst);

                    async move {
                        let cur_seq = seq_atomic.fetch_add(1, std::sync::atomic::Ordering::SeqCst).wrapping_add(1);
                        let cur_ts = ts_atomic.fetch_add(960, std::sync::atomic::Ordering::SeqCst).wrapping_add(960);

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

                        let header = RtpHeader::new(cur_seq, cur_ts, current_ssrc);
                        let mut header_buf = Vec::new();
                        header.write_to(&mut header_buf).unwrap();

                        let packet_payload = {
                            let mut dave_guard = dave.lock().await;
                            if dave_guard.is_active() {
                                match dave_guard.encrypt_frame(
                                    &sender_id_clone,
                                    &opus_data,
                                    cur_seq as u32,
                                    &header_buf,
                                ) {
                                    Ok(encrypted) => encrypted,
                                    Err(_) => opus_data,
                                }
                            } else {
                                opus_data
                            }
                        };

                        // Send if UDP is connected
                        let udp_guard = udp_ref.read().await;
                        if let Some(ref current_udp) = *udp_guard {
                            let mut tc_guard = transport_crypto.lock().await;
                            if let Some(ref mut tc) = *tc_guard {
                                if let Ok(encrypted_packet) = tc.encrypt_rtp_packet(&header_buf, &packet_payload) {
                                    let _ = current_udp.send_packet(&encrypted_packet).await;
                                }
                            } else {
                                let mut packet = header_buf;
                                packet.extend(packet_payload);
                                let _ = current_udp.send_packet(&packet).await;
                            }
                        }
                    }
                })
                .await;
        });

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
