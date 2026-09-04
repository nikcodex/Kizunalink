//! Route Planner — IP rotation for outbound requests (YouTube anti-ban & anti-rate-limiting).
//!
//! Complies 1:1 with the Lavalink v4 RoutePlanner specification:
//! - Supports `RotatingIpRoutePlanner`, `NanoIpRoutePlanner`, `RotatingNanoIpRoutePlanner`, and `BalancingIpRoutePlanner`.
//! - Accepts individual IPs and IPv4/IPv6 CIDR blocks (e.g. `2001:db8::/64`, `192.168.1.0/24`).
//! - Tracks failing addresses with timestamps and automatic cooldown expiration.
//! - Binds outbound HTTP sockets to specific source IPs via `reqwest::ClientBuilder::local_address`.
//! - Provides full status and address release endpoints (`/v4/routeplanner/*`).

use dashmap::DashMap;
use rand::Rng;
use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, USER_AGENT};
use std::collections::HashSet;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tracing::{info, warn};

/// How long an IP stays marked as failing before it is retried (default: 120s).
const DEFAULT_FAIL_COOLDOWN_SECS: u64 = 120;

/// Maximum number of cached HTTP clients before evicting all entries.
const MAX_CLIENT_CACHE_SIZE: usize = 100;

#[derive(Debug, Clone)]
pub enum IpBlock {
    Single(IpAddr),
    Ipv4Subnet {
        base: u32,
        mask: u32,
        prefix_len: u8,
        size: u64,
    },
    Ipv6Subnet {
        base: u128,
        mask: u128,
        prefix_len: u8,
        size: u128,
    },
}

