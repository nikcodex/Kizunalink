use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use url::Url;

/// Maximum allowed request body size (1 MB).
pub const MAX_BODY_SIZE: usize = 1_048_576;

/// Maximum allowed URL length.
pub const MAX_URL_LENGTH: usize = 2048;

/// Maximum allowed search query length.
pub const MAX_QUERY_LENGTH: usize = 512;

/// Check if an IPv4 address is in a private, loopback, link-local, or reserved range.
pub fn is_private_ipv4(ipv4: Ipv4Addr) -> bool {
    let octets = ipv4.octets();
    ipv4.is_loopback()                           // 127.0.0.0/8
        || ipv4.is_private()                     // 10.0.0.0/8, 172.16.0.0/12, 192.168.0.0/16
        || ipv4.is_link_local()                  // 169.254.0.0/16
        || ipv4.is_broadcast()                   // 255.255.255.255
        || ipv4.is_documentation()               // 192.0.2.0/24, 198.51.100.0/24, 203.0.113.0/24
        || ipv4.is_unspecified()                 // 0.0.0.0
        || octets[0] == 0                        // 0.0.0.0/8
        || (octets[0] == 100 && (64..=127).contains(&octets[1])) // 100.64.0.0/10 Carrier-grade NAT (RFC 6598)
        || (octets[0] == 192 && octets[1] == 0 && octets[2] == 0) // 192.0.0.0/24 IETF Protocol Assignments
        || (octets[0] == 198 && (18..=19).contains(&octets[1]))  // 198.18.0.0/15 Network Benchmark (RFC 2544)
        || octets[0] >= 224 // 224.0.0.0/4 Multicast & 240.0.0.0/4 Reserved
}

/// Check if an IPv6 address is in a private, loopback, link-local, or reserved range.
pub fn is_private_ipv6(ipv6: Ipv6Addr) -> bool {
    if ipv6.is_loopback() || ipv6.is_unspecified() || ipv6.is_multicast() {
        return true;
    }
    // Check IPv4-mapped IPv6 (::ffff:x.x.x.x) or IPv4-compatible IPv6 (::x.x.x.x)
    if let Some(v4) = ipv6.to_ipv4_mapped() {
        return is_private_ipv4(v4);
    }
    if let Some(v4) = ipv6.to_ipv4() {
        return is_private_ipv4(v4);
    }
    let segments = ipv6.segments();
    // Unique Local Address (fc00::/7)
    if (segments[0] & 0xfe00) == 0xfc00 {
        return true;
    }
    // Link-Local Unicast (fe80::/10)
    if (segments[0] & 0xffc0) == 0xfe80 {
        return true;
    }
    // Documentation (2001:db8::/32)
    if segments[0] == 0x2001 && segments[1] == 0x0db8 {
        return true;
    }
    // Discard prefix (100::/64)
    if segments[0] == 0x0100 && segments[1] == 0 && segments[2] == 0 && segments[3] == 0 {
        return true;
    }
    false
}

/// Check if an IP address is in a private/reserved range.
pub fn is_private_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_private_ipv4(v4),
        IpAddr::V6(v6) => is_private_ipv6(v6),
    }
}

/// Validate a URL for SSRF protection.
/// Returns Ok(()) if the URL is safe to fetch, Err with reason otherwise.
pub fn validate_url(url: &str) -> Result<(), String> {
    if url.len() > MAX_URL_LENGTH {
        return Err(format!("URL exceeds maximum length of {}", MAX_URL_LENGTH));
    }

    let parsed = Url::parse(url).map_err(|e| format!("Invalid URL: {}", e))?;

    // Only allow HTTP/HTTPS
    match parsed.scheme() {
        "http" | "https" => {}
        scheme => return Err(format!("Unsupported URL scheme: {}", scheme)),
    }

    // Check host
    match parsed.host() {
        Some(url::Host::Ipv4(v4)) => {
            if is_private_ipv4(v4) {
                return Err(format!("Blocked private IPv4: {}", v4));
            }
        }
        Some(url::Host::Ipv6(v6)) => {
            if is_private_ipv6(v6) {
                return Err(format!("Blocked private IPv6: {}", v6));
            }
        }
        Some(url::Host::Domain(domain)) => {
            let lower = domain.to_ascii_lowercase();
            if lower == "localhost"
                || lower.ends_with(".localhost")
                || lower.ends_with(".local")
                || lower.ends_with(".internal")
                || lower.ends_with(".localdomain")
                || lower.ends_with(".lan")
                || lower.ends_with(".home.arpa")
                || lower == "metadata.google.internal"
            {
                return Err(format!("Blocked domain: {}", domain));
            }
            // Check global security blocked hosts
            let sec = crate::config::global_security();
            if sec
                .blocked_hosts
                .iter()
                .any(|b| b.eq_ignore_ascii_case(&lower))
            {
                return Err(format!("Blocked configured host: {}", domain));
            }
        }
        None => return Err("URL has no host".to_string()),
    }

    Ok(())
}

