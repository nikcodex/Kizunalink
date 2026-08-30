with open("kizuna-voice/src/gateway/connection.rs", "r") as f:
    text = f.read()

text = text.replace("""    pub async fn send_identify(
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
        };""", """    pub async fn send_identify(
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
            dave_protocol_version: Some(1),
        };""")

with open("kizuna-voice/src/gateway/connection.rs", "w") as f:
    f.write(text)
