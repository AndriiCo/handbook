//! Native QUIC/HTTP-3 (goal-026, spec §2).
//!
//! One quinn UDP endpoint per bind IP on the SAME port as
//! `net.https_port`, serving the SAME merged Host-routed axum
//! `Router` as TCP via an h3→`Router::oneshot` bridge. Bodies are
//! fully buffered both ways (our payloads are small JSON and SPA
//! shells; streaming is a later optimization).
//!
//! TLS shape matches the TCP listener exactly: the same rustls
//! `ServerConfig` builder ([`mtls`](crate::mtls) with optional
//! client-CA verification, still `allow_unauthenticated`) plus
//! the mandatory `h3` ALPN. Per connection the bridge reads the
//! peer chain quinn captured and inserts the SAME
//! [`PeerCertificates`](crate::mtls::PeerCertificates) extension
//! the TCP acceptor inserts — from there the admin-branch mTLS
//! path is identical (same 401/403 semantics), and the public
//! branch has the extension stripped by Host dispatch.
//!
//! WebSocket stays TCP-only: requests under `/apostille/ws/`
//! get an explicit 501 naming TCP (browsers open a separate TCP
//! WebSocket; no WS-over-H3 in this slice).

use axum::{
    Router,
    body::Body,
    extract::ConnectInfo,
    http::{Request, Response, StatusCode, header},
};
use bytes::Buf as _;
use std::net::SocketAddr;
use tower::ServiceExt as _;

use crate::RequestTransport;

/// ALPN the h3 handshake requires (RFC 9114 §3): without it no
/// client completes a QUIC handshake against this endpoint.
const H3_ALPN: &[u8] = b"h3";

/// Cap for one buffered bridge body each way (2 MiB — orders of
/// magnitude above our JSON APIs; the admin upload path refuses
/// past its own cap before a body ever gets this far).
const MAX_BRIDGE_BODY: usize = 2 * 1024 * 1024;

/// Declared-size gate: reject an over-cap `Content-Length` before
/// reading a single body byte (unit-tested). Returns the 413 body
/// when the head already proves the body cannot fit.
fn bridge_body_allowed(content_length: u64) -> Option<serde_json::Value> {
    if content_length > MAX_BRIDGE_BODY as u64 {
        Some(serde_json::json!({
            "error": "request body too large (2 MiB bridge cap)"
        }))
    } else {
        None
    }
}

/// Build the QUIC endpoint crypto from the same PEM material as
/// the TCP listener: the [`mtls`](crate::mtls) server config
/// (optional client-CA verification) plus the `h3` ALPN.
/// Error strings carry labels only, never PEM bytes.
pub fn server_config(
    cert_pem: &[u8],
    key_pem: &[u8],
    ca_pem: Option<&[u8]>,
) -> Result<quinn::ServerConfig, String> {
    let mut rustls_config = crate::mtls::admin_server_config(cert_pem, key_pem, ca_pem)?;
    rustls_config.alpn_protocols = vec![H3_ALPN.to_vec()];
    let crypto = quinn::crypto::rustls::QuicServerConfig::try_from(rustls_config)
        .map_err(|e| format!("QUIC server crypto: {e}"))?;
    Ok(quinn::ServerConfig::with_crypto(std::sync::Arc::new(
        crypto,
    )))
}

/// Bind one UDP endpoint (the bind happens here — a disabled QUIC
/// gate never calls this, so no socket exists to probe).
pub fn bind_endpoint(
    addr: std::net::SocketAddr,
    config: &quinn::ServerConfig,
) -> std::io::Result<quinn::Endpoint> {
    quinn::Endpoint::server(config.clone(), addr)
}

/// Live QUIC endpoints for ACME hot-reload: renewal rebuilds the
/// endpoint crypto in place (`set_server_config`, no rebind, no
/// restart) instead of serving a stale pair until the next boot.
#[derive(Clone, Default)]
pub struct QuicReloader {
    endpoints: std::sync::Arc<std::sync::Mutex<Vec<quinn::Endpoint>>>,
}

impl QuicReloader {
    /// Register a newly bound endpoint for future reloads.
    pub fn register(&self, endpoint: &quinn::Endpoint) {
        self.endpoints
            .lock()
            .expect("quic reload registry poisoned")
            .push(endpoint.clone());
    }

    /// Swap every registered endpoint to a rebuilt config (ACME
    /// renewal path). Error strings carry labels, never PEM bytes.
    pub fn reload(
        &self,
        cert_pem: &[u8],
        key_pem: &[u8],
        ca_pem: Option<&[u8]>,
    ) -> Result<(), String> {
        let config = server_config(cert_pem, key_pem, ca_pem)?;
        let endpoints = self
            .endpoints
            .lock()
            .expect("quic reload registry poisoned");
        for endpoint in endpoints.iter() {
            endpoint.set_server_config(Some(config.clone()));
        }
        Ok(())
    }
}

