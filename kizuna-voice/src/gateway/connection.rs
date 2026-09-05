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

/// Map any displayable serialisation/transport error onto [`Error::Gateway`].
fn gateway_err<E: std::fmt::Display>(e: E) -> Error {
    Error::Gateway(e.to_string())
}

/// Build the IDENTIFY (opcode 0) payload.
///
/// `max_dave_protocol_version` is deliberately **not** sent. Advertising a DAVE
/// protocol version tells the voice gateway that this client can complete an MLS
/// group key exchange (see the DAVE whitepaper: ciphersuite
/// `DHKEMP256_AES128GCM_SHA256_P256`, key packages whose basic credential is the
/// big-endian snowflake user ID, the `external_senders` group extension, and the
/// client opcodes 23/26/28). [`crate::dave`] implements none of that, so a
/// gateway-driven transition could never complete while we would still start
/// encrypting audio frames with keys no other participant holds. Omitting the
/// field makes the gateway select protocol version 0 (transport-only
/// encryption), which `aead_aes256_gcm_rtpsize` implements correctly.
pub fn identify_payload(
    server_id: &str,
    user_id: &str,
    session_id: &str,
    token: &str,
) -> serde_json::Result<VoicePayload> {
    let identify = Identify {
        server_id: server_id.to_string(),
        user_id: user_id.to_string(),
        session_id: session_id.to_string(),
        token: token.to_string(),
        max_dave_protocol_version: None,
    };

    Ok(VoicePayload {
        op: 0,
        d: serde_json::to_value(identify)?,
    })
}

/// Build the SPEAKING (opcode 5) payload.
///
/// Bitmask: 1 = microphone, 2 = soundshare (stereo music stream), 4 = priority
/// speaker. Music playback sends `1 | 2` so Discord's media servers optimise
/// jitter buffering for stereo audio instead of mono speech. `ssrc` must be the
/// SSRC this session received in READY — the gateway identifies the sender by
/// SSRC, so a placeholder such as `0` makes the update meaningless.
pub fn speaking_payload(speaking: bool, ssrc: u32) -> VoicePayload {
    let bitmask = if speaking { 1 | 2 } else { 0 };
    VoicePayload {
        op: 5,
        d: json!({
            "speaking": bitmask,
            "delay": 0,
            "ssrc": ssrc
        }),
    }
}

