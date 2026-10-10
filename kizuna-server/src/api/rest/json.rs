// Copyright (c) 2026 nikcodex (KizunaLink)
// Licensed under the MIT License

//! JSON extractors and response wrappers that keep every `/v4` error body in the
//! same shape as the rest of the API.
//!
//! Axum's built-in `Query`/`Json` rejections reply with `text/plain`
//! (`Failed to deserialize query string: …` / `Failed to deserialize the JSON
//! body: …`), which no Lavalink client parses. `ApiQuery`/`ApiJson` wrap the
//! built-in extractors and translate a rejection into the project's
//! `KizunaLinkError` JSON (`{timestamp,status,error,message,path}`), so a client
//! sees one error contract on every route.

use axum::{
    extract::{FromRequest, FromRequestParts, OriginalUri, Request, rejection::JsonRejection},
    http::{StatusCode, Uri},
    response::{IntoResponse, Response},
};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::common::KizunaLinkError;

/// Canonical JSON error body for a request path.
pub fn error_response(status: StatusCode, message: impl Into<String>, path: &str) -> Response {
    let error = KizunaLinkError::new(
        status.as_u16(),
        status.canonical_reason().unwrap_or("Error"),
        message,
        path,
    );
    (status, axum::Json(error)).into_response()
}

/// The full client-facing request path.
///
/// `nest("/v4", …)` strips the mount prefix from `Request::uri()`, so an extractor
/// running inside the nested router would otherwise report `/loadtracks` instead of
/// `/v4/loadtracks`. `OriginalUri` carries the un-stripped URI when the outer router
/// installs it; fall back to the (possibly stripped) URI otherwise.
fn full_path(uri: &Uri, extensions: &axum::http::Extensions) -> String {
    let uri = extensions
        .get::<OriginalUri>()
        .map(|OriginalUri(u)| u)
        .unwrap_or(uri);
    uri.path_and_query()
        .map(|pq| pq.as_str().to_owned())
        .unwrap_or_else(|| uri.path().to_owned())
}

/// `axum::Json` that answers deserialization failures with the JSON error shape.
///
/// Every JSON rejection is mapped to `400 Bad Request`: the project (and official
/// Lavalink) treat a malformed body as a client error, whereas axum defaults some
/// variants to 422/415.
pub struct ApiJson<T>(pub T);

impl<T, S> FromRequest<S> for ApiJson<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = Response;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        let path = full_path(req.uri(), req.extensions());
        match axum::Json::<T>::from_request(req, state).await {
            Ok(axum::Json(value)) => Ok(ApiJson(value)),
            Err(rejection) => {
                let message = match rejection {
                    JsonRejection::MissingJsonContentType(_) => {
                        "Expected a JSON request body with Content-Type: application/json"
                            .to_owned()
                    }
                    other => other.body_text(),
                };
                Err(error_response(StatusCode::BAD_REQUEST, message, &path))
            }
        }
    }
}

impl<T: Serialize> IntoResponse for ApiJson<T> {
    fn into_response(self) -> Response {
        axum::Json(self.0).into_response()
    }
}

/// `axum::Query` that answers deserialization failures with the JSON error shape.
pub struct ApiQuery<T>(pub T);

impl<T, S> FromRequestParts<S> for ApiQuery<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = Response;

    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        state: &S,
    ) -> Result<Self, Self::Rejection> {
        let path = full_path(&parts.uri, &parts.extensions);
        match axum::extract::Query::<T>::from_request_parts(parts, state).await {
            Ok(axum::extract::Query(value)) => Ok(ApiQuery(value)),
            Err(rejection) => Err(error_response(
                StatusCode::BAD_REQUEST,
                rejection.body_text(),
                &path,
            )),
        }
    }
}

/// `axum::Path` that answers path-parameter deserialization failures with the
/// JSON error shape (e.g. a non-numeric `{guild_id}`).
pub struct ApiPath<T>(pub T);

impl<T, S> FromRequestParts<S> for ApiPath<T>
where
    T: DeserializeOwned + Send,
    S: Send + Sync,
{
    type Rejection = Response;

    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        state: &S,
    ) -> Result<Self, Self::Rejection> {
        let path = full_path(&parts.uri, &parts.extensions);
        match axum::extract::Path::<T>::from_request_parts(parts, state).await {
            Ok(axum::extract::Path(value)) => Ok(ApiPath(value)),
            Err(rejection) => Err(error_response(
                StatusCode::BAD_REQUEST,
                rejection.body_text(),
                &path,
            )),
        }
    }
}
