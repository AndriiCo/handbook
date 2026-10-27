//! mTLS for the ADMIN listener only (H6 fix part 2).
//!
//! Two layers, split where the failure belongs:
//!
//! 1. TLS handshake ([`MtlsAcceptor`] + [`admin_server_config`]):
//!    when `tls.client_ca_file` is set the server requests a client
//!    certificate and verifies its chain against that CA bundle.
//!    A cert from any other CA fails the handshake outright.
//!    No CA file → plain TLS exactly as before.
//! 2. App auth ([`inject_verified_cert`] + [`AdminAuth`](crate::admin::AdminAuth)):
//!    the verified leaf's SHA-256 fingerprint must name a live
//!    `admin_cert` row, or strict mode 403s (and audits).
//!
//! Presence is enforced at the app layer — not the handshake — on
//! purpose: the Telnyx webhook shares the admin listener and
//! authenticates via Ed25519 (it holds no client cert), and a
//! cert-less staff caller gets an auditable HTTP 403 instead of an
//! opaque TLS alert. End to end the requirement holds: no
//! CA-chained + registered cert, no admin access.
//!
//! Single-443 shares ONE handshake for both branches, so the
//! acceptor below captures peer chains on every connection — but
//! the public branch never reads this module's state: Host dispatch
//! strips the extension before forwarding, and the fingerprint
//! layer runs on the admin branch only.

use axum::{
    http::Request,
    middleware::Next,
    response::{IntoResponse, Response},
};
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

/// Peer certificates captured by [`MtlsAcceptor`] during the TLS
/// handshake, carried per request. The `Debug` impl prints the count
/// only — DER bytes are key material and never hit logs.
#[derive(Clone)]
pub struct PeerCertificates(Vec<rustls::pki_types::CertificateDer<'static>>);

impl PeerCertificates {
    /// Leaf (first) certificate DER, if the peer presented a chain.
    fn leaf(&self) -> Option<&[u8]> {
        self.0.first().map(|der| der.as_ref())
    }

    /// Constructor from raw DER blobs. The QUIC bridge uses this
    /// for the peer chain quinn captured (same shape the TCP
    /// acceptor builds); tests use it for synthetic chains.
    pub fn from_ders(ders: Vec<Vec<u8>>) -> Self {
        PeerCertificates(
            ders.into_iter()
                .map(rustls::pki_types::CertificateDer::from)
                .collect(),
        )
    }
}

impl std::fmt::Debug for PeerCertificates {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "PeerCertificates({} certs)", self.0.len())
    }
}

/// Tower middleware (axum `from_fn`): hash the verified leaf with the
/// SAME function `admin-cert-issue` stores
/// ([`handbook_store::store::sha256_hex`] over DER) and insert
/// [`VerifiedClientCert`](crate::admin::VerifiedClientCert) for
/// [`AdminAuth`](crate::admin::AdminAuth) to check. No-ops when the
/// acceptor supplied no chain (plain TLS, no client cert): strict
/// mode then 403s downstream, exactly as before this seam existed.
pub async fn inject_verified_cert(mut req: Request<axum::body::Body>, next: Next) -> Response {
    if let Some(peer) = req.extensions().get::<PeerCertificates>()
        && let Some(leaf) = peer.leaf()
    {
        let fingerprint = handbook_store::store::sha256_hex(leaf);
        req.extensions_mut()
            .insert(crate::admin::VerifiedClientCert(fingerprint));
    }
    next.run(req).await.into_response()
}

/// Parse a PEM CA bundle into a rustls trust store. Empty input and
/// input with no `CERTIFICATE` blocks are errors (a set-but-empty CA
/// file must refuse the boot, never silently disable mTLS). Error
/// strings name the key, never key bytes.
pub fn load_client_ca(pem: &[u8]) -> Result<rustls::RootCertStore, String> {
    let mut reader = std::io::BufReader::new(pem);
    let certs = rustls_pemfile::certs(&mut reader)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("tls.client_ca_file: parse PEM: {e}"))?;
    if certs.is_empty() {
        return Err("tls.client_ca_file: no CERTIFICATE blocks".to_string());
    }
    let mut store = rustls::RootCertStore::empty();
    for cert in certs {
        store
            .add(cert)
            .map_err(|e| format!("tls.client_ca_file: reject CA cert: {e}"))?;
    }
    Ok(store)
}

