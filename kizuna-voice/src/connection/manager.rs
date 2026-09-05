use crate::connection::state::ConnectionState;
use crate::dave::protocol::DaveSession;
use crate::gateway::connection::{GatewayEvent, VoiceGatewayClient};
use crate::transport::crypto::TransportCrypto;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{watch, Mutex, Notify};
use tracing;

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
    /// Set while a connection cycle reaches `Connected`. `run_gateway_loop` uses
    /// it to reset the retry budget after a connection that actually worked, so
    /// a long-lived session does not permanently exhaust `max_retries` over many
    /// transient drops.
    cycle_connected: Arc<AtomicBool>,
    /// Send time of the heartbeat that is currently awaiting an ack. Used to
    /// turn Discord's `HEARTBEAT_ACK` into a real round-trip measurement.
    heartbeat_sent: Arc<std::sync::Mutex<Option<std::time::Instant>>>,
    /// SSRC the voice gateway assigned in READY. Opcode 5 (SPEAKING) identifies
    /// the sender by SSRC, so the re-assert sent after SESSION_DESCRIPTION has to
    /// reuse it — a placeholder such as `0` describes nobody.
    ssrc: Arc<AtomicU32>,
    /// Last measured voice-gateway RTT in milliseconds, or `-1` while unknown.
    /// Reported as `playerUpdate.state.ping` (Lavalink v4: `-1` if not
    /// connected), which used to be a hardcoded `12`.
    ping_ms: Arc<AtomicI64>,
    #[allow(clippy::type_complexity)]
    on_fresh_identify:
        Arc<Mutex<Option<Box<dyn FnMut(u32, Arc<crate::transport::VoiceUdp>) + Send + Sync>>>>,
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
            cycle_connected: Arc::new(AtomicBool::new(false)),
            heartbeat_sent: Arc::new(std::sync::Mutex::new(None)),
            ping_ms: Arc::new(AtomicI64::new(-1)),
            ssrc: Arc::new(AtomicU32::new(0)),
            on_fresh_identify: Arc::new(Mutex::new(None)),
        }
    }

    /// Record the SSRC assigned by the gateway in READY.
    fn note_ready_ssrc(&self, ssrc: u32) {
        self.ssrc.store(ssrc, Ordering::Relaxed);
    }

    /// SSRC to report in SPEAKING (opcode 5) payloads.
    fn current_ssrc(&self) -> u32 {
        self.ssrc.load(Ordering::Relaxed)
    }

    /// Mark the current connection cycle as having reached `Connected`.
    async fn mark_cycle_connected(&self) {
        self.cycle_connected.store(true, Ordering::SeqCst);
        self.set_state(ConnectionState::Connected).await;
    }

    /// Lock the heartbeat send timestamp. Poisoning is recovered rather than
    /// propagated: a panic while noting a heartbeat must not take down the
    /// connection manager on the next one.
    fn heartbeat_sent_guard(&self) -> std::sync::MutexGuard<'_, Option<std::time::Instant>> {
        self.heartbeat_sent
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    }

    /// Remember when a heartbeat went out so its ack yields an RTT.
    fn note_heartbeat_sent(&self) {
        *self.heartbeat_sent_guard() = Some(std::time::Instant::now());
    }

    /// Turn a `HEARTBEAT_ACK` into the voice-gateway round-trip time.
    ///
    /// An ack that does not match a recorded send is ignored rather than
    /// reporting a bogus latency.
    fn note_heartbeat_ack(&self) {
        let sent = self.heartbeat_sent_guard().take();
        if let Some(sent) = sent {
            let rtt = sent.elapsed().as_millis() as i64;
            self.ping_ms.store(rtt, Ordering::Relaxed);
        }
    }

    /// Voice-gateway RTT in milliseconds, or `-1` when it has not been
    /// measured (no connection, or no heartbeat ack yet).
    pub fn ping_ms(&self) -> i64 {
        self.ping_ms.load(Ordering::Relaxed)
    }

    /// Shared handle to the measured RTT, for consumers that cannot await a
    /// lock (the synchronous player snapshot reads this on every response).
    pub fn ping_handle(&self) -> Arc<AtomicI64> {
        self.ping_ms.clone()
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
                    tracing::info!("VoiceConnectionManager: shutdown requested, exiting");
                    self.set_state(ConnectionState::Disconnected).await;
                    return Ok(());
                }
            }

            if attempt > 0 {
                self.set_state(ConnectionState::Reconnecting).await;
                tracing::info!(
                    "VoiceConnectionManager: reconnect attempt {}/{}",
                    attempt,
                    self.config.max_retries
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
                    tracing::warn!(
                        "VoiceConnectionManager: gateway loop ended without error, reconnecting"
                    );
                }
                Err(e) => {
                    tracing::error!("VoiceConnectionManager: gateway error: {}", e);
                }
            }

            // A cycle that actually reached Connected earns a fresh retry budget.
            // Without this, eight transient drops spread over the lifetime of a
            // player permanently disabled voice reconnects for it.
            if self.cycle_connected.swap(false, Ordering::SeqCst) {
                attempt = 0;
                delay = self.config.base_delay;
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
                tracing::error!(
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

            tracing::info!(
                "VoiceConnectionManager: waiting {:?} before reconnect attempt {}",
                sleep_duration,
                attempt
            );

            tokio::select! {
                _ = tokio::time::sleep(sleep_duration) => {}
                _ = self.shutdown.notified() => {
                    tracing::info!("VoiceConnectionManager: shutdown during backoff");
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
        self.cycle_connected.store(false, Ordering::SeqCst);
        // A new cycle has no measurement yet; never report the previous
        // session's latency as if it were current.
        self.ping_ms.store(-1, Ordering::Relaxed);
        *self.heartbeat_sent_guard() = None;

        let mut gw = VoiceGatewayClient::connect(&self.credentials.endpoint)
            .await
            .map_err(|e| format!("Gateway connect failed: {}", e))?;

        // Wait for Hello
        let hello = tokio::time::timeout(Duration::from_secs(5), gw.receive_event())
            .await
            .map_err(|_| "Timeout waiting for Hello")?
            .map_err(|e| format!("Failed to receive Hello: {}", e))?;

        let heartbeat_interval = match hello {
            GatewayEvent::Hello(interval) => interval,
            _ => return Err("Expected Hello event".into()),
        };
        // `tokio::time::interval` panics on a zero duration, and a panic aborts
        // the whole process under the release profile. Discord normally sends
        // ~41250 ms; clamp anything unusable (0, negative, NaN, infinite) into a
        // safe range before it reaches a timer.
        let heartbeat_interval = if heartbeat_interval.is_finite() {
            heartbeat_interval.clamp(1000.0, 300_000.0)
        } else {
            41_250.0
        };

        tracing::info!(
            "VoiceConnectionManager: heartbeat interval is {} ms",
            heartbeat_interval
        );

        let has_session = {
            let hs = self.has_established_session.lock().await;
            *hs
        };

        if is_reconnect && has_session {
            // Attempt Resume (Op 7)
            tracing::info!("VoiceConnectionManager: attempting Resume");
            gw.send_resume(
                &self.credentials.server_id,
                &self.credentials.session_id,
                &self.credentials.token,
            )
            .await
            .map_err(|e| format!("Resume send failed: {}", e))?;

            // Wait for Resumed (Op 9) with tolerant event loop
            let mut resume_success = false;
            let resume_deadline = tokio::time::Instant::now() + Duration::from_secs(5);
            while tokio::time::Instant::now() < resume_deadline {
                match tokio::time::timeout(Duration::from_millis(1500), gw.receive_event()).await {
                    Ok(Ok(GatewayEvent::Resumed)) => {
                        tracing::info!("VoiceConnectionManager: Resume successful");
                        self.mark_cycle_connected().await;
                        resume_success = true;
                        break;
                    }
                    Ok(Ok(GatewayEvent::HeartbeatAck)) => {
                        self.note_heartbeat_ack();
                        continue;
                    }
                    Ok(Ok(GatewayEvent::DaveMessage(dave_msg))) => {
                        let mut dave = self.dave.lock().await;
                        let out_msgs = dave.handle_gateway_message(dave_msg);
                        drop(dave);
                        for out in out_msgs {
                            let _ = gw.send_dave_message(out).await;
                        }
                        continue;
                    }
                    Ok(Ok(other)) => {
                        tracing::warn!(
                            "VoiceConnectionManager: Resume got unexpected event: {:?}",
                            other
                        );
                        break;
                    }
                    Ok(Err(e)) => {
                        tracing::error!(
                            "VoiceConnectionManager: Gateway error during resume: {}",
                            e
                        );
                        self.set_has_session(false).await;
                        return Err(format!("Resume failed: {}", e));
                    }
                    Err(_) => {
                        continue;
                    }
                }
            }

            if !resume_success {
                tracing::warn!(
                    "VoiceConnectionManager: Resume not confirmed, falling back to fresh Identify"
                );
                self.set_has_session(false).await;
                self.do_fresh_identify(&mut gw, heartbeat_interval).await?;
            }
        } else {
            // Fresh Identify
            self.do_fresh_identify(&mut gw, heartbeat_interval).await?;
        }

        // Run the event loop with heartbeats
        self.event_loop(&mut gw, heartbeat_interval).await
    }

    /// Perform a fresh Identify handshake
    async fn do_fresh_identify(
        &self,
        gw: &mut VoiceGatewayClient,
        heartbeat_interval: f64,
    ) -> Result<(), String> {
        gw.send_identify(
            &self.credentials.server_id,
            &self.credentials.user_id,
            &self.credentials.session_id,
            &self.credentials.token,
        )
        .await
        .map_err(|e| format!("Identify failed: {}", e))?;

        // Wait for Ready
        let ready = loop {
            // Include heartbeat while waiting for Ready
            let interval = Duration::from_millis(heartbeat_interval as u64);
            match tokio::time::timeout(interval, gw.receive_event()).await {
                Ok(Ok(GatewayEvent::Ready(ready))) => break ready,
                Ok(Ok(GatewayEvent::DaveMessage(dave_msg))) => {
                    let mut dave = self.dave.lock().await;
                    let responses = dave.handle_gateway_message(dave_msg);
                    for msg in responses {
                        let _ = gw.send_dave_message(msg).await;
                    }
                }
                Ok(Ok(GatewayEvent::HeartbeatAck)) => {
                    self.note_heartbeat_ack();
                    continue;
                }
                Ok(Ok(_)) => continue,
                Ok(Err(e)) => return Err(format!("Waiting for Ready failed: {}", e)),
                Err(_) => {
                    // Timeout -> send heartbeat
                    let nonce = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_millis() as u64;
                    self.note_heartbeat_sent();
                    let _ = gw.send_heartbeat(nonce).await;
                }
            }
        };

        self.note_ready_ssrc(ready.ssrc);

        let (udp, external_ip, external_port) =
            crate::transport::VoiceUdp::bind_and_discover(&ready.ip, ready.port, ready.ssrc)
                .await
                .map_err(|e| e.to_string())?;

        let udp_arc = Arc::new(udp);

        gw.send_select_protocol(
            "udp",
            &external_ip,
            external_port,
            "aead_aes256_gcm_rtpsize",
        )
        .await
        .map_err(|e| format!("Select Protocol failed: {}", e))?;

        let _ = gw.send_speaking(true, ready.ssrc).await;

        let mut cb = self.on_fresh_identify.lock().await;
        if let Some(cb_fn) = cb.as_mut() {
            cb_fn(ready.ssrc, udp_arc);
        }

        self.mark_cycle_connected().await;
        {
            let mut hs = self.has_established_session.lock().await;
            *hs = true;
        }

        Ok(())
    }

    /// Main event loop — processes gateway events until disconnect or shutdown.
    async fn event_loop(
        &self,
        gw: &mut VoiceGatewayClient,
        heartbeat_interval: f64,
    ) -> Result<(), String> {
        let interval_duration = Duration::from_millis(heartbeat_interval as u64);
        // Add a bit of buffer so we send slightly faster than requested
        let interval_duration = if interval_duration.as_secs() > 1 {
            interval_duration - Duration::from_millis(500)
        } else {
            interval_duration
        };
        let mut heartbeat_ticker = tokio::time::interval(interval_duration);
        // The first tick fires immediately; consume it so we wait for the first interval
        heartbeat_ticker.tick().await;

        loop {
            tokio::select! {
                _ = self.shutdown.notified() => {
                    tracing::info!("VoiceConnectionManager: shutdown in event loop");
                    self.set_state(ConnectionState::Disconnected).await;
                    return Ok(());
                }
                _ = heartbeat_ticker.tick() => {
                    let nonce = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_millis() as u64;
                    self.note_heartbeat_sent();
                    if let Err(e) = gw.send_heartbeat(nonce).await {
                        tracing::error!("Failed to send heartbeat: {}", e);
                        return Err(format!("Failed to send heartbeat: {}", e));
                    }
                }
                event_result = gw.receive_event() => {
                    match event_result {
                        Ok(GatewayEvent::DaveMessage(dave_msg)) => {
                            let mut dave = self.dave.lock().await;
                            let responses = dave.handle_gateway_message(dave_msg);
                            for msg in responses {
                                let _ = gw.send_dave_message(msg).await;
                            }
                        }
                        Ok(GatewayEvent::SessionDescription(sd)) => {
                            tracing::info!("VoiceConnectionManager: received SessionDescription");
                            match TransportCrypto::new(&sd.secret_key) {
                                Ok(crypto) => {
                                    let mut tc = self.transport_crypto.lock().await;
                                    *tc = Some(crypto);
                                }
                                Err(e) => {
                                    tracing::error!("Failed to setup transport crypto: {}", e);
                                }
                            }
                            let ssrc = self.current_ssrc();
                            let _ = gw.send_speaking(true, ssrc).await;
                        }
                        Ok(GatewayEvent::HeartbeatAck) => {
                            // Heartbeat acknowledged — connection is healthy,
                            // and the round trip gives us the reported ping.
                            self.note_heartbeat_ack();
                        }
                        Ok(GatewayEvent::Resumed) => {
                            tracing::info!("VoiceConnectionManager: resumed");
                        }
                        Ok(_) => {}
                        Err(e) => {
                            tracing::error!("VoiceConnectionManager: gateway error in event loop: {}", e);
                            self.set_state(ConnectionState::Reconnecting).await;
                            return Err(format!("Gateway disconnected: {}", e));
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_manager() -> VoiceConnectionManager {
        let credentials = VoiceCredentials {
            endpoint: "wss://example.test".to_string(),
            server_id: "111222333444555".to_string(),
            user_id: "999888777666555".to_string(),
            session_id: "session".to_string(),
            token: "token".to_string(),
        };
        let dave = Arc::new(Mutex::new(DaveSession::new("111222333444555".to_string())));
        let crypto = Arc::new(Mutex::new(None));
        VoiceConnectionManager::new(credentials, dave, crypto)
    }

    /// Opcode 5 identifies the sender by SSRC, so the value from READY has to be
    /// reused for the SPEAKING re-assert after SESSION_DESCRIPTION; it used to
    /// send `0`, which does not describe this session.
    #[test]
    fn ssrc_from_ready_is_remembered() {
        let manager = test_manager();
        assert_eq!(manager.current_ssrc(), 0);

        manager.note_ready_ssrc(0xC0FFEE);
        assert_eq!(manager.current_ssrc(), 0xC0FFEE);

        // A reconnect with a new READY replaces the SSRC.
        manager.note_ready_ssrc(42);
        assert_eq!(manager.current_ssrc(), 42);
    }

    #[test]
    fn ping_is_unknown_until_a_heartbeat_is_acked() {
        let manager = test_manager();
        assert_eq!(manager.ping_ms(), -1);

        // An ack with no recorded send must not invent a latency.
        manager.note_heartbeat_ack();
        assert_eq!(manager.ping_ms(), -1);
    }

    #[test]
    fn ping_measures_heartbeat_round_trip() {
        let manager = test_manager();
        manager.note_heartbeat_sent();
        std::thread::sleep(Duration::from_millis(5));
        manager.note_heartbeat_ack();

        let ping = manager.ping_ms();
        assert!(ping >= 5, "measured ping was {ping} ms");
        assert!(ping < 5_000, "measured ping was {ping} ms");

        // The send timestamp is consumed by the ack, so a stray second ack
        // cannot overwrite the measurement.
        manager.note_heartbeat_ack();
        assert_eq!(manager.ping_ms(), ping);
    }
}
