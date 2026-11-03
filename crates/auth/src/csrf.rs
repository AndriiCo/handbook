//! CSRF double-submit tokens plus Origin allowlist checks.
//!
//! Mutations require all three: session cookie, an allowed `Origin`,
//! and the `hb_csrf` cookie echoed in the `x-csrf` header.

use crate::{admin_host, public_host};

/// Fresh 256-bit CSRF token, hex.
pub fn mint_token() -> String {
    use rand::RngCore as _;
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    let mut out = String::with_capacity(64);
    for b in bytes {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

/// Constant-time token comparison (no early exit on content).
pub fn tokens_match(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() || a.is_empty() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// True when `Origin` is on the allowlist: `https://andrii.ai` or
/// `https://admin.andrii.ai` (exact host, no port), plus
/// `http://localhost[:port]` / `http://127.0.0.1[:port]` when
/// `dev_localhost` is set. A missing or malformed `Origin` fails
/// closed (`false`); mutations must always send one.
pub fn origin_allowed(origin: Option<&str>, dev_localhost: bool) -> bool {
    let Some(origin) = origin else {
        return false;
    };
    let (scheme, rest) = match origin.split_once("://") {
        Some(pair) => pair,
        None => return false,
    };
    if rest.contains('/') {
        return false;
    }
    if scheme == "https" && (rest == public_host() || rest == admin_host()) {
        return true;
    }
    if dev_localhost && scheme == "http" {
        let host = rest.split_once(':').map_or(rest, |(h, _)| h);
        if host == "localhost" || host == "127.0.0.1" {
            if let Some((_, port)) = rest.split_once(':') {
                return !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit());
            }
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_compare_in_constant_shape() {
        let a = mint_token();
        assert_eq!(a.len(), 64);
        assert!(tokens_match(&a, &a));
        assert!(!tokens_match(&a, &mint_token()));
        assert!(!tokens_match(&a, "short"));
        assert!(!tokens_match("", ""));
    }

    #[test]
    fn origin_allowlist() {
        assert!(origin_allowed(Some("https://andrii.ai"), false));
        assert!(origin_allowed(Some("https://admin.andrii.ai"), false));
        assert!(!origin_allowed(Some("https://andrii.ai.evil.test"), false));
        assert!(!origin_allowed(Some("https://evil.test"), false));
        assert!(!origin_allowed(Some("http://andrii.ai"), false));
        assert!(!origin_allowed(Some("https://andrii.ai/"), false));
        assert!(!origin_allowed(Some("https://andrii.ai:443"), false));
        assert!(!origin_allowed(None, false));
        assert!(!origin_allowed(Some("not a url"), false));
        assert!(!origin_allowed(Some("http://localhost:3001"), false));
        assert!(origin_allowed(Some("http://localhost:3001"), true));
        assert!(origin_allowed(Some("http://127.0.0.1"), true));
        assert!(!origin_allowed(Some("http://localhost:abc"), true));
        assert!(!origin_allowed(Some("https://localhost:3001"), true));
    }
}
