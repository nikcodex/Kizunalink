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

/// Check whether an IP address is in a blocked range (SEC-001).
///
/// Blocks loopback, private (RFC1918), link-local, multicast, unspecified,
/// broadcast, CGNAT, documentation, benchmarking, and other reserved/non-global
/// IPv4 ranges, plus IPv6 loopback/link-local/ULA/multicast/unspecified,
/// IPv4-mapped, IPv4-compatible, NAT64, 6to4, Teredo, and documentation ranges.
///
/// The policy is "only global unicast addresses may be contacted". Everything
/// else is refused, so a new RFC reservation defaults to *blocked* rather than
/// silently reachable.
pub(crate) fn is_blocked_ip(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            let o = ip.octets();
            ip.is_loopback()
                || ip.is_private()
                || ip.is_link_local()
                || ip.is_multicast()
                || ip.is_unspecified()
                || ip.is_broadcast()
                || ip.is_documentation()
                // "This network" 0.0.0.0/8 and IETF protocol assignments 192.0.0.0/24.
                || o[0] == 0
                || (o[0] == 192 && o[1] == 0 && o[2] == 0)
                // Deprecated 6to4 relay anycast 192.88.99.0/24.
                || (o[0] == 192 && o[1] == 88 && o[2] == 99)
                // Shared address space / CGNAT 100.64.0.0/10.
                || (o[0] == 100 && (o[1] & 0xc0) == 64)
                // Benchmarking 198.18.0.0/15.
                || (o[0] == 198 && (o[1] & 0xfe) == 18)
                // Reserved for future use 240.0.0.0/4 (0xF0 ..= 0xFF) incl. broadcast.
                || o[0] >= 0xf0
        }
        IpAddr::V6(ip) => {
            // IPv4-mapped (::ffff:a.b.c.d) embeds a v4 address; unmap and apply
            // the v4 policy so `::ffff:127.0.0.1` cannot slip past the v6 checks.
            if let Some(mapped_v4) = ip.to_ipv4_mapped() {
                return is_blocked_ip(&IpAddr::V4(mapped_v4));
            }
            let seg = ip.segments();
            let first = seg[0];
            // Allowlist: only global unicast 2000::/3 may be contacted. Everything
            // outside it — loopback, unspecified, multicast, ULA, link-local,
            // IPv4-compatible `::a.b.c.d`, NAT64 `64:ff9b::/96`, discard-only
            // `100::/64`, segment-routing `5f00::/16`, ... — is refused. New
            // reservations therefore default to blocked.
            if (first & 0xe000) != 0x2000 {
                return true;
            }
            // Special-purpose ranges *inside* 2000::/3.
            // 2001::/23 (Teredo 2001::/32, benchmarking 2001:2::/48, ORCHID
            // 2001:10::/28, ORCHIDv2 2001:20::/28, ...).
            if first == 0x2001 && seg[1] < 0x0200 {
                return true;
            }
            // Documentation 2001:db8::/32.
            if first == 0x2001 && seg[1] == 0x0db8 {
                return true;
            }
            // 6to4 2002::/16 (reaches the embedded v4).
            if first == 0x2002 {
                return true;
            }
            // Documentation 3fff::/20.
            if first == 0x3fff && seg[1] < 0x1000 {
                return true;
            }
            false
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
    fn redirect_policy_rejects_local_and_internal_hostnames_before_dns() {
        // The redirect hop is checked by name, before any DNS lookup, so a
        // redirect into the local network is stopped even though `can_handle`
        // style syntax checks cannot see where the name resolves.
        let from = url("https://media.example/track");
        for target in [
            "http://localhost/audio",
            "http://api.localhost/audio",
            "http://service.internal/audio",
            "http://printer.local/audio",
            "http://metadata.google.internal/computeMetadata/v1/",
        ] {
            assert!(
                !validate_redirect_url(&from, &url(target)),
                "local/internal name should be rejected: {target}"
            );
        }
        // Ordinary public names still pass the name check (connect-time address
        // validation is the authoritative layer for those).
        assert!(validate_redirect_url(&from, &url("https://cdn.example/a")));
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

    #[test]
    fn blocks_all_non_global_ipv4_ranges() {
        use Ipv4Addr as V4;
        // Only global unicast is allowed; everything reserved/non-public is
        // refused so a new RFC reservation defaults to blocked.
        for ip in [
            V4::new(0, 0, 0, 0),         // "this network" 0.0.0.0/8
            V4::new(0, 1, 2, 3),         // 0.0.0.0/8
            V4::new(127, 0, 0, 1),       // loopback
            V4::new(10, 0, 0, 1),        // private
            V4::new(172, 16, 0, 1),      // private
            V4::new(192, 168, 1, 1),     // private
            V4::new(169, 254, 169, 254), // link-local / cloud metadata
            V4::new(100, 64, 0, 1),      // CGNAT 100.64/10
            V4::new(100, 127, 255, 255), // CGNAT upper bound
            V4::new(192, 0, 0, 1),       // IETF protocol assignments 192.0.0.0/24
            V4::new(192, 0, 2, 1),       // TEST-NET-1 192.0.2.0/24
            V4::new(198, 18, 0, 1),      // benchmarking 198.18.0.0/15
            V4::new(198, 19, 255, 255),  // benchmarking upper bound
            V4::new(224, 0, 0, 1),       // multicast
            V4::new(240, 0, 0, 1),       // reserved 240.0.0.0/4
            V4::new(255, 255, 255, 255), // broadcast
        ] {
            assert!(is_blocked_ip(&IpAddr::V4(ip)), "{ip} must be blocked");
        }

        for ip in [
            V4::new(8, 8, 8, 8),
            V4::new(1, 1, 1, 1),
            V4::new(93, 184, 216, 34),
            V4::new(100, 63, 255, 255), // just below CGNAT
            V4::new(100, 128, 0, 0),    // just above CGNAT
            V4::new(198, 17, 255, 255), // just below benchmarking
            V4::new(198, 20, 0, 0),     // just above benchmarking
            V4::new(192, 88, 98, 255),  // below 6to4-relay 192.88.99.0/24
            V4::new(192, 88, 100, 0),   // above 6to4-relay 192.88.99.0/24
        ] {
            assert!(!is_blocked_ip(&IpAddr::V4(ip)), "{ip} must be allowed");
        }
    }

    #[test]
    fn blocks_iana_special_ranges_found_by_oracle_diff() {
        use Ipv4Addr as V4;
        use Ipv6Addr as V6;
        // These were missed before an independent oracle diff caught them.
        assert!(is_blocked_ip(&IpAddr::V4(V4::new(192, 88, 99, 1))));
        for ip in [
            "100::1",
            "2001:2::1",
            "2001:10::1",
            "2001:20::1",
            "3fff::1",
            "5f00::1",
        ] {
            assert!(
                is_blocked_ip(&IpAddr::V6(V6::from_str(ip).unwrap())),
                "{ip} must be blocked"
            );
        }
        // Global unicast just outside the special sub-ranges stays reachable.
        for ip in ["2001:4860:4860::8888", "2400:cb00::1", "2a00:1450:4001::1"] {
            assert!(
                !is_blocked_ip(&IpAddr::V6(V6::from_str(ip).unwrap())),
                "{ip} must be allowed"
            );
        }
    }

    #[test]
    fn blocks_ipv6_ranges_that_embed_or_reach_other_addresses() {
        use Ipv6Addr as V6;
        let parse = |s: &str| V6::from_str(s).unwrap();
        for ip in [
            "::1",                    // loopback
            "::",                     // unspecified
            "fe80::1",                // link-local
            "fc00::1",                // ULA
            "fdff::1",                // ULA upper
            "ff02::1",                // multicast
            "2001:db8::1",            // documentation
            "::ffff:127.0.0.1",       // v4-mapped loopback
            "::ffff:169.254.169.254", // v4-mapped metadata
            "::ffff:10.0.0.1",        // v4-mapped private
            "::127.0.0.1",            // IPv4-compatible loopback
            "64:ff9b::7f00:1",        // NAT64 → 127.0.0.1
            "64:ff9b::a9fe:a9fe",     // NAT64 → 169.254.169.254
            "2002:7f00:1::",          // 6to4 → 127.0.0.1
            "2001::1",                // Teredo
        ] {
            let addr = parse(ip);
            assert!(is_blocked_ip(&IpAddr::V6(addr)), "{ip} must be blocked");
        }

        for ip in ["2606:4700:4700::1111", "2001:4860:4860::8888"] {
            let addr = parse(ip);
            assert!(!is_blocked_ip(&IpAddr::V6(addr)), "{ip} must be allowed");
        }
    }
}

#[cfg(test)]
mod pinned_client_tests {
    use super::create_client_with_pinning;
    use crate::config::sources::HttpProxyConfig;
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};

    #[test]
    fn refuses_forwarding_proxy_for_pinned_user_supplied_source() {
        let proxy = HttpProxyConfig {
            url: Some("http://proxy.example:8080".into()),
            ..Default::default()
        };
        let result = create_client_with_pinning(
            "test".into(),
            None,
            Some(proxy),
            None,
            Some("media.example".into()),
            Some(vec![SocketAddr::from((
                Ipv4Addr::new(93, 184, 216, 34),
                80,
            ))]),
        );
        assert!(
            result.is_err(),
            "a forwarding proxy bypasses the pinned destination"
        );
    }

    #[test]
    fn refuses_private_pinned_address_even_when_caller_supplies_it() {
        let result = create_client_with_pinning(
            "test".into(),
            None,
            None,
            None,
            Some("media.example".into()),
            Some(vec![SocketAddr::from((Ipv4Addr::new(10, 0, 0, 1), 80))]),
        );
        assert!(result.is_err());
        let public = IpAddr::V4(Ipv4Addr::new(93, 184, 216, 34));
        assert!(!super::is_blocked_ip(&public));
    }
}

