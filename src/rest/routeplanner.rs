use axum::{
    extract::State,
    http::StatusCode,
    response::Json,
};
use crate::AppState;
use crate::rest::error::LavalinkError;

#[derive(serde::Serialize)]
pub struct RoutePlannerStatus {
    pub class: String,
    pub rotating: bool,
    #[serde(rename = "ipBlock")]
    pub ip_block: IpBlock,
    pub addresses: Vec<AddressEntry>,
}

#[derive(serde::Serialize)]
pub struct IpBlock {
    #[serde(rename = "type")]
    pub block_type: String,
    pub size: String,
}

#[derive(serde::Serialize)]
pub struct AddressEntry {
    pub ip: String,
    pub port: u16,
    pub expires_at: Option<String>,
}

pub async fn get_routeplanner_status(
    State(_state): State<AppState>,
) -> Json<RoutePlannerStatus> {
    Json(RoutePlannerStatus {
        class: "NanoIpRoutePlanner".to_string(),
        rotating: false,
        ip_block: IpBlock {
            block_type: "org.arbjerg.ipblock.LavapooledIpBlock".to_string(),
            size: "0".to_string(),
        },
        addresses: vec![],
    })
}

pub async fn free_routeplanner_address(
    State(_state): State<AppState>,
) -> Result<StatusCode, LavalinkError> {
    Ok(StatusCode::NO_CONTENT)
}

pub async fn free_routeplanner_all(
    State(_state): State<AppState>,
) -> Result<StatusCode, LavalinkError> {
    Ok(StatusCode::NO_CONTENT)
}
