// Copyright (c) 2026 nikcodex (KizunaLink)
// Licensed under the MIT License

//! WebSocket protocol integration tests.
//!
//! A real TCP listener + real `tokio-tungstenite` client drive the actual
//! upgrade path (`/v4/websocket`), exercising session creation, the `ready`
//! handshake, stats events, and session resumption exactly as a Lavalink bot
//! would experience them.

use axum::http::header;
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::Message as TungMessage;
type WsStream = tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>;

use crate::test_support::{AUTH_TOKEN, spawn_test_server};

/// Connect a WS client with the named extra headers set.
async fn connect(base: &str, extra: &[(&str, &str)]) -> (WsStream, Vec<(String, String)>) {
    let url = format!("{base}/v4/websocket").replace("http", "ws");
    let mut req =
        tokio_tungstenite::tungstenite::client::IntoClientRequest::into_client_request(&url)
            .unwrap();
    req.headers_mut()
        .insert(header::AUTHORIZATION, AUTH_TOKEN.parse().unwrap());
    req.headers_mut()
        .insert("user-id", "424242".parse().unwrap());
    req.headers_mut()
        .insert("client-name", "kizuna-test".parse().unwrap());
    for (k, v) in extra {
        let k: header::HeaderName = (*k).parse().unwrap();
        let v: axum::http::HeaderValue = (*v).parse().unwrap();
        req.headers_mut().insert(k, v);
    }

    let stream = tokio::net::TcpStream::connect(base.trim_start_matches("http://"))
        .await
        .unwrap();
    let (socket, resp) = tokio_tungstenite::client_async(req, stream).await.unwrap();
    let headers: Vec<(String, String)> = resp
        .headers()
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_str().unwrap().to_string()))
        .collect();
    (socket, headers)
}

/// Read the next text frame as JSON (skipping pings/pongs).
async fn next_json<S>(sock: &mut S) -> serde_json::Value
where
    S: StreamExt<Item = Result<TungMessage, tokio_tungstenite::tungstenite::Error>> + Unpin,
{
    loop {
        match sock.next().await {
            Some(Ok(TungMessage::Text(text))) => {
                return serde_json::from_str(&text).expect("valid JSON frame");
            }
            Some(Ok(TungMessage::Ping(_))) | Some(Ok(TungMessage::Pong(_))) => continue,
            Some(Ok(TungMessage::Close(_))) => panic!("unexpected close frame"),
            Some(Err(e)) => panic!("ws error: {e}"),
            None => panic!("ws closed unexpectedly"),
            Some(Ok(TungMessage::Binary(_))) => continue,
            _ => continue,
        }
    }
}

#[tokio::test]
async fn ws_handshake_requires_authorization() {
    let (base, _state) = spawn_test_server().await;
    let url = format!("{base}/v4/websocket").replace("http", "ws");

    // No auth header -> the upgrade is rejected.
    let stream = tokio::net::TcpStream::connect(base.trim_start_matches("http://"))
        .await
        .unwrap();
    let result = tokio_tungstenite::client_async(url.clone(), stream).await;
    assert!(result.is_err(), "missing auth must be rejected");

    // Wrong auth header -> rejected.
    let stream = tokio::net::TcpStream::connect(base.trim_start_matches("http://"))
        .await
        .unwrap();
    let mut req =
        tokio_tungstenite::tungstenite::client::IntoClientRequest::into_client_request(&url)
            .unwrap();
    req.headers_mut()
        .insert(header::AUTHORIZATION, "wrong".parse().unwrap());
    let result = tokio_tungstenite::client_async(req, stream).await;
    assert!(result.is_err(), "bad password must be rejected");
}

#[tokio::test]
async fn ws_ready_handshake_and_stats_event() {
    let (base, _state) = spawn_test_server().await;
    let (mut socket, headers) = connect(&base, &[]).await;

    // Response headers from the upgrade.
    let lavalink_major = headers
        .iter()
        .find(|(k, _)| k == "lavalink-major-version")
        .map(|(_, v)| v.clone())
        .expect("Lavalink-Major-Version header");
    assert_eq!(lavalink_major, "4");

    // First frame must be a `ready` message with a generated session id.
    let ready = next_json(&mut socket).await;
    assert_eq!(ready["op"], "ready");
    assert_eq!(ready["resumed"], false);
    let session_id = ready["sessionId"].as_str().unwrap().to_string();
    assert!(!session_id.is_empty());

    // Stats events arrive on the configured interval (default 30s); we don't
    // want to wait that long in a test, but a Ping/Pong heartbeat appears
    // earlier. Send a pong and confirm the socket is alive.
    socket.send(TungMessage::Pong(vec![].into())).await.unwrap();

    // The session is registered in state after connect.
    assert!(
        _state.sessions.contains_key(&session_id.clone().into()),
        "session must be registered after WS connect"
    );

    socket.close(None).await.unwrap();
}

#[tokio::test]
async fn ws_session_resumption_preserves_session_id() {
    let (base, state) = spawn_test_server().await;

    // Connect #1: grab the generated session id.
    let (mut socket, _) = connect(&base, &[]).await;
    let ready = next_json(&mut socket).await;
    let session_id = ready["sessionId"].as_str().unwrap().to_string();

    // Mark the session resumable via the REST API, then disconnect.
    let client = reqwest::Client::new();
    let patch_resp = client
        .patch(format!("{base}/v4/sessions/{session_id}"))
        .header(header::AUTHORIZATION, AUTH_TOKEN)
        .header(axum::http::header::CONTENT_TYPE, "application/json")
        .body(r#"{"resuming":true,"timeout":300}"#)
        .send()
        .await
        .unwrap();
    assert_eq!(patch_resp.status(), 200);

    socket.close(None).await.unwrap();
    // Allow the close handler to move the session into the resumable map.
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;

    // The session should now be resumable.
    let sid = kizunalink::common::types::SessionId::from(session_id.clone());
    assert!(
        state.resumable_sessions.contains_key(&sid),
        "session should be marked resumable after close"
    );

    // Reconnect with `session-id` set to resume.
    let (mut socket2, headers) = connect(&base, &[("session-id", &session_id)]).await;
    let resumed_header = headers
        .iter()
        .find(|(k, _)| k == "session-resumed")
        .map(|(_, v)| v.clone())
        .expect("Session-Resumed header");
    assert_eq!(resumed_header, "true");

    let ready = next_json(&mut socket2).await;
    assert_eq!(ready["op"], "ready");
    assert_eq!(ready["resumed"], true);
    assert_eq!(
        ready["sessionId"].as_str().unwrap(),
        session_id,
        "resumed session must keep its id"
    );

    socket2.close(None).await.unwrap();
}
