//! Apostille web API (S3 lane, roadmap Day 1–2 slice).
//!
//! All routes live under `/apostille`: health/version, static files, a
//! WS echo probe, and DEMO login/session (`/api/v1/demo/login`,
//! `/api/v1/me`). No real auth: passkey/OAuth come later per
//! `docs/specs/roadmap.md`.
//!
//! NOTE: real WS topics (`authority:{slug}`, `call:{id}`,
//! `submission:{id}`, `shipment:{id}`, `billing:{id}`) come later
//! (Day 2+).

use axum::{
    Router,
    extract::{
        State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Json},
    routing::{get, post},
};
use handbook::{db, store::Store};
use serde_json::json;
use std::net::SocketAddr;
use std::path::PathBuf;
use tower_http::services::{ServeDir, ServeFile};

/// Shared router state: SPA shell path plus the store root. Handlers
/// open a NEW SQLite connection per request (`rusqlite::Connection`
/// is `!Sync`, so one shared connection cannot cross await points;
/// a pool comes later).
#[derive(Clone)]
struct AppState {
    spa_index: PathBuf,
    store_root: PathBuf,
}

/// Build the full app router. `static_dir` roots `/apostille/static`;
/// `spa_dir` (the SvelteKit `build/` output) serves the app shell at
/// `/apostille/` and every other `/apostille/*` path via SPA fallback;
/// `store_root` locates the SQLite metadata DB for session handlers.
fn app(static_dir: &str, spa_dir: &str, store_root: PathBuf) -> Router {
    let spa_index = std::path::Path::new(spa_dir).join("index.html");
    let api = Router::new()
        .route("/healthz", get(healthz))
        .route("/version", get(version))
        .route("/ws/echo", get(ws_echo_upgrade))
        .route("/api/v1/demo/login", post(demo_login))
        .route("/api/v1/me", get(me))
        .nest_service(
            "/static",
            ServeDir::new(static_dir)
                .precompressed_gzip()
                .precompressed_br(),
        )
        .fallback_service(ServeDir::new(spa_dir).fallback(ServeFile::new(spa_index.clone())));
    // axum 0.8 `nest` does not route the bare `/apostille/` (trailing
    // slash) into the nest, so the shell gets explicit outer routes.
    Router::new()
        .route("/apostille", get(serve_spa_index))
        .route("/apostille/", get(serve_spa_index))
        .nest("/apostille", api)
        .with_state(AppState {
            spa_index,
            store_root,
        })
}

/// Serve the SPA shell for `/apostille/` exactly (the nested `/` never
/// reaches `ServeDir`'s directory-index handling, so it needs its own
/// route; deep links fall through to the `ServeDir` fallback above).
async fn serve_spa_index(State(state): State<AppState>) -> impl IntoResponse {
    match tokio::fs::read(&state.spa_index).await {
        Ok(bytes) => (
            [(axum::http::header::CONTENT_TYPE, "text/html; charset=utf-8")],
            bytes,
        )
            .into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn healthz() -> impl IntoResponse {
    Json(json!({"status": "ok", "version": env!("CARGO_PKG_VERSION")}))
}

async fn version() -> impl IntoResponse {
    Json(json!({
        "version": env!("CARGO_PKG_VERSION"),
        "build": "dev",
    }))
}

async fn ws_echo_upgrade(ws: WebSocketUpgrade) -> impl IntoResponse {
    ws.on_upgrade(ws_echo)
}

/// DEMO login: no password is checked (clearly-marked demo only).
/// Creates a `demo_session` row with 24h expiry and sets a
/// `demo_session` cookie. The full token is stored server-side; prod
/// upgrades to hashes before any real auth.
async fn demo_login(
    State(state): State<AppState>,
    Json(body): Json<LoginBody>,
) -> impl IntoResponse {
    if body.customer_ref.is_empty() || body.customer_ref.len() > 64 {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"error": "invalid customer_ref"})),
        )
            .into_response();
    }
    let bytes = rand::random::<[u8; 32]>();
    let mut token = String::with_capacity(64);
    for b in bytes {
        token.push_str(&format!("{b:02x}"));
    }
    let now = unix_now();
    let conn = match db::open(&Store::new(&state.store_root)) {
        Ok(conn) => conn,
        Err(_) => return store_unavailable(),
    };
    // M1: sweep expired rows on every login so the demo table cannot
    // grow unboundedly (a per-IP rate limit is still Day-7 work).
    let _ = db::delete_expired_demo_sessions(&conn, now);
    if db::create_demo_session(&conn, &token, &body.customer_ref, now, now + 86_400).is_err() {
        return store_unavailable();
    }
    // Built manually (no cookie crate): token is hex, so always valid.
    let cookie = format!("demo_session={token}; HttpOnly; SameSite=Lax; Path=/apostille");
    let Ok(set_cookie) = HeaderValue::from_str(&cookie) else {
        return store_unavailable();
    };
    (
        StatusCode::OK,
        [(header::SET_COOKIE, set_cookie)],
        Json(json!({"customer_ref": body.customer_ref})),
    )
        .into_response()
}

