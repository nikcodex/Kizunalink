use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::time::Duration;
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio::time;
use tokio_tungstenite::{
    connect_async, tungstenite::protocol::Message, MaybeTlsStream, WebSocketStream,
};
use tracing::{debug, error, info, warn};

use super::payload::{Identify, Ready, SessionDescription, VoicePayload};
use crate::error::{Error, Result};

#[derive(Debug)]
pub enum GatewayEvent {
    Ready(Ready),
    SessionDescription(SessionDescription),
    Resumed,
    HeartbeatAck,
    Hello(f64),
    Unknown(VoicePayload),
}

pub struct VoiceGatewayClient {
    ws_stream: WebSocketStream<MaybeTlsStream<TcpStream>>,
}

impl VoiceGatewayClient {
    pub async fn connect(endpoint: &str) -> Result<Self> {
        let url = format!("wss://{}?v=4", endpoint.trim_end_matches(":80"));
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
        };

        let payload = VoicePayload {
            op: 0,
            d: serde_json::to_value(identify).unwrap(),
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

    pub async fn send_heartbeat(&mut self, nonce: u64) -> Result<()> {
        let payload = VoicePayload {
            op: 3,
            d: json!(nonce),
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