/// Serve h3 on one endpoint until it closes, bridging every
/// request to `router`. Connections and requests fail
/// independently (a bad handshake or request ends only itself).
pub async fn serve(endpoint: quinn::Endpoint, router: Router) {
    loop {
        let Some(incoming) = endpoint.accept().await else {
            return;
        };
        let router = router.clone();
        tokio::spawn(async move {
            match incoming.await {
                Ok(conn) => serve_connection(conn, router).await,
                Err(e) => tracing::debug!(error = %e, "quic handshake failed"),
            }
        });
    }
}

/// Peer chain quinn captured on this connection (None when the
/// peer presented nothing — plain QUIC, no client cert). Same
/// shape the TCP acceptor builds, so the admin-branch fingerprint
/// layer cannot tell the transports apart.
fn peer_certificates(conn: &quinn::Connection) -> Option<crate::mtls::PeerCertificates> {
    let identity = conn.peer_identity()?;
    let certs = identity
        .downcast::<Vec<rustls::pki_types::CertificateDer<'static>>>()
        .ok()?;
    if certs.is_empty() {
        return None;
    }
    Some(crate::mtls::PeerCertificates::from_ders(
        certs.iter().map(|der| der.as_ref().to_vec()).collect(),
    ))
}

/// True for WebSocket paths, which stay TCP-only (spec §2.1).
pub(crate) fn is_ws_path(path: &str) -> bool {
    path.starts_with("/apostille/ws/")
}

/// One QUIC connection: accept h3 requests until it closes. Types
/// stay concrete (inferred `Bytes` buffers) — no generics needed.
async fn serve_connection(conn: quinn::Connection, router: Router) {
    let peer = peer_certificates(&conn);
    // TCP listeners inject this via `into_make_service_with_connect_info`;
    // the oneshot bridge must provide it or `ConnectInfo` extractors 500.
    let remote = conn.remote_address();
    let mut h3_conn = match h3::server::Connection::new(h3_quinn::Connection::new(conn)).await {
        Ok(conn) => conn,
        Err(e) => {
            tracing::debug!(error = %e, "h3 handshake failed");
            return;
        }
    };
    loop {
        match h3_conn.accept().await {
            Ok(Some(resolver)) => {
                let router = router.clone();
                let peer = peer.clone();
                tokio::spawn(async move {
                    if let Err(e) = serve_request(resolver, &router, remote, peer).await {
                        tracing::debug!(error = %e, "h3 request failed");
                    }
                });
            }
            Ok(None) => return,
            Err(e) => {
                tracing::debug!(error = %e, "h3 connection failed");
                return;
            }
        }
    }
}

