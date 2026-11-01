//! Admin IP allowlist (`admin.allow_ips`): the replacement for
//! client-cert auth on the admin listener.
//!
//! Rule: session cookie + staff role (checked by [`AdminAuth`]) plus a
//! peer IP inside the configured ranges, or HTTP 403. Proxy headers
//! (`X-Forwarded-For` and friends) are never trusted: the admin ports
//! are reached directly, so the socket peer is the caller. No peer
//! address on the request (QUIC paths that skip `ConnectInfo`) fails
//! closed. Denials name the detected IP and the config section that
//! changes the range, so a locked-out operator can self-serve.

use axum::{
    extract::ConnectInfo,
    http::{Request, StatusCode},
    response::{IntoResponse, Json, Response},
};
use serde_json::json;
use std::net::{IpAddr, SocketAddr};

/// Socket peer IP, or `None` when the stack supplied no
/// `ConnectInfo` (fails closed downstream).
pub fn peer_ip<T>(req: &Request<T>) -> Option<IpAddr> {
    req.extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|info| info.0.ip())
}

/// True when `ip` falls in any configured range (`None` never is).
pub fn ip_allowed(allow_ips: &[config::IpNet], ip: Option<IpAddr>) -> bool {
    match ip {
        Some(addr) => allow_ips.iter().any(|net| net.contains(&addr)),
        None => false,
    }
}

/// 403 for an out-of-range (or unknown) peer: names the detected IP
/// and points at the config section that owns the range.
pub fn denied_response(ip: Option<IpAddr>) -> Response {
    let seen = ip.map_or_else(|| "unknown".to_string(), |a| a.to_string());
    (
        StatusCode::FORBIDDEN,
        Json(json!({
            "error": format!(
                "admin access is limited to the configured IP range (admin.allow_ips) — \
                 your IP is {seen}; see \"Admin IP allowlist\" in docs/specs/config-db.md \
                 to change it"
            ),
        })),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loopback_passes_unknown_fails() {
        let allow = config::loopback_allow_ips();
        let local: IpAddr = "127.0.0.1".parse().unwrap();
        let lan: IpAddr = "10.9.8.7".parse().unwrap();
        assert!(ip_allowed(&allow, Some(local)));
        assert!(!ip_allowed(&allow, Some(lan)));
        assert!(!ip_allowed(&allow, None));
        assert!(!ip_allowed(&[], Some(local)));
    }
}