/// DEMO session check: missing/unknown/expired token → 401.
async fn me(State(state): State<AppState>, headers: HeaderMap) -> impl IntoResponse {
    let unauthorized = || {
        (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error": "unauthorized"})),
        )
            .into_response()
    };
    let token = headers
        .get(header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .and_then(|cookies| {
            cookies
                .split(';')
                .map(str::trim)
                .find_map(|pair| pair.strip_prefix("demo_session="))
        });
    let Some(token) = token.filter(|t| !t.is_empty()) else {
        return unauthorized();
    };
    let conn = match db::open(&Store::new(&state.store_root)) {
        Ok(conn) => conn,
        Err(_) => return store_unavailable(),
    };
    let Ok(Some((customer_ref, expires_at))) = db::get_demo_session(&conn, token) else {
        return unauthorized();
    };
    if unix_now() >= expires_at {
        return unauthorized();
    }
    (
        StatusCode::OK,
        Json(json!({"customer_ref": customer_ref, "expires_at": expires_at})),
    )
        .into_response()
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)] // m3: mass-assignment hygiene on mutating DTOs
struct LoginBody {
    customer_ref: String,
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn store_unavailable() -> axum::response::Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({"error": "store unavailable"})),
    )
        .into_response()
}

async fn ws_echo(mut socket: WebSocket) {
    while let Some(msg) = socket.recv().await {
        match msg {
            Ok(msg) => {
                let reply = match msg {
                    Message::Text(t) => Message::Text(t),
                    Message::Binary(b) => Message::Binary(b),
                    Message::Ping(p) => Message::Pong(p),
                    Message::Pong(_) | Message::Close(_) => break,
                };
                if socket.send(reply).await.is_err() {
                    break;
                }
            }
            Err(_) => break,
        }
    }
}

/// Env port with a default; garbage parses to the default (fail-safe:
/// never bind a surprising port because of a typo).
fn port_env(name: &str, default: u16) -> u16 {
    std::env::var(name)
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(default)
}

/// IPs to bind: `BIND` (default 127.0.0.1) plus optional `BIND6`.
/// Prod sets the public .41 addresses explicitly — never 0.0.0.0
/// (other services own :80/:443 on sibling IPs of the same box).
fn bind_ips() -> Vec<std::net::IpAddr> {
    let v4 = std::env::var("BIND")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(std::net::IpAddr::from([127, 0, 0, 1]));
    let mut ips = vec![v4];
    if let Some(v6) = std::env::var("BIND6").ok().and_then(|s| s.parse().ok()) {
        ips.push(v6);
    }
    ips
}

/// Load rustls config from `TLS_CERT`/`TLS_KEY` PEM paths.
/// Neither set → plain HTTP dev mode; exactly one set, or an
/// unreadable file → Err, and main refuses to start (TLS
/// misconfiguration must never silently downgrade to HTTP).
async fn load_tls(
    cert: Option<String>,
    key: Option<String>,
) -> Result<Option<axum_server::tls_rustls::RustlsConfig>, String> {
    match (cert, key) {
        (None, None) => Ok(None),
        (Some(cert), Some(key)) => {
            axum_server::tls_rustls::RustlsConfig::from_pem_file(&cert, &key)
                .await
                .map(Some)
                .map_err(|e| format!("read TLS_CERT={cert} TLS_KEY=<redacted>: {e}"))
        }
        _ => Err("set both TLS_CERT and TLS_KEY, or neither".to_string()),
    }
}

