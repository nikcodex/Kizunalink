// Copyright (c) 2026 nikcodex (KizunaLink)
// Licensed under the MIT License

use std::{
    io,
    net::{IpAddr, SocketAddr},
    time::Duration,
};

use reqwest::{Client, Proxy, header::HeaderMap};
use tracing::warn;

use crate::{common::types::AnyResult, config::sources::HttpProxyConfig};

/// Check whether an IP address is in a blocked range (loopback, private,
/// link-local, multicast, unspecified, IPv4-mapped IPv6 loopback, ULA, metadata).
pub(crate) fn is_blocked_ip(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            let octets = ip.octets();
            ip.is_loopback()
                || ip.is_private()
                || ip.is_link_local()
                || ip.is_multicast()
                || ip.is_unspecified()
                || (octets[0] == 100 && (octets[1] & 0xc0) == 64) // CGNAT 100.64.0.0/10
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

fn validate_resolved_addresses(addresses: &[SocketAddr]) -> io::Result<()> {
    if addresses.is_empty() {
        return Err(io::Error::other("hostname resolved to no addresses"));
    }
    if addresses.iter().any(|address| is_blocked_ip(&address.ip())) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "hostname resolved to a private or local address",
        ));
    }
    Ok(())
}

/// Resolve asynchronously and filter before handing addresses to Hyper. This
/// avoids blocking Tokio workers and makes validation apply to the actual
/// addresses the connector receives, including every redirect destination.
struct PublicDnsResolver;

impl reqwest::dns::Resolve for PublicDnsResolver {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let host = name.as_str().to_owned();
        Box::pin(async move {
            let addresses: Vec<SocketAddr> =
                tokio::net::lookup_host((host.as_str(), 0)).await?.collect();
            validate_resolved_addresses(&addresses)?;
            Ok(Box::new(addresses.into_iter()) as reqwest::dns::Addrs)
        })
    }
}

fn validate_redirect_url(from: &reqwest::Url, target: &reqwest::Url) -> bool {
    if !matches!(target.scheme(), "http" | "https")
        || (from.scheme() == "https" && target.scheme() == "http")
    {
        return false;
    }
    let Some(host) = target.host_str() else {
        return false;
    };
    let lower_host = host.to_ascii_lowercase();
    if lower_host == "localhost"
        || lower_host.ends_with(".localhost")
        || lower_host.ends_with(".internal")
        || lower_host.ends_with(".local")
        || lower_host == "metadata.google.internal"
    {
        return false;
    }
    let ip_host = host
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
        .unwrap_or(host);
    !ip_host.parse::<IpAddr>().is_ok_and(|ip| is_blocked_ip(&ip))
}

/// Custom redirect policy that re-validates every redirect hop.
/// Rejects:
/// - Non-http(s) schemes
/// - HTTPS→HTTP downgrade
/// - Redirect targets resolving to private/loopback/link-local/multicast/unspecified IPs
fn make_redirect_policy() -> reqwest::redirect::Policy {
    reqwest::redirect::Policy::custom(|attempt| {
        let Some(from) = attempt.previous().last() else {
            warn!("SSRF: rejecting redirect without a previous URL");
            return attempt.stop();
        };
        if attempt.previous().len() > 10 || !validate_redirect_url(from, attempt.url()) {
            // Redirect URLs may contain signed tokens in their query strings.
            warn!("SSRF: rejecting unsafe or excessive redirect");
            attempt.stop()
        } else {
            attempt.follow()
        }
    })
}

pub fn create_client(
    user_agent: String,
    local_addr: Option<IpAddr>,
    proxy: Option<HttpProxyConfig>,
    headers: Option<HeaderMap>,
) -> AnyResult<Client> {
    create_client_with_pinning(user_agent, local_addr, proxy, headers, None, None)
}

