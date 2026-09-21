// Copyright (c) 2026 nikcodex (KizunaLink)
// Licensed under the MIT License

//! End-to-end REST integration tests.
//!
//! These spin up the *real* axum router (auth, rate limiting, response headers
//! — exactly what `main.rs` installs) and exercise it via `tower::ServiceExt`.
//! No network is involved, so the tests are fast, hermetic, and deterministic.

use std::sync::Arc;

use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode, header},
};
use http_body_util::BodyExt;
use tower::ServiceExt;

use crate::test_support::{self, AUTH_TOKEN};

/// Issue a GET on a fresh router and collect (status, body).
async fn get(uri: &str) -> (StatusCode, serde_json::Value) {
    let app = test_support::test_router(test_support::test_state());
    request_on(app, "GET", uri, None).await
}

/// Issue a request against a specific router (so tests can pre-register sessions).
async fn request_on(
    app: Router,
    method: &str,
    uri: &str,
    body: Option<serde_json::Value>,
) -> (StatusCode, serde_json::Value) {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::AUTHORIZATION, AUTH_TOKEN);
    if body.is_some() {
        builder = builder.header(header::CONTENT_TYPE, "application/json");
    }
    let body = body.map(|v| v.to_string()).unwrap_or_default();
    let resp = app
        .oneshot(builder.body(Body::from(body)).unwrap())
        .await
        .unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let body = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    (status, body)
}

#[tokio::test]
async fn auth_required_on_all_rest_paths() {
    let app = test_support::test_router(test_support::test_state());
    for uri in [
        "/v4/info",
        "/v4/stats",
        "/version",
        "/v4/loadtracks?identifier=test",
    ] {
        let resp = app
            .clone()
            .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::UNAUTHORIZED,
            "expected 401 for {uri}"
        );
    }
}

#[tokio::test]
async fn wrong_password_rejected() {
    let app = test_support::test_router(test_support::test_state());
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/v4/info")
                .header(header::AUTHORIZATION, "wrong")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::FORBIDDEN,
        "wrong password must be 403, matching official Lavalink"
    );
}

#[tokio::test]
async fn version_endpoint_returns_headers_and_version() {
    let app = test_support::test_router(test_support::test_state());
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/version")
                .header(header::AUTHORIZATION, AUTH_TOKEN)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(resp.headers().get("Lavalink-Api-Version").unwrap(), "4");
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    // The endpoint returns a bare crate version string (`1.1.0`), not JSON.
    let version = String::from_utf8(bytes.to_vec()).unwrap();
    assert!(version.starts_with("1."), "unexpected version: {version}");
}

#[tokio::test]
async fn info_endpoint_returns_lavalink_v4_schema() {
    let (status, body) = get("/v4/info").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["version"]["semver"], env!("CARGO_PKG_VERSION"));
    assert!(body["lavaplayer"].as_str().is_some());
    assert!(body["jvm"].as_str().is_some());
    assert_eq!(body["plugins"], serde_json::json!([]));
    // The response carries Lavalink's camelCase shape.
    assert!(body["sourceManagers"].is_array());
    assert!(body["filters"].is_array());
}

#[tokio::test]
async fn loadtracks_unknown_identifier_returns_empty_with_null_data() {
    // Official Lavalink serializes `NoMatches` with `data: null`; the response
    // must byte-match that shape (not `"data": {}`).
    let (status, body) = get("/v4/loadtracks?identifier=zzz-kizuna-no-source-matches-this").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["loadType"], "empty");
    assert!(body["data"].is_null(), "data must be null, got: {body}");
}

#[tokio::test]
async fn stats_endpoint_returns_numeric_counts() {
    let (status, body) = get("/v4/stats").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["players"], 0);
    assert_eq!(body["playingPlayers"], 0);
    assert!(body["uptime"].as_u64().is_some());
    assert!(body["memory"]["free"].as_u64().is_some());
    assert!(body["cpu"]["cores"].as_i64().is_some());
}

#[tokio::test]
async fn session_lifecycle_patch_get() {
    let state = test_support::test_state();
    let app = test_support::test_router(state.clone());

    // Create a session registered in the state's session map (the same way the
    // WS handler does after accept).
    let (tx, _rx) = flume::unbounded();
    let sid = kizunalink::common::types::SessionId::generate();
    let session = Arc::new(crate::server::Session::new(
        sid.clone(),
        None,
        tx,
        state.config.server.max_event_queue_size,
    ));
    state.sessions.insert(sid.clone(), session);

    // PATCH a session update (resuming=true, timeout=30).
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PATCH")
                .uri(format!("/v4/sessions/{sid}"))
                .header(header::AUTHORIZATION, AUTH_TOKEN)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::json!({ "resuming": true, "timeout": 30 }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let info: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(info["resuming"], true);
    assert_eq!(info["timeout"], 30);

    // GET the session back.
    let resp = app
        .oneshot(
            Request::builder()
                .uri(format!("/v4/sessions/{sid}"))
                .header(header::AUTHORIZATION, AUTH_TOKEN)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let info2: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(info2["resuming"], true);
}

#[tokio::test]
async fn player_update_requires_existing_session() {
    let app = test_support::test_router(test_support::test_state());
    let sid = kizunalink::common::types::SessionId::generate();
    let resp = app
        .oneshot(
            Request::builder()
                .method("PATCH")
                .uri(format!("/v4/sessions/{sid}/players/123"))
                .header(header::AUTHORIZATION, AUTH_TOKEN)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from("{}".to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn player_create_update_and_delete_lifecycle() {
    let state = test_support::test_state();
    let app = test_support::test_router(state.clone());
    let guid = kizunalink::common::types::GuildId::from("123456789".to_string());

    // Register a session as the WS handler would.
    let (tx, _rx) = flume::unbounded();
    let sid = kizunalink::common::types::SessionId::generate();
    let session = Arc::new(crate::server::Session::new(
        sid.clone(),
        None,
        tx,
        state.config.server.max_event_queue_size,
    ));
    state.sessions.insert(sid.clone(), session);
    let path = format!("/v4/sessions/{sid}/players/{}", guid.0);

    // First PATCH creates the player with a paused state and volume.
    let (status, body) = request_on(
        app.clone(),
        "PATCH",
        &path,
        Some(serde_json::json!({ "paused": true, "volume": 25 })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "creating player: {body}");
    assert_eq!(body["guildId"], guid.0.to_string());
    assert_eq!(body["volume"], 25);
    assert_eq!(body["paused"], true);
    // No track, so the player is not connected.
    assert_eq!(body["state"]["connected"], false);

    // GET the player.
    let (status, body) = request_on(app.clone(), "GET", &path, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["volume"], 25);
    assert_eq!(body["paused"], true);

    // DELETE destroys the player.
    let (status, _) = request_on(app.clone(), "DELETE", &path, None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // A subsequent GET now reports the player missing.
    let (status, _) = request_on(app, "GET", &path, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn routeplanner_status_returns_204_when_disabled() {
    let (status, _body) = get("/v4/routeplanner/status").await;
    // When the route planner is not enabled the Lavalink contract is `204 No
    // Content` rather than an error.
    assert_eq!(status, StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn health_endpoint_is_unauthenticated_and_reports_uptime() {
    let app = test_support::test_router(test_support::test_state());

    // No authorization header: orchestrator probes should not need the secret.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body["status"], "ok");
    assert!(body["uptime_ms"].as_u64().is_some());
    assert_eq!(body["players"], 0);
}
