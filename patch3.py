with open("kizuna-voice/src/gateway/connection.rs", "r") as f:
    text = f.read()

text = text.replace("""        let identify = Identify {
            server_id: server_id.to_string(),
            user_id: user_id.to_string(),
            session_id: session_id.to_string(),
            token: token.to_string(),
        };""", """        let identify = Identify {
            server_id: server_id.to_string(),
            user_id: user_id.to_string(),
            session_id: session_id.to_string(),
            token: token.to_string(),
            dave_protocol_version: Some(1),
        };""")

with open("kizuna-voice/src/gateway/connection.rs", "w") as f:
    f.write(text)
