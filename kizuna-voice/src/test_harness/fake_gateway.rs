use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::{mpsc, Mutex};
use tokio_tungstenite::tungstenite::protocol::Message;

use crate::gateway::payload::VoicePayload;

#[derive(Debug, Clone)]
pub struct GatewaySessionConfig {
    pub ssrc: u32,
    pub udp_port: u16,
    pub heartbeat_interval: f64,
    pub auto_handshake: bool,
}

impl Default for GatewaySessionConfig {
    fn default() -> Self {
        Self {
            ssrc: 123456,
            udp_port: 0,
            heartbeat_interval: 41250.0,
            auto_handshake: true,
        }
    }
}

pub struct FakeVoiceGateway {
    endpoint: String,
    port: u16,
    shutdown_tx: Option<tokio::sync::oneshot::Sender<()>>,
    pub received_payloads: Arc<Mutex<Vec<VoicePayload>>>,
    pub outgoing_tx: Arc<Mutex<Option<mpsc::Sender<VoicePayload>>>>,
    pub client_tasks: Arc<Mutex<Vec<tokio::task::JoinHandle<()>>>>,
}

impl FakeVoiceGateway {
    pub async fn start(config: GatewaySessionConfig) -> Result<Self, String> {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|e| format!("Failed to bind TCP listener: {}", e))?;

        let port = listener
            .local_addr()
            .map_err(|e| format!("Failed to get local addr: {}", e))?
            .port();

        let endpoint = format!("127.0.0.1:{}", port);
        let received_payloads = Arc::new(Mutex::new(Vec::new()));
        let outgoing_tx = Arc::new(Mutex::new(None));
        let client_tasks = Arc::new(Mutex::new(Vec::new()));
        let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel::<()>();

