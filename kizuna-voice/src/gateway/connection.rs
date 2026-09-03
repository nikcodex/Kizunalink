use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::net::TcpStream;
use tokio_tungstenite::{
    connect_async, tungstenite::protocol::Message, MaybeTlsStream, WebSocketStream,
};
use tracing::{debug, info};

use super::payload::{Identify, Ready, SessionDescription, VoicePayload};
use crate::dave::protocol::{DaveClientMessage, DaveGatewayMessage};
use crate::error::{Error, Result};

#[derive(Debug)]
pub enum GatewayEvent {
    Ready(Ready),
    SessionDescription(SessionDescription),
    Resumed,
    HeartbeatAck,
    Hello(f64),
    DaveMessage(DaveGatewayMessage),
    Unknown(VoicePayload),
}

pub struct VoiceGatewayClient {
    ws_stream: WebSocketStream<MaybeTlsStream<TcpStream>>,
}


fn parse_bytes(val: Option<&serde_json::Value>) -> Vec<u8> {
    if let Some(v) = val {
        if let Some(s) = v.as_str() {
            // First try base64
            use base64::{Engine as _, engine::general_purpose::STANDARD};
            if let Ok(b) = STANDARD.decode(s) {
                return b;
            }
            // Then try hex
            if s.len() % 2 == 0 {
                let mut decoded = Vec::new();
                for i in (0..s.len()).step_by(2) {
                    if let Ok(byte) = u8::from_str_radix(&s[i..i+2], 16) {
                        decoded.push(byte);
                    } else {
                        break;
                    }
                }
                if decoded.len() == s.len() / 2 {
                    return decoded;
                }
            }
            // fallback: string bytes
            return s.as_bytes().to_vec();
        } else if let Some(arr) = v.as_array() {
            return arr.iter().filter_map(|x| x.as_u64().map(|n| n as u8)).collect();
        }
    }
    vec![]
}

impl VoiceGatewayClient {
    pub async fn connect(endpoint: &str) -> Result<Self> {
        let clean_endpoint = endpoint.strip_suffix(":80").unwrap_or(endpoint);
        let url = if clean_endpoint.starts_with("ws://") || clean_endpoint.starts_with("wss://") {
            if clean_endpoint.contains('?') {
                clean_endpoint.to_string()
            } else if clean_endpoint.ends_with('/') {
                format!("{}?v=8", clean_endpoint)
            } else {
                format!("{}/?v=8", clean_endpoint)
            }
        } else if clean_endpoint.starts_with("127.0.0.1") || clean_endpoint.starts_with("localhost") {
            let host_port = clean_endpoint.trim_end_matches('/');
            format!("ws://{}/?v=8", host_port)
        } else {
            let host_port = clean_endpoint.trim_end_matches('/');
            format!("wss://{}/?v=8", host_port)
        };
        info!("Connecting to voice gateway: {}", url);
        let (ws_stream, _) = connect_async(&url)
            .await
            .map_err(|e| Error::Gateway(e.to_string()))?;

        Ok(Self { ws_stream })
    }

    pub async fn send_identify(
        &mut self,
        server_id: &str,
        user_id: &str,
        session_id: &str,
        token: &str,
    ) -> Result<()> {
        let identify = Identify {
            server_id: server_id.to_string(),
            user_id: user_id.to_string(),
            session_id: session_id.to_string(),
            token: token.to_string(),
            max_dave_protocol_version: Some(1),
        };

        let payload = VoicePayload {
            op: 0,
            d: serde_json::to_value(identify).unwrap(),
        };

        self.send_payload(&payload).await
    }

    pub async fn send_resume(
        &mut self,
        server_id: &str,
        session_id: &str,
        token: &str,
    ) -> Result<()> {
        let resume = super::payload::Resume {
            server_id: server_id.to_string(),
            session_id: session_id.to_string(),
            token: token.to_string(),
        };

        let payload = VoicePayload {
            op: 7,
            d: serde_json::to_value(resume).unwrap(),
        };

        self.send_payload(&payload).await
    }

    pub async fn send_select_protocol(
        &mut self,
        protocol: &str,
        address: &str,
        port: u16,
        mode: &str,
    ) -> Result<()> {
        let data = json!({
            "protocol": protocol,
            "data": {
                "address": address,
                "port": port,
                "mode": mode
            }
        });

        let payload = VoicePayload { op: 1, d: data };
        self.send_payload(&payload).await
    }

