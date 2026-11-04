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

/// `tcp` / `quic` as stamped by the serving path, `unknown` when
/// the marker is missing (never fail on it — the denial names it).
pub fn transport_of(ext: &axum::http::Extensions) -> &'static str {
    ext.get::<crate::RequestTransport>()
        .map(|marked| marked.0)
        .unwrap_or("unknown")
}

/// True when `ip` falls in any configured range (`None` never is).
pub fn ip_allowed(allow_ips: &[config::IpNet], ip: Option<IpAddr>) -> bool {
    match ip {
        Some(addr) => allow_ips.iter().any(|net| net.contains(&addr)),
        None => false,
    }
}

/// Machine-readable denial codes: every admin-branch 403 carries
/// one, so the UI can show the server's answer instead of guessing.
pub const CODE_IP_NOT_ALLOWED: &str = "ip_not_allowed";
pub const CODE_STAFF_ROLE_REQUIRED: &str = "staff_role_required";
pub const CODE_CSRF_FAILED: &str = "csrf_failed";
pub const CODE_ADMIN_HOST_REQUIRED: &str = "admin_host_required";

/// Canonical display form: a v4-mapped peer prints as dotted quad.
pub fn seen_ip(ip: Option<IpAddr>) -> String {
    ip.map(|a| a.to_canonical().to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

/// "IPv4" / "IPv6" / "unknown" for the denial text.
pub fn family_name(ip: Option<IpAddr>) -> &'static str {
    match ip.map(|a| a.to_canonical()) {
        Some(IpAddr::V4(_)) => "IPv4",
        Some(IpAddr::V6(_)) => "IPv6",
        None => "unknown",
    }
}

/// The `/64` covering an IPv6 peer (allowlisting one privacy address
/// at a time is hopeless: they rotate per connection).
pub fn slash64(ip: std::net::Ipv6Addr) -> String {
    let s = ip.segments();
    std::net::Ipv6Addr::new(s[0], s[1], s[2], s[3], 0, 0, 0, 0).to_string() + "/64"
}

/// The socket peer as an extractor that never rejects (`None` when the
/// stack supplied no `ConnectInfo`, e.g. in router unit tests).
pub struct PeerIp(pub Option<IpAddr>);

impl<S: Send + Sync> axum::extract::FromRequestParts<S> for PeerIp {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        _: &S,
    ) -> Result<Self, Self::Rejection> {
        Ok(PeerIp(
            parts
                .extensions
                .get::<ConnectInfo<SocketAddr>>()
                .map(|c| c.0.ip()),
        ))
    }
}

/// Rate-limit key for a peer: the IPv4 address (v4-mapped included) or
/// the IPv6 /64, so rotating privacy addresses share one budget.
pub fn rate_key(ip: Option<IpAddr>) -> String {
    match ip {
        Some(IpAddr::V4(v4)) => v4.to_string(),
        Some(IpAddr::V6(v6)) => match v6.to_ipv4_mapped() {
            Some(v4) => v4.to_string(),
            None => slash64(v6),
        },
        None => "unknown".to_string(),
    }
}

/// One structured denial shape for every admin-branch 403: human
/// text (kept backward-compatible), the machine `code`, and the
/// detected peer `ip`.
pub fn denial(code: &str, ip: Option<IpAddr>, error: String) -> Response {
    (
        StatusCode::FORBIDDEN,
        Json(json!({
            "error": error,
            "code": code,
            "ip": seen_ip(ip),
        })),
    )
        .into_response()
}

/// 403 for an out-of-range (or unknown) peer: names the detected IP,
/// its family, the transport it arrived on, and the exact fix (bare
/// IPv4, or the /64 for IPv6), and points at the config section that
/// owns the range.
pub fn denied_response(ip: Option<IpAddr>, transport: &str) -> Response {
    let seen = seen_ip(ip);
    let fix = match ip.map(|a| a.to_canonical()) {
        Some(IpAddr::V4(_)) => format!("Add {seen} to admin.allow_ips, then restart apostille."),
        Some(IpAddr::V6(v6)) => format!(
            "Add {seen} or its /64 {} to admin.allow_ips, then restart apostille.",
            slash64(v6)
        ),
        None => "No peer address was detected for this request. ".to_string(),
    };
    denial(
        CODE_IP_NOT_ALLOWED,
        ip,
        format!(
            "admin access is limited to the configured IP range (admin.allow_ips) — \
             your IP is {seen}; see \"Admin IP allowlist\" in docs/specs/config-db.md \
             to change it. Detected {} {} over {}. {}",
            family_name(ip),
            seen,
            transport,
            fix
        ),
    )
}