pub fn create_client_with_pinning(
    user_agent: String,
    local_addr: Option<IpAddr>,
    proxy: Option<HttpProxyConfig>,
    headers: Option<HeaderMap>,
    pinned_host: Option<String>,
    pinned_ips: Option<Vec<SocketAddr>>,
) -> AnyResult<Client> {
    // A forwarding proxy resolves the destination itself: reqwest's DNS
    // validator and address pins cannot constrain that connection. Refuse
    // this combination for user-supplied HTTP sources rather than silently
    // bypassing the SSRF boundary. Trusted provider clients may still proxy.
    if pinned_host.is_some() && proxy.as_ref().and_then(|p| p.url.as_ref()).is_some() {
        return Err("pinned HTTP source cannot use a forwarding proxy".into());
    }
    if let Some(ips) = &pinned_ips {
        validate_resolved_addresses(ips)?;
    }
    let mut builder = Client::builder()
        .user_agent(user_agent)
        .connect_timeout(Duration::from_secs(5))
        .read_timeout(Duration::from_secs(8))
        .tcp_nodelay(true)
        .tcp_keepalive(Duration::from_secs(25))
        .pool_max_idle_per_host(64)
        .pool_idle_timeout(Duration::from_secs(70))
        // Don't inherit environment proxy settings: the proxy could resolve
        // names outside this process's address validation policy.
        .no_proxy()
        .dns_resolver(std::sync::Arc::new(PublicDnsResolver));

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

#[cfg(test)]
mod tests {
    use super::{is_blocked_ip, validate_redirect_url, validate_resolved_addresses};
    use std::{
        net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
        str::FromStr,
    };

    fn url(value: &str) -> reqwest::Url {
        reqwest::Url::parse(value).expect("test URL should parse")
    }

    #[test]
    fn rejects_direct_loopback_private_and_mapped_ipv6_redirects() {
        let from = url("https://media.example/track");
        for target in [
            "http://localhost/audio",
            "http://127.0.0.1/audio",
            "http://10.0.0.4/audio",
            "http://[::1]/audio",
            "http://[::ffff:127.0.0.1]/audio",
        ] {
            assert!(
                !validate_redirect_url(&from, &url(target)),
                "unsafe target should be rejected: {target}"
            );
        }
        assert!(is_blocked_ip(&IpAddr::V6(
            Ipv6Addr::from_str("::ffff:127.0.0.1").unwrap()
        )));
    }

    #[test]
    fn rejects_public_hostname_that_resolves_to_private_address() {
        // The resolver validates the exact socket addresses it passes to
        // Hyper, so a public-looking hostname cannot rebind to a private IP.
        let private = [SocketAddr::from((Ipv4Addr::new(10, 20, 30, 40), 0))];
        assert!(validate_resolved_addresses(&private).is_err());
        let mapped = [SocketAddr::from((
            Ipv6Addr::from_str("::ffff:127.0.0.1").unwrap(),
            0,
        ))];
        assert!(validate_resolved_addresses(&mapped).is_err());
        let public = [SocketAddr::from((Ipv4Addr::new(93, 184, 216, 34), 0))];
        assert!(validate_resolved_addresses(&public).is_ok());
    }

    #[test]
    fn rejects_private_redirect_at_every_hop() {
        let first_hop = url("https://public.example/audio");
        let public_second_hop = url("https://cdn.example/redirect");
        let private_final_hop = url("http://169.254.169.254/latest/meta-data/");
        assert!(validate_redirect_url(&first_hop, &public_second_hop));
        assert!(!validate_redirect_url(
            &public_second_hop,
            &private_final_hop
        ));
    }

    #[test]
    fn rejects_https_downgrade_but_allows_public_to_public_redirect() {
        let https_from = url("https://media.example/audio");
        assert!(!validate_redirect_url(
            &https_from,
            &url("http://cdn.example/audio")
        ));
        assert!(validate_redirect_url(
            &https_from,
            &url("https://cdn.example/audio")
        ));
    }
}

#[cfg(test)]
mod pinned_client_tests {
    use super::create_client_with_pinning;
    use crate::config::sources::HttpProxyConfig;
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};

    #[test]
    fn refuses_forwarding_proxy_for_pinned_user_supplied_source() {
        let proxy = HttpProxyConfig { url: Some("http://proxy.example:8080".into()), ..Default::default() };
        let result = create_client_with_pinning("test".into(), None, Some(proxy), None,
            Some("media.example".into()), Some(vec![SocketAddr::from((Ipv4Addr::new(93, 184, 216, 34), 80))]));
        assert!(result.is_err(), "a forwarding proxy bypasses the pinned destination");
    }

    #[test]
    fn refuses_private_pinned_address_even_when_caller_supplies_it() {
        let result = create_client_with_pinning("test".into(), None, None, None,
            Some("media.example".into()), Some(vec![SocketAddr::from((Ipv4Addr::new(10, 0, 0, 1), 80))]));
        assert!(result.is_err());
        let public = IpAddr::V4(Ipv4Addr::new(93, 184, 216, 34));
        assert!(!super::is_blocked_ip(&public));
    }
}