    pub async fn send_speaking(&mut self, speaking: bool, ssrc: u32) -> Result<()> {
        // Bitmask: 1 = Microphone, 2 = Soundshare (High-Quality Stereo Audio Stream), 4 = Priority Speaker.
        // For music streaming, sending (1 | 2) = 3 signals Discord's media servers to optimize jitter buffers
        // and audio packet delivery for stereo music rather than mono speech.
        let bitmask = if speaking { 1 | 2 } else { 0 };
        let data = json!({
            "speaking": bitmask,
            "delay": 0,
            "ssrc": ssrc
        });
        let payload = VoicePayload { op: 5, d: data };
        self.send_payload(&payload).await
    }

    pub async fn send_heartbeat(&mut self, nonce: u64) -> Result<()> {
        let payload = VoicePayload {
            op: 3,
            d: json!({ "t": nonce }),
        };
        self.send_payload(&payload).await
    }

    pub async fn send_dave_message(&mut self, message: DaveClientMessage) -> Result<()> {
        let payload = match message {
            DaveClientMessage::KeyPackage(kp) => VoicePayload {
                op: 26,
                d: json!({"key_package": kp}),
            },
            DaveClientMessage::MlsMessage(mls) => VoicePayload {
                op: 24, // Assuming 24 is proposal
                d: json!({"data": mls}),
            },
        };
        self.send_payload(&payload).await
    }

    pub async fn send_payload(&mut self, payload: &VoicePayload) -> Result<()> {
        let msg = serde_json::to_string(payload).unwrap();
        debug!("Sending payload: {}", msg);
        self.ws_stream
            .send(Message::Text(msg))
            .await
            .map_err(|e| Error::Gateway(e.to_string()))
    }

    pub async fn receive_event(&mut self) -> Result<GatewayEvent> {
        while let Some(msg) = self.ws_stream.next().await {
            let msg = msg.map_err(|e| Error::Gateway(e.to_string()))?;
            match msg {
                Message::Text(text) => {
                    debug!("Received payload: {}", text);
                    let payload: VoicePayload =
                        serde_json::from_str(&text).map_err(|e| Error::Gateway(e.to_string()))?;

                    return match payload.op {
                        2 => {
                            let ready: Ready = serde_json::from_value(payload.d)
                                .map_err(|e| Error::Gateway(e.to_string()))?;
                            Ok(GatewayEvent::Ready(ready))
                        }
                        4 => {
                            let sd: SessionDescription = serde_json::from_value(payload.d)
                                .map_err(|e| Error::Gateway(e.to_string()))?;
                            Ok(GatewayEvent::SessionDescription(sd))
                        }
                        6 => Ok(GatewayEvent::HeartbeatAck),
                        8 => {
                            let interval = payload
                                .d
                                .get("heartbeat_interval")
                                .and_then(Value::as_f64)
                                .unwrap_or(0.0);
                            Ok(GatewayEvent::Hello(interval))
                        }
                        9 => Ok(GatewayEvent::Resumed),
                        21 => {
                            let transition_id = payload
                                .d
                                .get("transition_id")
                                .and_then(Value::as_u64)
                                .unwrap_or(0) as u8;
                            let protocol_version = payload
                                .d
                                .get("protocol_version")
                                .and_then(Value::as_u64)
                                .unwrap_or(0)
                                as u32;
                            Ok(GatewayEvent::DaveMessage(
                                DaveGatewayMessage::PrepareTransition {
                                    transition_id,
                                    protocol_version,
                                },
                            ))
                        }
                        22 => {
                            let transition_id = payload
                                .d
                                .get("transition_id")
                                .and_then(Value::as_u64)
                                .unwrap_or(0) as u8;
                            Ok(GatewayEvent::DaveMessage(
                                DaveGatewayMessage::ExecuteTransition { transition_id },
                            ))
                        }
                        24 => {
                            let epoch_id = payload
                                .d
                                .get("epoch_id")
                                .and_then(Value::as_u64)
                                .unwrap_or(0);
                            Ok(GatewayEvent::DaveMessage(
                                DaveGatewayMessage::PrepareEpoch { epoch_id },
                            ))
                        }
                        25 => {
                            let credential = parse_bytes(payload.d.get("credential"));
                            let signature_key = parse_bytes(payload.d.get("signature_key"));
                            Ok(GatewayEvent::DaveMessage(
                                DaveGatewayMessage::MlsExternalSenderPackage {
                                    credential,
                                    signature_key,
                                },
                            ))
                        }
                        26 => {
                            let key_package = parse_bytes(payload.d.get("key_package"));
                            Ok(GatewayEvent::DaveMessage(
                                DaveGatewayMessage::MlsKeyPackage { key_package },
                            ))
                        }
                        _ => Ok(GatewayEvent::Unknown(payload)),
                    };
                }
                Message::Close(cf) => {
                    return Err(Error::Gateway(format!("WebSocket closed: {:?}", cf)));
                }
                _ => continue,
            }
        }
        Err(Error::Gateway("Connection terminated unexpectedly".into()))
    }
}