/// Bridge one h3 request to `Router::oneshot` and write the
/// response back. Any failure ends ONLY this request (the stream
/// drops = reset; the connection serves on).
async fn serve_request(
    resolver: h3::server::RequestResolver<h3_quinn::Connection, bytes::Bytes>,
    router: &Router,
    remote: SocketAddr,
    peer: Option<crate::mtls::PeerCertificates>,
) -> Result<(), String> {
    let (head, mut stream) = resolver
        .resolve_request()
        .await
        .map_err(|e| format!("h3 resolve request: {e}"))?;
    let (mut parts, ()) = head.into_parts();
    // h3 carries the host as `:authority` (in the URI), not as a
    // `host` header — but Host dispatch and the admin host guard
    // both read the header. Synthesize it when missing so QUIC
    // and TCP route identically.
    if !parts.headers.contains_key(header::HOST)
        && let Some(authority) = parts.uri.authority()
    {
        let value = header::HeaderValue::from_str(authority.as_str())
            .map_err(|e| format!("h3 authority to host: {e}"))?;
        parts.headers.insert(header::HOST, value);
    }
    if is_ws_path(parts.uri.path()) {
        return send_synthetic(
            &mut stream,
            StatusCode::NOT_IMPLEMENTED,
            serde_json::json!({"error": "websocket requires TCP; use the https endpoint"}),
        )
        .await;
    }
    // Early reject: a declared over-cap length never reaches
    // the read loop (slow-drip bodies still trip the loop cap).
    if let Some(reject) = parts
        .headers
        .get(header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok())
        .and_then(bridge_body_allowed)
    {
        return send_synthetic(&mut stream, StatusCode::PAYLOAD_TOO_LARGE, reject).await;
    }
    let mut body_bytes = Vec::new();
    while let Some(mut chunk) = stream
        .recv_data()
        .await
        .map_err(|e| format!("h3 receive body: {e}"))?
    {
        if body_bytes.len() + chunk.remaining() > MAX_BRIDGE_BODY {
            return Err("h3 request body exceeds 2 MiB".to_string());
        }
        while chunk.has_remaining() {
            body_bytes.extend_from_slice(chunk.chunk());
            let advanced = chunk.chunk().len();
            chunk.advance(advanced);
        }
    }
    let mut builder = Request::builder().method(parts.method).uri(parts.uri);
    *builder
        .headers_mut()
        .ok_or_else(|| "h3 request head invalid".to_string())? = parts.headers;
    let mut axum_req = builder
        .body(Body::from(body_bytes))
        .map_err(|e| format!("bridge request head: {e}"))?;
    // The admin branch's fingerprint layer reads this (public has
    // it stripped at dispatch — same as TCP).
    if let Some(peer) = peer {
        axum_req.extensions_mut().insert(peer);
    }
    // Rate-limit buckets read the peer IP from here (same value TCP
    // injects via `into_make_service_with_connect_info`).
    axum_req.extensions_mut().insert(ConnectInfo(remote));
    // Access logs read the transport from here (TCP stamps its own
    // in `insert_tcp_transport`, outside the served routers).
    axum_req.extensions_mut().insert(RequestTransport("quic"));
    let res = match router.clone().oneshot(axum_req).await {
        Ok(res) => res,
        Err(never) => match never {},
    };
    let (res_parts, res_body) = res.into_parts();
    let mut res_builder = Response::builder().status(res_parts.status);
    *res_builder
        .headers_mut()
        .ok_or_else(|| "bridge response head invalid".to_string())? = res_parts.headers;
    let res_head = res_builder
        .body(())
        .map_err(|e| format!("bridge response head: {e}"))?;
    stream
        .send_response(res_head)
        .await
        .map_err(|e| format!("h3 send response: {e}"))?;
    let bytes = axum::body::to_bytes(res_body, MAX_BRIDGE_BODY)
        .await
        .map_err(|e| format!("bridge collect body: {e}"))?;
    if !bytes.is_empty() {
        stream
            .send_data(bytes)
            .await
            .map_err(|e| format!("h3 send body: {e}"))?;
    }
    stream
        .finish()
        .await
        .map_err(|e| format!("h3 finish: {e}"))?;
    Ok(())
}

/// Write a small synthetic response (the WS-exclusion 501) without
/// touching the router.
async fn send_synthetic(
    stream: &mut h3::server::RequestStream<h3_quinn::BidiStream<bytes::Bytes>, bytes::Bytes>,
    status: StatusCode,
    value: serde_json::Value,
) -> Result<(), String> {
    let head = Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "application/json")
        .body(())
        .map_err(|e| format!("synthetic response head: {e}"))?;
    stream
        .send_response(head)
        .await
        .map_err(|e| format!("h3 send response: {e}"))?;
    stream
        .send_data(bytes::Bytes::from(value.to_string()))
        .await
        .map_err(|e| format!("h3 send body: {e}"))?;
    stream
        .finish()
        .await
        .map_err(|e| format!("h3 finish: {e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bridge_cap_rejects_declared_oversize_early() {
        assert!(bridge_body_allowed(0).is_none(), "empty body fits");
        assert!(
            bridge_body_allowed(MAX_BRIDGE_BODY as u64).is_none(),
            "exactly the cap fits"
        );
        let reject = bridge_body_allowed(MAX_BRIDGE_BODY as u64 + 1);
        let body = reject.expect("one byte over must reject");
        assert_eq!(
            body["error"].as_str().unwrap_or(""),
            "request body too large (2 MiB bridge cap)"
        );
        assert!(
            bridge_body_allowed(u64::MAX).is_some(),
            "absurd lengths reject without reading"
        );
    }

    #[test]
    fn ws_paths_stay_tcp_only() {
        for path in [
            "/apostille/ws/echo",
            "/apostille/ws/bus",
            "/apostille/ws/future",
        ] {
            assert!(is_ws_path(path), "path: {path}");
        }
        for path in [
            "/apostille/healthz",
            "/apostille/api/v1/me",
            "/apostille/ws",
            "/apostille/ws-legacy",
            "/",
        ] {
            assert!(!is_ws_path(path), "path: {path}");
        }
    }

    #[test]
    fn server_config_requires_h3_alpn_material() {
        let pair =
            rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).expect("rcgen");
        let cert = pair.cert.pem().into_bytes();
        let key = pair.key_pair.serialize_pem().into_bytes();
        let config = server_config(&cert, &key, None).expect("quic config builds");
        // Round-trip proof would need a handshake; here we pin the
        // constructor's failure modes (bad PEM fails closed).
        drop(config);
        server_config(b"", &key, None).expect_err("empty cert fails");
        server_config(&cert, b"no key here", None).expect_err("bad key fails");
    }
}
