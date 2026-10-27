//! Apostille web API (S3 lane, roadmap items 2+6; G4 admin track).
//!
//! Public listener: everything under `/apostille` (health/version,
//! static, WS bus, accepted-facts reads, DEMO login/session) plus
//! the P2 mount stub. Admin listener (own port/origin): approval
//! API, encrypted-config API, bus publish — all behind the admin
//! host guard, staff sessions, mTLS-cert seam, and CSRF/Origin.
//!
//! Real login (passkey/Apple/Google/Meta + magic-link fallback) is
//! still ahead per `docs/auth-playbook.md`; the policy lives in the
//! `auth` crate and the DEMO session stays until then.

mod admin;
mod admin_workers;
mod approval;
mod p2;
mod p2_labels;
mod ws_bus;

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
use handbook_store::{db, store::Store};
use serde_json::json;
use std::net::SocketAddr;
use std::path::PathBuf;
use tower_http::services::{ServeDir, ServeFile};

/// Request access log, one span per request: `id method uri` in,
/// `status latency_ms` out. Logged fields are fixed and PII-free by
/// construction (our URLs never carry tokens or customer data).
/// The same opaque ID goes back in `x-request-id` for correlation.
async fn request_id_and_log(
    req: axum::http::Request<axum::body::Body>,
    next: axum::middleware::Next,
) -> impl IntoResponse {
    let id = format!(
        "{:016x}{:016x}",
        rand::random::<u64>(),
        rand::random::<u64>()
    );
    let span = tracing::info_span!(
        "request",
        id = %id,
        method = %req.method(),
        uri = %req.uri(),
    );
    let _guard = span.enter();
    let started = std::time::Instant::now();
    let mut res = next.run(req).await;
    if let Ok(value) = HeaderValue::from_str(&id) {
        res.headers_mut().insert("x-request-id", value);
    }
    tracing::info!(
        status = %res.status(),
        latency_ms = started.elapsed().as_millis(),
        "response"
    );
    res
}

/// Shared router state: SPA shell path plus the store root. Handlers
/// open a NEW SQLite connection per request (`rusqlite::Connection`
/// is `!Sync`, so one shared connection cannot cross await points;
/// a pool comes later).
#[derive(Clone)]
struct AppState {
    spa_index: PathBuf,
    static_dir: PathBuf,
    store_root: PathBuf,
    bus: ws_bus::Bus,
    kek: Option<std::sync::Arc<config::Kek>>,
    /// `ADMIN_MTLS=off` dev seam: skips the client-cert requirement
    /// and allows loopback origins. Prod runs strict (false).
    admin_mtls_off: bool,
    /// Admin SPA build dir (`ADMIN_SPA_DIR`, default `web/build-admin`),
    /// served from the admin listener root with SPA fallback.
    admin_spa_dir: PathBuf,
    /// Media terms gate (`MEDIA_TERMS_READ=1` once SEC confirms the
    /// Meta data-handling read). Until then only synthetic fixtures
    /// upload; real media fails closed.
    media_terms_read: bool,
    /// Prepaid-label config (USPS base URL + certification flag).
    label_cfg: p2_labels::LabelCfg,
}

/// Build the full PUBLIC app router. `static_dir` roots
/// `/apostille/static`; `spa_dir` (the SvelteKit `build/` output)
/// serves the app shell at `/apostille/` and every other
/// `/apostille/*` path via SPA fallback; `store_root` locates the
/// SQLite metadata DB for session handlers; `acme_dir` serves
/// `/.well-known/acme-challenge/` token files for certbot http-01
/// (missing dir → 404s, harmless in dev).
///
/// Default state (fresh bus, no KEK, dev TLS seam): enough for the
/// public surface; `main` builds richer state via
/// [`public_router`]. Test-only: production always wires explicit
/// state.
#[cfg(test)]
fn app(static_dir: &str, spa_dir: &str, store_root: PathBuf, acme_dir: &str) -> Router {
    public_router(
        AppState {
            spa_index: std::path::Path::new(spa_dir).join("index.html"),
            static_dir: PathBuf::from(static_dir),
            store_root,
            bus: ws_bus::Bus::new(),
            kek: None,
            admin_mtls_off: true,
            admin_spa_dir: PathBuf::from("test-admin-spa"),
            media_terms_read: false,
            label_cfg: p2_labels::LabelCfg::default(),
        },
        static_dir,
        spa_dir,
        acme_dir,
    )
}

