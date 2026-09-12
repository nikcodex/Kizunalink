// Copyright (c) 2026 nikcodex (KizunaLink)
// Licensed under the MIT License

use serde::Serialize;

#[derive(Debug, Serialize, Clone)]
#[serde(tag = "class", content = "details")]
pub enum RoutePlannerStatus {
    RotatingIpRoutePlanner(RotatingIpDetails),
    NanoIpRoutePlanner(NanoIpDetails),
    RotatingNanoIpRoutePlanner(RotatingNanoIpDetails),
    BalancingIpRoutePlanner(BalancingIpDetails),
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct RotatingIpDetails {
    pub ip_block: IpBlock,
    pub failing_addresses: Vec<FailingAddress>,
    pub rotate_index: String,
    pub ip_index: String,
    pub current_address: String,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct NanoIpDetails {
    pub ip_block: IpBlock,
    pub failing_addresses: Vec<FailingAddress>,
    pub current_address: String,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct RotatingNanoIpDetails {
    pub ip_block: IpBlock,
    pub failing_addresses: Vec<FailingAddress>,
    pub block_index: String,
    pub current_address_index: String,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct BalancingIpDetails {
    pub ip_block: IpBlock,
    pub failing_addresses: Vec<FailingAddress>,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct IpBlock {
    #[serde(rename = "type")]
    pub block_type: String,
    pub size: String,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct FailingAddress {
    pub failing_address: String,
    pub failing_timestamp: u64,
    pub failing_time: String,
}

#[derive(Debug, serde::Deserialize)]
pub struct FreeAddressRequest {
    pub address: String,
}
