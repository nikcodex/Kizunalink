// Copyright (c) 2026 nikcodex (KizunaLink)
// Licensed under the MIT License

use std::{
    net::{IpAddr, SocketAddr},
    time::Duration,
};

use reqwest::{Client, Proxy, header::HeaderMap};
use tracing::warn;

use crate::{common::types::AnyResult, config::sources::HttpProxyConfig};

/// Check whether an IP address is in a blocked range (loopback, private,
/// link-local, multicast, unspecified, IPv4-mapped IPv6 loopback, ULA, metadata).
fn is_blocked_ip(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            ip.is_loopback()
                || ip.is_private()
                || ip.is_link_local()
                || ip.is_multicast()
                || ip.is_unspecified()
                || ip.is_shared() // carrier-grade NAT 100.64.0.0/10
                || ip.is_documentation() // 192.0.2.0/24 etc.
                || (ip.is_private() && ip.is_unspecified())
        }
        IpAddr::V6(ip) => {
            // Check for IPv4-mapped IPv6 addresses (::ffff:a.b.c.d)
            if let Some(mapped_v4) = ip.to_ipv4_mapped() {
                return is_blocked_ip(&IpAddr::V4(mapped_v4));
            }
            let segments = ip.segments();
            let first = segments[0];
            ip.is_loopback()
                || ip.is_multicast()
                || ip.is_unspecified()
                || (first & 0xfe00) == 0xfc00 // unique-local fc00::/7
                || (first & 0xffc0) == 0xfe80 // link-local fe80::/10
        }
    }
}

/// Custom redirect policy that re-validates every redirect hop.
/// Rejects:
/// - Non-http(s) schemes
/// - HTTPS→HTTP downgrade
/// - Redirect targets resolving to private/loopback/link-local/multicast/unspecified IPs
fn make_redirect_policy() -> reqwest::redirect::Policy {
    reqwest::redirect::Policy::custom(|resp: &reqwest::Response| -> reqwest::redirect::Action {
        let location = match resp.headers().get(reqwest::header::LOCATION) {
            Some(loc) => loc,
            None => reqwest::redirect::Action::Abort,
        };

        let location_str = match location.to_str() {
            Ok(s) => s,
            Err(_) => reqwest::redirect::Action::Abort,
        };

        // Resolve relative redirect targets against the current URL
        let redirect_url = match resp.url().join(location_str) {
            Ok(url) => url,
            Err(_) => {
                // Try parsing as absolute URL
                match reqwest::Url::parse(location_str) {
                    Ok(url) => url,
                    Err(_) => reqwest::redirect::Action::Abort,
                }
            }
        };

        // Reject non-http schemes
        if !matches!(redirect_url.scheme(), "http" | "https") {
            warn!(
                "SSRF: rejecting redirect to unsupported scheme '{}'",
                redirect_url.scheme()
            );
            return reqwest::redirect::Action::Abort;
        }

        // Reject HTTPS→HTTP downgrade
        if resp.url().scheme() == "https" && redirect_url.scheme() == "http" {
            warn!("SSRF: rejecting HTTPS→HTTP downgrade redirect");
            return reqwest::redirect::Action::Abort;
        }

        // Validate redirect target host resolves to a public IP
        let host = match redirect_url.host_str() {
            Some(h) => h,
            None => return reqwest::redirect::Action::Abort,
        };
        let port = redirect_url.port_or_known_default().unwrap_or(80);

        // Block localhost hostnames (without DNS resolution)
        let lower_host = host.to_lowercase();
        if lower_host == "localhost"
            || lower_host.ends_with(".localhost")
            || lower_host.ends_with(".internal")
            || lower_host.ends_with(".local")
            || lower_host == "metadata.google.internal"
        {
            warn!("SSRF: rejecting redirect to blocked hostname '{lower_host}'");
            return reqwest::redirect::Action::Abort;
        }

        // Block IP literals in redirect target (catches 127.0.0.1, 0.0.0.0, ::1, etc.)
        if let Ok(ip) = host.parse::<IpAddr>() {
            if is_blocked_ip(&ip) {
                warn!("SSRF: rejecting redirect to blocked IP literal '{host}'");
                return reqwest::redirect::Action::Abort;
            }
        }

        // Resolve and validate redirect target IPs (blocking but security-critical)
        let addresses: Vec<_> = match (host, port).to_socket_addrs() {
            Ok(addrs) => addrs.collect(),
            Err(e) => {
                warn!("SSRF: failed to resolve redirect target '{host}': {e}");
                return reqwest::redirect::Action::Abort;
            }
        };

        if addresses.is_empty() {
            warn!("SSRF: redirect target '{host}' resolved to no addresses");
            return reqwest::redirect::Action::Abort;
        }

        for addr in &addresses {
            if is_blocked_ip(&addr.ip()) {
                warn!(
                    "SSRF: rejecting redirect to '{host}' which resolves to blocked IP {}",
                    addr.ip()
                );
                return reqwest::redirect::Action::Abort;
            }
        }

        reqwest::redirect::Action::Follow
    })
}

pub fn create_client(
    user_agent: String,
    local_addr: Option<IpAddr>,
    proxy: Option<HttpProxyConfig>,
    headers: Option<HeaderMap>,
    pinned_host: Option<String>,
    pinned_ips: Option<Vec<SocketAddr>>,
) -> AnyResult<Client> {
    let mut builder = Client::builder()
        .user_agent(user_agent)
        .connect_timeout(Duration::from_secs(5))
        .read_timeout(Duration::from_secs(8))
        .tcp_nodelay(true)
        .tcp_keepalive(Duration::from_secs(25))
        .pool_max_idle_per_host(64)
        .pool_idle_timeout(Duration::from_secs(70));

    if let Some(headers) = headers {
        builder = builder.default_headers(headers);
    }

    if let Some(ip) = local_addr {
        builder = builder.local_address(ip);
    }

    if let Some(p_cfg) = proxy
        && let Some(p_url) = p_cfg.url
    {
        match Proxy::all(&p_url) {
            Ok(mut p) => {
                if let (Some(u), Some(pw)) = (p_cfg.username, p_cfg.password) {
                    p = p.basic_auth(&u, &pw);
                }
                builder = builder.proxy(p);
            }
            Err(e) => warn!("Failed to parse proxy URL '{}': {}", p_url, e),
        }
    }

    // Pin resolved IPs to prevent DNS rebinding (SEC-001).
    if let (Some(host), Some(ips)) = (pinned_host, pinned_ips) {
        for addr in &ips {
            builder = builder.resolve(&host, *addr);
        }
    }

    // Custom redirect policy: validate every redirect hop (SEC-001).
    builder = builder.redirect(make_redirect_policy());

    Ok(builder.build()?)
}

