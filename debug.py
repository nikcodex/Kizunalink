with open("kizuna-voice/src/gateway/connection.rs", "r") as f:
    text = f.read()

text = text.replace('let payload = VoicePayload {\n            op: 0,\n            d: serde_json::to_value(identify.clone()).unwrap(),\n        };', 'println!("Identify: {}", serde_json::to_string(&identify).unwrap());\n        let payload = VoicePayload {\n            op: 0,\n            d: serde_json::to_value(identify).unwrap(),\n        };')

with open("kizuna-voice/src/gateway/connection.rs", "w") as f:
    f.write(text)