impl IpBlock {
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim();
        if let Some((ip_part, prefix_part)) = s.split_once('/') {
            let prefix_len = prefix_part.parse::<u8>().ok()?;
            if let Ok(ipv4) = ip_part.parse::<Ipv4Addr>() {
                if prefix_len > 32 {
                    return None;
                }
                let mask = if prefix_len == 0 {
                    0
                } else {
                    !0u32 << (32 - prefix_len)
                };
                let base = u32::from(ipv4) & mask;
                let size = if prefix_len == 32 {
                    1
                } else {
                    1u64 << (32 - prefix_len)
                };
                return Some(IpBlock::Ipv4Subnet {
                    base,
                    mask,
                    prefix_len,
                    size,
                });
            } else if let Ok(ipv6) = ip_part.parse::<Ipv6Addr>() {
                if prefix_len > 128 {
                    return None;
                }
                let mask = if prefix_len == 0 {
                    0
                } else {
                    !0u128 << (128 - prefix_len)
                };
                let base = u128::from(ipv6) & mask;
                let host_bits = 128 - prefix_len;
                let size = if host_bits >= 128 {
                    u128::MAX
                } else {
                    1u128 << host_bits
                };
                return Some(IpBlock::Ipv6Subnet {
                    base,
                    mask,
                    prefix_len,
                    size,
                });
            }
        } else if let Ok(ip) = s.parse::<IpAddr>() {
            return Some(IpBlock::Single(ip));
        }
        None
    }

    pub fn size_string(&self) -> String {
        match self {
            IpBlock::Single(_) => "1".to_string(),
            IpBlock::Ipv4Subnet { size, .. } => size.to_string(),
            IpBlock::Ipv6Subnet { size, .. } => size.to_string(),
        }
    }

    pub fn ip_type(&self) -> &'static str {
        match self {
            IpBlock::Single(IpAddr::V4(_)) | IpBlock::Ipv4Subnet { .. } => "Inet4Address",
            IpBlock::Single(IpAddr::V6(_)) | IpBlock::Ipv6Subnet { .. } => "Inet6Address",
        }
    }

    pub fn get_ip_at(&self, index: u128) -> IpAddr {
        match self {
            IpBlock::Single(ip) => *ip,
            IpBlock::Ipv4Subnet {
                base, mask, size, ..
            } => {
                let host = (index as u64 % size) as u32;
                let addr_u32 = (base & mask) | (host & !mask);
                IpAddr::V4(Ipv4Addr::from(addr_u32))
            }
            IpBlock::Ipv6Subnet {
                base, mask, size, ..
            } => {
                let host = if *size == 0 { 0 } else { index % size };
                let addr_u128 = (base & mask) | (host & !mask);
                IpAddr::V6(Ipv6Addr::from(addr_u128))
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Strategy {
    RotatingIpRoutePlanner,
    NanoIpRoutePlanner,
    RotatingNanoIpRoutePlanner,
    BalancingIpRoutePlanner,
}

impl Strategy {
    pub fn parse(s: &str) -> Self {
        match s.to_ascii_lowercase().as_str() {
            "nano" | "nanoiprouteplanner" => Strategy::NanoIpRoutePlanner,
            "rotatingnano" | "rotatingnanoiprouteplanner" => Strategy::RotatingNanoIpRoutePlanner,
            "balancing" | "balancingiprouteplanner" => Strategy::BalancingIpRoutePlanner,
            _ => Strategy::RotatingIpRoutePlanner,
        }
    }

    pub fn class_name(&self) -> &'static str {
        match self {
            Strategy::RotatingIpRoutePlanner => "RotatingIpRoutePlanner",
            Strategy::NanoIpRoutePlanner => "NanoIpRoutePlanner",
            Strategy::RotatingNanoIpRoutePlanner => "RotatingNanoIpRoutePlanner",
            Strategy::BalancingIpRoutePlanner => "BalancingIpRoutePlanner",
        }
    }
}

pub struct RoutePlanner {
    blocks: Vec<IpBlock>,
    excluded: HashSet<IpAddr>,
    strategy: Strategy,
    rotate_index: AtomicUsize,
    failing_addresses: DashMap<IpAddr, (u64, u64)>, // (failing_timestamp_ms, fail_count)
    client_cache: DashMap<IpAddr, reqwest::Client>,
    default_client: reqwest::Client,
    cooldown_secs: u64,
}

impl RoutePlanner {
    pub fn new(ip_blocks: &[String], strategy_str: &str, excluded_ips: &[String]) -> Option<Self> {
        let mut blocks = Vec::new();
        for s in ip_blocks {
            if let Some(block) = IpBlock::parse(s) {
                blocks.push(block);
            } else {
                warn!("RoutePlanner: Skipping invalid IP block/address: {}", s);
            }
        }

        if blocks.is_empty() {
            return None;
        }

        let mut excluded = HashSet::new();
        for s in excluded_ips {
            if let Ok(ip) = s.trim().parse::<IpAddr>() {
                excluded.insert(ip);
            }
        }

        let strategy = Strategy::parse(strategy_str);

        let mut default_headers = HeaderMap::new();
        default_headers.insert(
            USER_AGENT,
            HeaderValue::from_static(
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36",
            ),
        );
        default_headers.insert(ACCEPT, HeaderValue::from_static("application/json"));

        let default_client = reqwest::Client::builder()
            .default_headers(default_headers)
            .timeout(Duration::from_secs(5))
            .redirect(crate::security::redirect_policy())
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());

        info!(
            "⛩️ RoutePlanner initialized: strategy={}, blocks={}, excluded={}",
            strategy.class_name(),
            blocks.len(),
            excluded.len()
        );

        Some(Self {
            blocks,
            excluded,
            strategy,
            rotate_index: AtomicUsize::new(0),
            failing_addresses: DashMap::new(),
            client_cache: DashMap::new(),
            default_client,
            cooldown_secs: DEFAULT_FAIL_COOLDOWN_SECS,
        })
    }

    /// Selects the next available, healthy IP address according to configured strategy.
    pub fn next_ip(&self) -> IpAddr {
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        let max_attempts = 32;

        for _ in 0..max_attempts {
            let idx = match self.strategy {
                Strategy::RotatingIpRoutePlanner => {
                    self.rotate_index.fetch_add(1, Ordering::Relaxed) as u128
                }
                Strategy::NanoIpRoutePlanner => SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos(),
                Strategy::RotatingNanoIpRoutePlanner => {
                    let r = self.rotate_index.fetch_add(1, Ordering::Relaxed) as u128;
                    let nanos = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_nanos();
                    r.wrapping_add(nanos)
                }
                Strategy::BalancingIpRoutePlanner => {
                    let mut rng = rand::thread_rng();
                    rng.gen::<u128>()
                }
            };

            let block_idx = (idx % (self.blocks.len() as u128)) as usize;
            let block = &self.blocks[block_idx];
            let ip = block.get_ip_at(idx);

            if self.excluded.contains(&ip) {
                continue;
            }

            if let Some(entry) = self.failing_addresses.get(&ip) {
                let (fail_ts, _) = *entry.value();
                let elapsed_secs = (now_ms.saturating_sub(fail_ts)) / 1000;
                if elapsed_secs < self.cooldown_secs {
                    // Still in cooldown, skip this IP
                    continue;
                } else {
                    // Cooldown expired, clear failure
                    drop(entry);
                    self.failing_addresses.remove(&ip);
                }
            }

            return ip;
        }

        // If all attempts resulted in failing/excluded IPs, return from the first block
        self.blocks[0].get_ip_at(0)
    }

    /// Gets an HTTP client bound to the selected rotated IP address.
    pub fn get_client(&self) -> (reqwest::Client, Option<IpAddr>) {
        let ip = self.next_ip();

        if let Some(client) = self.client_cache.get(&ip) {
            return (client.value().clone(), Some(ip));
        }

        let mut headers = HeaderMap::new();
        headers.insert(
            USER_AGENT,
            HeaderValue::from_static(
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36",
            ),
        );
        headers.insert(ACCEPT, HeaderValue::from_static("application/json"));

        match reqwest::Client::builder()
            .default_headers(headers)
            .timeout(Duration::from_secs(5))
            .redirect(crate::security::redirect_policy())
            .local_address(Some(ip))
            .build()
        {
            Ok(client) => {
                // Evict cache if it grows too large
                if self.client_cache.len() >= MAX_CLIENT_CACHE_SIZE {
                    self.client_cache.clear();
                }
                self.client_cache.insert(ip, client.clone());
                (client, Some(ip))
            }
            Err(e) => {
                warn!(
                    "RoutePlanner: Could not bind local address {}: {}. Falling back to default interface.",
                    ip, e
                );
                (self.default_client.clone(), Some(ip))
            }
        }
    }

    /// Mark an IP address as failing (called on HTTP 429, 403, or connection drop).
    pub fn mark_failed(&self, addr: IpAddr) {
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        self.failing_addresses
            .entry(addr)
            .and_modify(|(_ts, count)| {
                *_ts = now_ms;
                *count += 1;
            })
            .or_insert((now_ms, 1));

        warn!(
            "⚠️ RoutePlanner: Marked {} as failing at {}ms",
            addr, now_ms
        );
    }

    /// Unmark a specific failing address (Lavalink REST endpoint).
    pub fn unmark(&self, addr: IpAddr) -> bool {
        let removed = self.failing_addresses.remove(&addr).is_some();
        if removed {
            info!("RoutePlanner: Unmarked failing address {}", addr);
        }
        removed
    }

    /// Unmark all failing addresses (Lavalink REST endpoint).
    pub fn unmark_all(&self) {
        self.failing_addresses.clear();
        info!("RoutePlanner: Cleared all failing addresses");
    }

    /// Generate the Lavalink v4 JSON status response.
    pub fn status_json(&self) -> serde_json::Value {
        let primary_block = self
            .blocks
            .first()
            .cloned()
            .unwrap_or(IpBlock::Single(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1))));

        let failing: Vec<serde_json::Value> = self
            .failing_addresses
            .iter()
            .map(|entry| {
                let addr = entry.key();
                let (ts, _) = *entry.value();
                serde_json::json!({
                    "failingAddress": format!("/{}", addr),
                    "failingTimestamp": ts,
                    "failingTime": format!("{} (Unix Timestamp ms)", ts),
                })
            })
            .collect();

        let rotate_idx = self.rotate_index.load(Ordering::Relaxed);
        let curr_ip = primary_block.get_ip_at(rotate_idx as u128);

        serde_json::json!({
            "class": self.strategy.class_name(),
            "details": {
                "ipBlock": {
                    "type": primary_block.ip_type(),
                    "size": primary_block.size_string(),
                },
                "failingAddresses": failing,
                "rotateIndex": rotate_idx.to_string(),
                "ipIndex": (rotate_idx % 1000).to_string(),
                "currentAddress": format!("/{}", curr_ip),
                "currentAddressIndex": rotate_idx.to_string(),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ip_block_parsing_single_ipv4() {
        let block = IpBlock::parse("192.168.1.1").unwrap();
        assert_eq!(block.ip_type(), "Inet4Address");
        assert_eq!(block.size_string(), "1");
        assert_eq!(
            block.get_ip_at(0),
            IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1))
        );
    }

    #[test]
    fn test_ip_block_parsing_ipv4_cidr() {
        let block = IpBlock::parse("192.168.1.0/24").unwrap();
        assert_eq!(block.ip_type(), "Inet4Address");
        assert_eq!(block.size_string(), "256");
        assert_eq!(
            block.get_ip_at(0),
            IpAddr::V4(Ipv4Addr::new(192, 168, 1, 0))
        );
        assert_eq!(
            block.get_ip_at(10),
            IpAddr::V4(Ipv4Addr::new(192, 168, 1, 10))
        );
        assert_eq!(
            block.get_ip_at(255),
            IpAddr::V4(Ipv4Addr::new(192, 168, 1, 255))
        );
    }

    #[test]
    fn test_ip_block_parsing_ipv6_cidr() {
        let block = IpBlock::parse("2001:db8::/64").unwrap();
        assert_eq!(block.ip_type(), "Inet6Address");
        assert_eq!(block.size_string(), "18446744073709551616");
        let ip_0 = block.get_ip_at(0);
        let ip_1 = block.get_ip_at(1);
        assert_ne!(ip_0, ip_1);
    }

    #[test]
    fn test_route_planner_rotation() {
        let ips = vec![
            "192.168.1.1".to_string(),
            "192.168.1.2".to_string(),
            "192.168.1.3".to_string(),
        ];
        let rp = RoutePlanner::new(&ips, "RotatingIpRoutePlanner", &[]).unwrap();
        let ip1 = rp.next_ip();
        let ip2 = rp.next_ip();
        let ip3 = rp.next_ip();
        let ip4 = rp.next_ip();

        assert_ne!(ip1, ip2);
        assert_ne!(ip2, ip3);
        assert_eq!(ip1, ip4);
    }

    #[test]
    fn test_route_planner_mark_failing_and_unmark() {
        let ips = vec!["10.0.0.1".to_string(), "10.0.0.2".to_string()];
        let rp = RoutePlanner::new(&ips, "RotatingIpRoutePlanner", &[]).unwrap();

        let failing_ip = "10.0.0.1".parse::<IpAddr>().unwrap();
        rp.mark_failed(failing_ip);

        // Next IPs should avoid 10.0.0.1
        let ip = rp.next_ip();
        assert_eq!(ip, "10.0.0.2".parse::<IpAddr>().unwrap());

        // Status json should reflect failing address
        let status = rp.status_json();
        let list = status["details"]["failingAddresses"].as_array().unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0]["failingAddress"], "/10.0.0.1");

        // Unmark address
        assert!(rp.unmark(failing_ip));
        assert_eq!(rp.failing_addresses.len(), 0);
    }

    #[test]
    fn test_route_planner_unmark_all() {
        let ips = vec!["10.0.0.1".to_string(), "10.0.0.2".to_string()];
        let rp = RoutePlanner::new(&ips, "RotatingIpRoutePlanner", &[]).unwrap();

        rp.mark_failed("10.0.0.1".parse().unwrap());
        rp.mark_failed("10.0.0.2".parse().unwrap());
        assert_eq!(rp.failing_addresses.len(), 2);

        rp.unmark_all();
        assert_eq!(rp.failing_addresses.len(), 0);
    }

    #[test]
    fn test_empty_blocks_disabled() {
        assert!(RoutePlanner::new(&[], "RotatingIpRoutePlanner", &[]).is_none());
    }
}
