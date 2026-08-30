use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Identify {
    pub server_id: String,
    pub user_id: String,
    pub session_id: String,
    pub token: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dave_protocol_version: Option<u8>,
}

fn main() {
    let id = Identify {
        server_id: "test".into(),
        user_id: "test".into(),
        session_id: "test".into(),
        token: "test".into(),
        dave_protocol_version: Some(1),
    };
    println!("{}", serde_json::to_string(&id).unwrap());
}
