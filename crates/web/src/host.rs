//! Single-443 Host routing + Alt-Svc (goal-026, spec §1–§2.4).
//!
//! ONE TCP+TLS listener serves both branches: requests for the
//! admin host go to the merged admin router (with the mTLS
//! fingerprint layer); EVERYTHING else — public host, unknown
//! host, missing host — goes to the public router. Unknown hosts
//! fail open to the harmless branch: the admin branch keeps its
//! own host guard + session + cert checks, so misrouting can
//! never grant admin access.
//!
//! The TLS handshake is shared (one cert pair, optional client
//! certs), so the acceptor captures peer chains for every
//! connection. Before forwarding to the PUBLIC branch the
//! dispatcher strips the [`PeerCertificates`](crate::mtls::PeerCertificates)
//! extension: the public branch never sees client-cert state.

use axum::{
    Router,
    body::Body,
    extract::State,
    http::{Request, header},
    middleware::Next,
    response::Response,
};
use tower::ServiceExt as _;

/// True when a raw `Host` header value addresses the admin branch:
/// exactly [`ADMIN_HOST`](auth::ADMIN_HOST), case-insensitive,
/// with an optional `:port` suffix stripped. No loopback/dev
/// exception here — plain-HTTP dev keeps its own two listeners
/// and never routes through this dispatcher.
pub fn is_admin_host(host: &str) -> bool {
    host.split_once(':')
        .map_or(host, |(bare, _)| bare)
        .eq_ignore_ascii_case(auth::ADMIN_HOST)
}

/// Both branches for the dispatcher (cloned per request; `Router`
/// is cheap to clone).
#[derive(Clone)]
struct HostRoutes {
    public: Router,
    admin: Router,
}

/// Merge both branches behind Host dispatch (a fallback route, so
/// every method and path dispatches — no route table is shared or
/// shadowed between branches).
pub fn host_router(public: Router, admin: Router) -> Router {
    Router::new()
        .fallback(dispatch)
        .with_state(HostRoutes { public, admin })
}

/// Dispatch by `Host`, then `oneshot` the chosen branch. Routers
/// cannot fail (`Infallible`), so no error mapping exists.
async fn dispatch(State(routes): State<HostRoutes>, mut req: Request<Body>) -> Response {
    let host = req
        .headers()
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    if is_admin_host(host) {
        return match routes.admin.oneshot(req).await {
            Ok(res) => res,
            Err(never) => match never {},
        };
    }
    req.extensions_mut()
        .remove::<crate::mtls::PeerCertificates>();
    match routes.public.oneshot(req).await {
        Ok(res) => res,
        Err(never) => match never {},
    }
}

/// Tower middleware (axum `from_fn_with_state`, state = the HTTPS
/// port): advertise the QUIC endpoint sharing the TLS port. Wired
/// ONLY when `quic.enabled` is true — advertising h3 with no UDP
/// listener would black-hole upgrade attempts.
pub async fn add_alt_svc(
    State(https_port): State<u16>,
    req: Request<Body>,
    next: Next,
) -> Response {
    let mut res = next.run(req).await;
    if let Ok(value) = header::HeaderValue::from_str(&format!("h3=\":{https_port}\"")) {
        res.headers_mut().insert(header::ALT_SVC, value);
    }
    res
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Json, routing::get};
    use serde_json::json;

    #[test]
    fn admin_host_matching_strips_port_and_case() {
        for host in [
            "admin.andrii.ai",
            "ADMIN.ANDRII.AI",
            "Admin.Andrii.Ai:443",
            "admin.andrii.ai:8443",
        ] {
            assert!(is_admin_host(host), "host: {host}");
        }
        for host in [
            "",
            "andrii.ai",
            "admin.andrii.ai.evil.test",
            "evil-admin.andrii.ai",
            "localhost",
            "127.0.0.1",
        ] {
            assert!(!is_admin_host(host), "host: {host}");
        }
    }

    /// Stub branch router reporting its name plus whether cert
    /// state reached it (proves the public-branch stripping).
    fn stub_branch(name: &'static str) -> Router {
        async fn report(
            State(name): State<&'static str>,
            req: Request<Body>,
        ) -> impl axum::response::IntoResponse {
            let saw_certs = req
                .extensions()
                .get::<crate::mtls::PeerCertificates>()
                .is_some();
            Json(json!({"branch": name, "saw_certs": saw_certs}))
        }
        Router::new().route("/probe", get(report)).with_state(name)
    }

    async fn dispatch_body(router: &Router, host: Option<&str>, certs: bool) -> serde_json::Value {
        let mut builder = Request::builder().uri("/probe");
        if let Some(host) = host {
            builder = builder.header("host", host);
        }
        let mut req = builder.body(Body::empty()).unwrap();
        if certs {
            // The shared-handshake acceptor captured a chain.
            req.extensions_mut()
                .insert(crate::mtls::PeerCertificates::from_ders(vec![vec![
                    0x30, 0x03,
                ]]));
        }
        let res = router.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status(), axum::http::StatusCode::OK);
        let body = axum::body::to_bytes(res.into_body(), 1024).await.unwrap();
        serde_json::from_slice(&body).unwrap()
    }

    #[tokio::test]
    async fn dispatch_matrix_with_cert_stripping() {
        let router = host_router(stub_branch("public"), stub_branch("admin"));
        // Admin host → admin branch, cert state intact.
        let value = dispatch_body(&router, Some("admin.andrii.ai"), true).await;
        assert_eq!(value["branch"], "admin");
        assert_eq!(value["saw_certs"], true);
        // Admin host with port → still admin.
        let value = dispatch_body(&router, Some("admin.andrii.ai:443"), false).await;
        assert_eq!(value["branch"], "admin");
        // Public, unknown, and missing hosts → public branch.
        for host in [Some("andrii.ai"), Some("unknown.test"), None] {
            let value = dispatch_body(&router, host, true).await;
            assert_eq!(value["branch"], "public", "host: {host:?}");
            // The shared handshake's chain never reaches public.
            assert_eq!(value["saw_certs"], false, "host: {host:?}");
        }
    }

    #[tokio::test]
    async fn alt_svc_advertises_h3_on_the_tls_port() {
        let inner = Router::new().route("/probe", get(|| async { "ok" }));
        let router = inner.layer(axum::middleware::from_fn_with_state(443u16, add_alt_svc));
        let res = router
            .oneshot(
                Request::builder()
                    .uri("/probe")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.headers().get(header::ALT_SVC).unwrap(), "h3=\":443\"");
    }
}