/// Whether a redirect hop may be followed: the same SSRF policy as a direct load.
///
/// Split out of [`redirect_policy`] because `reqwest::redirect::Attempt` cannot be
/// constructed outside reqwest, which made the policy itself untestable.
pub fn redirect_allowed(url: &str) -> bool {
    validate_url(url).is_ok()
}

/// Reqwest redirect policy that revalidates every redirect hop against the same
/// SSRF policy as direct URL loads. Redirects to private/loopback/link-local
/// ranges, metadata endpoints, or blocked hostnames are never followed.
pub fn redirect_policy() -> reqwest::redirect::Policy {
    reqwest::redirect::Policy::custom(|attempt| match validate_url(attempt.url().as_str()) {
        Ok(()) => attempt.follow(),
        Err(e) => {
            tracing::warn!(
                "Blocked SSRF-unsafe redirect to '{}': {}",
                sanitize_for_log(attempt.url().as_str()),
                e
            );
            attempt.stop()
        }
    })
}

// ---------------------------------------------------------------------------
// Playback stream targets
// ---------------------------------------------------------------------------

/// Where a track's audio should be read from, after validation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamTarget {
    /// A local file. Only produced when the operator enabled `sources.local`.
    LocalFile(String),
    /// A remote http(s) URL. `pin` is a resolved **public** address that the
    /// request must be pinned to, so the host cannot be re-resolved to a private
    /// address between validation and connect (DNS rebinding).
    Remote {
        url: String,
        host: String,
        pin: SocketAddr,
    },
}

/// A playback URL split into the shapes that need different handling.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamRequest {
    /// Local file path (requires `sources.local`).
    LocalFile(String),
    /// http(s) URL whose host is an IP literal — already range-checked.
    IpLiteral { url: String, pin: SocketAddr },
    /// http(s) URL whose host is a name that still has to be resolved.
    Hostname { url: String, host: String, port: u16 },
}

/// Classify a playback URL without touching the network.
///
/// Local paths (`file://` URLs and absolute paths) require `local_sources_enabled`;
/// everything else must pass [`validate_url`], which restricts the scheme to
/// http/https and rejects private, loopback, link-local, metadata and configured
/// blocked hosts.
pub fn classify_stream_url(
    url: &str,
    local_sources_enabled: bool,
) -> Result<StreamRequest, String> {
    let local_path = if let Some(path) = url.strip_prefix("file://") {
        Some(path.to_string())
    } else if url.starts_with('/') {
        Some(url.to_string())
    } else {
        None
    };

    if let Some(path) = local_path {
        return if local_sources_enabled {
            Ok(StreamRequest::LocalFile(path))
        } else {
            Err(format!(
                "Blocked local file source '{}': sources.local is disabled",
                sanitize_for_log(&path)
            ))
        };
    }

    validate_url(url)?;

    let parsed = Url::parse(url).map_err(|e| format!("Invalid URL: {}", e))?;
    let port = parsed.port_or_known_default().unwrap_or(80);

    match parsed.host() {
        Some(url::Host::Ipv4(v4)) => Ok(StreamRequest::IpLiteral {
            url: url.to_string(),
            pin: SocketAddr::new(IpAddr::V4(v4), port),
        }),
        Some(url::Host::Ipv6(v6)) => Ok(StreamRequest::IpLiteral {
            url: url.to_string(),
            pin: SocketAddr::new(IpAddr::V6(v6), port),
        }),
        Some(url::Host::Domain(domain)) => Ok(StreamRequest::Hostname {
            url: url.to_string(),
            host: domain.to_string(),
            port,
        }),
        None => Err("URL has no host".to_string()),
    }
}

