use crate::connection::state::ConnectionState;
use crate::dave::protocol::{DaveClientMessage, DaveGatewayMessage, DaveSession};
use crate::gateway::connection::{GatewayEvent, VoiceGatewayClient};
use crate::gateway::payload::SessionDescription;
use crate::transport::crypto::TransportCrypto;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{watch, Mutex, Notify};
use tracing::{error, info, warn};

/// Configuration for reconnect behavior
#[derive(Clone, Debug)]
pub struct ReconnectConfig {
    pub max_retries: u32,
    pub base_delay: Duration,
    pub max_delay: Duration,
    pub jitter_factor: f64,
}

impl Default for ReconnectConfig {
    fn default() -> Self {
        Self {
            max_retries: 8,
            base_delay: Duration::from_millis(500),
            max_delay: Duration::from_secs(30),
            jitter_factor: 0.25,
        }
    }
}

/// Credentials needed to connect/resume a voice session
#[derive(Clone, Debug)]
pub struct VoiceCredentials {
    pub endpoint: String,
    pub server_id: String,
    pub user_id: String,
    pub session_id: String,
    pub token: String,
}

/// Events emitted by the connection manager
#[derive(Clone, Debug)]
pub enum ConnectionEvent {
    Connected,
    Disconnected,
    Reconnecting { attempt: u32 },
    Reconnected,
    ReconnectFailed,
    SessionDescriptionReceived(Vec<u8>), // secret_key
}

/// Manages the voice gateway connection lifecycle including automatic reconnect/resume.
pub struct VoiceConnectionManager {
    credentials: VoiceCredentials,
    config: ReconnectConfig,
    state: Arc<Mutex<ConnectionState>>,
    state_tx: watch::Sender<ConnectionState>,
    state_rx: watch::Receiver<ConnectionState>,
    dave: Arc<Mutex<DaveSession>>,
    transport_crypto: Arc<Mutex<Option<TransportCrypto>>>,
    shutdown: Arc<Notify>,
    is_shutdown: Arc<Mutex<bool>>,
    /// Whether we have successfully completed a full handshake at least once.
    /// If true, we can attempt Resume (Op 7) instead of fresh Identify (Op 0).
    has_established_session: Arc<Mutex<bool>>,
    #[allow(clippy::type_complexity)]
    on_fresh_identify: Arc<Mutex<Option<Box<dyn FnMut(u32, Arc<crate::transport::VoiceUdp>) + Send + Sync>>>>,
}

impl VoiceConnectionManager {
    pub fn new(
        credentials: VoiceCredentials,
        dave: Arc<Mutex<DaveSession>>,
        transport_crypto: Arc<Mutex<Option<TransportCrypto>>>,
    ) -> Self {
        let (state_tx, state_rx) = watch::channel(ConnectionState::Disconnected);
        Self {
            credentials,
            config: ReconnectConfig::default(),
            state: Arc::new(Mutex::new(ConnectionState::Disconnected)),
            state_tx,
            state_rx,
            dave,
            transport_crypto,
            shutdown: Arc::new(Notify::new()),
            is_shutdown: Arc::new(Mutex::new(false)),
            has_established_session: Arc::new(Mutex::new(false)),
            on_fresh_identify: Arc::new(Mutex::new(None)),
        }
    }

    pub async fn set_has_session(&self, value: bool) {
        let mut hs = self.has_established_session.lock().await;
        *hs = value;
    }

    pub async fn set_on_fresh_identify<F>(&self, cb: F)
    where
        F: FnMut(u32, Arc<crate::transport::VoiceUdp>) + Send + Sync + 'static,
    {
        let mut on_id = self.on_fresh_identify.lock().await;
        *on_id = Some(Box::new(cb));
    }

    pub fn with_config(mut self, config: ReconnectConfig) -> Self {
        self.config = config;
        self
    }

    pub fn state_receiver(&self) -> watch::Receiver<ConnectionState> {
        self.state_rx.clone()
    }

    /// Request shutdown of the connection manager.
    /// This will cancel any ongoing reconnect attempts.
    pub async fn shutdown(&self) {
        let mut shut = self.is_shutdown.lock().await;
        *shut = true;
        self.shutdown.notify_waiters();
    }

    async fn set_state(&self, new_state: ConnectionState) {
        let mut s = self.state.lock().await;
        *s = new_state;
        let _ = self.state_tx.send(new_state);
    }