/// Build the admin listener's rustls config from PEM bytes. With
/// `ca_pem` the server requests + chain-verifies client certs
/// (wrong CA → handshake failure; presence stays an app-level
/// check — see the module docs). Without it, plain TLS identical to
/// what `axum_server::tls_rustls::RustlsConfig::from_pem` builds.
/// Error strings carry labels only, never PEM bytes.
pub fn admin_server_config(
    cert_pem: &[u8],
    key_pem: &[u8],
    ca_pem: Option<&[u8]>,
) -> Result<rustls::ServerConfig, String> {
    let mut cert_reader = std::io::BufReader::new(cert_pem);
    let certs = rustls_pemfile::certs(&mut cert_reader)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("admin TLS cert: parse PEM: {e}"))?;
    if certs.is_empty() {
        return Err("admin TLS cert: no CERTIFICATE blocks".to_string());
    }
    let mut key_reader = std::io::BufReader::new(key_pem);
    let key = rustls_pemfile::private_key(&mut key_reader)
        .map_err(|e| format!("admin TLS key: parse PEM: {e}"))?
        .ok_or_else(|| "admin TLS key: no PRIVATE KEY block".to_string())?;
    match ca_pem {
        Some(ca) => {
            let roots = load_client_ca(ca)?;
            let verifier =
                rustls::server::WebPkiClientVerifier::builder(std::sync::Arc::new(roots))
                    .allow_unauthenticated()
                    .build()
                    .map_err(|e| format!("admin mTLS verifier: {e}"))?;
            rustls::ServerConfig::builder()
                .with_client_cert_verifier(verifier)
                .with_single_cert(certs, key)
                .map_err(|e| format!("admin TLS config: {e}"))
        }
        None => rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(certs, key)
            .map_err(|e| format!("admin TLS config: {e}")),
    }
}

/// True when the FIRST certificate in `cert_pem` carries
/// `dns_name` as a DNS SAN (case-insensitive compare).
/// Single-443 serves one pair for both hosts, so boot warns loudly
/// when the admin SAN is missing (the pair still loads — a missing
/// SAN only breaks browsers on the admin host, and refusing the
/// boot would brick staging before the reissue lands).
/// Error strings name the problem, never PEM bytes.
pub fn cert_has_dns_san(cert_pem: &[u8], dns_name: &str) -> Result<bool, String> {
    let mut reader = std::io::BufReader::new(cert_pem);
    let certs = rustls_pemfile::certs(&mut reader)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("TLS cert SAN check: parse PEM: {e}"))?;
    let Some(first) = certs.first() else {
        return Err("TLS cert SAN check: no CERTIFICATE blocks".to_string());
    };
    let (_, cert) = x509_parser::parse_x509_certificate(first.as_ref())
        .map_err(|e| format!("TLS cert SAN check: parse DER: {e}"))?;
    let sans = cert
        .subject_alternative_name()
        .map_err(|e| format!("TLS cert SAN check: read SANs: {e}"))?;
    let Some(sans) = sans else {
        return Ok(false);
    };
    Ok(sans.value.general_names.iter().any(|name| {
        matches!(name, x509_parser::extensions::GeneralName::DNSName(dns) if dns.eq_ignore_ascii_case(dns_name))
    }))
}

/// axum-server acceptor for the single-443 listener:
/// TLS-handshakes via the stock [`axum_server`] rustls acceptor,
/// then wraps the per-connection service so every request carries
/// the verified peer chain ([`PeerCertificates`]) for
/// [`inject_verified_cert`] (Host dispatch strips it again on the
/// public branch). Connections without a client cert get the inner
/// service unwrapped — indistinguishable from plain TLS
/// downstream.
#[derive(Clone)]
pub struct MtlsAcceptor {
    inner: axum_server::tls_rustls::RustlsAcceptor,
}