/// Resolve `host` and reject the request when **any** address it resolves to is
/// private, loopback, link-local, multicast or otherwise reserved.
///
/// Returns the address to pin the request to. `validate_url` can only inspect the
/// hostname, so without this a public-looking name that resolves to
/// `169.254.169.254` or `127.0.0.1` (including a rebinding name whose answer
/// changes between lookups) would slip through.
pub async fn resolve_public_addr(host: &str, port: u16) -> Result<SocketAddr, String> {
    let addrs: Vec<SocketAddr> = tokio::net::lookup_host((host, port))
        .await
        .map_err(|e| format!("DNS resolution failed for '{}': {}", host, e))?
        .collect();

    if addrs.is_empty() {
        return Err(format!("No addresses found for '{}'", host));
    }

    if let Some(blocked) = addrs.iter().find(|addr| is_private_ip(addr.ip())) {
        let blocked_ip = blocked.ip();
        return Err(format!(
            "Blocked '{}': resolves to private/reserved address {}",
            host, blocked_ip
        ));
    }

    Ok(addrs[0])
}

/// Validate a playback stream URL end to end: local-source gate, SSRF policy and
/// DNS-level range check with the resulting address pinned.
pub async fn resolve_stream_target(
    url: &str,
    local_sources_enabled: bool,
) -> Result<StreamTarget, String> {
    match classify_stream_url(url, local_sources_enabled)? {
        StreamRequest::LocalFile(path) => Ok(StreamTarget::LocalFile(path)),
        StreamRequest::IpLiteral { url, pin } => Ok(StreamTarget::Remote {
            host: pin.ip().to_string(),
            url,
            pin,
        }),
        StreamRequest::Hostname { url, host, port } => {
            let pin = resolve_public_addr(&host, port).await?;
            Ok(StreamTarget::Remote { url, host, pin })
        }
    }
}

/// Validate a search query.
pub fn validate_query(query: &str) -> Result<(), String> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return Err("Empty query".to_string());
    }
    if trimmed.len() > MAX_QUERY_LENGTH {
        return Err(format!(
            "Query exceeds maximum length of {}",
            MAX_QUERY_LENGTH
        ));
    }
    // Block potentially dangerous characters
    if trimmed.contains('\0') || trimmed.contains('\r') || trimmed.contains('\n') {
        return Err("Query contains invalid characters".to_string());
    }
    Ok(())
}

/// Validate a track identifier.
pub fn validate_identifier(identifier: &str) -> Result<(), String> {
    let trimmed = identifier.trim();
    if trimmed.is_empty() {
        return Err("Empty identifier".to_string());
    }
    if trimmed.len() > MAX_URL_LENGTH {
        return Err(format!(
            "Identifier exceeds maximum length of {}",
            MAX_URL_LENGTH
        ));
    }
    Ok(())
}

/// Validate session ID format.
pub fn validate_session_id(session_id: &str) -> Result<(), String> {
    if session_id.is_empty() {
        return Err("Empty session ID".to_string());
    }
    if session_id.len() > 128 {
        return Err("Session ID too long".to_string());
    }
    // Only allow hex chars and hyphens (UUID-like)
    if !session_id
        .chars()
        .all(|c| c.is_ascii_hexdigit() || c == '-')
    {
        return Err("Session ID contains invalid characters".to_string());
    }
    Ok(())
}

/// Validate guild ID format.
pub fn validate_guild_id(guild_id: &str) -> Result<(), String> {
    if guild_id.is_empty() {
        return Err("Empty guild ID".to_string());
    }
    if guild_id.len() > 32 {
        return Err("Guild ID too long".to_string());
    }
    if !guild_id.chars().all(|c| c.is_ascii_digit()) {
        return Err("Guild ID must be numeric".to_string());
    }
    Ok(())
}