/// Public router with explicit state (admin routes are NEVER here —
/// they live on the admin listener via [`admin::router`]).
fn public_router(state: AppState, static_dir: &str, spa_dir: &str, acme_dir: &str) -> Router {
    let spa_index = state.spa_index.clone();
    let api = Router::new()
        .route("/healthz", get(healthz))
        .route("/version", get(version))
        .route("/ws/echo", get(ws_echo_upgrade))
        .route("/ws/bus", get(ws_bus::ws_bus_upgrade))
        .route("/api/v1/demo/login", post(demo_login))
        .route("/api/v1/me", get(me))
        .route(
            "/api/v1/authorities/{slug}/facts",
            get(approval::public_facts),
        )
        .nest("/api/v1/p2", p2::router())
        // Unknown API paths must 404 as JSON, not fall through to
        // the SPA shell (static routes above still win over this
        // wildcard, so real endpoints are unaffected).
        .route("/api/{*path}", get(api_not_found))
        .nest_service(
            "/static",
            ServeDir::new(static_dir)
                .precompressed_gzip()
                .precompressed_br(),
        )
        .fallback_service(ServeDir::new(spa_dir).fallback(ServeFile::new(spa_index.clone())));
    // axum 0.8 `nest` does not route the bare `/apostille/` (trailing
    // slash) into the nest, so the shell gets explicit outer routes.
    // The ACME challenge path lives at the domain root (not under
    // /apostille): port 80 serves it directly for http-01, and this
    // HTTPS route covers CAs that follow the 308 here instead.
    Router::new()
        .route("/apostille", get(serve_spa_index))
        .route("/apostille/", get(serve_spa_index))
        .route("/robots.txt", get(serve_robots))
        .nest("/apostille", api)
        .nest_service("/.well-known/acme-challenge", ServeDir::new(acme_dir))
        .layer(axum::middleware::from_fn(request_id_and_log))
        .with_state(state)
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

/// Domain-root robots.txt (crawlers never look under `/apostille`).
async fn serve_robots(State(state): State<AppState>) -> impl IntoResponse {
    match tokio::fs::read(state.static_dir.join("robots.txt")).await {
        Ok(bytes) => (
            [(
                axum::http::header::CONTENT_TYPE,
                "text/plain; charset=utf-8",
            )],
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

async fn api_not_found() -> impl IntoResponse {
    (StatusCode::NOT_FOUND, Json(json!({"error": "not found"})))
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
///
/// One path is exempt: `/.well-known/acme-challenge/` serves token
/// files straight from port 80. The CA must fetch http-01 answers
/// over plain HTTP, and on first issuance there is no certificate
/// yet to complete a redirected HTTPS fetch — so the redirector
/// answers challenges itself instead of bouncing them.
fn redirect_router(public_host: String, acme_dir: &str) -> Router {
    Router::new()
        .nest_service("/.well-known/acme-challenge", ServeDir::new(acme_dir))
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
    // Structured access logs to stdout (journald collects). `info`
    // default; `debug` only on staging via RUST_LOG, never prod.
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();
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
    let p2conn =
        db::open_p2(&store).unwrap_or_else(|e| panic!("STORE_DIR={store_dir}: open P2 DB: {e}"));
    db::migrate_p2(&p2conn).unwrap_or_else(|e| panic!("STORE_DIR={store_dir}: migrate P2 DB: {e}"));
    drop(p2conn);
    auth::migrate_auth_tables(&conn)
        .unwrap_or_else(|e| panic!("STORE_DIR={store_dir}: migrate auth tables: {e}"));
    config::migrate_config_tables(&conn)
        .unwrap_or_else(|e| panic!("STORE_DIR={store_dir}: migrate config tables: {e}"));
    drop(conn);
    let admin_mtls_off = std::env::var("ADMIN_MTLS").ok().is_some_and(|v| v == "off");
    if admin_mtls_off {
        eprintln!(
            "WARNING: ADMIN_MTLS=off — admin listener skips client-cert auth (local dev only)"
        );
    }
    let kek = match config::load_kek() {
        Ok((kek, source)) => {
            kek.lock_memory();
            eprintln!("config KEK loaded via {source:?}");
            Some(std::sync::Arc::new(kek))
        }
        Err(e) => {
            eprintln!("config unavailable ({e}); admin config API will answer 503");
            None
        }
    };
    let acme_dir = std::env::var("ACME_CHALLENGE_DIR").unwrap_or_else(|_| "acme".to_string());
    let admin_spa_dir =
        std::env::var("ADMIN_SPA_DIR").unwrap_or_else(|_| "web/build-admin".to_string());
    let media_terms_read = std::env::var("MEDIA_TERMS_READ")
        .ok()
        .is_some_and(|v| v == "1");
    let state = AppState {
        spa_index: std::path::Path::new(&spa_dir).join("index.html"),
        static_dir: PathBuf::from(&static_dir),
        store_root: PathBuf::from(store_dir.as_str()),
        bus: ws_bus::Bus::new(),
        kek,
        admin_mtls_off,
        admin_spa_dir: PathBuf::from(admin_spa_dir),
        media_terms_read,
        label_cfg: p2_labels::LabelCfg::from_env(),
    };
    let public = public_router(state.clone(), &static_dir, &spa_dir, &acme_dir);
    // ===== ADMIN-WORKERS (goal-025 stream 1; parent reconciles) =====
    // Staff workers/captures API merged onto the ADMIN listener only.
    // Same guards as admin::router (staff AdminAuth per handler, host
    // guard + CSRF layers inside admin_workers::router); the merge is
    // fallback-safe because admin_workers sets no custom fallback, so
    // the admin SPA fallback still wins. Sibling streams: keep your
    // mount in your own clearly-commented block, do not edit this one.
    // ================================================================
    let admin = admin::router(state.clone()).merge(admin_workers::router(state));
    let tls = load_tls(
        std::env::var("TLS_CERT").ok(),
        std::env::var("TLS_KEY").ok(),
    )
    .await;
    match tls {
        // Explicit files always win: ACME stays off when the
        // operator pinned a certificate.
        Ok(Some(tls)) => serve_tls(public, admin, tls).await,
        Ok(None) if acme_enabled() => serve_acme(public, admin, &store_dir, &acme_dir).await,
        Ok(None) => serve_http(public, admin).await,
        Err(message) => {
            eprintln!("refusing to start: {message}");
            std::process::exit(1);
        }
    }
}

/// In-binary ACME is opt-in: `ACME_ENABLED=1` plus the `ACME_*`
/// variables (`docs/ops/acme.md`). Anything else is plain HTTP dev
/// mode (or explicit TLS files above).
fn acme_enabled() -> bool {
    std::env::var("ACME_ENABLED").ok().is_some_and(|v| v == "1")
}

/// ACME mode: load the persisted pair or issue it (Google Public CA,
/// http-01 via `acme_dir`), serve TLS with it, and renew in the
/// background with hot reloads — no restarts, ever.
async fn serve_acme(public: Router, admin: Router, store_dir: &str, acme_dir: &str) {
    let config = acme::AcmeConfig::from_env(std::path::Path::new(store_dir), acme_dir.into())
        .unwrap_or_else(|e| {
            eprintln!("refusing to start: {e}");
            std::process::exit(1);
        });
    if config.staging() {
        eprintln!("WARNING: ACME staging directory — issued certs are UNTRUSTED (test only)");
    }
    let pair = acme::ensure_certificate(&config).await.unwrap_or_else(|e| {
        eprintln!("refusing to start: {e}");
        std::process::exit(1);
    });
    let tls = axum_server::tls_rustls::RustlsConfig::from_pem(pair.cert_pem, pair.key_pem)
        .await
        .unwrap_or_else(|e| {
            eprintln!("refusing to start: ACME pair does not load: {e}");
            std::process::exit(1);
        });
    let loop_tls = tls.clone();
    tokio::spawn(async move {
        acme::renewal_loop(config, |cert, key| {
            let tls = loop_tls.clone();
            async move {
                tls.reload_from_pem(cert, key)
                    .await
                    .map_err(|e| e.to_string())
            }
        })
        .await
    });
    serve_tls(public, admin, tls).await
}

/// Plain-HTTP dev mode: public on `PORT` (default 3000) and admin
/// on `ADMIN_PORT` (default 3001), on every bind IP.
async fn serve_http(public: Router, admin: Router) {
    let port = port_env("PORT", 3000);
    let admin_port = port_env("ADMIN_PORT", 3001);
    let mut servers = tokio::task::JoinSet::new();
    let mut handles = Vec::new();
    for ip in bind_ips() {
        for (router, port, name) in [(&public, port, "public"), (&admin, admin_port, "admin")] {
            let addr = SocketAddr::new(ip, port);
            let handle = axum_server::Handle::new();
            handles.push(handle.clone());
            eprintln!("listening on http://{addr}/apostille ({name})");
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
    }
    supervise(servers, handles).await;
}

/// Prod mode: HTTPS public on `HTTPS_PORT` (443) + HTTPS admin on
/// `ADMIN_HTTPS_PORT` (444) plus an HTTP→HTTPS redirector on
/// `HTTP_PORT` (80), on every bind IP.
async fn serve_tls(public: Router, admin: Router, tls: axum_server::tls_rustls::RustlsConfig) {
    let https_port = port_env("HTTPS_PORT", 443);
    let admin_https_port = port_env("ADMIN_HTTPS_PORT", 444);
    let http_port = port_env("HTTP_PORT", 80);
    let public_host = std::env::var("PUBLIC_HOST").unwrap_or_else(|_| "andrii.ai".to_string());
    let acme_dir = std::env::var("ACME_CHALLENGE_DIR").unwrap_or_else(|_| "acme".to_string());
    let mut servers = tokio::task::JoinSet::new();
    let mut handles = Vec::new();
    for ip in bind_ips() {
        for (router, port, name) in [
            (&public, https_port, "public"),
            (&admin, admin_https_port, "admin"),
        ] {
            let addr = SocketAddr::new(ip, port);
            let handle = axum_server::Handle::new();
            handles.push(handle.clone());
            eprintln!("listening on https://{addr}/apostille ({name})");
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
        let addr = SocketAddr::new(ip, http_port);
        let handle = axum_server::Handle::new();
        handles.push(handle.clone());
        eprintln!("listening on http://{addr} (redirect + ACME challenges)");
        servers.spawn({
            let redirect = redirect_router(public_host.clone(), &acme_dir);
            async move {
                axum_server::bind(addr)
                    .handle(handle)
                    .serve(redirect.into_make_service())
                    .await
            }
        });
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
        let acme = std::env::var("ACME_CHALLENGE_DIR").unwrap_or_else(|_| "test-acme".to_string());
        // Static/health tests never touch the DB; the path stays unused.
        app(&dir, &spa, PathBuf::from("test-store-unused"), &acme)
    }

    /// App wired to a migrated temp store (handlers read state, not env).
    fn session_app(store_root: &std::path::Path) -> Router {
        let dir = std::env::var("STATIC_DIR").unwrap_or_else(|_| "static".to_string());
        let spa = std::env::var("SPA_DIR").unwrap_or_else(|_| "test-spa".to_string());
        let acme = std::env::var("ACME_CHALLENGE_DIR").unwrap_or_else(|_| "test-acme".to_string());
        let store = Store::new(store_root);
        store.init().unwrap();
        let conn = db::open(&store).unwrap();
        db::migrate(&conn).unwrap();
        app(&dir, &spa, store_root.to_path_buf(), &acme)
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
    async fn unknown_api_path_is_json_404() {
        let res = test_app()
            .oneshot(
                Request::builder()
                    .uri("/apostille/api/v1/nope")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::NOT_FOUND);
        let body = axum::body::to_bytes(res.into_body(), 1024).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["error"], "not found");
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
    async fn redirector_serves_challenge_but_redirects_rest() {
        // The CA's http-01 fetch must get the token over plain HTTP;
        // everything else 308s to the configured host.
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("tok-1"), "key-auth-body").unwrap();
        let dir = tmp.path().to_str().unwrap().to_string();
        let hit = redirect_router("example.test".to_string(), &dir)
            .oneshot(
                Request::builder()
                    .uri("/.well-known/acme-challenge/tok-1")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(hit.status(), StatusCode::OK);
        let body = axum::body::to_bytes(hit.into_body(), 4096).await.unwrap();
        assert_eq!(body.as_ref(), b"key-auth-body");
        let miss = redirect_router("example.test".to_string(), &dir)
            .oneshot(
                Request::builder()
                    .uri("/.well-known/acme-challenge/nope")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(miss.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn https_redirect_points_at_public_host() {
        let res = redirect_router("example.test".to_string(), "test-acme")
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
    async fn responses_carry_opaque_request_id() {
        let res = test_app()
            .oneshot(
                Request::builder()
                    .uri("/apostille/healthz")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let id = res
            .headers()
            .get("x-request-id")
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        assert_eq!(id.len(), 32);
        assert!(id.bytes().all(|b| b.is_ascii_hexdigit()), "got: {id}");
    }

    #[tokio::test]
    async fn acme_challenge_serves_token_file() {
        let res = test_app()
            .oneshot(
                Request::builder()
                    .uri("/.well-known/acme-challenge/FixtureToken123")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = axum::body::to_bytes(res.into_body(), 4096).await.unwrap();
        assert_eq!(body.as_ref(), b"acme-fixture-token-body\n");
        let missing = test_app()
            .oneshot(
                Request::builder()
                    .uri("/.well-known/acme-challenge/NoSuchToken")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(missing.status(), StatusCode::NOT_FOUND);
    }

    /// Migrated temp store + wired state for admin/approval tests.
    struct Fixture {
        _tmp: tempfile::TempDir,
        state: AppState,
    }

    fn fixture() -> Fixture {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::new(tmp.path());
        store.init().unwrap();
        let conn = db::open(&store).unwrap();
        db::migrate(&conn).unwrap();
        auth::migrate_auth_tables(&conn).unwrap();
        config::migrate_config_tables(&conn).unwrap();
        drop(conn);
        let state = AppState {
            spa_index: PathBuf::from("test-spa/index.html"),
            static_dir: PathBuf::from("static"),
            store_root: tmp.path().to_path_buf(),
            bus: ws_bus::Bus::new(),
            kek: Some(std::sync::Arc::new(config::Kek::generate())),
            admin_mtls_off: true,
            admin_spa_dir: tmp.path().join("admin-spa"),
            media_terms_read: false,
            label_cfg: p2_labels::LabelCfg::default(),
        };
        Fixture { _tmp: tmp, state }
    }

    fn make_session(store_root: &std::path::Path, id: &str, role: auth::Role) {
        let store = Store::new(store_root);
        let conn = db::open(&store).unwrap();
        auth::create_session(
            &conn,
            id,
            &auth::NewSession {
                user_id: id,
                role,
                customer_ref: Some("cust-1"),
                now: unix_now(),
                ip: "127.0.0.1",
                user_agent: "test",
            },
        )
        .unwrap();
    }

    /// Two proposed facts, same authority + topic, different
    /// statements (a conflict pair). Returns `(slug, fact_a, fact_b)`.
    fn seed_conflicting_facts(store_root: &std::path::Path) -> (String, i64, i64) {
        let store = Store::new(store_root);
        let conn = db::open(&store).unwrap();
        let authority = db::add_authority(
            &conn,
            &db::NewAuthority {
                kind: "state",
                country: "USA",
                region: "WA",
                name: "WA",
                slug: "usa-wa-sos",
                actor: "t",
                at: 1,
            },
        )
        .unwrap();
        conn.execute(
            "INSERT INTO capture(authority_id, captured_at, kind, path,
                                  bytes_raw, bytes_stored, sha256, trust)
             VALUES (?1, 1, 'file', 'p', 0, 0, 's', 'staff_verified')",
            [authority],
        )
        .unwrap();
        let cap = conn.last_insert_rowid();
        let mut ids = Vec::new();
        for statement in ["open 9 to 5", "open 8 to 6"] {
            conn.execute(
                "INSERT INTO fact(authority_id, capture_id, topic, statement,
                                   status, created_by, created_at)
                 VALUES (?1, ?2, 'hours', ?3, 'proposed', 't', 1)",
                rusqlite::params![authority, cap, statement],
            )
            .unwrap();
            ids.push(conn.last_insert_rowid());
        }
        ("usa-wa-sos".to_string(), ids[0], ids[1])
    }

    /// Admin request builder: host guard + session + CSRF pieces.
    fn admin_request(
        method: &str,
        uri: &str,
        host: Option<&str>,
        session: Option<&str>,
        csrf: Option<&str>,
        origin: Option<&str>,
        body: Option<serde_json::Value>,
    ) -> Request<Body> {
        let mut builder = Request::builder().uri(uri).method(method);
        if let Some(host) = host {
            builder = builder.header("host", host);
        }
        let mut cookies = Vec::new();
        if let Some(session) = session {
            cookies.push(format!("{}={session}", auth::ADM_COOKIE));
        }
        if let Some(csrf) = csrf {
            cookies.push(format!("{}={csrf}", auth::CSRF_COOKIE));
            builder = builder.header(auth::CSRF_HEADER, csrf);
        }
        if !cookies.is_empty() {
            builder = builder.header("cookie", cookies.join("; "));
        }
        if let Some(origin) = origin {
            builder = builder.header("origin", origin);
        }
        let body = body.map_or_else(Body::empty, |v| Body::from(v.to_string()));
        if method == "POST" || method == "PUT" {
            builder = builder.header("content-type", "application/json");
        }
        builder.body(body).unwrap()
    }

    async fn json_body(res: axum::response::Response) -> serde_json::Value {
        let body = axum::body::to_bytes(res.into_body(), 65536).await.unwrap();
        serde_json::from_slice(&body).unwrap()
    }

    fn admin_router_for(state: &AppState) -> Router {
        admin::router(state.clone())
    }

    fn public_router_for(state: &AppState) -> Router {
        public_router(state.clone(), "static", "test-spa", "test-acme")
    }

    #[tokio::test]
    async fn approval_flow_accept_then_reject_with_audit() {
        let fx = fixture();
        let (slug, fact_a, fact_b) = seed_conflicting_facts(&fx.state.store_root);
        make_session(&fx.state.store_root, "staff-1", auth::Role::Staff);

        // List: both proposed, conflicting with each other.
        let res = admin_router_for(&fx.state)
            .oneshot(admin_request(
                "GET",
                "/apostille/api/v1/admin/facts",
                Some("admin.andrii.ai"),
                Some("staff-1"),
                None,
                None,
                None,
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let listed = json_body(res).await;
        assert_eq!(listed["count"], 2);
        assert_eq!(listed["facts"][0]["conflict"], true);
        assert_eq!(listed["facts"][0]["conflicts"][0]["id"], fact_b);

        // Accept A: banner policy — conflicts do not block.
        let res = admin_router_for(&fx.state)
            .oneshot(admin_request(
                "POST",
                &format!("/apostille/api/v1/admin/facts/{fact_a}/accept"),
                Some("admin.andrii.ai"),
                Some("staff-1"),
                Some("csrf-token"),
                Some("http://localhost:3001"),
                Some(serde_json::json!({"reason": "verified by phone"})),
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let accepted = json_body(res).await;
        assert_eq!(accepted["status"], "accepted");
        assert_eq!(accepted["conflict"], true);

        // Second accept is a 409 (no longer proposed).
        let res = admin_router_for(&fx.state)
            .oneshot(admin_request(
                "POST",
                &format!("/apostille/api/v1/admin/facts/{fact_a}/accept"),
                Some("admin.andrii.ai"),
                Some("staff-1"),
                Some("csrf-token"),
                Some("http://localhost:3001"),
                Some(serde_json::json!({})),
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::CONFLICT);

        // Reject demands a reason, then lands.
        let res = admin_router_for(&fx.state)
            .oneshot(admin_request(
                "POST",
                &format!("/apostille/api/v1/admin/facts/{fact_b}/reject"),
                Some("admin.andrii.ai"),
                Some("staff-1"),
                Some("csrf-token"),
                Some("http://localhost:3001"),
                Some(serde_json::json!({})),
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let res = admin_router_for(&fx.state)
            .oneshot(admin_request(
                "POST",
                &format!("/apostille/api/v1/admin/facts/{fact_b}/reject"),
                Some("admin.andrii.ai"),
                Some("staff-1"),
                Some("csrf-token"),
                Some("http://localhost:3001"),
                Some(serde_json::json!({"reason": "superseded by A"})),
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(json_body(res).await["status"], "rejected");

        // Audit captured both decisions with the actor.
        let store = Store::new(&fx.state.store_root);
        let conn = db::open(&store).unwrap();
        let actions: Vec<(String, String, i64)> = conn
            .prepare("SELECT action, actor, entity_id FROM audit WHERE entity = 'fact' ORDER BY id")
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        assert_eq!(
            actions,
            vec![
                ("fact_accept".to_string(), "staff-1".to_string(), fact_a),
                ("fact_reject".to_string(), "staff-1".to_string(), fact_b),
            ]
        );

        // Public read: accepted only (B vanished), anonymous OK.
        let res = public_router_for(&fx.state)
            .oneshot(
                Request::builder()
                    .uri(format!("/apostille/api/v1/authorities/{slug}/facts"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let public = json_body(res).await;
        assert_eq!(public["facts"].as_array().unwrap().len(), 1);
        assert_eq!(public["facts"][0]["id"], fact_a);
        // B is rejected, so no live conflict remains on the public banner.
        assert_eq!(public["facts"][0]["conflict"], false);
    }

    #[tokio::test]
    async fn admin_auth_matrix() {
        let fx = fixture();
        make_session(&fx.state.store_root, "staff-1", auth::Role::Staff);
        make_session(&fx.state.store_root, "cust-1", auth::Role::Customer);

        // No session → 401.
        let res = admin_router_for(&fx.state)
            .oneshot(admin_request(
                "GET",
                "/apostille/api/v1/admin/me",
                Some("admin.andrii.ai"),
                None,
                None,
                None,
                None,
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);

        // Customer session → 403.
        let res = admin_router_for(&fx.state)
            .oneshot(admin_request(
                "GET",
                "/apostille/api/v1/admin/me",
                Some("admin.andrii.ai"),
                Some("cust-1"),
                None,
                None,
                None,
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::FORBIDDEN);

        // Wrong / missing host → 403 even for staff.
        for host in [Some("andrii.ai"), Some("evil.test"), None] {
            let res = admin_router_for(&fx.state)
                .oneshot(admin_request(
                    "GET",
                    "/apostille/api/v1/admin/me",
                    host,
                    Some("staff-1"),
                    None,
                    None,
                    None,
                ))
                .await
                .unwrap();
            assert_eq!(res.status(), StatusCode::FORBIDDEN, "host: {host:?}");
        }

        // Staff + admin host → 200 with the role.
        let res = admin_router_for(&fx.state)
            .oneshot(admin_request(
                "GET",
                "/apostille/api/v1/admin/me",
                Some("admin.andrii.ai"),
                Some("staff-1"),
                None,
                None,
                None,
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(json_body(res).await["role"], "staff");

        // Revoked dev cert → 403 + audit.
        let store = Store::new(&fx.state.store_root);
        let conn = db::open(&store).unwrap();
        auth::issue_admin_cert(&conn, "fp-revoked", "alice", 100, 200, "").unwrap();
        auth::revoke_admin_cert(&conn, "fp-revoked", 200).unwrap();
        drop(conn);
        let req = admin_request(
            "GET",
            "/apostille/api/v1/admin/me",
            Some("admin.andrii.ai"),
            Some("staff-1"),
            None,
            None,
            None,
        );
        let (mut parts, body) = req.into_parts();
        parts.headers.insert(
            "x-admin-cert-sha256",
            HeaderValue::from_static("fp-revoked"),
        );
        let res = admin_router_for(&fx.state)
            .oneshot(Request::from_parts(parts, body))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::FORBIDDEN);

        // Strict mode without a cert → 403 (fail closed).
        let mut strict = fx.state.clone();
        strict.admin_mtls_off = false;
        let res = admin::router(strict)
            .oneshot(admin_request(
                "GET",
                "/apostille/api/v1/admin/me",
                Some("admin.andrii.ai"),
                Some("staff-1"),
                None,
                None,
                None,
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn strict_mode_cert_seam_accepts_verified_extension() {
        let fx = fixture();
        make_session(&fx.state.store_root, "staff-1", auth::Role::Staff);
        let store = Store::new(&fx.state.store_root);
        let conn = db::open(&store).unwrap();
        auth::issue_admin_cert(&conn, "fp-live", "alice", 100, 200, "").unwrap();
        drop(conn);
        let mut strict = fx.state.clone();
        strict.admin_mtls_off = false;

        // Test TLS terminator: inserts the verified-cert extension.
        async fn inject(mut req: Request<Body>, next: axum::middleware::Next) -> impl IntoResponse {
            req.extensions_mut()
                .insert(admin::VerifiedClientCert("fp-live".to_string()));
            next.run(req).await
        }
        let app = Router::new()
            .fallback_service(admin::router(strict))
            .layer(axum::middleware::from_fn(inject));
        let res = app
            .oneshot(admin_request(
                "GET",
                "/apostille/api/v1/admin/me",
                Some("admin.andrii.ai"),
                Some("staff-1"),
                None,
                None,
                None,
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(json_body(res).await["cert_fingerprint"], "fp-live");
    }

    #[tokio::test]
    async fn csrf_matrix() {
        let fx = fixture();
        make_session(&fx.state.store_root, "staff-1", auth::Role::Staff);
        let uri = "/apostille/api/v1/admin/bus/publish";
        let body = Some(serde_json::json!({"topic": "call:1", "kind": "note", "body": {}}));

        // Mutation without Origin → 403.
        let res = admin_router_for(&fx.state)
            .oneshot(admin_request(
                "POST",
                uri,
                Some("admin.andrii.ai"),
                Some("staff-1"),
                Some("csrf-token"),
                None,
                body.clone(),
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::FORBIDDEN);

        // Evil Origin → 403.
        let res = admin_router_for(&fx.state)
            .oneshot(admin_request(
                "POST",
                uri,
                Some("admin.andrii.ai"),
                Some("staff-1"),
                Some("csrf-token"),
                Some("https://evil.test"),
                body.clone(),
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::FORBIDDEN);

        // Origin without the double-submit pair → 403.
        let res = admin_router_for(&fx.state)
            .oneshot(admin_request(
                "POST",
                uri,
                Some("admin.andrii.ai"),
                Some("staff-1"),
                None,
                Some("https://admin.andrii.ai"),
                body.clone(),
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::FORBIDDEN);

        // Valid CSRF but no session → 401 (CSRF passed, auth failed).
        let res = admin_router_for(&fx.state)
            .oneshot(admin_request(
                "POST",
                uri,
                Some("admin.andrii.ai"),
                None,
                Some("csrf-token"),
                Some("https://admin.andrii.ai"),
                body.clone(),
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);

        // Full set → through to the handler.
        let res = admin_router_for(&fx.state)
            .oneshot(admin_request(
                "POST",
                uri,
                Some("admin.andrii.ai"),
                Some("staff-1"),
                Some("csrf-token"),
                Some("https://admin.andrii.ai"),
                body,
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);

        // Authority channels carry fact_accepted only.
        let res = admin_router_for(&fx.state)
            .oneshot(admin_request(
                "POST",
                uri,
                Some("admin.andrii.ai"),
                Some("staff-1"),
                Some("csrf-token"),
                Some("https://admin.andrii.ai"),
                Some(serde_json::json!({"topic": "authority:x", "kind": "draft", "body": {}})),
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[tokio::test]
    async fn admin_config_roundtrip_keeps_secrets_write_only() {
        let fx = fixture();
        make_session(&fx.state.store_root, "staff-1", auth::Role::Staff);
        let store = Store::new(&fx.state.store_root);
        let conn = db::open(&store).unwrap();
        config::record_kek(&conn, 1, "test-cred", 100).unwrap();
        drop(conn);

        // PUT secret: set-new-only, never echoed.
        let res = admin_router_for(&fx.state)
            .oneshot(admin_request(
                "PUT",
                "/apostille/api/v1/admin/config/mailer.key",
                Some("admin.andrii.ai"),
                Some("staff-1"),
                Some("csrf-token"),
                Some("https://admin.andrii.ai"),
                Some(serde_json::json!({"type": "secret", "value": "s3cr3t-bytes"})),
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let set = json_body(res).await;
        assert_eq!(set["set"], true);
        assert!(set.get("value").is_none());

        // PUT bool + text.
        for (key, key_type, value) in [
            ("mailer.enabled", "bool", serde_json::json!(true)),
            (
                "mailer.host",
                "text",
                serde_json::json!("mail.example.test"),
            ),
        ] {
            let res = admin_router_for(&fx.state)
                .oneshot(admin_request(
                    "PUT",
                    &format!("/apostille/api/v1/admin/config/{key}"),
                    Some("admin.andrii.ai"),
                    Some("staff-1"),
                    Some("csrf-token"),
                    Some("https://admin.andrii.ai"),
                    Some(serde_json::json!({"type": key_type, "value": value})),
                ))
                .await
                .unwrap();
            assert_eq!(res.status(), StatusCode::OK, "key: {key}");
        }

        // GET: bool/text decrypt, secret is presence-only.
        let res = admin_router_for(&fx.state)
            .oneshot(admin_request(
                "GET",
                "/apostille/api/v1/admin/config",
                Some("admin.andrii.ai"),
                Some("staff-1"),
                None,
                None,
                None,
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let listed = json_body(res).await;
        let raw = listed.to_string();
        assert!(!raw.contains("s3cr3t-bytes"), "secret bytes leaked: {raw}");
        let by_key = |key: &str| {
            listed["keys"]
                .as_array()
                .unwrap()
                .iter()
                .find(|k| k["key"] == key)
                .cloned()
                .unwrap()
        };
        assert_eq!(by_key("mailer.key")["set"], true);
        assert!(by_key("mailer.key").get("value").is_none());
        assert_eq!(by_key("mailer.enabled")["value"], true);
        assert_eq!(by_key("mailer.host")["value"], "mail.example.test");

        // Unknown type → 422.
        let res = admin_router_for(&fx.state)
            .oneshot(admin_request(
                "PUT",
                "/apostille/api/v1/admin/config/x",
                Some("admin.andrii.ai"),
                Some("staff-1"),
                Some("csrf-token"),
                Some("https://admin.andrii.ai"),
                Some(serde_json::json!({"type": "yaml", "value": "x"})),
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);

        // No KEK in state → 503 (the API degrades, the server stands).
        let mut no_kek = fx.state.clone();
        no_kek.kek = None;
        let res = admin::router(no_kek)
            .oneshot(admin_request(
                "GET",
                "/apostille/api/v1/admin/config",
                Some("admin.andrii.ai"),
                Some("staff-1"),
                None,
                None,
                None,
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn admin_spa_shell_and_json_api_404() {
        let fx = fixture();
        make_session(&fx.state.store_root, "staff-1", auth::Role::Staff);
        let spa = &fx.state.admin_spa_dir;
        std::fs::create_dir_all(spa).unwrap();
        std::fs::write(spa.join("index.html"), "<html>admin shell</html>").unwrap();

        // Non-API path serves the shell (no session needed for bytes).
        let res = admin_router_for(&fx.state)
            .oneshot(admin_request(
                "GET",
                "/admin/config",
                Some("admin.andrii.ai"),
                None,
                None,
                None,
                None,
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = axum::body::to_bytes(res.into_body(), 65536).await.unwrap();
        assert!(String::from_utf8_lossy(&body).contains("admin shell"));

        // Unknown API path stays a JSON 404, never the shell.
        let res = admin_router_for(&fx.state)
            .oneshot(admin_request(
                "GET",
                "/apostille/api/v1/admin/nope",
                Some("admin.andrii.ai"),
                Some("staff-1"),
                None,
                None,
                None,
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::NOT_FOUND);
        assert_eq!(json_body(res).await["error"], "not found");

        // Real API routes are still guarded, not shadowed by static.
        let res = admin_router_for(&fx.state)
            .oneshot(admin_request(
                "GET",
                "/apostille/api/v1/admin/config",
                Some("admin.andrii.ai"),
                None,
                None,
                None,
                None,
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn admin_media_upload_roundtrip() {
        let fx = fixture();
        let (slug, _, _) = seed_conflicting_facts(&fx.state.store_root);
        make_session(&fx.state.store_root, "staff-1", auth::Role::Staff);

        fn multipart(slug: &str, grant: bool) -> (String, Vec<u8>) {
            let boundary = "TESTBOUNDARY";
            let mut body = Vec::new();
            let mut part = |name: &str, extra: &str, value: &[u8]| {
                body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
                body.extend_from_slice(
                    format!("Content-Disposition: form-data; name=\"{name}\"{extra}\r\n")
                        .as_bytes(),
                );
                body.extend_from_slice(b"\r\n");
                body.extend_from_slice(value);
                body.extend_from_slice(b"\r\n");
            };
            part("authority", "", slug.as_bytes());
            if grant {
                part("grant_version", "", b"andrii-co-media-grant-v1");
                part("grant_accepted_at", "", b"1700000000");
            }
            part("synthetic", "", b"true");
            body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
            body.extend_from_slice(
                b"Content-Disposition: form-data; name=\"file\"; filename=\"photo.jpg\"\r\n",
            );
            body.extend_from_slice(b"Content-Type: image/jpeg\r\n\r\n");
            body.extend_from_slice(b"fake-jpeg-bytes");
            body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
            (format!("multipart/form-data; boundary={boundary}"), body)
        }

        // With grant: quarantined capture + audit.
        let (content_type, body) = multipart(&slug, true);
        let req = Request::builder()
            .uri("/apostille/api/v1/admin/media/upload")
            .method("POST")
            .header("host", "admin.andrii.ai")
            .header(
                "cookie",
                format!("{}=staff-1; {}=t", auth::ADM_COOKIE, auth::CSRF_COOKIE),
            )
            .header(auth::CSRF_HEADER, "t")
            .header("origin", "https://admin.andrii.ai")
            .header("content-type", content_type)
            .body(Body::from(body))
            .unwrap();
        let res = admin_router_for(&fx.state).oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let uploaded = axum::body::to_bytes(res.into_body(), 65536).await.unwrap();
        let uploaded: serde_json::Value = serde_json::from_slice(&uploaded).unwrap();
        assert_eq!(uploaded["quarantined"], true);
        let capture_id = uploaded["capture_id"].as_i64().unwrap();
        let store = Store::new(&fx.state.store_root);
        let conn = db::open(&store).unwrap();
        let (kind, trust, notes): (String, String, String) = conn
            .query_row(
                "SELECT kind, trust, notes FROM capture WHERE id = ?1",
                [capture_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(
            (kind.as_str(), trust.as_str(), notes.as_str()),
            ("photo", "community", "quarantine:intake")
        );

        // Without grant: 422, nothing stored.
        let (content_type, body) = multipart(&slug, false);
        let req = Request::builder()
            .uri("/apostille/api/v1/admin/media/upload")
            .method("POST")
            .header("host", "admin.andrii.ai")
            .header(
                "cookie",
                format!("{}=staff-1; {}=t", auth::ADM_COOKIE, auth::CSRF_COOKIE),
            )
            .header(auth::CSRF_HEADER, "t")
            .header("origin", "https://admin.andrii.ai")
            .header("content-type", content_type)
            .body(Body::from(body))
            .unwrap();
        let res = admin_router_for(&fx.state).oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[tokio::test]
    async fn p2_stub_robots_and_no_admin_on_public() {
        let fx = fixture();
        // P2 mount is live (S6 landed): /status answers 200.
        let res = public_router_for(&fx.state)
            .oneshot(
                Request::builder()
                    .uri("/apostille/api/v1/p2/status")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(json_body(res).await["flow"], "p2-forms");

        // Domain-root robots.txt.
        let res = public_router_for(&fx.state)
            .oneshot(
                Request::builder()
                    .uri("/robots.txt")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = axum::body::to_bytes(res.into_body(), 4096).await.unwrap();
        assert!(body.starts_with(b"User-agent:"));

        // Admin paths are JSON 404 on the public listener, never mounted.
        let res = public_router_for(&fx.state)
            .oneshot(
                Request::builder()
                    .uri("/apostille/api/v1/admin/me")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::NOT_FOUND);
        let body = axum::body::to_bytes(res.into_body(), 1024).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["error"], "not found");
    }

    #[tokio::test]
    async fn ws_authz_live() {
        use futures_util::StreamExt as _;

        let fx = fixture();
        make_session(&fx.state.store_root, "staff-1", auth::Role::Staff);
        make_session(&fx.state.store_root, "cust-1", auth::Role::Customer);

        // Real server on an ephemeral port (never a fixed one).
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(
            axum_server::from_tcp(listener).serve(public_router_for(&fx.state).into_make_service()),
        );

        async fn next_text(
            ws: &mut tokio_tungstenite::WebSocketStream<
                tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
            >,
        ) -> String {
            tokio::time::timeout(std::time::Duration::from_secs(5), ws.next())
                .await
                .expect("ws message within 5s")
                .expect("stream open")
                .expect("message ok")
                .into_text()
                .expect("text frame")
                .to_string()
        }

        async fn ws_connect(
            addr: &std::net::SocketAddr,
            topic: &str,
            cookie: Option<String>,
        ) -> Result<
            (
                tokio_tungstenite::WebSocketStream<
                    tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
                >,
                tokio_tungstenite::tungstenite::handshake::client::Response,
            ),
            tokio_tungstenite::tungstenite::Error,
        > {
            let uri: axum::http::Uri = format!("ws://{addr}/apostille/ws/bus?topic={topic}")
                .parse()
                .unwrap();
            let mut builder = tokio_tungstenite::tungstenite::ClientRequestBuilder::new(uri);
            if let Some(cookie) = cookie {
                builder = builder.with_header("cookie", cookie);
            }
            tokio_tungstenite::connect_async(builder).await
        }

        // Anonymous authority subscribe: hello + live fan-out.
        let (mut anon, _) = ws_connect(&addr, "authority:usa-wa-sos", None)
            .await
            .unwrap();
        let hello: serde_json::Value = serde_json::from_str(&next_text(&mut anon).await).unwrap();
        assert_eq!(hello["type"], "subscribed");
        assert_eq!(hello["topic"], "authority:usa-wa-sos");
        fx.state.bus.publish(
            "authority:usa-wa-sos",
            r#"{"type":"fact_accepted","fact_id":1}"#.to_string(),
        );
        let event = next_text(&mut anon).await;
        assert!(event.contains("fact_accepted"), "got: {event}");
        anon.close(None).await.unwrap();

        // Anonymous call subscribe: 403, no upgrade.
        let denied = ws_connect(&addr, "call:1", None).await;
        match denied {
            Err(tokio_tungstenite::tungstenite::Error::Http(res)) => {
                assert_eq!(res.status(), 403);
            }
            other => panic!("expected HTTP 403, got {other:?}"),
        }

        // Customer submission subscribe: 403 (fail-closed ownership seam).
        let denied = ws_connect(
            &addr,
            "submission:7",
            Some(format!("{}=cust-1", auth::PUB_COOKIE)),
        )
        .await;
        match denied {
            Err(tokio_tungstenite::tungstenite::Error::Http(res)) => {
                assert_eq!(res.status(), 403);
            }
            other => panic!("expected HTTP 403, got {other:?}"),
        }

        // Staff call subscribe: hello.
        let (mut staff, _) = ws_connect(
            &addr,
            "call:1",
            Some(format!("{}=staff-1", auth::ADM_COOKIE)),
        )
        .await
        .unwrap();
        let hello: serde_json::Value = serde_json::from_str(&next_text(&mut staff).await).unwrap();
        assert_eq!(hello["topic"], "call:1");
        staff.close(None).await.unwrap();

        server.abort();
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
        let acme = std::env::var("ACME_CHALLENGE_DIR").unwrap_or_else(|_| "test-acme".to_string());
        let res = app(&dir, &spa, blocker, &acme)
            .oneshot(login_request("demo-1"))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let body = axum::body::to_bytes(res.into_body(), 1024).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["error"], "store unavailable");
    }
}