impl MtlsAcceptor {
    /// Wrap an admin [`axum_server`] rustls config (built by
    /// [`admin_server_config`] + `RustlsConfig::from_config`).
    pub fn new(config: axum_server::tls_rustls::RustlsConfig) -> Self {
        MtlsAcceptor {
            inner: axum_server::tls_rustls::RustlsAcceptor::new(config),
        }
    }
}

impl<S> axum_server::accept::Accept<tokio::net::TcpStream, S> for MtlsAcceptor
where
    S: Clone + Send + 'static,
{
    type Stream = tokio_rustls::server::TlsStream<tokio::net::TcpStream>;
    type Service = PeerCertService<S>;
    type Future =
        Pin<Box<dyn Future<Output = std::io::Result<(Self::Stream, Self::Service)>> + Send>>;

    fn accept(&self, stream: tokio::net::TcpStream, service: S) -> Self::Future {
        let inner = self.inner.clone();
        Box::pin(async move {
            let (tls_stream, service) = inner.accept(stream, service).await?;
            let peer = tls_stream
                .get_ref()
                .1
                .peer_certificates()
                .unwrap_or(&[])
                .to_vec();
            let service = PeerCertService {
                inner: service,
                peer: if peer.is_empty() {
                    None
                } else {
                    Some(PeerCertificates(peer))
                },
            };
            Ok((tls_stream, service))
        })
    }
}

/// Per-connection tower service wrapper from [`MtlsAcceptor`]:
/// inserts the connection's [`PeerCertificates`] into every
/// request's extensions before delegating.
#[derive(Clone, Debug)]
pub struct PeerCertService<S> {
    inner: S,
    peer: Option<PeerCertificates>,
}