        let received_clone = received_payloads.clone();
        let outgoing_clone = outgoing_tx.clone();
        let tasks_clone = client_tasks.clone();

        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = &mut shutdown_rx => {
                        break;
                    }
                    accept_res = listener.accept() => {
                        if let Ok((stream, _peer_addr)) = accept_res {
                            let received_in_task = received_clone.clone();
                            let outgoing_in_task = outgoing_clone.clone();
                            let tasks_in_task = tasks_clone.clone();
                            let cfg = config.clone();

                            let connection_task = tokio::spawn(async move {
                                if let Ok(ws) = tokio_tungstenite::accept_async(stream).await {
                                    let (mut ws_tx, mut ws_rx) = ws.split();
                                    let (msg_tx, mut msg_rx) = mpsc::channel::<VoicePayload>(32);

                                    {
                                        let mut out = outgoing_in_task.lock().await;
                                        *out = Some(msg_tx.clone());
                                    }

                                    // Send Hello (Op 8)
                                    let hello = json!({
                                        "op": 8,
                                        "d": {
                                            "heartbeat_interval": cfg.heartbeat_interval,
                                            "v": 4
                                        }
                                    });
                                    let _ = ws_tx.send(Message::Text(hello.to_string())).await;

                                    let msg_tx_in_task = msg_tx.clone();

                                    // Spawn sender task
                                    let send_task = tokio::spawn(async move {
                                        while let Some(payload) = msg_rx.recv().await {
                                            let text = serde_json::to_string(&payload).unwrap();
                                            if ws_tx.send(Message::Text(text)).await.is_err() {
                                                break;
                                            }
                                        }
                                    });

                                    struct AbortOnDrop(Option<tokio::task::JoinHandle<()>>);
                                    impl Drop for AbortOnDrop {
                                        fn drop(&mut self) {
                                            if let Some(jh) = self.0.take() {
                                                jh.abort();
                                            }
                                        }
                                    }
                                    let _guard = AbortOnDrop(Some(send_task));

                                    // Process incoming messages
                                    while let Some(msg) = ws_rx.next().await {
                                        if let Ok(Message::Text(text)) = msg {
                                            if let Ok(payload) = serde_json::from_str::<VoicePayload>(&text) {
                                                {
                                                    let mut list = received_in_task.lock().await;
                                                    list.push(VoicePayload {
                                                        op: payload.op,
                                                        d: payload.d.clone(),
                                                    });
                                                }

                                                if cfg.auto_handshake {
                                                    match payload.op {
                                                        0 => {
                                                            let ready = VoicePayload {
                                                                op: 2,
                                                                d: json!({
                                                                    "ssrc": cfg.ssrc,
                                                                    "ip": "127.0.0.1",
                                                                    "port": cfg.udp_port,
                                                                    "modes": ["aead_aes256_gcm_rtpsize"]
                                                                }),
                                                            };
                                                            let _ = msg_tx_in_task.send(ready).await;
                                                        }
                                                        1 => {
                                                            let sd = VoicePayload {
                                                                op: 4,
                                                                d: json!({
                                                                    "mode": "aead_aes256_gcm_rtpsize",
                                                                    "secret_key": vec![1u8; 32]
                                                                }),
                                                            };
                                                            let _ = msg_tx_in_task.send(sd).await;
                                                        }
                                                        3 => {
                                                            let ack = VoicePayload {
                                                                op: 6,
                                                                d: payload.d,
                                                            };
                                                            let _ = msg_tx_in_task.send(ack).await;
                                                        }
                                                        7 => {
                                                            let resumed = VoicePayload {
                                                                op: 9,
                                                                d: json!(null),
                                                            };
                                                            let _ = msg_tx_in_task.send(resumed).await;
                                                        }
                                                        _ => {}
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            });

                            let mut tasks = tasks_in_task.lock().await;
                            tasks.push(connection_task);
                        }
                    }
                }
            }
        });

        Ok(Self {
            endpoint: format!("ws://{}", endpoint),
            port,
            received_payloads,
            outgoing_tx,
            client_tasks,
            shutdown_tx: Some(shutdown_tx),
        })
    }

    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub async fn send_payload(&self, payload: VoicePayload) -> Result<(), String> {
        let out = self.outgoing_tx.lock().await;
        if let Some(tx) = out.as_ref() {
            tx.send(payload)
                .await
                .map_err(|e| format!("Failed to send payload: {}", e))?;
            Ok(())
        } else {
            Err("No active client connection".to_string())
        }
    }

    pub async fn send_dave_prepare_transition(&self, transition_id: u8, protocol_version: u32) -> Result<(), String> {
        self.send_payload(VoicePayload {
            op: 21,
            d: json!({
                "transition_id": transition_id,
                "protocol_version": protocol_version
            }),
        }).await
    }

    pub async fn send_dave_execute_transition(&self, transition_id: u8) -> Result<(), String> {
        self.send_payload(VoicePayload {
            op: 22,
            d: json!({
                "transition_id": transition_id
            }),
        }).await
    }

    pub async fn send_dave_prepare_epoch(&self, epoch_id: u64) -> Result<(), String> {
        self.send_payload(VoicePayload {
            op: 24,
            d: json!({
                "epoch_id": epoch_id
            }),
        }).await
    }

    pub async fn get_received_payloads(&self) -> Vec<VoicePayload> {
        let list = self.received_payloads.lock().await;
        list.clone()
    }

    pub async fn has_received_opcode(&self, op: u8) -> bool {
        let list = self.received_payloads.lock().await;
        list.iter().any(|p| p.op == op)
    }

    pub async fn clear_received(&self) {
        let mut list = self.received_payloads.lock().await;
        list.clear();
    }

    pub async fn drop_clients(&self) {
        let mut tasks = self.client_tasks.lock().await;
        for task in tasks.drain(..) {
            task.abort();
        }
        let mut out = self.outgoing_tx.lock().await;
        *out = None;
    }
}

impl Drop for FakeVoiceGateway {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
        
        let tasks_arc = self.client_tasks.clone();
        tokio::spawn(async move {
            let mut tasks = tasks_arc.lock().await;
            for task in tasks.drain(..) {
                task.abort();
            }
        });
    }
}
