// Copyright (c) 2026 nikcodex (KizunaLink)
// Licensed under the MIT License

use std::sync::Arc;

use axum::{
    extract::{Request, State},
    http::{HeaderValue, StatusCode},
    middleware::Next,
    response::Response,
};
use tracing::warn;

use crate::common::server_hooks::ServerContext;

pub async fn check_auth(
    State(state): State<Arc<dyn ServerContext>>,
    req: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    let auth_header = req
        .headers()
        .get("authorization")
        .and_then(|h| h.to_str().ok());

    use subtle::ConstantTimeEq;
    match auth_header {
        Some(auth) if bool::from(auth.as_bytes().ct_eq(state.config.server.authorization.as_bytes())) => Ok(next.run(req).await),
        Some(_) => {
            warn!("REST authorization failed: invalid password");
            Err(StatusCode::UNAUTHORIZED)
        }
        None => {
            warn!("REST authorization failed: missing authorization header");
            Err(StatusCode::UNAUTHORIZED)
        }
    }
}

pub async fn add_response_headers(req: Request, next: Next) -> Response {
    let mut response = next.run(req).await;
    response
        .headers_mut()
        .insert("Lavalink-Major-Version", HeaderValue::from_static("4"));
    response
}