fn parse_bytes(val: Option<&serde_json::Value>) -> Vec<u8> {
    if let Some(v) = val {
        if let Some(s) = v.as_str() {
            // First try base64
            use base64::{engine::general_purpose::STANDARD, Engine as _};
            if let Ok(b) = STANDARD.decode(s) {
                return b;
            }
            // Then try hex. Index the byte slice rather than the `str`: slicing
            // a `str` at a non-char boundary panics, and an even-length payload
            // containing multi-byte UTF-8 reaches exactly that path (which would
            // abort the process under the release profile).
            let bytes = s.as_bytes();
            if bytes.len() % 2 == 0 {
                let mut decoded = Vec::with_capacity(bytes.len() / 2);
                for chunk in bytes.as_chunks::<2>().0 {
                    let Ok(hex) = std::str::from_utf8(chunk) else {
                        break;
                    };
                    let Ok(byte) = u8::from_str_radix(hex, 16) else {
                        break;
                    };
                    decoded.push(byte);
                }
                if decoded.len() == bytes.len() / 2 {
                    return decoded;
                }
            }
            // fallback: string bytes
            return s.as_bytes().to_vec();
        } else if let Some(arr) = v.as_array() {
            return arr
                .iter()
                .filter_map(|x| x.as_u64().map(|n| n as u8))
                .collect();
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
        } else if clean_endpoint.starts_with("127.0.0.1") || clean_endpoint.starts_with("localhost")
        {
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

    /// Voice gateway IDENTIFY (opcode 0).
    ///
    /// DAVE (end-to-end encrypted voice) is **not** advertised: see
    /// [`identify_payload`] for why the protocol version field is omitted.
    pub async fn send_identify(
        &mut self,
        server_id: &str,
        user_id: &str,
        session_id: &str,
        token: &str,
    ) -> Result<()> {
        let payload =
            identify_payload(server_id, user_id, session_id, token).map_err(gateway_err)?;
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
            d: serde_json::to_value(resume).map_err(gateway_err)?,
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

    /// Voice gateway SPEAKING (opcode 5). See [`speaking_payload`].
    pub async fn send_speaking(&mut self, speaking: bool, ssrc: u32) -> Result<()> {
        let payload = speaking_payload(speaking, ssrc);
        self.send_payload(&payload).await
    }

    pub async fn send_heartbeat(&mut self, nonce: u64) -> Result<()> {
        let payload = VoicePayload {
            op: 3,
            d: json!({ "t": nonce }),
        };
        self.send_payload(&payload).await
    }

    /// Send a DAVE client message to the voice gateway.
    ///
    /// Unreachable in practice: DAVE is not advertised on IDENTIFY and
    /// [`crate::dave::protocol::DaveSession`] never produces outgoing messages,
    /// so the opcode mapping below is kept only for completeness. It is *not*
    /// DAVE v1 compatible — commit/welcome belongs on opcode 28
    /// (`dave_mls_commit_welcome`) rather than 24, and binary fields must be
    /// base64 strings, not JSON number arrays.
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
        // Serialisation failure must surface as an error: a panic on this task
        // aborts the whole process under the release profile (panic = "abort").
        let msg = serde_json::to_string(payload).map_err(gateway_err)?;
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

#[cfg(test)]
mod tests {
    use super::{identify_payload, parse_bytes, speaking_payload};
    use serde_json::json;

    /// DAVE protocol v1 is not implemented (wrong MLS ciphersuite, missing
    /// `ready_for_transition`, no proposal/commit/welcome handling), so IDENTIFY
    /// must not advertise support: a gateway that believes we support DAVE can
    /// drive a transition that ends with our audio encrypted under keys nobody
    /// else has. The field must be absent, not `null`.
    #[test]
    fn test_identify_payload_does_not_advertise_dave() {
        let payload = identify_payload("guild-1", "user-2", "session-3", "token-4")
            .expect("identify payload");

        assert_eq!(payload.op, 0);
        assert_eq!(payload.d["server_id"], "guild-1");
        assert_eq!(payload.d["user_id"], "user-2");
        assert_eq!(payload.d["session_id"], "session-3");
        assert_eq!(payload.d["token"], "token-4");

        let fields = payload.d.as_object().expect("identify is a JSON object");
        assert!(!fields.contains_key("max_dave_protocol_version"));
    }

    /// Opcode 5 identifies the sender by SSRC; the identify-time value came from
    /// READY while the post-SessionDescription re-assert used to send `0`.
    #[test]
    fn test_speaking_payload_carries_session_ssrc() {
        let speaking = speaking_payload(true, 0xC0FFEE);
        assert_eq!(speaking.op, 5);
        assert_eq!(speaking.d["ssrc"].as_u64(), Some(0xC0FFEE));
        // 1 (microphone) | 2 (soundshare) for stereo music streaming
        assert_eq!(speaking.d["speaking"].as_u64(), Some(3));
        assert_eq!(speaking.d["delay"].as_u64(), Some(0));

        let stopped = speaking_payload(false, 7);
        assert_eq!(stopped.d["ssrc"].as_u64(), Some(7));
        assert_eq!(stopped.d["speaking"].as_u64(), Some(0));
    }

    #[test]
    fn test_parse_bytes_decodes_base64_first() {
        // "hello" in base64
        let value = json!("aGVsbG8=");
        assert_eq!(parse_bytes(Some(&value)), b"hello".to_vec());
    }

    #[test]
    fn test_parse_bytes_decodes_hex_when_base64_fails() {
        // "0f" is not valid base64 but is valid hex.
        let value = json!("0f10ff");
        assert_eq!(parse_bytes(Some(&value)), vec![0x0f, 0x10, 0xff]);
    }

    #[test]
    fn test_parse_bytes_multibyte_hex_candidate_does_not_panic() {
        // Two 3-byte characters: the `str` length is even (2), so the hex path is
        // entered, but byte index 2 is not a char boundary. Indexing the `str`
        // directly used to panic here with "byte index 2 is not a char boundary".
        let value = json!("\u{20ac}\u{20ac}");
        let decoded = parse_bytes(Some(&value));
        assert_eq!(decoded, "\u{20ac}\u{20ac}".as_bytes().to_vec());
    }

    #[test]
    fn test_parse_bytes_falls_back_to_string_bytes() {
        let value = json!("\u{1f600}");
        let expected = "\u{1f600}".as_bytes().to_vec();
        assert_eq!(parse_bytes(Some(&value)), expected);
    }

    #[test]
    fn test_parse_bytes_handles_arrays_and_null() {
        let value = json!([1, 2, 255]);
        assert_eq!(parse_bytes(Some(&value)), vec![1, 2, 255]);
        assert!(parse_bytes(None).is_empty());
        assert!(parse_bytes(Some(&json!(null))).is_empty());
    }
}
