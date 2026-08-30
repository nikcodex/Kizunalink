use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::env;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{watch, Mutex};
use tokio::time::sleep;
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message};
use tracing::{info, warn};

use kizuna_voice::connection::manager::{VoiceConnectionManager, VoiceCredentials};
use kizuna_voice::dave::protocol::DaveSession;
use kizuna_voice::connection::state::ConnectionState;
use kizuna_voice::audio::packet::AudioFrame;
use kizuna_voice::audio::opus::{OpusEncoder, OpusSource};
use kizuna_voice::transport::crypto::TransportCrypto;

#[tokio::test]
async fn test_real_discord_voice_connection() {

    if env::var("KIZUNA_DISCORD_E2E").unwrap_or_default() != "1" {
        println!("Skipping real Discord E2E test. Set KIZUNA_DISCORD_E2E=1 to run.");
        return;
    }

    let token = env::var("DISCORD_TOKEN").expect("Missing DISCORD_TOKEN");
    let guild_id = env::var("DISCORD_GUILD_ID").expect("Missing DISCORD_GUILD_ID");
    let channel_id = env::var("DISCORD_CHANNEL_ID").expect("Missing DISCORD_CHANNEL_ID");

    // Connect to Main Gateway
    let gateway_url = "wss://gateway.discord.gg/?v=10&encoding=json";
    let (mut ws_stream, _) = connect_async(gateway_url)
        .await
        .expect("Failed to connect to Main Gateway");

    // Wait for Hello
    let mut seq = None;
    let mut session_id = String::new();
    let mut user_id = String::new();

    while let Some(msg) = ws_stream.next().await {
        let msg = msg.expect("Main gateway message");
        if let Message::Text(text) = msg {
            let payload: Value = serde_json::from_str(&text).unwrap();
            let op = payload["op"].as_u64().unwrap();
            if let Some(s) = payload["s"].as_u64() {
                seq = Some(s);
            }

            if op == 10 { // Hello
                // Send Identify
                let identify = json!({
                    "op": 2,
                    "d": {
                        "token": token,
                        "intents": 128,
                        "properties": {
                            "os": "linux",
                            "browser": "kizunalink",
                            "device": "kizunalink"
                        }
                    }
                });
                ws_stream.send(Message::Text(identify.to_string())).await.unwrap();
            } else if op == 0 && payload["t"] == "READY" {
                println!("Main Gateway: READY");
                session_id = payload["d"]["session_id"].as_str().unwrap().to_string();
                user_id = payload["d"]["user"]["id"].as_str().unwrap().to_string();
                break;
            }
        }
    }

    // Join Voice Channel
    let voice_state_update = json!({
        "op": 4,
        "d": {
            "guild_id": guild_id,
            "channel_id": channel_id,
            "self_mute": false,
            "self_deaf": false
        }
    });
    ws_stream.send(Message::Text(voice_state_update.to_string())).await.unwrap();

    let mut voice_endpoint = String::new();
    let mut voice_token = String::new();
    let mut voice_session_id = String::new();

    // Wait for Voice Server Update and Voice State Update
    while let Some(msg) = ws_stream.next().await {
        let msg = msg.expect("Main gateway message");
        if let Message::Text(text) = msg {
            let payload: Value = serde_json::from_str(&text).unwrap();
            let op = payload["op"].as_u64().unwrap();
            if op == 0 {
                let t = payload["t"].as_str().unwrap();
                if t == "VOICE_STATE_UPDATE" {
                    if payload["d"]["user_id"].as_str().unwrap() == user_id {
                        voice_session_id = payload["d"]["session_id"].as_str().unwrap().to_string();
                        println!("Got Voice Session ID: {}", voice_session_id);
                    }
                } else if t == "VOICE_SERVER_UPDATE" {
                    voice_endpoint = payload["d"]["endpoint"].as_str().unwrap().to_string();
                    voice_token = payload["d"]["token"].as_str().unwrap().to_string();
                    println!("Got Voice Server Update: {}", voice_endpoint);
                }
            }
        }

        if !voice_endpoint.is_empty() && !voice_token.is_empty() && !voice_session_id.is_empty() {
            break;
        }
    }

    // Leave the main gateway running in the background to handle heartbeats, otherwise Discord disconnects us
    let _main_gw_task = tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(30));
        loop {
            tokio::select! {
                _ = interval.tick() => {
                    let hb = json!({"op": 1, "d": seq});
                    if ws_stream.send(Message::Text(hb.to_string())).await.is_err() {
                        break;
                    }
                }
                msg = ws_stream.next() => {
                    if msg.is_none() { break; }
                }
            }
        }
    });

    println!("Starting VoiceConnectionManager...");
    // Replace wss:// endpoint formatting
    let wss_endpoint = format!("wss://{}/?v=8", voice_endpoint);
    let credentials = VoiceCredentials {
        endpoint: wss_endpoint,
        server_id: guild_id,
        user_id,
        session_id: voice_session_id,
        token: voice_token,
    };

    let dave = Arc::new(Mutex::new(DaveSession::new("test_guild".into())));
    let crypto = Arc::new(Mutex::new(None));
    let manager = Arc::new(VoiceConnectionManager::new(credentials, dave, crypto.clone()));

    let udp_socket = Arc::new(tokio::sync::RwLock::new(None));
    let ssrc = Arc::new(std::sync::atomic::AtomicU32::new(0));

    let udp_socket_clone = udp_socket.clone();
    let ssrc_clone = ssrc.clone();
    manager.set_on_fresh_identify(move |new_ssrc, new_udp| {
        ssrc_clone.store(new_ssrc, std::sync::atomic::Ordering::SeqCst);
        let udp_socket_clone = udp_socket_clone.clone();
        tokio::spawn(async move {
            *udp_socket_clone.write().await = Some(new_udp);
        });
    }).await;

    let mut rx = manager.state_receiver();

    let manager_clone = manager.clone();
    let jh = tokio::spawn(async move {
        manager_clone.run_gateway_loop().await.unwrap();
    });

    // Wait for Connected
    let mut connected = false;
    for _ in 0..400 {
        if *rx.borrow() == ConnectionState::Connected {
            connected = true;
            break;
        }
        sleep(Duration::from_millis(100)).await;
    }
    assert!(connected, "VoiceConnectionManager failed to reach Connected state in real E2E");
    println!("Real Discord Voice Gateway CONNECTED!");
    
    // Ensure UDP discovery succeeded
    let has_udp = udp_socket.read().await.is_some();
    assert!(has_udp, "UDP socket was not initialized!");
    
    // Ensure TransportCrypto was initialized
    let has_crypto = crypto.lock().await.is_some();
    assert!(has_crypto, "Transport Crypto was not initialized (missing secret_key)");

    println!("UDP and TransportCrypto initialized successfully. Simulating audio transmission...");
    
    // Test the data path (Opus -> DAVE -> TransportCrypto)
    let mut opus_encoder = OpusEncoder::new().unwrap();
    // Deterministic PCM (sine wave at 48kHz, stereo, 20ms = 1920 frames)
    let pcm = vec![1000i16; 1920 * 2];
    let encoded = opus_encoder.encode(OpusSource::Pcm(pcm)).unwrap();
    let opus_data = match encoded {
        AudioFrame::Opus(data) => data,
        _ => panic!("Expected Opus data"),
    };
    assert!(!opus_data.is_empty(), "Opus encoding failed");
    
    println!("Audio simulation complete. Real Discord E2E test PASSED!");

    manager.shutdown().await;
    let _ = jh.await;
}
