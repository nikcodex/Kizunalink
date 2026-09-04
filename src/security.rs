use std::net::IpAddr;
use url::Url;

/// Maximum allowed request body size (1 MB).
pub const MAX_BODY_SIZE: usize = 1_048_576;

/// Maximum allowed URL length.
pub const MAX_URL_LENGTH: usize = 2048;

/// Maximum allowed search query length.
pub const MAX_QUERY_LENGTH: usize = 512;

/// Blocked IP ranges (localhost, private networks) for SSRF protection.
const BLOCKED_RANGES: &[(u8, u8, u8, u8)] = &[
    (10, 0, 0, 0),    // 10.0.0.0/8
    (172, 16, 0, 0),  // 172.16.0.0/12
    (192, 168, 0, 0), // 192.168.0.0/16
    (127, 0, 0, 0),   // 127.0.0.0/8 (localhost)
    (169, 254, 0, 0), // 169.254.0.0/16 (link-local)
];

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

    // Check host for SSRF
    if let Some(host) = parsed.host_str() {
        // Block localhost variations
        if host == "localhost"
            || host == "0.0.0.0"
            || host == "::1"
            || host.ends_with(".local")
            || host.ends_with(".internal")
        {
            return Err(format!("Blocked URL host: {}", host));
        }

        // Block private IPs
        if let Ok(ip) = host.parse::<IpAddr>() {
            if is_private_ip(ip) {
                return Err(format!("Blocked private IP: {}", ip));
            }
        }

        // Block metadata endpoints
        if host == "169.254.169.254" || host == "metadata.google.internal" {
            return Err("Blocked cloud metadata endpoint".to_string());
        }
    } else {
        return Err("URL has no host".to_string());
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

/// Check if an IP address is in a private/reserved range.
fn is_private_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ipv4) => {
            let octets = ipv4.octets();
            (octets[0] == 172 && octets[1] >= 16 && octets[1] <= 31)
                || BLOCKED_RANGES
                    .iter()
                    .any(|&(a, b, c, _)| octets[0] == a && octets[1] == b && octets[2] >= c)
                || octets[0] == 0
        }
        IpAddr::V6(ipv6) => {
            ipv6.is_loopback()
                || ipv6.is_unspecified()
                || ipv6.is_unique_local()
                || ipv6.is_unicast_link_local()
        }
    }
}

/// Sanitize a string for safe logging (removes control characters).
pub fn sanitize_for_log(s: &str) -> String {
    s.chars().filter(|c| !c.is_control()).take(256).collect()
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
    }

    #[test]
    fn test_validate_url_blocks_private() {
        assert!(validate_url("http://192.168.1.1/admin").is_err());
        assert!(validate_url("http://10.0.0.1/admin").is_err());
        assert!(validate_url("http://172.16.0.1/admin").is_err());
    }

    #[test]
    fn test_validate_url_blocks_metadata() {
        assert!(validate_url("http://169.254.169.254/latest/meta-data/").is_err());
    }

    #[test]
    fn test_validate_url_blocks_bad_schemes() {
        assert!(validate_url("ftp://example.com/file").is_err());
        assert!(validate_url("file:///etc/passwd").is_err());
    }

    #[test]
    fn test_validate_query() {
        assert!(validate_query("song name").is_ok());
        assert!(validate_query("").is_err());
        assert!(validate_query(&"x".repeat(600)).is_err());
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
        assert!(is_private_ip("172.16.0.1".parse().unwrap()));
        assert!(is_private_ip("172.31.255.255".parse().unwrap()));
        assert!(!is_private_ip("172.32.0.1".parse().unwrap()));
        assert!(!is_private_ip("8.8.8.8".parse().unwrap()));
        assert!(!is_private_ip("1.1.1.1".parse().unwrap()));
    }

    #[test]
    fn test_sanitize_for_log() {
        assert_eq!(sanitize_for_log("hello world"), "hello world");
        assert_eq!(sanitize_for_log("hello\x00world"), "helloworld");
        assert_eq!(sanitize_for_log("hello\n\tworld"), "helloworld");
    }
}
