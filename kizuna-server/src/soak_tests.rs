// Copyright (c) 2026 nikcodex (KizunaLink)
// Licensed under the MIT License

//! On-demand soak/load harness (ignored by default).
//!
//! Run with:
//!
//! ```text
//! cargo test -p kizuna-server --release -- --ignored soak --nocapture
//! ```
//!
//! Spawns the real in-process router on an ephemeral port, opens `SESSIONS`
//! WebSocket clients, creates `PLAYERS` players per session via the REST API,
//! then hammers the surface for `DURATION`. At the end it asserts the node
//! still reports healthy and the player/session count matches, which catches:
//! session leaks, panics under sustained traffic, and deadlock/timeout bugs.

use std::time::{Duration, Instant};

use axum::http::{StatusCode, header};
use futures_util::StreamExt;
use tokio_tungstenite::tungstenite::Message as TungMessage;

use crate::test_support::{self, AUTH_TOKEN};

const SESSIONS: usize = 8;
const PLAYERS_PER_SESSION: usize = 16;
const DURATION: Duration = Duration::from_secs(5);
const TOTAL_PLAYERS: usize = SESSIONS * PLAYERS_PER_SESSION;

type WsStream = tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>;

async fn ws_connect(base: &str) -> WsStream {
    let url = format!("{base}/v4/websocket").replace("http", "ws");
    let mut req =
        tokio_tungstenite::tungstenite::client::IntoClientRequest::into_client_request(&url)
            .unwrap();
    req.headers_mut()
        .insert(header::AUTHORIZATION, AUTH_TOKEN.parse().unwrap());
    let stream = tokio::net::TcpStream::connect(base.trim_start_matches("http://"))
        .await
        .unwrap();
    tokio_tungstenite::client_async(req, stream)
        .await
        .unwrap()
        .0
}

/// Read the `ready` frame and return the session id.
async fn read_ready<S>(sock: &mut S) -> String
where
    S: StreamExt<Item = Result<TungMessage, tokio_tungstenite::tungstenite::Error>> + Unpin,
{
    loop {
        match sock.next().await {
            Some(Ok(TungMessage::Text(text))) => {
                let v: serde_json::Value = serde_json::from_str(&text).unwrap();
                if v["op"] == "ready" {
                    return v["sessionId"].as_str().unwrap().to_string();
                }
            }
            Some(Ok(TungMessage::Ping(_))) => {
                // Auto-respond so the server keeps the connection healthy.
            }
            Some(Ok(_)) => {}
            Some(Err(e)) => panic!("ws error: {e}"),
            None => panic!("ws closed during setup"),
        }
    }
}

#[tokio::test]
#[ignore = "long-running soak harness; run explicitly"]
async fn soak_many_sessions_and_players_with_rest_traffic() {
    let (base, state) = test_support::spawn_test_server().await;
    let client = reqwest::Client::new();
    let base = base.clone();

    let started = Instant::now();

    // 1. Open SESSIONS WebSocket connections.
    let mut sockets = Vec::new();
    for _ in 0..SESSIONS {
        let sock = ws_connect(&base).await;
        sockets.push(sock);
    }

    // 2. Register each session and create players REST-side.
    let mut all_sids: Vec<String> = Vec::new();
    let mut all_guids: Vec<Vec<String>> = Vec::new();
    for (i, sock) in sockets.iter_mut().enumerate() {
        let sid = read_ready(sock).await;
        all_sids.push(sid.clone());
        let mut guids = Vec::new();
        for j in 0..PLAYERS_PER_SESSION {
            let guid = format!("{i}00000{j}");
            guids.push(guid.clone());
            let path = format!("/v4/sessions/{sid}/players/{guid}");
            let resp = client
                .patch(format!("{base}{path}"))
                .header(header::AUTHORIZATION, AUTH_TOKEN)
                .header(header::CONTENT_TYPE, "application/json")
                .body(serde_json::json!({ "volume": 50, "paused": true }).to_string())
                .send()
                .await
                .unwrap();
            assert_eq!(resp.status(), StatusCode::OK, "create player at {path}");
        }
        all_guids.push(guids);
    }

    // 3. Churn volume updates on all players for DURATION.
    let mut update_count = 0u64;
    while started.elapsed() < DURATION {
        for (sid, guids) in all_sids.iter().zip(all_guids.iter()) {
            for (j, guid) in guids.iter().enumerate() {
                let path = format!("/v4/sessions/{sid}/players/{guid}");
                let vol = (update_count % 100) as i32;
                let resp = client
                    .patch(format!("{base}{path}"))
                    .header(header::AUTHORIZATION, AUTH_TOKEN)
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(serde_json::json!({ "volume": vol, "paused": (j % 2 == 0) }).to_string())
                    .send()
                    .await
                    .unwrap();
                assert!(resp.status().is_success());
                update_count += 1;
            }
        }
        // Pace the loop so the test generator doesn't starve background tasks.
        tokio::time::sleep(Duration::from_millis(2)).await;
    }

    // 4. /health still says ok and reports all players.
    let health = client
        .get(format!("{base}/health"))
        .send()
        .await
        .unwrap()
        .json::<serde_json::Value>()
        .await
        .unwrap();
    assert_eq!(health["status"], "ok");
    let reported = health["players"].as_u64().unwrap();
    assert_eq!(
        reported, TOTAL_PLAYERS as u64,
        "player count drifted after soak: expected {TOTAL_PLAYERS}, reported {reported}"
    );

    // 5. Sessions map cleanly matches what we registered.
    assert_eq!(
        state.sessions.len(),
        SESSIONS,
        "session count leaked after soak"
    );

    // 6. Stats endpoint agrees.
    let stats = client
        .get(format!("{base}/v4/stats"))
        .header(header::AUTHORIZATION, AUTH_TOKEN)
        .send()
        .await
        .unwrap()
        .json::<serde_json::Value>()
        .await
        .unwrap();
    assert!(
        stats["players"].as_u64().unwrap_or(0) == TOTAL_PLAYERS as u64,
        "stats should report all players"
    );

    // Close sockets cleanly.
    for mut sock in sockets.into_iter() {
        let _ = sock.close(None).await;
    }

    println!(
        "soak ok: {} sessions x {} players, {update_count} REST updates in {:?}",
        SESSIONS,
        PLAYERS_PER_SESSION,
        started.elapsed()
    );
}