/// Deterministic, hermetic SSRF regression tests (SEC-001).
///
/// All targets are loopback fixtures the test itself binds on an ephemeral port;
/// nothing external is contacted. The tests prove that the IP address actually
/// used for the connection is subject to the policy — not merely an earlier
/// lookup — for DNS rebinding, redirects, and a malicious resolver.
#[cfg(test)]
mod ssrf_regression_tests {
    use super::{
        Client, SocketAddr, create_client, create_client_with_pinning, make_redirect_policy,
    };
    use std::{
        io::{Read, Write},
        net::{Ipv4Addr, TcpListener},
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
        time::Duration,
    };

    struct Fixture {
        port: u16,
        hits: Arc<AtomicUsize>,
    }

    /// Spawn a single-shot HTTP responder on loopback, recording the number of
    /// requests received.
    fn fixture(body: &str) -> Fixture {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind fixture");
        let port = listener.local_addr().unwrap().port();
        let hits = Arc::new(AtomicUsize::new(0));
        let h = hits.clone();
        let body = body.to_owned();
        std::thread::spawn(move || {
            for mut s in listener.incoming().flatten() {
                let mut buf = [0u8; 4096];
                let _ = s.read(&mut buf);
                h.fetch_add(1, Ordering::SeqCst);
                let _ = s.write_all(body.as_bytes());
                let _ = s.flush();
            }
        });
        Fixture { port, hits }
    }