/// Tiny static 403 page for browser navigations (`/`, `/config`,
/// `/review`, …): no SPA, no JS, no admin asset. Names the refused
/// peer, its family and the arriving transport so the person can
/// pass them to the operator, but not the config key or docs path
/// (anyone probing the host sees this page). Peer strings are
/// dotted quads / colon hex only, so no escaping is needed.
pub fn denied_html(ip: Option<IpAddr>, transport: &str) -> String {
    let seen = seen_ip(ip);
    let ask = match ip.map(|a| a.to_canonical()) {
        Some(IpAddr::V4(_)) => {
            format!("Ask the server operator to allow {seen}, then reload this page.")
        }
        Some(IpAddr::V6(v6)) => format!(
            "Ask the server operator to allow {seen} or its /64 {}, then reload this page.",
            slash64(v6)
        ),
        None => "No peer address was detected for this request.".to_string(),
    };
    format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
         <title>403 - Admin access refused</title></head>\
         <body style=\"font-family:system-ui,sans-serif;max-width:40rem;margin:2rem auto;padding:0 1rem;line-height:1.5\">\
         <h1>Admin access refused (403)</h1>\
         <p>This network address is not on the staff console allowlist.</p>\
         <p>Your IP address is {seen} ({}), over {transport}.</p>\
         <p>{ask}</p>\
         </body></html>",
        family_name(ip),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn denied_body(res: Response) -> serde_json::Value {
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .expect("body");
        serde_json::from_slice(&bytes).expect("json")
    }

    #[tokio::test]
    async fn denial_shape_names_ip_family_and_fix() {
        let v4: IpAddr = "203.0.113.7".parse().unwrap();
        let body = denied_body(denied_response(Some(v4), "tcp")).await;
        assert_eq!(body["code"], "ip_not_allowed");
        assert_eq!(body["ip"], "203.0.113.7");
        let text = body["error"].as_str().unwrap();
        assert!(text.contains("IPv4 203.0.113.7 over tcp"), "{text}");
        assert!(
            text.contains("Add 203.0.113.7 to admin.allow_ips"),
            "{text}"
        );
        assert!(text.contains("your IP is 203.0.113.7"), "{text}");

        let v6: IpAddr = "2001:db8:1:2:3:4:5:6".parse().unwrap();
        let body = denied_body(denied_response(Some(v6), "quic")).await;
        assert_eq!(body["code"], "ip_not_allowed");
        assert!(body["error"].as_str().unwrap().contains("IPv6"), "family");
        assert!(
            body["error"]
                .as_str()
                .unwrap()
                .contains("2001:db8:1:2::/64"),
            "slash64 fix: {}",
            body["error"]
        );

        let mapped: IpAddr = "::ffff:203.0.113.7".parse().unwrap();
        assert_eq!(seen_ip(Some(mapped)), "203.0.113.7");
        assert_eq!(family_name(Some(mapped)), "IPv4");
        assert_eq!(seen_ip(None), "unknown");
    }

    #[test]
    fn denied_page_fits_phones_and_names_no_config_key() {
        let v4: IpAddr = "203.0.113.7".parse().unwrap();
        let page = denied_html(Some(v4), "tcp");
        assert!(page.contains("name=\"viewport\""), "{page}");
        assert!(page.contains("203.0.113.7 (IPv4), over tcp"), "{page}");
        assert!(!page.contains("allow_ips"), "{page}");
        assert!(!page.contains("docs/"), "{page}");
        let v6: IpAddr = "2001:db8:1:2:3:4:5:6".parse().unwrap();
        let page = denied_html(Some(v6), "quic");
        assert!(page.contains("2001:db8:1:2::/64"), "{page}");
        assert!(!page.contains("allow_ips"), "{page}");
    }

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