/// 308 every HTTP request to `https://{PUBLIC_HOST}{path}`.
/// The host comes from config, never from the Host header (a
/// header-derived target would be an open redirect).
fn redirect_router(public_host: String) -> Router {
    Router::new()
        .fallback(redirect_to_https)
        .with_state(public_host)
}

async fn redirect_to_https(State(host): State<String>, uri: axum::http::Uri) -> impl IntoResponse {
    let path = uri.path_and_query().map(|p| p.as_str()).unwrap_or("/");
    match HeaderValue::from_str(&format!("https://{host}{path}")) {
        Ok(location) => (
            StatusCode::PERMANENT_REDIRECT,
            [(header::LOCATION, location)],
        )
            .into_response(),
        Err(_) => StatusCode::BAD_REQUEST.into_response(),
    }
}

#[tokio::main]
async fn main() {
    let static_dir =
        std::env::var("STATIC_DIR").unwrap_or_else(|_| "crates/web/static".to_string());
    let spa_dir = std::env::var("SPA_DIR").unwrap_or_else(|_| "web/build".to_string());
    let store_dir = std::env::var("STORE_DIR").unwrap_or_else(|_| "store".to_string());
    let store = Store::new(&store_dir);
    if let Err(e) = store.init() {
        panic!("STORE_DIR={store_dir}: init store: {e}");
    }
    let conn =
        db::open(&store).unwrap_or_else(|e| panic!("STORE_DIR={store_dir}: open metadata DB: {e}"));
    db::migrate(&conn)
        .unwrap_or_else(|e| panic!("STORE_DIR={store_dir}: migrate metadata DB: {e}"));
    drop(conn);
    let router = app(&static_dir, &spa_dir, PathBuf::from(store_dir));
    let tls = load_tls(
        std::env::var("TLS_CERT").ok(),
        std::env::var("TLS_KEY").ok(),
    )
    .await;
    match tls {
        Ok(Some(tls)) => serve_tls(router, tls).await,
        Ok(None) => serve_http(router).await,
        Err(message) => {
            eprintln!("refusing to start: {message}");
            std::process::exit(1);
        }
    }
}

/// Plain-HTTP dev mode: `PORT` (default 3000) on every bind IP.
async fn serve_http(router: Router) {
    let port = port_env("PORT", 3000);
    let mut servers = tokio::task::JoinSet::new();
    let mut handles = Vec::new();
    for ip in bind_ips() {
        let addr = SocketAddr::new(ip, port);
        let handle = axum_server::Handle::new();
        handles.push(handle.clone());
        eprintln!("listening on http://{addr}/apostille");
        servers.spawn({
            let router = router.clone();
            async move {
                axum_server::bind(addr)
                    .handle(handle)
                    .serve(router.into_make_service())
                    .await
            }
        });
    }
    supervise(servers, handles).await;
}

/// Prod mode: HTTPS on `HTTPS_PORT` (443) plus an HTTP→HTTPS
/// redirector on `HTTP_PORT` (80), on every bind IP.
async fn serve_tls(router: Router, tls: axum_server::tls_rustls::RustlsConfig) {
    let https_port = port_env("HTTPS_PORT", 443);
    let http_port = port_env("HTTP_PORT", 80);
    let public_host = std::env::var("PUBLIC_HOST").unwrap_or_else(|_| "andrii.ai".to_string());
    let mut servers = tokio::task::JoinSet::new();
    let mut handles = Vec::new();
    for ip in bind_ips() {
        for (port, plain) in [(https_port, false), (http_port, true)] {
            let addr = SocketAddr::new(ip, port);
            let handle = axum_server::Handle::new();
            handles.push(handle.clone());
            if plain {
                eprintln!("listening on http://{addr} (redirect only)");
                servers.spawn({
                    let redirect = redirect_router(public_host.clone());
                    async move {
                        axum_server::bind(addr)
                            .handle(handle)
                            .serve(redirect.into_make_service())
                            .await
                    }
                });
            } else {
                eprintln!("listening on https://{addr}/apostille");
                servers.spawn({
                    let router = router.clone();
                    let tls = tls.clone();
                    async move {
                        axum_server::bind_rustls(addr, tls)
                            .handle(handle)
                            .serve(router.into_make_service())
                            .await
                    }
                });
            }
        }
    }
    supervise(servers, handles).await;
}