    /// Run the gateway event loop with automatic reconnect.
    /// This is the main entry point — spawn this as a tokio task.
    ///
    /// Returns the SSRC, external IP, and external port from the initial handshake
    /// so the caller can set up UDP and Select Protocol.
    /// After that, this function manages the gateway event loop internally.
    pub async fn run_gateway_loop(&self) -> Result<(), String> {
        let mut attempt: u32 = 0;
        let mut delay = self.config.base_delay;

        loop {
            // Check shutdown
            {
                let shut = self.is_shutdown.lock().await;
                if *shut {
                    info!("VoiceConnectionManager: shutdown requested, exiting");
                    self.set_state(ConnectionState::Disconnected).await;
                    return Ok(());
                }
            }

            if attempt > 0 {
                self.set_state(ConnectionState::Reconnecting).await;
                info!(
                    "VoiceConnectionManager: reconnect attempt {}/{}",
                    attempt, self.config.max_retries
                );
            } else {
                self.set_state(ConnectionState::Connecting).await;
            }

            match self.try_connect_and_run(attempt > 0).await {
                Ok(()) => {
                    // Graceful shutdown or clean exit
                    let shut = self.is_shutdown.lock().await;
                    if *shut {
                        return Ok(());
                    }
                    // Connection ended unexpectedly but without error — treat as disconnect
                    warn!("VoiceConnectionManager: gateway loop ended without error, reconnecting");
                }
                Err(e) => {
                    warn!("VoiceConnectionManager: gateway error: {}", e);
                }
            }

            // Check shutdown before sleeping
            {
                let shut = self.is_shutdown.lock().await;
                if *shut {
                    return Ok(());
                }
            }

            attempt += 1;
            if attempt > self.config.max_retries {
                error!(
                    "VoiceConnectionManager: max retries ({}) exceeded",
                    self.config.max_retries
                );
                self.set_state(ConnectionState::Failed).await;
                return Err("Max reconnect retries exceeded".into());
            }

            // Exponential backoff with jitter
            let jitter = {
                // Simple deterministic jitter: vary by attempt number
                let jitter_ms = (delay.as_millis() as f64 * self.config.jitter_factor) as u64;
                Duration::from_millis(jitter_ms.wrapping_mul(attempt as u64 + 1) % (jitter_ms + 1))
            };
            let sleep_duration = delay.min(self.config.max_delay) + jitter;

            info!(
                "VoiceConnectionManager: waiting {:?} before reconnect attempt {}",
                sleep_duration, attempt
            );

            tokio::select! {
                _ = tokio::time::sleep(sleep_duration) => {}
                _ = self.shutdown.notified() => {
                    info!("VoiceConnectionManager: shutdown during backoff");
                    self.set_state(ConnectionState::Disconnected).await;
                    return Ok(());
                }
            }

            // Exponential increase
            delay = (delay * 2).min(self.config.max_delay);
        }
    }

    /// Attempt a single connection + event loop cycle.
    /// If `is_reconnect` is true and we have a previous session, attempt Resume first.
    async fn try_connect_and_run(&self, is_reconnect: bool) -> Result<(), String> {
        let mut gw = VoiceGatewayClient::connect(&self.credentials.endpoint)
            .await
            .map_err(|e| format!("Gateway connect failed: {}", e))?;

        // Wait for Hello
        let hello = gw
            .receive_event()
            .await
            .map_err(|e| format!("Failed to receive Hello: {}", e))?;

        let heartbeat_interval = match hello {
            GatewayEvent::Hello(interval) => interval,
            _ => return Err("Expected Hello event".into()),
        };
        
        info!("VoiceConnectionManager: heartbeat interval is {} ms", heartbeat_interval);

        let has_session = {
            let hs = self.has_established_session.lock().await;
            *hs
        };

        if is_reconnect && has_session {
            // Attempt Resume (Op 7)
            info!("VoiceConnectionManager: attempting Resume");
            gw.send_resume(
                &self.credentials.server_id,
                &self.credentials.session_id,
                &self.credentials.token,
            )
            .await
            .map_err(|e| format!("Resume send failed: {}", e))?;

            // Wait for Resumed (Op 9) or an error
            match gw.receive_event().await {
                Ok(GatewayEvent::Resumed) => {
                    info!("VoiceConnectionManager: Resume successful");
                    self.set_state(ConnectionState::Connected).await;
                }
                Ok(_other) => {
                    // Resume was rejected or we got something else — do fresh Identify
                    warn!("VoiceConnectionManager: Resume rejected, falling back to fresh Identify");
                    return self.do_fresh_identify(&mut gw, heartbeat_interval).await;
                }
                Err(e) => {
                    warn!("VoiceConnectionManager: Resume failed: {}, falling back to fresh Identify", e);
                    // Need a new connection for fresh identify since this one errored
                    return Err(format!("Resume failed: {}", e));
                }
            }
        } else {
            // Fresh Identify
            self.do_fresh_identify(&mut gw, heartbeat_interval).await?;
        }

        // Run the event loop with heartbeats
        self.event_loop(&mut gw, heartbeat_interval).await
    }

