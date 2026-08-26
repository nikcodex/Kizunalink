use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Json, Response},
};
use serde::Deserialize;

use crate::rest::auth::require_auth;
use crate::rest::error::LavalinkError;
use crate::AppState;

pub async fn get_routeplanner_status(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Response, LavalinkError> {
    require_auth(&headers, &state.password, "/v4/routeplanner/status")?;

    match &state.route_planner {
        Some(rp) => Ok(Json(rp.status_json()).into_response()),
        None => Ok(StatusCode::NO_CONTENT.into_response()),
    }
}

#[derive(Deserialize)]
pub struct FreeAddressBody {
    pub address: String,
}

pub async fn free_routeplanner_address(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<FreeAddressBody>,
) -> Result<StatusCode, LavalinkError> {
    require_auth(&headers, &state.password, "/v4/routeplanner/free/address")?;

    if let Some(rp) = &state.route_planner {
        let clean = body.address.strip_prefix('/').unwrap_or(&body.address).trim();
        if let Ok(addr) = clean.parse() {
            rp.unmark(addr);
        }
    }

    Ok(StatusCode::NO_CONTENT)
}

pub async fn free_routeplanner_all(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<StatusCode, LavalinkError> {
    require_auth(&headers, &state.password, "/v4/routeplanner/free/all")?;

    if let Some(rp) = &state.route_planner {
        rp.unmark_all();
    }

    Ok(StatusCode::NO_CONTENT)
}
