// [FAKE DISCORD PROTOCOL TEST]
// [LOCAL INTEGRATION]
use kizuna_voice::gateway::connection::GatewayEvent;
use kizuna_voice::gateway::VoiceGatewayClient;
use kizuna_voice::test_harness::{FakeVoiceGateway, FakeVoiceUdpServer, GatewaySessionConfig};
use std::time::Duration;

#[tokio::test]
async fn test_voice_gateway_complete_handshake_and_heartbeat() {
    let ssrc = 778899;
    let fake_udp = FakeVoiceUdpServer::start(ssrc).await.expect("Start fake UDP");
    let udp_port = fake_udp.port();

    let config = GatewaySessionConfig {
        ssrc,
        udp_port,
        heartbeat_interval: 1000.0,
        auto_handshake: true,
    };
    let fake_gw = FakeVoiceGateway::start(config).await.expect("Start fake GW");

    // Connect real VoiceGatewayClient to local FakeVoiceGateway
    let mut client = VoiceGatewayClient::connect(fake_gw.endpoint())
        .await
        .expect("Client connect");

    // 1. Hello (Op 8)
    let hello = client.receive_event().await.expect("Receive Hello");
    assert!(matches!(hello, GatewayEvent::Hello(interval) if (interval - 1000.0).abs() < 0.1));

    // 2. Identify (Op 0)
    client
        .send_identify("server_123", "user_456", "session_789", "token_abc")
        .await
        .expect("Send identify");

    // 3. Ready (Op 2)
    let ready = client.receive_event().await.expect("Receive Ready");
    let GatewayEvent::Ready(ready_data) = ready else {
        panic!("Expected Ready event, got {:?}", ready);
    };
    assert_eq!(ready_data.ssrc, ssrc);
    assert_eq!(ready_data.port, udp_port);

    // 4. Select Protocol (Op 1)
    client
        .send_select_protocol("udp", "127.0.0.1", 12345, "aead_aes256_gcm_rtpsize")
        .await
        .expect("Send select protocol");

    // 5. Session Description (Op 4)
    let sd = client.receive_event().await.expect("Receive SessionDescription");
    assert!(matches!(sd, GatewayEvent::SessionDescription(_)));

    // 6. Heartbeat (Op 3) -> Heartbeat ACK (Op 6)
    client.send_heartbeat(42).await.expect("Send heartbeat");
    let ack = client.receive_event().await.expect("Receive HeartbeatAck");
    assert!(matches!(ack, GatewayEvent::HeartbeatAck));

    // Verify all opcodes received by fake gateway
    assert!(fake_gw.has_received_opcode(0).await, "Opcode 0 (Identify) was received");
    assert!(fake_gw.has_received_opcode(1).await, "Opcode 1 (SelectProtocol) was received");
    assert!(fake_gw.has_received_opcode(3).await, "Opcode 3 (Heartbeat) was received");
}

#[tokio::test]
async fn test_dave_gateway_opcodes_dispatch() {
    let config = GatewaySessionConfig {
        ssrc: 112233,
        udp_port: 5000,
        heartbeat_interval: 5000.0,
        auto_handshake: false,
    };
    let fake_gw = FakeVoiceGateway::start(config).await.expect("Start fake GW");

    let mut client = VoiceGatewayClient::connect(fake_gw.endpoint())
        .await
        .expect("Client connect");

    let _hello = client.receive_event().await.expect("Receive Hello");

    // Fake gateway sends Prepare Transition (Op 21)
    fake_gw
        .send_dave_prepare_transition(1, 1)
        .await
        .expect("Send prepare transition");

    let ev21 = client.receive_event().await.expect("Receive Op 21");
    assert!(matches!(
        ev21,
        GatewayEvent::DaveMessage(kizuna_voice::dave::protocol::DaveGatewayMessage::PrepareTransition {
            transition_id: 1,
            protocol_version: 1
        })
    ));

    // Fake gateway sends Execute Transition (Op 22)
    fake_gw
        .send_dave_execute_transition(1)
        .await
        .expect("Send execute transition");

    let ev22 = client.receive_event().await.expect("Receive Op 22");
    assert!(matches!(
        ev22,
        GatewayEvent::DaveMessage(kizuna_voice::dave::protocol::DaveGatewayMessage::ExecuteTransition {
            transition_id: 1
        })
    ));

    // Fake gateway sends Prepare Epoch (Op 24)
    fake_gw
        .send_dave_prepare_epoch(10)
        .await
        .expect("Send prepare epoch");

    let ev24 = client.receive_event().await.expect("Receive Op 24");
    assert!(matches!(
        ev24,
        GatewayEvent::DaveMessage(kizuna_voice::dave::protocol::DaveGatewayMessage::PrepareEpoch {
            epoch_id: 10
        })
    ));
}

#[tokio::test]
async fn test_gateway_disconnect_and_reconnect() {
    let config = GatewaySessionConfig::default();
    let fake_gw = FakeVoiceGateway::start(config).await.expect("Start fake GW");

    // First connection
    {
        let mut client1 = VoiceGatewayClient::connect(fake_gw.endpoint())
            .await
            .expect("Client 1 connect");
        let hello = client1.receive_event().await.expect("Client 1 Hello");
        assert!(matches!(hello, GatewayEvent::Hello(_)));
        // Dropping client1 terminates connection
    }

    tokio::time::sleep(Duration::from_millis(50)).await;

    // Second connection (reconnect)
    {
        let mut client2 = VoiceGatewayClient::connect(fake_gw.endpoint())
            .await
            .expect("Client 2 reconnect");
        let hello = client2.receive_event().await.expect("Client 2 Hello");
        assert!(matches!(hello, GatewayEvent::Hello(_)));
    }
}