    /// Perform a fresh Identify handshake
    async fn do_fresh_identify(&self, gw: &mut VoiceGatewayClient, heartbeat_interval: f64) -> Result<(), String> {
        gw.send_identify(
            &self.credentials.server_id,
            &self.credentials.user_id,
            &self.credentials.session_id,
            &self.credentials.token,
        )
        .await
        .map_err(|e| format!("Identify failed: {}", e))?;

        let mut ready_data = None;

        // Wait for Ready
        loop {
            // Include heartbeat while waiting for Ready
            let interval = Duration::from_millis(heartbeat_interval as u64);
            match tokio::time::timeout(interval, gw.receive_event()).await {
                Ok(Ok(GatewayEvent::Ready(ready))) => {
                    ready_data = Some(ready);
                    break;
                }
                Ok(Ok(GatewayEvent::DaveMessage(dave_msg))) => {
                    let mut dave = self.dave.lock().await;
                    let responses = dave.handle_gateway_message(dave_msg);
                    for msg in responses {
                        let _ = gw.send_dave_message(msg).await;
                    }
                }
                Ok(Ok(_)) => continue,
                Ok(Err(e)) => return Err(format!("Waiting for Ready failed: {}", e)),
                Err(_) => {
                    // Timeout -> send heartbeat
                    let nonce = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_millis() as u64;
                    let _ = gw.send_heartbeat(nonce).await;
                }
            }
        }

        let ready = ready_data.unwrap();

        let (udp, external_ip, external_port) =
            crate::transport::VoiceUdp::bind_and_discover(&ready.ip, ready.port, ready.ssrc)
                .await
                .map_err(|e| e.to_string())?;

        let udp_arc = Arc::new(udp);
        let _ = self.state_tx.send(ConnectionState::Reconnecting); 

        gw.send_select_protocol(
            "udp",
            &external_ip,
            external_port,
            "aead_aes256_gcm_rtpsize",
        )
        .await
        .map_err(|e| format!("Select Protocol failed: {}", e))?;
        
        let mut cb = self.on_fresh_identify.lock().await;
        if let Some(cb_fn) = cb.as_mut() {
            cb_fn(ready.ssrc, udp_arc);
        }

        self.set_state(ConnectionState::Connected).await;
        {
            let mut hs = self.has_established_session.lock().await;
            *hs = true;
        }

        Ok(())
    }

    /// Main event loop — processes gateway events until disconnect or shutdown.
    async fn event_loop(&self, gw: &mut VoiceGatewayClient, heartbeat_interval: f64) -> Result<(), String> {
        let interval_duration = Duration::from_millis(heartbeat_interval as u64);
        // Add a bit of buffer so we send slightly faster than requested
        let interval_duration = if interval_duration.as_secs() > 1 {
            interval_duration - Duration::from_millis(500)
        } else {
            interval_duration
        };

        loop {
            tokio::select! {
                _ = self.shutdown.notified() => {
                    info!("VoiceConnectionManager: shutdown in event loop");
                    self.set_state(ConnectionState::Disconnected).await;
                    return Ok(());
                }
                event_result = tokio::time::timeout(interval_duration, gw.receive_event()) => {
                    match event_result {
                        Ok(Ok(GatewayEvent::DaveMessage(dave_msg))) => {
                            let mut dave = self.dave.lock().await;
                            let responses = dave.handle_gateway_message(dave_msg);
                            for msg in responses {
                                let _ = gw.send_dave_message(msg).await;
                            }
                        }
                        Ok(Ok(GatewayEvent::SessionDescription(sd))) => {
                            info!("VoiceConnectionManager: received SessionDescription");
                            match TransportCrypto::new(&sd.secret_key) {
                                Ok(crypto) => {
                                    let mut tc = self.transport_crypto.lock().await;
                                    *tc = Some(crypto);
                                }
                                Err(e) => {
                                    error!("Failed to setup transport crypto: {}", e);
                                }
                            }
                        }
                        Ok(Ok(GatewayEvent::HeartbeatAck)) => {
                            // Heartbeat acknowledged — connection is healthy
                        }
                        Ok(Ok(GatewayEvent::Resumed)) => {
                            info!("VoiceConnectionManager: resumed");
                        }
                        Ok(Ok(_)) => {}
                        Ok(Err(e)) => {
                            warn!("VoiceConnectionManager: gateway error in event loop: {}", e);
                            self.set_state(ConnectionState::Reconnecting).await;
                            return Err(format!("Gateway disconnected: {}", e));
                        }
                        Err(_) => {
                            // Timeout -> send heartbeat
                            let nonce = std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap_or_default()
                                .as_millis() as u64;
                            if let Err(e) = gw.send_heartbeat(nonce).await {
                                warn!("Failed to send heartbeat: {}", e);
                                return Err(format!("Failed to send heartbeat: {}", e));
                            }
                        }
                    }
                }
            }
        }
    }
}