/// Supervise every listener: SIGINT/SIGTERM (unix-only: our dev and
/// prod are both unix) starts a 10s graceful drain; a listener that
/// fails BEFORE the signal (e.g. bind error) fails the whole
/// process at once — half-listening on one IP while another errors
/// out would split the service into halves silently.
async fn supervise(
    mut servers: tokio::task::JoinSet<std::io::Result<()>>,
    handles: Vec<axum_server::Handle>,
) -> ! {
    fn drain(handles: &[axum_server::Handle]) {
        eprintln!("shutting down");
        for handle in handles {
            handle.graceful_shutdown(Some(std::time::Duration::from_secs(10)));
        }
    }
    let ctrl_c = tokio::signal::ctrl_c();
    tokio::pin!(ctrl_c);
    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .expect("install SIGTERM handler");
    let mut draining = false;
    let mut failed = false;
    loop {
        tokio::select! {
            _ = &mut ctrl_c, if !draining => {
                drain(&handles);
                draining = true;
            }
            _ = sigterm.recv(), if !draining => {
                drain(&handles);
                draining = true;
            }
            outcome = servers.join_next() => {
                match outcome {
                    None => std::process::exit(i32::from(failed)),
                    Some(Ok(Ok(()))) => {}
                    Some(Ok(Err(e))) => {
                        eprintln!("listener failed: {e}");
                        failed = true;
                        if !draining {
                            drain(&handles);
                            draining = true;
                        }
                    }
                    Some(Err(e)) => {
                        eprintln!("listener task failed: {e}");
                        failed = true;
                        if !draining {
                            drain(&handles);
                            draining = true;
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    fn test_app() -> Router {
        // Tests run with cwd = crates/web, so the workspace-relative
        // defaults would miss; point at local fixture dirs.
        let dir = std::env::var("STATIC_DIR").unwrap_or_else(|_| "static".to_string());
        let spa = std::env::var("SPA_DIR").unwrap_or_else(|_| "test-spa".to_string());
        // Static/health tests never touch the DB; the path stays unused.
        app(&dir, &spa, PathBuf::from("test-store-unused"))
    }

    /// App wired to a migrated temp store (handlers read state, not env).
    fn session_app(store_root: &std::path::Path) -> Router {
        let dir = std::env::var("STATIC_DIR").unwrap_or_else(|_| "static".to_string());
        let spa = std::env::var("SPA_DIR").unwrap_or_else(|_| "test-spa".to_string());
        let store = Store::new(store_root);
        store.init().unwrap();
        let conn = db::open(&store).unwrap();
        db::migrate(&conn).unwrap();
        app(&dir, &spa, store_root.to_path_buf())
    }

    fn login_request(customer_ref: &str) -> Request<Body> {
        Request::builder()
            .uri("/apostille/api/v1/demo/login")
            .method("POST")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::json!({"customer_ref": customer_ref}).to_string(),
            ))
            .unwrap()
    }

    #[tokio::test]
    async fn healthz_returns_ok_json() {
        let res = test_app()
            .oneshot(
                Request::builder()
                    .uri("/apostille/healthz")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = axum::body::to_bytes(res.into_body(), 1024).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["status"], "ok");
        assert_eq!(v["version"], env!("CARGO_PKG_VERSION"));
    }

    #[tokio::test]
    async fn deep_route_serves_spa_index() {
        for uri in [
            "/apostille",
            "/apostille/",
            "/apostille/nope",
            "/apostille/deep/link",
        ] {
            let res = test_app()
                .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(res.status(), StatusCode::OK, "uri: {uri}");
            let body = axum::body::to_bytes(res.into_body(), 4096).await.unwrap();
            assert!(
                body.windows(b"spa-fixture".len())
                    .any(|w| w == b"spa-fixture"),
                "uri: {uri}"
            );
        }
    }

    #[tokio::test]
    async fn static_robots_txt_serves() {
        let res = test_app()
            .oneshot(
                Request::builder()
                    .uri("/apostille/static/robots.txt")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = axum::body::to_bytes(res.into_body(), 4096).await.unwrap();
        assert!(body.starts_with(b"User-agent:"));
    }

    #[tokio::test]
    async fn demo_login_then_me_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let login = session_app(tmp.path())
            .oneshot(login_request("demo-1"))
            .await
            .unwrap();
        assert_eq!(login.status(), StatusCode::OK);
        let set_cookie = login
            .headers()
            .get(axum::http::header::SET_COOKIE)
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        assert!(set_cookie.starts_with("demo_session="));
        assert!(set_cookie.contains("HttpOnly"));
        assert!(set_cookie.contains("SameSite=Lax"));
        assert!(set_cookie.contains("Path=/apostille"));
        let body = axum::body::to_bytes(login.into_body(), 1024).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["customer_ref"], "demo-1");

        let cookie = set_cookie.split(';').next().unwrap().to_string();
        let res = session_app(tmp.path())
            .oneshot(
                Request::builder()
                    .uri("/apostille/api/v1/me")
                    .header("cookie", cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = axum::body::to_bytes(res.into_body(), 1024).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["customer_ref"], "demo-1");
        assert!(v["expires_at"].as_i64().unwrap() > unix_now());
    }

    #[tokio::test]
    async fn me_without_cookie_is_unauthorized() {
        let tmp = tempfile::tempdir().unwrap();
        let res = session_app(tmp.path())
            .oneshot(
                Request::builder()
                    .uri("/apostille/api/v1/me")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
        let body = axum::body::to_bytes(res.into_body(), 1024).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["error"], "unauthorized");
    }

    #[tokio::test]
    async fn me_with_bogus_cookie_is_unauthorized() {
        let tmp = tempfile::tempdir().unwrap();
        let res = session_app(tmp.path())
            .oneshot(
                Request::builder()
                    .uri("/apostille/api/v1/me")
                    .header("cookie", "demo_session=bogus")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
        let body = axum::body::to_bytes(res.into_body(), 1024).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["error"], "unauthorized");
    }

    #[tokio::test]
    async fn demo_login_rejects_bad_ref() {
        let tmp = tempfile::tempdir().unwrap();
        for customer_ref in [String::new(), "x".repeat(65)] {
            let res = session_app(tmp.path())
                .oneshot(login_request(&customer_ref))
                .await
                .unwrap();
            assert_eq!(
                res.status(),
                StatusCode::UNPROCESSABLE_ENTITY,
                "ref len {}",
                customer_ref.len()
            );
        }
    }

    #[tokio::test]
    async fn demo_login_rejects_unknown_fields() {
        let tmp = tempfile::tempdir().unwrap();
        let res = session_app(tmp.path())
            .oneshot(
                Request::builder()
                    .uri("/apostille/api/v1/demo/login")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({"customer_ref": "demo-1", "admin": true}).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[tokio::test]
    async fn me_with_expired_cookie_is_unauthorized() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::new(tmp.path());
        store.init().unwrap();
        let conn = db::open(&store).unwrap();
        db::migrate(&conn).unwrap();
        db::create_demo_session(&conn, "tok-old", "demo-1", 1000, 1001).unwrap();
        drop(conn);
        let res = session_app(tmp.path())
            .oneshot(
                Request::builder()
                    .uri("/apostille/api/v1/me")
                    .header("cookie", "demo_session=tok-old")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
        let body = axum::body::to_bytes(res.into_body(), 1024).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["error"], "unauthorized");
    }

    #[tokio::test]
    async fn https_redirect_points_at_public_host() {
        let res = redirect_router("example.test".to_string())
            .oneshot(
                Request::builder()
                    .uri("/apostille/order?q=1")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::PERMANENT_REDIRECT);
        let location = res
            .headers()
            .get(axum::http::header::LOCATION)
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        assert_eq!(location, "https://example.test/apostille/order?q=1");
    }

    #[tokio::test]
    async fn load_tls_requires_both_or_neither() {
        assert!(load_tls(None, None).await.unwrap().is_none());
        load_tls(Some("a".to_string()), None)
            .await
            .expect_err("cert without key must fail");
        load_tls(None, Some("b".to_string()))
            .await
            .expect_err("key without cert must fail");
        load_tls(
            Some("no-such-cert.pem".to_string()),
            Some("no-such-key.pem".to_string()),
        )
        .await
        .expect_err("missing files must fail");
    }

    /// rcgen localhost cert + key as PEM bytes (test-only CA-less fixture).
    fn self_signed_localhost() -> (Vec<u8>, Vec<u8>) {
        let key = rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();
        (
            key.cert.pem().into_bytes(),
            key.key_pair.serialize_pem().into_bytes(),
        )
    }

    /// Test-only TLS client that skips verification (the rcgen cert
    /// is self-signed). NEVER use outside tests.
    #[derive(Debug)]
    struct AcceptAny;

    impl rustls::client::danger::ServerCertVerifier for AcceptAny {
        fn verify_server_cert(
            &self,
            _end_entity: &rustls::pki_types::CertificateDer<'_>,
            _intermediates: &[rustls::pki_types::CertificateDer<'_>],
            _server_name: &rustls::pki_types::ServerName<'_>,
            _ocsp_response: &[u8],
            _now: rustls::pki_types::UnixTime,
        ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
            Ok(rustls::client::danger::ServerCertVerified::assertion())
        }

        fn verify_tls12_signature(
            &self,
            _message: &[u8],
            _cert: &rustls::pki_types::CertificateDer<'_>,
            _dss: &rustls::DigitallySignedStruct,
        ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
            Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
        }

        fn verify_tls13_signature(
            &self,
            _message: &[u8],
            _cert: &rustls::pki_types::CertificateDer<'_>,
            _dss: &rustls::DigitallySignedStruct,
        ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
            Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
        }

        fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
            vec![
                rustls::SignatureScheme::ECDSA_NISTP256_SHA256,
                rustls::SignatureScheme::ECDSA_NISTP384_SHA384,
                rustls::SignatureScheme::ED25519,
                rustls::SignatureScheme::RSA_PSS_SHA256,
            ]
        }
    }

    #[tokio::test]
    async fn tls_serves_healthz_end_to_end() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let (cert_pem, key_pem) = self_signed_localhost();
        let tls = axum_server::tls_rustls::RustlsConfig::from_pem(cert_pem, key_pem)
            .await
            .unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = axum_server::Handle::new();
        let server = tokio::spawn({
            let handle = handle.clone();
            async move {
                axum_server::tls_rustls::from_tcp_rustls(listener, tls)
                    .handle(handle)
                    .serve(test_app().into_make_service())
                    .await
            }
        });

        let config = rustls::ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(std::sync::Arc::new(AcceptAny))
            .with_no_client_auth();
        let connector = tokio_rustls::TlsConnector::from(std::sync::Arc::new(config));
        let stream = tokio::net::TcpStream::connect(addr).await.unwrap();
        let name = rustls::pki_types::ServerName::try_from("localhost").unwrap();
        let mut tls = connector.connect(name, stream).await.unwrap();
        tls.write_all(
            b"GET /apostille/healthz HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
        )
        .await
        .unwrap();
        let mut raw = Vec::new();
        tls.read_to_end(&mut raw).await.unwrap();
        let text = String::from_utf8_lossy(&raw);
        assert!(
            text.starts_with("HTTP/1.1 200"),
            "got: {}",
            text.lines().next().unwrap_or("")
        );
        assert!(text.contains("\"status\""), "got: {text}");

        handle.shutdown();
        server.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn login_with_broken_store_is_500() {
        let tmp = tempfile::tempdir().unwrap();
        // A regular FILE as store root: db::open cannot create
        // `<file>/handbook.sqlite`, so handlers must 500, not panic.
        let blocker = tmp.path().join("blocker");
        std::fs::write(&blocker, b"not a dir").unwrap();
        let dir = std::env::var("STATIC_DIR").unwrap_or_else(|_| "static".to_string());
        let spa = std::env::var("SPA_DIR").unwrap_or_else(|_| "test-spa".to_string());
        let res = app(&dir, &spa, blocker)
            .oneshot(login_request("demo-1"))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let body = axum::body::to_bytes(res.into_body(), 1024).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["error"], "store unavailable");
    }
}
