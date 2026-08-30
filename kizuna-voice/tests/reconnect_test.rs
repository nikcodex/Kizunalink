// [RECONNECT INTEGRATION TESTS]
// Verifies VoiceConnectionManager and Gateway interactions.
use kizuna_voice::connection::manager::{VoiceConnectionManager, VoiceCredentials};
use kizuna_voice::connection::state::ConnectionState;
use kizuna_voice::dave::protocol::DaveSession;
use kizuna_voice::test_harness::{FakeVoiceGateway, GatewaySessionConfig};
use std::sync::Arc;
use tokio::sync::Mutex;
use std::time::Duration;

#[tokio::test]
async fn test_manager_fresh_identify_and_resume() {
    let ssrc = 778899;
    let config = GatewaySessionConfig {
        ssrc,
        udp_port: 0, // Not testing UDP in this layer directly
        heartbeat_interval: 1000.0,
        auto_handshake: true,
    };
    let fake_gw = FakeVoiceGateway::start(config).await.expect("Start fake GW");

    let credentials = VoiceCredentials {
        endpoint: fake_gw.endpoint().to_string(),
        server_id: "server".into(),
        user_id: "user".into(),
        session_id: "session".into(),
        token: "token".into(),
    };

    let dave = Arc::new(Mutex::new(DaveSession::new("guild".into())));
    let crypto = Arc::new(Mutex::new(None));
    let manager = Arc::new(VoiceConnectionManager::new(credentials, dave, crypto));

    let manager_clone = manager.clone();
    let jh = tokio::spawn(async move {
        manager_clone.run_gateway_loop().await.unwrap();
    });

    let mut rx = manager.state_receiver();

    // Wait for Connected
    loop {
        if *rx.borrow() == ConnectionState::Connected {
            break;
        }
        let _ = rx.changed().await;
    }

    assert!(fake_gw.has_received_opcode(0).await); // Identify sent

    // Now let's drop the connection to force a reconnect!
    drop(fake_gw);

    loop {
        let st = *rx.borrow();
        if st == ConnectionState::Failed || st == ConnectionState::Reconnecting {
            break;
        }
        let _ = rx.changed().await;
    }

    jh.abort();
}

#[tokio::test]
async fn test_manager_resume_success() {
    let ssrc = 111222;
    let mut config = GatewaySessionConfig {
        ssrc,
        udp_port: 0,
        heartbeat_interval: 1000.0,
        auto_handshake: true,
    };
    let mut fake_gw = FakeVoiceGateway::start(config.clone()).await.expect("Start fake GW");

    let credentials = VoiceCredentials {
        endpoint: fake_gw.endpoint().to_string(),
        server_id: "server".into(),
        user_id: "user".into(),
        session_id: "session".into(),
        token: "token".into(),
    };

    let dave = Arc::new(Mutex::new(DaveSession::new("guild".into())));
    let crypto = Arc::new(Mutex::new(None));
    
    // We want fast retries for tests
    let reconnect_cfg = kizuna_voice::connection::manager::ReconnectConfig {
        max_retries: 5,
        base_delay: Duration::from_millis(100),
        max_delay: Duration::from_millis(500),
        jitter_factor: 0.1,
    };

    let manager = Arc::new(VoiceConnectionManager::new(credentials.clone(), dave, crypto)
        .with_config(reconnect_cfg));

    let manager_clone = manager.clone();
    let jh = tokio::spawn(async move {
        manager_clone.run_gateway_loop().await.unwrap();
    });

    let mut rx = manager.state_receiver();

    // 1. Wait for initial connection (Identify)
    loop {
        if *rx.borrow() == ConnectionState::Connected {
            break;
        }
        let _ = rx.changed().await;
    }

    assert!(fake_gw.has_received_opcode(0).await, "Did not send Identify");
    
    // 2. Kill the gateway connection
    fake_gw.clear_received().await;
    fake_gw.drop_clients().await; // drops the active WS connection
    
    // We expect the manager to see the disconnect, reconnect, and send Resume (Op 7).
    // Let's just wait until the fake gateway receives Op 7.
    let mut received_resume = false;
    for _ in 0..50 { // wait up to 5 seconds
        if fake_gw.has_received_opcode(7).await {
            received_resume = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }

    assert!(received_resume, "Did not receive Resume");
    assert!(!fake_gw.has_received_opcode(0).await, "Sent fresh Identify instead of Resume!");

    jh.abort();
}
