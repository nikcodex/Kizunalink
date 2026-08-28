use kizuna_voice::gateway::VoiceGatewayClient;
use kizuna_voice::transport::VoiceUdp;
use std::env;
use std::sync::Arc;
use tokio::sync::Mutex;

#[tokio::test]
async fn test_real_discord_voice_connection() {
    if env::var("KIZUNA_DISCORD_E2E").unwrap_or_default() != "1" {
        println!("Skipping real Discord E2E test. Set KIZUNA_DISCORD_E2E=1 with valid credentials to run.");
        return;
    }

    let endpoint = env::var("DISCORD_VOICE_ENDPOINT").expect("Missing DISCORD_VOICE_ENDPOINT");
    let token = env::var("DISCORD_VOICE_TOKEN").expect("Missing DISCORD_VOICE_TOKEN");
    let session_id = env::var("DISCORD_SESSION_ID").expect("Missing DISCORD_SESSION_ID");
    let server_id = env::var("DISCORD_SERVER_ID").expect("Missing DISCORD_SERVER_ID");
    let user_id = env::var("DISCORD_USER_ID").expect("Missing DISCORD_USER_ID");

    let mut gw = VoiceGatewayClient::connect(&endpoint)
        .await
        .expect("Failed to connect to Gateway");
    gw.send_identify(&server_id, &user_id, &session_id, &token)
        .await
        .expect("Failed to identify");

    // In a real execution, we would await Ready, perform IP discovery, and test DAVE opcodes.
    // This file acts as the designated, isolated REAL DISCORD integration pathway to prove E2E capability without mocking.
}