/// Sanitize a string for safe logging (removes control characters, newlines, CR, and ANSI escape sequences).
pub fn sanitize_for_log(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_control() && *c != '\x1b' && *c != '\r' && *c != '\n')
        .take(256)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_url_safe() {
        assert!(validate_url("https://example.com/audio.mp3").is_ok());
        assert!(validate_url("http://cdn.example.com/song.ogg").is_ok());
    }

    #[test]
    fn test_validate_url_blocks_localhost() {
        assert!(validate_url("http://localhost/admin").is_err());
        assert!(validate_url("http://127.0.0.1/admin").is_err());
        assert!(validate_url("http://0.0.0.0/admin").is_err());
        assert!(validate_url("http://[::1]/admin").is_err());
    }

    #[test]
    fn test_validate_url_blocks_private() {
        assert!(validate_url("http://192.168.1.1/admin").is_err());
        assert!(validate_url("http://10.0.0.1/admin").is_err());
        assert!(validate_url("http://10.254.1.2/admin").is_err());
        assert!(validate_url("http://172.16.0.1/admin").is_err());
        assert!(validate_url("http://172.31.255.255/admin").is_err());
        assert!(validate_url("http://100.64.0.1/admin").is_err());
    }

    #[test]
    fn test_validate_url_blocks_metadata() {
        assert!(validate_url("http://169.254.169.254/latest/meta-data/").is_err());
        assert!(validate_url("http://metadata.google.internal/computeMetadata").is_err());
    }

    #[test]
    fn test_validate_url_blocks_bad_schemes() {
        assert!(validate_url("ftp://example.com/file").is_err());
        assert!(validate_url("file:///etc/passwd").is_err());
        assert!(validate_url("gopher://example.com/").is_err());
    }

    #[test]
    fn test_validate_query() {
        assert!(validate_query("song name").is_ok());
        assert!(validate_query("").is_err());
        assert!(validate_query(&"x".repeat(600)).is_err());
        assert!(validate_query("song\nname").is_err());
        assert!(validate_query("song\rname").is_err());
    }

    #[test]
    fn test_validate_session_id() {
        assert!(validate_session_id("abc123").is_ok());
        assert!(validate_session_id("550e8400-e29b-41d4-a716-446655440000").is_ok());
        assert!(validate_session_id("").is_err());
        assert!(validate_session_id("has spaces").is_err());
    }

    #[test]
    fn test_validate_guild_id() {
        assert!(validate_guild_id("123456789").is_ok());
        assert!(validate_guild_id("abc").is_err());
        assert!(validate_guild_id("").is_err());
    }

    #[test]
    fn test_is_private_ip() {
        assert!(is_private_ip("127.0.0.1".parse().unwrap()));
        assert!(is_private_ip("192.168.1.1".parse().unwrap()));
        assert!(is_private_ip("10.0.0.1".parse().unwrap()));
        assert!(is_private_ip("10.123.45.67".parse().unwrap()));
        assert!(is_private_ip("172.16.0.1".parse().unwrap()));
        assert!(is_private_ip("172.31.255.255".parse().unwrap()));
        assert!(!is_private_ip("172.32.0.1".parse().unwrap()));
        assert!(!is_private_ip("8.8.8.8".parse().unwrap()));
        assert!(!is_private_ip("1.1.1.1".parse().unwrap()));
        // IPv6
        assert!(is_private_ip("::1".parse().unwrap()));
        assert!(is_private_ip("fc00::1".parse().unwrap()));
        assert!(is_private_ip("fe80::1".parse().unwrap()));
        assert!(is_private_ip("::ffff:192.168.1.1".parse().unwrap()));
    }

    #[test]
    fn test_sanitize_for_log() {
        assert_eq!(sanitize_for_log("hello world"), "hello world");
        assert_eq!(sanitize_for_log("hello\x00world"), "helloworld");
        assert_eq!(sanitize_for_log("hello\n\tworld"), "helloworld");
        assert_eq!(
            sanitize_for_log("hello\x1b[31mred\x1b[0m"),
            "hello[31mred[0m"
        );
    }

    /// The redirect policy must reject exactly the targets `validate_url` rejects,
    /// because hops are followed from attacker-influenced URLs. Asserted through
    /// `redirect_allowed`, the decision the policy actually makes: the previous
    /// version of this test only constructed the policy and dropped it, so it
    /// passed no matter what the closure did.
    #[test]
    fn test_redirect_policy_revalidates_ssrf() {
        assert!(redirect_allowed("https://cdn.example.com/audio.mp3"));
        assert!(!redirect_allowed("http://127.0.0.1:8080/admin"));
        assert!(!redirect_allowed("http://[::1]/admin"));
        assert!(!redirect_allowed("http://10.0.0.1/admin"));
        assert!(!redirect_allowed("http://169.254.169.254/latest/meta-data/"));
        assert!(!redirect_allowed("http://localhost/admin"));
        assert!(!redirect_allowed("http://metadata.google.internal/computeMetadata"));
        assert!(!redirect_allowed("file:///etc/passwd"));
        // The constructed policy exists and is usable by reqwest.
        let _ = redirect_policy();
    }

    /// An encoded track is unsigned client input, so the playback path must refuse
    /// local files unless the operator enabled the local source.
    #[test]
    fn test_classify_stream_url_gates_local_files() {
        assert!(classify_stream_url("file:///etc/passwd", false).is_err());
        assert!(classify_stream_url("/srv/music/song.flac", false).is_err());

        let file_url = "file:///srv/music/song.flac";
        assert_eq!(
            classify_stream_url(file_url, true).unwrap(),
            StreamRequest::LocalFile("/srv/music/song.flac".to_string())
        );

        let abs_path = "/srv/music/song.flac";
        assert_eq!(
            classify_stream_url(abs_path, true).unwrap(),
            StreamRequest::LocalFile("/srv/music/song.flac".to_string())
        );
    }

    #[test]
    fn test_classify_stream_url_applies_ssrf_policy() {
        let metadata = "http://169.254.169.254/latest/meta-data/";
        assert!(classify_stream_url(metadata, true).is_err());
        assert!(classify_stream_url("http://127.0.0.1/admin", true).is_err());
        assert!(classify_stream_url("http://[::1]/admin", true).is_err());
        assert!(classify_stream_url("http://localhost/admin", true).is_err());
        assert!(classify_stream_url("ftp://example.com/a.mp3", true).is_err());
        assert!(classify_stream_url("http://10.0.0.1/a.mp3", true).is_err());
    }

    #[test]
    fn test_classify_stream_url_remote_shapes() {
        let cdn = "https://cdn.example.com/a.mp3";
        assert_eq!(
            classify_stream_url(cdn, false).unwrap(),
            StreamRequest::Hostname {
                url: cdn.to_string(),
                host: "cdn.example.com".to_string(),
                port: 443,
            }
        );

        let direct = "http://93.184.216.34:8080/a.mp3";
        assert_eq!(
            classify_stream_url(direct, false).unwrap(),
            StreamRequest::IpLiteral {
                url: direct.to_string(),
                pin: "93.184.216.34:8080".parse().unwrap(),
            }
        );
    }

    /// Names that resolve into private ranges must be rejected even though the
    /// name itself looks harmless (split-horizon DNS, DNS rebinding, or a host
    /// record pointing at the metadata endpoint). Numeric hosts never hit the
    /// network, and `localhost` is resolved from the hosts file, so this is
    /// deterministic offline.
    #[tokio::test]
    async fn test_resolve_public_addr_rejects_private_addresses() {
        assert!(resolve_public_addr("127.0.0.1", 80).await.is_err());
        assert!(resolve_public_addr("::1", 80).await.is_err());
        assert!(resolve_public_addr("localhost", 80).await.is_err());
    }

    #[tokio::test]
    async fn test_resolve_stream_target_pins_validated_address() {
        let direct = "http://93.184.216.34/a.mp3";
        let target = resolve_stream_target(direct, false).await.unwrap();
        match target {
            StreamTarget::Remote { host, pin, .. } => {
                assert_eq!(host, "93.184.216.34");
                assert_eq!(pin.ip().to_string(), "93.184.216.34");
                assert_eq!(pin.port(), 80);
            }
            other => panic!("expected a remote target, got {:?}", other),
        }

        let localhost = "http://localhost/a.mp3";
        assert!(resolve_stream_target(localhost, false).await.is_err());
        assert!(resolve_stream_target("file:///etc/passwd", false).await.is_err());
    }
}