    fn ok_body() -> String {
        "HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok".to_owned()
    }

    fn redirect_body(location: &str) -> String {
        format!(
            "HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        )
    }

    async fn settle() {
        tokio::time::sleep(Duration::from_millis(150)).await;
    }

    #[tokio::test]
    async fn hostname_resolving_to_private_is_refused_at_connect() {
        // `localhost` genuinely resolves to 127.0.0.1; the connect-time resolver
        // must refuse it, so the fixture is never contacted.
        let server = fixture(&ok_body());
        let client = create_client("test".into(), None, None, None).unwrap();
        let result = client
            .get(format!("http://localhost:{}/x", server.port))
            .send()
            .await;
        assert!(result.is_err(), "loopback hostname must not connect");
        settle().await;
        assert_eq!(
            server.hits.load(Ordering::SeqCst),
            0,
            "fixture was contacted"
        );
    }

    #[tokio::test]
    async fn dns_rebinding_cannot_reach_loopback() {
        // Simulate rebinding: the URL host resolves to loopback, but validation
        // pinned a *public* address. The connector must use the pinned address
        // (unreachable here) and must NOT fall back to the rebinding lookup.
        let server = fixture(&ok_body());
        let pinned_public = SocketAddr::from((Ipv4Addr::new(93, 184, 216, 34), server.port));
        let client = create_client_with_pinning(
            "test".into(),
            None,
            None,
            None,
            Some("localhost".into()),
            Some(vec![pinned_public]),
        )
        .unwrap();
        let result = client
            .get(format!("http://localhost:{}/x", server.port))
            .send()
            .await;
        assert!(
            result.is_err(),
            "pinned public IP must not fall back to loopback"
        );
        settle().await;
        assert_eq!(
            server.hits.load(Ordering::SeqCst),
            0,
            "rebinding fallback reached the loopback fixture"
        );
    }

    #[tokio::test]
    async fn redirect_to_private_is_not_followed() {
        // First hop is a public-looking redirect that points at a private target.
        // The custom redirect policy must leave the 302 un-followed, so the
        // private fixture records zero hits.
        let private = fixture(&ok_body());
        let first = fixture(&redirect_body(&format!(
            "http://127.0.0.1:{}/secret",
            private.port
        )));
        let client = Client::builder()
            .redirect(make_redirect_policy())
            .build()
            .unwrap();
        let response = client
            .get(format!("http://127.0.0.1:{}/start", first.port))
            .send()
            .await
            .expect("first hop should respond");
        assert_eq!(response.status(), 302, "redirect must NOT be followed");
        settle().await;
        assert_eq!(first.hits.load(Ordering::SeqCst), 1);
        assert_eq!(
            private.hits.load(Ordering::SeqCst),
            0,
            "private redirect target was contacted"
        );
    }

    #[tokio::test]
    async fn redirect_to_metadata_ip_is_not_followed() {
        // A 302 to a link-local metadata address must be left un-followed.
        let first = fixture(&redirect_body("http://169.254.169.254/latest/meta-data/"));
        let client = Client::builder()
            .redirect(make_redirect_policy())
            .build()
            .unwrap();
        let response = client
            .get(format!("http://127.0.0.1:{}/start", first.port))
            .send()
            .await
            .expect("first hop should respond");
        assert_eq!(
            response.status(),
            302,
            "metadata redirect must NOT be followed"
        );
        settle().await;
        assert_eq!(first.hits.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn pinning_uses_pinned_ip_but_keeps_hostname_for_host_and_sni() {
        // TLS SNI and the HTTP `Host` header come from the URL hostname, never
        // from the pinned address: `ClientBuilder::resolve` only changes the dialed
        // IP. Pin a public hostname to an unroutable public IP and assert (a) the
        // connector dials the pinned IP rather than a fresh lookup, and (b) the
        // request URL — the source of Host/SNI — still carries the hostname.
        let pinned = SocketAddr::from((Ipv4Addr::new(93, 184, 216, 34), 443));
        let client = create_client_with_pinning(
            "test".into(),
            None,
            None,
            None,
            Some("media.example".into()),
            Some(vec![pinned]),
        )
        .unwrap();
        let err = client
            .get("https://media.example/audio.mp3")
            .send()
            .await
            .expect_err("unroutable pinned IP must fail to connect");

        assert_eq!(
            err.url().expect("error carries the request URL").host_str(),
            Some("media.example"),
            "pinning must preserve the hostname used for Host/SNI"
        );
        let text = format!("{err:?}");
        assert!(
            text.contains(&pinned.ip().to_string()),
            "expected a connect to the pinned IP, got: {text}"
        );
        assert!(
            !text.to_ascii_lowercase().contains("dns error"),
            "pinning must bypass a fresh DNS lookup, got: {text}"
        );
    }
}
