use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
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
}
