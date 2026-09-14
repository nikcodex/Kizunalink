// Copyright (c) 2026 nikcodex (KizunaLink)
// Licensed under the MIT License

//! Minimal liveness/readiness surface for orchestrators (k8s, docker healthcheck).
//!
//! Deliberately *not* placed behind the `/v4` auth middleware: probes such as
//! the Docker `HEALTHCHECK` cannot rotate the authorization secret. The endpoint
//! exposes no state and logs nothing at rest.

use std::sync::Arc;

use axum::{Json, extract::State, http::StatusCode, response::IntoResponse};
use serde::Serialize;

use crate::server::AppState;

#[derive(Serialize)]
struct Health {
    status: &'static str,
    uptime_ms: u128,
    players: usize,
}

/// GET /health — 200 when the process is serving; returns uptime and the
/// current player count for a bare-minimum readiness signal.
pub async fn health_check(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let health = Health {
        status: "ok",
        uptime_ms: state.start_time.elapsed().as_millis(),
        players: state.sessions.iter().map(|s| s.players.len()).sum(),
    };
    (StatusCode::OK, Json(health))
}