impl<S, ReqBody, ResBody> tower::Service<Request<ReqBody>> for PeerCertService<S>
where
    S: tower::Service<Request<ReqBody>, Response = axum::response::Response<ResBody>>,
{
    type Response = axum::response::Response<ResBody>;
    type Error = S::Error;
    type Future = S::Future;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, mut req: Request<ReqBody>) -> Self::Future {
        if let Some(peer) = &self.peer {
            req.extensions_mut().insert(peer.clone());
        }
        self.inner.call(req)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Router, body::Body, http::StatusCode, routing::get};
    use tower::ServiceExt;

    /// Echo handler: reports whether the seam supplied an identity.
    async fn whoami(req: Request<Body>) -> impl IntoResponse {
        let fingerprint = req
            .extensions()
            .get::<crate::admin::VerifiedClientCert>()
            .map(|verified| verified.0.clone());
        axum::Json(serde_json::json!({"cert": fingerprint}))
    }

    fn layered() -> Router {
        Router::new()
            .route("/whoami", get(whoami))
            .layer(axum::middleware::from_fn(inject_verified_cert))
    }

    #[tokio::test]
    async fn middleware_hashes_leaf_like_admin_cert_issue() {
        // Same hash function, same input (leaf DER): the fingerprint
        // must equal what `admin-cert-issue` stored for this cert.
        let leaf_der = vec![0x30, 0x82, 0x01, 0x0a, 0x02, 0x01, 0x03];
        let mut req = Request::builder()
            .uri("/whoami")
            .body(Body::empty())
            .unwrap();
        req.extensions_mut()
            .insert(PeerCertificates::from_ders(vec![
                leaf_der.clone(),
                vec![0x30, 0x03],
            ]));
        let res = layered().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = axum::body::to_bytes(res.into_body(), 1024).await.unwrap();
        let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(
            value["cert"],
            serde_json::Value::String(handbook_store::store::sha256_hex(&leaf_der))
        );
    }

    #[tokio::test]
    async fn middleware_noops_without_peer_certs() {
        for req in [
            Request::builder()
                .uri("/whoami")
                .body(Body::empty())
                .unwrap(),
            {
                let mut req = Request::builder()
                    .uri("/whoami")
                    .body(Body::empty())
                    .unwrap();
                req.extensions_mut()
                    .insert(PeerCertificates::from_ders(Vec::new()));
                req
            },
        ] {
            let res = layered().oneshot(req).await.unwrap();
            assert_eq!(res.status(), StatusCode::OK);
            let body = axum::body::to_bytes(res.into_body(), 1024).await.unwrap();
            let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
            assert_eq!(value["cert"], serde_json::Value::Null);
        }
    }

    #[test]
    fn peer_certs_debug_redacts_der_bytes() {
        let peer = PeerCertificates::from_ders(vec![vec![0xde, 0xad, 0xbe, 0xef]]);
        let shown = format!("{peer:?}");
        assert!(shown.contains('1'), "{shown}");
        assert!(!shown.contains("222"), "{shown}");
        assert!(!shown.contains("ad"), "{shown}");
    }

    #[test]
    fn client_ca_parsing_fails_closed_and_redacted() {
        load_client_ca(b"").expect_err("empty bundle must fail");
        load_client_ca(b"not a pem file").expect_err("no blocks must fail");
        for bad in [b"".as_slice(), b"not a pem file".as_slice()] {
            let err = load_client_ca(bad).expect_err("must fail");
            assert!(err.contains("tls.client_ca_file"), "{err}");
            assert!(!err.contains("not a pem"), "{err}");
        }
        // A key file in the CA slot: no CERTIFICATE blocks, refused.
        let pair = rcgen::generate_simple_self_signed(vec!["leaf.test".to_string()]).unwrap();
        let err = load_client_ca(pair.key_pair.serialize_pem().as_bytes())
            .expect_err("key-only file is no CA bundle");
        assert!(err.contains("tls.client_ca_file"), "{err}");
        assert!(!err.contains("BEGIN"), "{err}");
    }

    #[test]
    fn san_check_matches_admin_host_case_insensitively() {
        let both = rcgen::generate_simple_self_signed(vec![
            "andrii.ai".to_string(),
            "admin.andrii.ai".to_string(),
        ])
        .unwrap();
        let pem = both.cert.pem().into_bytes();
        assert!(cert_has_dns_san(&pem, "admin.andrii.ai").unwrap());
        assert!(cert_has_dns_san(&pem, "ADMIN.ANDRII.AI").unwrap());
        assert!(cert_has_dns_san(&pem, "andrii.ai").unwrap());
        assert!(!cert_has_dns_san(&pem, "other.test").unwrap());

        let public_only =
            rcgen::generate_simple_self_signed(vec!["andrii.ai".to_string()]).unwrap();
        let pem = public_only.cert.pem().into_bytes();
        assert!(!cert_has_dns_san(&pem, "admin.andrii.ai").unwrap());

        // Garbage input errors redacted (labels only, never bytes).
        for bad in [b"".as_slice(), b"not a pem file".as_slice()] {
            let err = cert_has_dns_san(bad, "admin.andrii.ai").expect_err("must fail");
            assert!(err.contains("TLS cert SAN check"), "{err}");
            assert!(!err.contains("not a pem"), "{err}");
        }
    }

    #[test]
    fn server_config_needs_cert_and_key() {
        let pair = rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();
        let cert = pair.cert.pem().into_bytes();
        let key = pair.key_pair.serialize_pem().into_bytes();
        admin_server_config(&cert, &key, None).expect("plain config builds");
        let err = admin_server_config(b"", &key, None).expect_err("empty cert fails");
        assert!(!err.contains("BEGIN"), "{err}");
        let err = admin_server_config(&cert, b"no key here", None).expect_err("bad key fails");
        assert!(!err.contains("no key here"), "{err}");
        let err = admin_server_config(&cert, &key, Some(b"")).expect_err("empty CA fails closed");
        assert!(err.contains("tls.client_ca_file"), "{err}");
    }
}
