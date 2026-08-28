use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Serialize, Deserialize, Debug)]
pub struct VoicePayload {
    pub op: u8,
    #[serde(default)]
    pub d: Value,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Identify {
    pub server_id: String,
    pub user_id: String,
    pub session_id: String,
    pub token: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct SelectProtocol {
    pub protocol: String,
    pub data: ProtocolData,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ProtocolData {
    pub address: String,
    pub port: u16,
    pub mode: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Ready {
    pub ssrc: u32,
    pub ip: String,
    pub port: u16,
    pub modes: Vec<String>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct SessionDescription {
    pub mode: String,
    pub secret_key: Vec<u8>,
}
