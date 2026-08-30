with open("kizuna-voice/src/gateway/connection.rs", "r") as f:
    text = f.read()

text = text.replace('println!("Sending payload: {}", serde_json::to_string(let payload = VoicePayload {identify).unwrap());', 'println!("Sending payload: {}", serde_json::to_string(&identify).unwrap());')

with open("kizuna-voice/src/gateway/connection.rs", "w") as f:
    f.write(text)

