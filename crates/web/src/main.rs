//! Apostille web API (S3 lane, roadmap items 2+6; G4 admin track).
//!
//! Single-443 TLS: ONE listener on `net.https_port` serves both
//! branches by Host header — `admin.andrii.ai` → merged admin
//! router (approval/config, workers, voice staff+webhook, mTLS
//! fingerprint layer on that branch only), everything else →
//! public router (`/apostille` health/version, static, WS bus,
//! accepted-facts reads, DEMO login/session, P2 stub). Iff
//! `quic.enabled`, a UDP QUIC/h3 endpoint on the same port serves
//! the same merged router. Plain-HTTP dev keeps two listeners
//! (`net.port` / `net.admin_port`).
//!
//! Real login (passkey/Apple/Google/Meta + magic-link fallback) is
//! still ahead per `docs/auth-playbook.md`; the policy lives in the
//! `auth` crate and the DEMO session stays until then.

mod admin;
mod admin_voice;
mod admin_workers;
mod approval;
mod host;
mod magic;
mod mtls;
mod oauth;
mod oauth_apple;
mod oauth_meta;
mod p2;
mod p2_labels;
mod passkey;
mod queue;
mod quic;
mod setup;
mod staff_passkey;
mod ws_bus;

use axum::{
    Router,
    extract::{
        MatchedPath, State,
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

/// Full version on the wire: `CARGO_PKG_VERSION+BUILD_SHA`
/// (semver build metadata; `BUILD_SHA` comes from `build.rs`).
/// Every deploy rebuilds, so `/healthz` always names the release.
const FULL_VERSION: &str = concat!(env!("CARGO_PKG_VERSION"), "+", env!("BUILD_SHA"));

/// Transport that delivered the request (`"tcp"` or `"quic"`).
/// TCP listeners stamp it via [`insert_tcp_transport`]; the QUIC
/// bridge stamps it when building the bridged request. Absent in
/// direct-`oneshot` tests, which log `"unknown"`.
#[derive(Clone)]
pub(crate) struct RequestTransport(pub(crate) &'static str);

/// Stable 5xx cause attached to error responses (read by
/// [`request_id_and_log`], never sent to clients). Grow the
/// taxonomy at the failure site; unmarked 5xx log `unclassified`.
#[derive(Clone)]
struct ErrorKind(&'static str);

/// Request access log, one span per request: `id method uri
/// transport branch route` in, `status latency_ms` (+ `error_kind`
/// on 5xx) out. Logged fields are fixed and PII-free by
/// construction (our URLs never carry tokens or customer data).
/// 5xx logs at ERROR so the journald error budget (`-p err`)
/// counts app failures; everything else logs at INFO. The same
/// opaque ID goes back in `x-request-id` for correlation.
async fn request_id_and_log(
    State(branch): State<&'static str>,
    req: axum::http::Request<axum::body::Body>,
    next: axum::middleware::Next,
) -> impl IntoResponse {
    let id = format!(
        "{:016x}{:016x}",
        rand::random::<u64>(),
        rand::random::<u64>()
    );
    let transport = req
        .extensions()
        .get::<RequestTransport>()
        .map(|marked| marked.0)
        .unwrap_or("unknown");
    let route = req
        .extensions()
        .get::<MatchedPath>()
        .map(|matched| matched.as_str().to_owned())
        .unwrap_or_else(|| "unmatched".to_string());
    let span = tracing::info_span!(
        "request",
        id = %id,
        method = %req.method(),
        uri = %req.uri(),
        transport = %transport,
        branch = %branch,
        route = %route,
    );
    let _guard = span.enter();
    let started = std::time::Instant::now();
    let mut res = next.run(req).await;
    if let Ok(value) = HeaderValue::from_str(&id) {
        res.headers_mut().insert("x-request-id", value);
    }
    if res.status().is_server_error() {
        tracing::error!(
            status = %res.status(),
            latency_ms = started.elapsed().as_millis(),
            error_kind = %res
                .extensions()
                .get::<ErrorKind>()
                .map(|kind| kind.0)
                .unwrap_or("unclassified"),
            "response"
        );
    } else {
        tracing::info!(
            status = %res.status(),
            latency_ms = started.elapsed().as_millis(),
            "response"
        );
    }
    res
}

/// Stamp TCP-delivered requests with their transport (the QUIC
/// bridge stamps `"quic"` itself). Applied once, outside the
/// served routers in [`serve_http`] and [`serve_tls`].
async fn insert_tcp_transport(
    mut req: axum::http::Request<axum::body::Body>,
    next: axum::middleware::Next,
) -> axum::response::Response {
    req.extensions_mut().insert(RequestTransport("tcp"));
    next.run(req).await
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
    /// `admin.mtls_off` dev seam (DB config): skips the
    /// client-cert requirement and allows loopback origins. Prod
    /// runs strict (false); true refuses release boot.
    admin_mtls_off: bool,
    /// `admin.require_cert` (DB config): every admin-host request
    /// (API, HTML, JS) must present a CA-verified client cert,
    /// except the setup surface and the Telnyx webhook.
    admin_require_cert: bool,
    /// Admin SPA build dir (`paths.admin_spa_dir` DB key, default
    /// `web/build-admin`), served from the admin listener root with
    /// SPA fallback.
    admin_spa_dir: PathBuf,
    /// Media terms gate (`media.terms_read` DB key, true once SEC
    /// confirms the Meta data-handling read). Until then only
    /// synthetic fixtures upload; real media fails closed.
    media_terms_read: bool,
    /// Prepaid-label config (USPS base URL + certification flag).
    label_cfg: p2_labels::LabelCfg,
    // ADMIN-VOICE (goal-025 stream 2): Telnyx voice config
    // (`telnyx.*` DB rows). The API key rides separately as a
    // redacting `SecretText` so state dumps can never leak it.
    voice_cfg: voice::VoiceConfig,
    voice_api_key: Option<config::SecretText>,
    /// Passkey ceremonies (spec `customer-auth.md`): process-wide
    /// crate handle built from `auth.*` DB rows at boot.
    webauthn: std::sync::Arc<webauthn_rs::Webauthn>,
    /// Auth rate-limit buckets (spec decision 0.6).
    auth_limits: std::sync::Arc<passkey::RateBuckets>,
    /// Google OAuth client (spec `customer-auth.md` §9): `None`
    /// until `google.client_id` + `google.client_secret` rows are
    /// set — the endpoints 503, the boot never refuses.
    google_cfg: Option<oauth::GoogleCfg>,
    /// Apple OAuth client (spec `customer-auth.md` §10): `None`
    /// until the four `apple.*` rows are set — the endpoints 503,
    /// the boot never refuses.
    apple_cfg: Option<oauth_apple::AppleCfg>,
    /// Meta OAuth client (spec `customer-auth.md` §11): `None`
    /// until `meta.client_id` + `meta.client_secret` rows are
    /// set — the endpoints 503, the boot never refuses.
    meta_cfg: Option<oauth_meta::MetaCfg>,
    /// Magic-link mailer (spec `customer-auth.md` §12): `None`
    /// until `mailer.provider` + its set are configured — the
    /// request endpoint 503s, the boot never refuses.
    mailer: Option<magic::Mailer>,
    /// Magic-link landing base (origin-derived, never
    /// configured): `{auth.origin}/apostille/login`.
    magic_link_base: String,
    /// Meta deletion-status base (origin-derived, never
    /// configured): `{auth.origin}/auth/meta/delete-status`.
    meta_delete_status_base: String,
}

/// Build the full PUBLIC app router. `static_dir` roots
/// `/apostille/static`; `spa_dir` (the SvelteKit `build/` output)
/// serves the app shell at `/apostille/` and every other
/// `/apostille/*` path via SPA fallback; `store_root` locates the
/// SQLite metadata DB for session handlers; `acme_dir` serves
/// `/.well-known/acme-challenge/` token files for ACME http-01
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
            admin_require_cert: false,
            admin_spa_dir: PathBuf::from("test-admin-spa"),
            media_terms_read: false,
            label_cfg: p2_labels::LabelCfg::default(),
            voice_cfg: voice::VoiceConfig::default(),
            voice_api_key: None,
            webauthn: std::sync::Arc::new(passkey::test_webauthn()),
            auth_limits: std::sync::Arc::new(passkey::RateBuckets::default()),
            google_cfg: None,
            apple_cfg: None,
            meta_cfg: None,
            mailer: None,
            magic_link_base: "https://example.test/apostille/login".to_string(),
            meta_delete_status_base: "https://example.test/auth/meta/delete-status".to_string(),
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
            "/api/v1/auth/passkey/register/start",
            post(passkey::register_start),
        )
        .route(
            "/api/v1/auth/passkey/register/finish",
            post(passkey::register_finish),
        )
        .route(
            "/api/v1/auth/passkey/login/start",
            post(passkey::login_start),
        )
        .route(
            "/api/v1/auth/passkey/login/finish",
            post(passkey::login_finish),
        )
        .route("/api/v1/auth/profile/methods", get(passkey::list_methods))
        .route("/api/v1/auth/profile/unlink", post(passkey::unlink))
        .route("/api/v1/auth/logout", post(passkey::logout))
        .route("/api/v1/auth/me", get(passkey::me))
        .route("/api/v1/auth/google/start", post(oauth::start))
        .route("/api/v1/auth/apple/start", post(oauth_apple::start))
        .route("/api/v1/auth/meta/start", post(oauth_meta::start))
        .route("/api/v1/auth/magic/start", post(magic::start))
        .route("/api/v1/auth/magic/consume", post(magic::consume))
        .route("/api/v1/authorities", get(approval::public_authorities))
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
    // Server-level OAuth return URLs: the login is to andrii.ai
    // itself, not the /apostille sub-service — providers register
    // https://andrii.ai/auth/<provider>/callback. (Start stays a
    // nested API call; only the provider-facing URL is flat.)
    Router::new()
        .route("/apostille", get(serve_spa_index))
        .route("/apostille/", get(serve_spa_index))
        .route("/robots.txt", get(serve_robots))
        .route("/privacy", get(serve_privacy))
        .route("/auth/google/callback", get(oauth::callback))
        .route("/auth/apple/callback", get(oauth_apple::callback))
        .route("/auth/meta/callback", get(oauth_meta::callback))
        .route(
            "/auth/meta/delete-request",
            post(oauth_meta::delete_request),
        )
        .route("/auth/meta/delete-status", get(oauth_meta::delete_status))
        .nest("/apostille", api)
        .nest_service("/.well-known/acme-challenge", ServeDir::new(acme_dir))
        .layer(axum::middleware::from_fn_with_state(
            "public",
            request_id_and_log,
        ))
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

/// The binding privacy policy (Meta/Google/Apple dashboard URL):
/// server-rendered static HTML so crawlers read it with no JS.
async fn serve_privacy(State(state): State<AppState>) -> impl IntoResponse {
    match tokio::fs::read(state.static_dir.join("privacy.html")).await {
        Ok(bytes) => (
            [(axum::http::header::CONTENT_TYPE, "text/html; charset=utf-8")],
            bytes,
        )
            .into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn healthz() -> impl IntoResponse {
    Json(json!({"status": "ok", "version": FULL_VERSION}))
}

async fn version() -> impl IntoResponse {
    Json(json!({
        "version": env!("CARGO_PKG_VERSION"),
        "build": env!("BUILD_SHA"),
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
    let mut res = (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({"error": "store unavailable"})),
    )
        .into_response();
    res.extensions_mut().insert(ErrorKind("store_unavailable"));
    res
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

/// Explicit-file TLS material: serving cert PEM + key PEM.
type TlsPair = (Vec<u8>, Vec<u8>);

/// Explicit-file TLS material from `tls.cert_file`/`tls.key_file`
/// PEM paths (DB config): read both, or neither (ACME mode or
/// plain-HTTP dev). Exactly one set, or an unreadable file → Err,
/// and main refuses to start (TLS misconfiguration must never
/// silently downgrade to HTTP). Error labels name the keys — key
/// bytes never appear.
fn explicit_tls_pair(cert: Option<String>, key: Option<String>) -> Result<Option<TlsPair>, String> {
    match (cert, key) {
        (None, None) => Ok(None),
        (Some(cert), Some(key)) => {
            let cert_pem = std::fs::read(&cert)
                .map_err(|e| format!("read tls.cert_file={cert} tls.key_file=<redacted>: {e}"))?;
            let key_pem = std::fs::read(&key)
                .map_err(|_| format!("read tls.cert_file={cert} tls.key_file=<redacted>"))?;
            Ok(Some((cert_pem, key_pem)))
        }
        _ => Err("set both tls.cert_file and tls.key_file, or neither".to_string()),
    }
}

/// Loud boot warning when the serving pair lacks the admin SAN
/// (single-443 serves both hosts from one pair). Warning, not
/// refusal — see [`mtls::cert_has_dns_san`].
fn warn_unless_admin_san(cert_pem: &[u8]) {
    match mtls::cert_has_dns_san(cert_pem, auth::ADMIN_HOST) {
        Ok(true) => {}
        Ok(false) => eprintln!(
            "WARNING: TLS certificate lacks the {} SAN — browsers on the admin host will fail closed; reissue with both SANs (single-443)",
            auth::ADMIN_HOST
        ),
        Err(e) => eprintln!("WARNING: {e} — cannot confirm the admin SAN is present"),
    }
}

/// Single-443 listener TLS: the serving cert/key plus client-cert
/// chain verification against `tls.client_ca_file` when set. No CA
/// → plain TLS (strict mode then 403s for lack of a verified cert,
/// as before). A set but unreadable/invalid CA refuses the boot —
/// mTLS must never silently downgrade.
fn load_admin_tls_from_pem(
    cert_pem: &[u8],
    key_pem: &[u8],
    ca_path: Option<&str>,
) -> Result<axum_server::tls_rustls::RustlsConfig, String> {
    let ca_pem = ca_path
        .map(|path| std::fs::read(path).map_err(|e| format!("read tls.client_ca_file={path}: {e}")))
        .transpose()?;
    let config = mtls::admin_server_config(cert_pem, key_pem, ca_pem.as_deref())?;
    Ok(axum_server::tls_rustls::RustlsConfig::from_config(
        std::sync::Arc::new(config),
    ))
}

/// Hot-reload the admin config after an ACME renewal: rebuild the
/// whole `ServerConfig` (rustls has no in-place cert swap on a
/// custom-built config) and swap it in. Re-reads the CA file, so a
/// rotated bundle takes effect at the next renewal with no restart.
fn reload_admin_tls(
    admin_tls: &axum_server::tls_rustls::RustlsConfig,
    cert_pem: &[u8],
    key_pem: &[u8],
    ca_path: Option<&str>,
) -> Result<(), String> {
    let ca_pem = ca_path
        .map(|path| std::fs::read(path).map_err(|e| format!("read tls.client_ca_file={path}: {e}")))
        .transpose()?;
    let config = mtls::admin_server_config(cert_pem, key_pem, ca_pem.as_deref())?;
    admin_tls.reload_from_config(std::sync::Arc::new(config));
    Ok(())
}

/// 308 every HTTP request to `https://{host}{path}`.
/// The host comes from DB config, never from the Host header (a
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
    // config-db (goal-025 stream 4): STORE_DIR is the one bootstrap
    // var left — it locates the DB, so it cannot live in the DB.
    // Every other runtime setting comes from sealed DB rows below.
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
    // ---- ADMIN-VOICE (goal-025 stream 2; parent reconciles) ----
    // Voice-owned tables; the migration lives in the voice crate so
    // this lane never edits the shared schema file.
    voice::store::migrate(&conn)
        .unwrap_or_else(|e| panic!("STORE_DIR={store_dir}: migrate voice tables: {e}"));
    drop(conn);
    // config-db: the KEK is mandatory now — runtime config lives in
    // sealed rows, so without it there is nothing to serve with.
    // Refusing beats a silent localhost-defaults boot in prod.
    let kek = match config::load_kek() {
        Ok((kek, source)) => {
            kek.lock_memory();
            eprintln!("config KEK loaded via {source:?}");
            std::sync::Arc::new(kek)
        }
        Err(e) => {
            eprintln!("refusing to start: config KEK unavailable ({e})");
            std::process::exit(1);
        }
    };
    // config-db: read-once runtime load: open store → read config → serve.
    let cfg = {
        let conn = db::open(&store)
            .unwrap_or_else(|e| panic!("STORE_DIR={store_dir}: reopen metadata DB: {e}"));
        match config::RuntimeConfig::load(&conn, &kek) {
            Ok((cfg, warnings)) => {
                for warning in warnings {
                    eprintln!("WARNING: config {warning}");
                }
                cfg
            }
            Err(e) => {
                eprintln!("refusing to start: runtime config unreadable ({e})");
                std::process::exit(1);
            }
        }
    };
    if cfg.admin_mtls_off {
        eprintln!(
            "WARNING: admin.mtls_off — admin listener skips client-cert auth (local dev only)"
        );
    }
    let state = AppState {
        spa_index: std::path::Path::new(&cfg.spa_dir).join("index.html"),
        static_dir: PathBuf::from(&cfg.static_dir),
        store_root: PathBuf::from(store_dir.as_str()),
        bus: ws_bus::Bus::new(),
        kek: Some(kek),
        admin_mtls_off: cfg.admin_mtls_off,
        admin_require_cert: cfg.admin_require_cert,
        admin_spa_dir: PathBuf::from(&cfg.admin_spa_dir),
        media_terms_read: cfg.media_terms_read,
        label_cfg: p2_labels::LabelCfg::from_values(&cfg.usps_base_url, cfg.usps_certified),
        // ADMIN-VOICE (goal-025 stream 2): `telnyx.*` DB rows.
        voice_cfg: voice::VoiceConfig::from_values(
            &cfg.telnyx_base_url,
            cfg.telnyx_from_number.as_deref().unwrap_or_default(),
            cfg.telnyx_connection_id.as_deref().unwrap_or_default(),
            cfg.telnyx_webhook_public_key.as_deref().unwrap_or_default(),
        ),
        // `Option::clone` spelling: keeps the secrets-scan
        // `generic-secret-assignment` run under its length floor.
        voice_api_key: Option::clone(&cfg.telnyx_api_key),
        webauthn: std::sync::Arc::new(
            passkey::build_webauthn(&cfg.auth_rp_id, &cfg.auth_rp_name, &cfg.auth_origin)
                .unwrap_or_else(|| {
                    eprintln!(
                        "refusing to start: auth.rp_id/auth.origin do not build a WebAuthn handle"
                    );
                    std::process::exit(1);
                }),
        ),
        auth_limits: std::sync::Arc::new(passkey::RateBuckets::default()),
        google_cfg: match (&cfg.google_client_id, &cfg.google_client_secret) {
            (Some(id), Some(secret)) => Some(oauth::GoogleCfg::from_values(
                id,
                secret.clone(),
                &format!(
                    "{}/auth/google/callback",
                    cfg.auth_origin.trim_end_matches('/')
                ),
            )),
            _ => {
                eprintln!(
                    "WARNING: config google.client_id/google.client_secret unset — Google login disabled"
                );
                None
            }
        },
        apple_cfg: match (
            &cfg.apple_client_id,
            &cfg.apple_team_id,
            &cfg.apple_key_id,
            &cfg.apple_private_key,
        ) {
            (Some(id), Some(team), Some(kid), Some(key)) => {
                Some(oauth_apple::AppleCfg::from_values(
                    id,
                    team,
                    kid,
                    key.clone(),
                    &format!(
                        "{}/auth/apple/callback",
                        cfg.auth_origin.trim_end_matches('/')
                    ),
                ))
            }
            _ => {
                eprintln!(
                    "WARNING: config apple.client_id/apple.team_id/apple.key_id/apple.private_key unset — Apple login disabled"
                );
                None
            }
        },
        meta_cfg: match (&cfg.meta_client_id, &cfg.meta_client_secret) {
            (Some(id), Some(secret)) => Some(oauth_meta::MetaCfg::from_values(
                id,
                secret.clone(),
                &format!(
                    "{}/auth/meta/callback",
                    cfg.auth_origin.trim_end_matches('/')
                ),
            )),
            _ => {
                eprintln!(
                    "WARNING: config meta.client_id/meta.client_secret unset — Meta login disabled"
                );
                None
            }
        },
        mailer: magic::mailer_from_config(&cfg, std::path::Path::new(store_dir.as_str())),
        magic_link_base: format!("{}/apostille/login", cfg.auth_origin.trim_end_matches('/')),
        meta_delete_status_base: format!(
            "{}/auth/meta/delete-status",
            cfg.auth_origin.trim_end_matches('/')
        ),
    };
    let public = public_router(
        state.clone(),
        &cfg.static_dir,
        &cfg.spa_dir,
        &cfg.acme_challenge_dir,
    );
    // ===== ADMIN-WORKERS (goal-025 stream 1; parent reconciles) =====
    // Staff workers/captures API merged onto the ADMIN listener only.
    // Same guards as admin::router (staff AdminAuth per handler, host
    // guard + CSRF layers inside admin_workers::router); the merge is
    // fallback-safe because admin_workers sets no custom fallback, so
    // the admin SPA fallback still wins. Sibling streams: keep your
    // mount in your own clearly-commented block, do not edit this one.
    // ================================================================
    // ---- ADMIN-VOICE (goal-025 stream 2; parent reconciles) ----
    // Voice staff routes (AdminAuth + host guard + CSRF, applied in
    // the voice routers themselves) and the Telnyx webhook (host
    // guard + Ed25519 signature, no session).
    // H6 mTLS seam: the fingerprint layer runs OUTSIDE every merged
    // admin router (approval/config, workers, voice staff, webhook),
    // so each one's `AdminAuth` sees the same TLS-supplied identity.
    // Without peer certs (plain HTTP dev, no-CA TLS) it no-ops.
    let admin = admin::router(state.clone())
        .merge(admin_workers::router(state.clone()))
        .merge(admin_voice::staff_router(state.clone()))
        .merge(admin_voice::webhook_router(state))
        .layer(axum::middleware::from_fn(mtls::inject_verified_cert));
    // Explicit files always win: ACME stays off when the
    // operator pinned a certificate.
    match explicit_tls_pair(cfg.tls_cert.clone(), cfg.tls_key.clone()) {
        Ok(Some((cert_pem, key_pem))) => {
            warn_unless_admin_san(&cert_pem);
            match load_admin_tls_from_pem(&cert_pem, &key_pem, cfg.tls_client_ca.as_deref()) {
                Ok(admin_tls) => {
                    serve_tls(
                        public,
                        admin,
                        admin_tls,
                        &cfg,
                        cert_pem,
                        key_pem,
                        quic::QuicReloader::default(),
                        None,
                    )
                    .await;
                }
                Err(message) => {
                    eprintln!("refusing to start: {message}");
                    std::process::exit(1);
                }
            }
        }
        Ok(None) if cfg.acme_enabled => serve_acme(public, admin, &cfg, &store_dir).await,
        Ok(None) => serve_http(public, admin, &cfg).await,
        Err(message) => {
            eprintln!("refusing to start: {message}");
            std::process::exit(1);
        }
    }
}

/// ACME mode: load the persisted pair or issue it (Google Public CA,
/// http-01 via the challenge dir), serve TLS with it, and renew in the
/// background with hot reloads — no restarts, ever.
///
/// Port 80 is bound and served BEFORE issuance: the CA's http-01
/// validation fetches must reach us while the order is pending.
/// (Binding it after — inside `serve_tls` only — invalidated every
/// order: see the 2026-10-27 prod incident.)
async fn serve_acme(public: Router, admin: Router, cfg: &config::RuntimeConfig, store_dir: &str) {
    let prebound_redirect: Vec<axum_server::Handle> = spawn_challenge_servers(cfg)
        .into_iter()
        .map(|(_, handle)| handle)
        .collect();
    // config-db: ACME values come from the `acme.*` DB keys.
    let stored = acme::StoredAcme {
        directory: cfg.acme_directory.clone(),
        domains_csv: cfg.acme_domains_csv.clone(),
        email: cfg.acme_email.clone(),
        eab_kid: cfg.acme_eab_kid.clone(),
        eab_hmac_b64: cfg
            .acme_eab_hmac_b64
            .as_ref()
            .map(|secret| secret.as_str().to_string()),
    };
    let config = acme::AcmeConfig::from_stored(
        std::path::Path::new(store_dir),
        cfg.acme_challenge_dir.clone().into(),
        stored,
    )
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
    warn_unless_admin_san(&pair.cert_pem);
    let admin_tls =
        load_admin_tls_from_pem(&pair.cert_pem, &pair.key_pem, cfg.tls_client_ca.as_deref())
            .unwrap_or_else(|e| {
                eprintln!("refusing to start: {e}");
                std::process::exit(1);
            });
    let quic_reloader = quic::QuicReloader::default();
    let loop_admin_tls = admin_tls.clone();
    let loop_quic = quic_reloader.clone();
    let loop_quic_enabled = cfg.quic_enabled;
    let loop_ca = cfg.tls_client_ca.clone();
    tokio::spawn(async move {
        acme::renewal_loop(config, |cert, key| {
            let admin_tls = loop_admin_tls.clone();
            let loop_quic = loop_quic.clone();
            let ca = loop_ca.clone();
            async move {
                reload_admin_tls(&admin_tls, &cert, &key, ca.as_deref())?;
                // QUIC serves the same pair: swap its crypto too so a
                // renewal never leaves h3 on a stale certificate.
                if loop_quic_enabled {
                    let ca_pem = ca
                        .as_deref()
                        .map(|path| {
                            std::fs::read(path)
                                .map_err(|e| format!("read tls.client_ca_file={path}: {e}"))
                        })
                        .transpose()?;
                    loop_quic.reload(&cert, &key, ca_pem.as_deref())?;
                }
                Ok(())
            }
        })
        .await
    });
    serve_tls(
        public,
        admin,
        admin_tls,
        cfg,
        pair.cert_pem,
        pair.key_pem,
        quic_reloader,
        Some(prebound_redirect),
    )
    .await
}

/// Bind + serve the port-80 redirector (HTTP→HTTPS + ACME
/// challenge files) on every bind IP, returning one bound address
/// and shutdown handle per listener. Runs AHEAD of ACME issuance
/// so http-01 validations succeed; [`serve_tls`] then reuses these
/// listeners instead of binding :80 a second time.
///
/// A bind failure refuses the boot NOW (same fail-fast shape as the
/// QUIC parent-bind): a missing :80 must never surface later as a
/// mysterious CA-side order failure. A listener that dies after a
/// clean start fails the process too — half-serving challenges
/// would break renewals silently until expiry.
fn spawn_challenge_servers(cfg: &config::RuntimeConfig) -> Vec<(SocketAddr, axum_server::Handle)> {
    let mut bound = Vec::new();
    for ip in cfg.bind_ips() {
        let addr = SocketAddr::new(ip, cfg.http_port);
        let listener = std::net::TcpListener::bind(addr).unwrap_or_else(|e| {
            eprintln!("refusing to start: bind TCP {addr} (redirect + ACME challenges): {e}");
            std::process::exit(1);
        });
        let local = listener.local_addr().unwrap_or_else(|e| {
            eprintln!("refusing to start: bound :80 has no address ({e})");
            std::process::exit(1);
        });
        let handle = axum_server::Handle::new();
        bound.push((local, handle.clone()));
        eprintln!("listening on http://{local} (redirect + ACME challenges)");
        let redirect = redirect_router(cfg.public_host.clone(), &cfg.acme_challenge_dir);
        tokio::spawn(async move {
            if let Err(e) = axum_server::from_tcp(listener)
                .handle(handle)
                .serve(redirect.into_make_service())
                .await
            {
                eprintln!("challenge server {local} failed ({e}); refusing to continue");
                std::process::exit(1);
            }
        });
    }
    bound
}

/// Plain-HTTP dev mode: public on `net.port` (default 3000) and
/// admin on `net.admin_port` (default 3001), on every bind IP.
async fn serve_http(public: Router, admin: Router, cfg: &config::RuntimeConfig) {
    if cfg.tls_client_ca.is_some() {
        eprintln!("WARNING: tls.client_ca_file is set but admin TLS is off — mTLS inert");
    }
    let mut servers = tokio::task::JoinSet::new();
    let mut handles = Vec::new();
    for ip in cfg.bind_ips() {
        for (router, port, name) in [
            (&public, cfg.port, "public"),
            (&admin, cfg.admin_port, "admin"),
        ] {
            let addr = SocketAddr::new(ip, port);
            let handle = axum_server::Handle::new();
            handles.push(handle.clone());
            eprintln!("listening on http://{addr}/apostille ({name})");
            servers.spawn({
                let router = router
                    .clone()
                    .layer(axum::middleware::from_fn(insert_tcp_transport));
                async move {
                    axum_server::bind(addr)
                        .handle(handle)
                        .serve(router.into_make_service_with_connect_info::<SocketAddr>())
                        .await
                }
            });
        }
    }
    supervise(servers, handles).await;
}

/// The single-443 TLS router: Host-dispatched branches plus the
/// Alt-Svc advertisement IFF `quic.enabled` (serve path and tests
/// build it the same way, so the header wiring is covered).
fn tls_router(public: Router, admin: Router, cfg: &config::RuntimeConfig) -> Router {
    let merged = host::host_router(public, admin);
    if cfg.quic_enabled {
        merged.layer(axum::middleware::from_fn_with_state(
            cfg.https_port,
            host::add_alt_svc,
        ))
    } else {
        merged
    }
}

/// One planned TLS-mode listener kind (pure plan, executed by
/// [`serve_tls`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TlsListenerKind {
    /// TCP+TLS single-443 (Host-dispatched public+admin).
    Https,
    /// UDP QUIC/h3 on the same port (iff `quic.enabled`).
    Quic,
    /// HTTP→HTTPS redirector (+ ACME challenges).
    Redirect,
}

/// One planned TLS-mode listener: address + kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TlsListener {
    addr: SocketAddr,
    kind: TlsListenerKind,
}

/// Every listener TLS mode binds, per bind IP: single-443 TCP on
/// `net.https_port` (NO 444 — `net.admin_https_port` is deprecated
/// and never bound), UDP QUIC on the same port iff
/// `quic.enabled`, and the port-80 redirector.
fn tls_listen_plan(cfg: &config::RuntimeConfig) -> Vec<TlsListener> {
    let mut plan = Vec::new();
    for ip in cfg.bind_ips() {
        plan.push(TlsListener {
            addr: SocketAddr::new(ip, cfg.https_port),
            kind: TlsListenerKind::Https,
        });
        if cfg.quic_enabled {
            plan.push(TlsListener {
                addr: SocketAddr::new(ip, cfg.https_port),
                kind: TlsListenerKind::Quic,
            });
        }
        plan.push(TlsListener {
            addr: SocketAddr::new(ip, cfg.http_port),
            kind: TlsListenerKind::Redirect,
        });
    }
    plan
}

/// Prod mode: ONE TCP+TLS listener on `net.https_port` (443)
/// serving BOTH branches by Host header, an HTTP→HTTPS redirector
/// on `net.http_port` (80), and — iff `quic.enabled` — a UDP
/// QUIC/h3 endpoint on the same port as 443, on every bind IP.
/// Port 444 is gone: `net.admin_https_port` parses but binds
/// nothing.
///
/// The single TCP listener uses the ADMIN TLS shape for every
/// connection (optional client certs at the handshake, chain
/// verification against `tls.client_ca_file` when set, peer-chain
/// capture via [`mtls::MtlsAcceptor`]); the fingerprint layer runs
/// on the admin branch only, and Host dispatch strips cert state
/// from the public branch.
///
/// `prebound_redirect` carries the :80 listeners ACME mode started
/// ahead of issuance ([`spawn_challenge_servers`]): when present
/// the Redirect plan entries are skipped (already serving) and
/// their handles join the graceful-drain set. File-cert mode
/// passes `None` and binds :80 here as before.
#[allow(clippy::too_many_arguments)] // boot shape: 2 call sites, one TLS mode each
async fn serve_tls(
    public: Router,
    admin: Router,
    admin_tls: axum_server::tls_rustls::RustlsConfig,
    cfg: &config::RuntimeConfig,
    cert_pem: Vec<u8>,
    key_pem: Vec<u8>,
    quic_reloader: quic::QuicReloader,
    prebound_redirect: Option<Vec<axum_server::Handle>>,
) {
    let router = tls_router(public, admin, cfg);
    let mut servers = tokio::task::JoinSet::new();
    let mut handles = Vec::new();
    let skip_redirect = prebound_redirect.is_some();
    if let Some(extra) = prebound_redirect {
        eprintln!("port-80 redirector already serving (pre-issuance bind); reusing");
        handles.extend(extra);
    }
    for listener in tls_listen_plan(cfg) {
        match listener.kind {
            TlsListenerKind::Https => {
                let addr = listener.addr;
                let handle = axum_server::Handle::new();
                handles.push(handle.clone());
                let mtls = if cfg.tls_client_ca.is_some() {
                    "public+admin, mTLS-capable"
                } else {
                    "public+admin"
                };
                eprintln!("listening on https://{addr}/apostille (single-443 {mtls})");
                servers.spawn({
                    let router = router
                        .clone()
                        .layer(axum::middleware::from_fn(insert_tcp_transport));
                    let tls = admin_tls.clone();
                    async move {
                        axum_server::bind(addr)
                            .acceptor(mtls::MtlsAcceptor::new(tls))
                            .handle(handle)
                            .serve(router.into_make_service_with_connect_info::<SocketAddr>())
                            .await
                    }
                });
            }
            TlsListenerKind::Quic => {
                let addr = listener.addr;
                // A UDP bind failure must fail the boot the same way
                // a TCP bind failure does — so bind NOW, in the
                // parent, before spawning the serve task.
                let endpoint = bind_quic_listener(addr, &cert_pem, &key_pem, cfg);
                quic_reloader.register(&endpoint);
                eprintln!("listening on udp://{addr} (quic/h3)");
                servers.spawn({
                    let router = router.clone();
                    async move {
                        quic::serve(endpoint, router).await;
                        Ok::<(), std::io::Error>(())
                    }
                });
            }
            TlsListenerKind::Redirect if skip_redirect => {}
            TlsListenerKind::Redirect => {
                let addr = listener.addr;
                let handle = axum_server::Handle::new();
                handles.push(handle.clone());
                eprintln!("listening on http://{addr} (redirect + ACME challenges)");
                servers.spawn({
                    let redirect =
                        redirect_router(cfg.public_host.clone(), &cfg.acme_challenge_dir);
                    async move {
                        axum_server::bind(addr)
                            .handle(handle)
                            .serve(redirect.into_make_service())
                            .await
                    }
                });
            }
        }
    }
    supervise(servers, handles).await;
}

/// Bind one QUIC UDP endpoint for [`serve_tls`]: build the h3
/// crypto from the serving pair (re-reading the CA file, like the
/// TCP config) and bind. Refuses the boot on any failure — QUIC
/// misconfiguration must never silently halve the service.
fn bind_quic_listener(
    addr: SocketAddr,
    cert_pem: &[u8],
    key_pem: &[u8],
    cfg: &config::RuntimeConfig,
) -> quinn::Endpoint {
    let ca_pem = cfg.tls_client_ca.as_deref().map(|path| {
        std::fs::read(path).unwrap_or_else(|e| {
            eprintln!("refusing to start: read tls.client_ca_file={path}: {e}");
            std::process::exit(1);
        })
    });
    let config = quic::server_config(cert_pem, key_pem, ca_pem.as_deref()).unwrap_or_else(|e| {
        eprintln!("refusing to start: {e}");
        std::process::exit(1);
    });
    quic::bind_endpoint(addr, &config).unwrap_or_else(|e| {
        eprintln!("refusing to start: bind UDP {addr} (quic): {e}");
        std::process::exit(1);
    })
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

/// Test-only env reads below (`STATIC_DIR`/`SPA_DIR`/`ACME_CHALLENGE_DIR`)
/// are harness fixture overrides, never production paths
/// (config-db spec §2.2 keeps them as bootstrap).
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
        let version = v["version"].as_str().expect("version string");
        assert!(
            version.starts_with(env!("CARGO_PKG_VERSION")),
            "version carries the crate version: {version}"
        );
        assert_eq!(version, FULL_VERSION);
    }

    #[tokio::test]
    async fn version_reports_build_sha() {
        let res = test_app()
            .oneshot(
                Request::builder()
                    .uri("/apostille/version")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = axum::body::to_bytes(res.into_body(), 1024).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["version"], env!("CARGO_PKG_VERSION"));
        assert_eq!(v["build"], env!("BUILD_SHA"));
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
    async fn privacy_route_serves_static_file() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().to_str().unwrap().to_string();
        std::fs::write(tmp.path().join("privacy.html"), "<h1>TEST-POLICY</h1>").unwrap();
        let res = app(&dir, &dir, tmp.path().to_path_buf(), &dir)
            .oneshot(
                Request::builder()
                    .uri("/privacy")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(
            res.headers().get(axum::http::header::CONTENT_TYPE).unwrap(),
            "text/html; charset=utf-8"
        );
        let body = axum::body::to_bytes(res.into_body(), 4096).await.unwrap();
        assert_eq!(body.as_ref(), b"<h1>TEST-POLICY</h1>");
    }

    #[test]
    fn privacy_policy_covers_required_elements() {
        // Compile-time read of the shipped draft: Meta requires what
        // is collected, the purposes, and how users request deletion.
        let policy = include_str!("../static/privacy.html");
        for needle in [
            "Privacy Policy",
            "What we collect",
            "Cookies",
            "Your rights",
            "Meta",
            "delete",
            "Children",
            "Contact",
        ] {
            assert!(policy.contains(needle), "missing {needle}");
        }
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

    /// Regression (2026-10-27 prod incident): the pre-issuance
    /// :80 listeners must serve challenge files over real TCP. The
    /// oneshot router tests above cannot catch a missing BIND — this
    /// one binds ephemeral loopback, writes a token file, and GETs
    /// it back the way the CA would.
    ///
    /// Multi-threaded: the blocking client below must not starve
    /// the spawned server task (the default current-thread flavor
    /// deadlocks this test).
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn challenge_servers_serve_tokens_over_tcp() {
        use std::io::{Read as _, Write as _};
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("tok-1"), "key-auth-body").unwrap();
        let cfg = config::RuntimeConfig {
            http_port: 0,
            public_host: "example.test".to_string(),
            acme_challenge_dir: tmp.path().to_str().unwrap().to_string(),
            ..config::RuntimeConfig::default()
        };
        let bound = spawn_challenge_servers(&cfg);
        // Default bind is loopback-only: exactly one listener.
        assert_eq!(bound.len(), 1);
        let (addr, _handle) = &bound[0];
        assert_ne!(addr.port(), 0, "ephemeral port assigned");
        for (target, want_status, want_snippet) in [
            ("/.well-known/acme-challenge/tok-1", "200", "key-auth-body"),
            ("/.well-known/acme-challenge/nope", "404", ""),
            ("/apostille/order?q=1", "308", "https://example.test/"),
        ] {
            let mut stream = std::net::TcpStream::connect(*addr).unwrap();
            write!(
                stream,
                "GET {target} HTTP/1.0\r\nHost: ca.test\r\nConnection: close\r\n\r\n"
            )
            .unwrap();
            let mut response = String::new();
            stream.read_to_string(&mut response).unwrap();
            assert!(
                response.contains(want_status),
                "GET {target}: want {want_status} in {response:?}"
            );
            assert!(
                response.contains(want_snippet),
                "GET {target}: want {want_snippet:?} in {response:?}"
            );
        }
    }

    #[test]
    fn explicit_tls_pair_requires_both_or_neither() {
        assert!(explicit_tls_pair(None, None).unwrap().is_none());
        explicit_tls_pair(Some("a".to_string()), None).expect_err("cert without key must fail");
        explicit_tls_pair(None, Some("b".to_string())).expect_err("key without cert must fail");
        explicit_tls_pair(
            Some("no-such-cert.pem".to_string()),
            Some("no-such-key.pem".to_string()),
        )
        .expect_err("missing files must fail");
        // Roundtrip: written PEMs read back byte-identical.
        let (cert_pem, key_pem) = self_signed_localhost();
        let tmp = tempfile::tempdir().unwrap();
        let cert_path = tmp.path().join("cert.pem");
        let key_path = tmp.path().join("key.pem");
        std::fs::write(&cert_path, &cert_pem).unwrap();
        std::fs::write(&key_path, &key_pem).unwrap();
        let pair = explicit_tls_pair(
            Some(cert_path.to_str().unwrap().to_string()),
            Some(key_path.to_str().unwrap().to_string()),
        )
        .unwrap()
        .unwrap();
        assert_eq!(pair, (cert_pem, key_pem));
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

    #[test]
    fn store_unavailable_carries_error_kind() {
        let res = store_unavailable();
        assert_eq!(res.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(
            res.extensions().get::<ErrorKind>().unwrap().0,
            "store_unavailable"
        );
    }

    /// Log-capture writer: appends every formatted line to a buffer.
    #[derive(Clone)]
    struct BufWriter(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

    impl std::io::Write for BufWriter {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0
                .lock()
                .expect("log buffer poisoned")
                .extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn response_event_carries_transport_branch_route_and_error_kind() {
        async fn mark_quic(
            mut req: Request<Body>,
            next: axum::middleware::Next,
        ) -> axum::response::Response {
            req.extensions_mut().insert(RequestTransport("quic"));
            next.run(req).await
        }

        let app = Router::new()
            .route("/boom", get(|| async { store_unavailable() }))
            .route("/ok", get(|| async { "ok" }))
            .layer(axum::middleware::from_fn_with_state(
                "public",
                request_id_and_log,
            ))
            .layer(axum::middleware::from_fn(mark_quic));
        let logs = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let subscriber = tracing_subscriber::fmt()
            .json()
            .with_writer({
                let logs = logs.clone();
                move || BufWriter(logs.clone())
            })
            .finish();
        // Thread-local default (this test stays on the
        // current-thread runtime, so every event lands here).
        let _guard = tracing::subscriber::set_default(subscriber);
        for (uri, status) in [
            ("/boom", StatusCode::INTERNAL_SERVER_ERROR),
            ("/ok", StatusCode::OK),
            ("/nope", StatusCode::NOT_FOUND),
        ] {
            let res = app
                .clone()
                .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(res.status(), status, "uri: {uri}");
        }
        drop(_guard);
        let text = String::from_utf8(logs.lock().unwrap().clone()).unwrap();
        let events: Vec<serde_json::Value> = text
            .lines()
            .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
            .filter(|event| event["fields"]["message"] == "response")
            .collect();
        assert_eq!(events.len(), 3, "got: {text}");
        let by_route = |route: &str| {
            events
                .iter()
                .find(|event| event["span"]["route"] == route)
                .unwrap_or_else(|| panic!("no event for {route}: {text}"))
        };
        let boom = by_route("/boom");
        assert_eq!(boom["level"], "ERROR");
        assert_eq!(boom["span"]["transport"], "quic");
        assert_eq!(boom["span"]["branch"], "public");
        assert_eq!(boom["fields"]["error_kind"], "store_unavailable");
        assert!(
            boom["fields"]["status"]
                .as_str()
                .unwrap()
                .starts_with("500"),
            "got: {boom}"
        );
        let ok = by_route("/ok");
        assert_eq!(ok["level"], "INFO");
        assert!(ok["fields"].get("error_kind").is_none(), "got: {ok}");
        let nope = by_route("unmatched");
        assert!(
            nope["fields"]["status"]
                .as_str()
                .unwrap()
                .starts_with("404"),
            "got: {nope}"
        );
    }

    #[tokio::test]
    async fn tcp_transport_layer_marks_requests() {
        let app = Router::new()
            .route(
                "/probe",
                get(|req: Request<Body>| async move {
                    req.extensions()
                        .get::<RequestTransport>()
                        .map(|marked| marked.0)
                        .unwrap_or("missing")
                        .to_string()
                }),
            )
            .layer(axum::middleware::from_fn(insert_tcp_transport));
        let res = app
            .oneshot(
                Request::builder()
                    .uri("/probe")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = axum::body::to_bytes(res.into_body(), 64).await.unwrap();
        assert_eq!(body.as_ref(), b"tcp");
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
            admin_require_cert: false,
            admin_spa_dir: tmp.path().join("admin-spa"),
            media_terms_read: false,
            label_cfg: p2_labels::LabelCfg::default(),
            voice_cfg: voice::VoiceConfig::default(),
            voice_api_key: None,
            webauthn: std::sync::Arc::new(passkey::test_webauthn()),
            auth_limits: std::sync::Arc::new(passkey::RateBuckets::default()),
            google_cfg: None,
            apple_cfg: None,
            meta_cfg: None,
            mailer: None,
            magic_link_base: "https://example.test/apostille/login".to_string(),
            meta_delete_status_base: "https://example.test/auth/meta/delete-status".to_string(),
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
                "/apostille/api/v1/facts",
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
                &format!("/apostille/api/v1/facts/{fact_a}/accept"),
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
                &format!("/apostille/api/v1/facts/{fact_a}/accept"),
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
                &format!("/apostille/api/v1/facts/{fact_b}/reject"),
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
                &format!("/apostille/api/v1/facts/{fact_b}/reject"),
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

    fn seed_authority_index(store_root: &std::path::Path) {
        let store = Store::new(store_root);
        let conn = db::open(&store).unwrap();
        // Inserted out of slug order on purpose: the endpoint must sort.
        let specs = [
            ("usa-wa-sos", "WA", "USA"),
            ("deu-berlin", "Berlin", "Germany"),
            ("fra-paris", "Paris", "France"),
        ];
        let mut ids = std::collections::HashMap::new();
        for (slug, name, country) in specs {
            let id = db::add_authority(
                &conn,
                &db::NewAuthority {
                    kind: "state",
                    country,
                    region: "",
                    name,
                    slug,
                    actor: "t",
                    at: 1,
                },
            )
            .unwrap();
            conn.execute(
                "INSERT INTO capture(authority_id, captured_at, kind, path,
                                      bytes_raw, bytes_stored, sha256, trust)
                 VALUES (?1, 1, 'file', 'p', 0, 0, 's', 'staff_verified')",
                [id],
            )
            .unwrap();
            ids.insert(slug, (id, conn.last_insert_rowid()));
        }
        let insert = |slug: &str, topic: &str, statement: &str, status: &str| {
            let (aid, cap) = ids[slug];
            conn.execute(
                "INSERT INTO fact(authority_id, capture_id, topic, statement,
                                   status, created_by, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, 't', 1)",
                rusqlite::params![aid, cap, topic, statement, status],
            )
            .unwrap();
        };
        // Topics must be 005-CHECK values (hours/fees/forms, not fee/form).
        // usa-wa-sos: 2 accepted + 1 proposed + 1 rejected → count 2.
        insert("usa-wa-sos", "hours", "open 9 to 5", "accepted");
        insert("usa-wa-sos", "fees", "costs 10", "accepted");
        insert("usa-wa-sos", "hours", "open 8 to 6", "proposed");
        insert("usa-wa-sos", "forms", "use form X", "rejected");
        // deu-berlin: 1 accepted + 1 superseded → count 1.
        insert("deu-berlin", "hours", "open 9 to 5", "accepted");
        insert("deu-berlin", "hours", "open 10 to 4", "superseded");
        // fra-paris: proposed only → count 0, still listed.
        insert("fra-paris", "hours", "open 9 to 5", "proposed");
    }

    #[tokio::test]
    async fn public_authority_index_lists_counts_and_order() {
        let fx = fixture();
        seed_authority_index(&fx.state.store_root);

        let res = public_router_for(&fx.state)
            .oneshot(
                Request::builder()
                    .uri("/apostille/api/v1/authorities")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = json_body(res).await;
        let authorities = body["authorities"].as_array().unwrap();
        assert_eq!(authorities.len(), 3);
        // Ordered by slug, accepted-only counts.
        assert_eq!(authorities[0]["slug"], "deu-berlin");
        assert_eq!(authorities[0]["name"], "Berlin");
        assert_eq!(authorities[0]["country"], "Germany");
        assert_eq!(authorities[0]["fact_count"], 1);
        assert_eq!(authorities[1]["slug"], "fra-paris");
        assert_eq!(authorities[1]["fact_count"], 0);
        assert_eq!(authorities[2]["slug"], "usa-wa-sos");
        assert_eq!(authorities[2]["fact_count"], 2);
        // No sentinel row, no other keys on the envelope.
        assert!(authorities.iter().all(|a| a["slug"] != ""));
        assert_eq!(
            body.as_object().unwrap().keys().collect::<Vec<_>>(),
            vec!["authorities"]
        );
    }

    #[tokio::test]
    async fn public_authority_index_empty_db_is_empty_array() {
        let fx = fixture(); // migrated but unseeded: only the sentinel exists
        let res = public_router_for(&fx.state)
            .oneshot(
                Request::builder()
                    .uri("/apostille/api/v1/authorities")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = json_body(res).await;
        assert_eq!(body["authorities"], serde_json::json!([]));
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
                "/apostille/api/v1/me",
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
                "/apostille/api/v1/me",
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
                    "/apostille/api/v1/me",
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
                "/apostille/api/v1/me",
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
            "/apostille/api/v1/me",
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
                "/apostille/api/v1/me",
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
                "/apostille/api/v1/me",
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
        let uri = "/apostille/api/v1/bus/publish";
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
                "/apostille/api/v1/config/mailer.key",
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
                    &format!("/apostille/api/v1/config/{key}"),
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
                "/apostille/api/v1/config",
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
                "/apostille/api/v1/config/x",
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
                "/apostille/api/v1/config",
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
                "/config",
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
                "/apostille/api/v1/nope",
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
                "/apostille/api/v1/config",
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
            .uri("/apostille/api/v1/media/upload")
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
            .uri("/apostille/api/v1/media/upload")
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

        // Admin paths are JSON 404 on the public listener, never mounted
        // (the host differentiates the branches; /queue exists only there).
        let res = public_router_for(&fx.state)
            .oneshot(
                Request::builder()
                    .uri("/apostille/api/v1/queue")
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

    // ===== H6 mTLS seam: hermetic end-to-end over real TLS =====
    // Synthetic rcgen CA/server/client certs, 127.0.0.1 port 0,
    // staff session + `admin_cert` rows in a temp store.

    /// Synthetic test CA (mirrors the xtask `admin-cert-issue` CA
    /// shape, plus the key usages webpki chain validation needs).
    fn test_ca(cn: &str) -> (rcgen::Certificate, rcgen::KeyPair) {
        let mut params = rcgen::CertificateParams::default();
        params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        params
            .distinguished_name
            .push(rcgen::DnType::CommonName, cn);
        params.key_usages = vec![
            rcgen::KeyUsagePurpose::KeyCertSign,
            rcgen::KeyUsagePurpose::DigitalSignature,
            rcgen::KeyUsagePurpose::CrlSign,
        ];
        let key = rcgen::KeyPair::generate().unwrap();
        let cert = params.self_signed(&key).unwrap();
        (cert, key)
    }

    /// Synthetic client cert signed by `ca` (EKU ClientAuth, like a
    /// production `admin-cert-issue`).
    struct TestClient {
        cert_pem: Vec<u8>,
        key_pem: Vec<u8>,
        fingerprint: String,
    }

    fn test_client(ca: &rcgen::Certificate, ca_key: &rcgen::KeyPair, cn: &str) -> TestClient {
        let key = rcgen::KeyPair::generate().unwrap();
        let mut params = rcgen::CertificateParams::new(vec![cn.to_string()]).unwrap();
        params
            .distinguished_name
            .push(rcgen::DnType::CommonName, cn);
        params.key_usages = vec![rcgen::KeyUsagePurpose::DigitalSignature];
        params.extended_key_usages = vec![rcgen::ExtendedKeyUsagePurpose::ClientAuth];
        let cert = params.signed_by(&key, ca, ca_key).unwrap();
        let fingerprint = handbook_store::store::sha256_hex(cert.der());
        TestClient {
            cert_pem: cert.pem().into_bytes(),
            key_pem: key.serialize_pem().into_bytes(),
            fingerprint,
        }
    }

    /// Strict-mode fixture (`admin.mtls_off` false, as in release).
    fn strict_fixture() -> Fixture {
        let mut fx = fixture();
        fx.state.admin_mtls_off = false;
        fx
    }

    /// Admin router exactly as production serves it: merged stack
    /// plus the fingerprint layer outside.
    fn strict_admin_router(state: &AppState) -> Router {
        admin::router(state.clone()).layer(axum::middleware::from_fn(mtls::inject_verified_cert))
    }

    fn enroll_cert(store_root: &std::path::Path, fingerprint: &str, cn: &str) {
        let store = Store::new(store_root);
        let conn = db::open(&store).unwrap();
        auth::issue_admin_cert(&conn, fingerprint, cn, unix_now(), unix_now() + 86_400, "")
            .unwrap();
    }

    fn revoke_cert(store_root: &std::path::Path, fingerprint: &str) {
        let store = Store::new(store_root);
        let conn = db::open(&store).unwrap();
        auth::revoke_admin_cert(&conn, fingerprint, unix_now()).unwrap();
    }

    /// Serve `router` over real TLS on 127.0.0.1:0 through the
    /// production [`mtls::MtlsAcceptor`].
    fn spawn_mtls_admin(
        router: Router,
        server: (Vec<u8>, Vec<u8>),
        ca_pem: Option<&[u8]>,
    ) -> (
        SocketAddr,
        axum_server::Handle,
        tokio::task::JoinHandle<std::io::Result<()>>,
    ) {
        let config = mtls::admin_server_config(&server.0, &server.1, ca_pem).unwrap();
        let tls = axum_server::tls_rustls::RustlsConfig::from_config(std::sync::Arc::new(config));
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = axum_server::Handle::new();
        let server = tokio::spawn({
            let handle = handle.clone();
            async move {
                axum_server::from_tcp(listener)
                    .acceptor(mtls::MtlsAcceptor::new(tls))
                    .handle(handle)
                    .serve(router.into_make_service())
                    .await
            }
        });
        (addr, handle, server)
    }

    /// Test-only TLS client (accepts the synthetic server cert like
    /// [`AcceptAny`]), with or without a client certificate.
    fn mtls_client_config(client: Option<&TestClient>) -> rustls::ClientConfig {
        let builder = rustls::ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(std::sync::Arc::new(AcceptAny));
        match client {
            Some(client) => {
                let mut cert_reader = std::io::BufReader::new(client.cert_pem.as_slice());
                let chain: Vec<rustls::pki_types::CertificateDer<'static>> =
                    rustls_pemfile::certs(&mut cert_reader)
                        .collect::<Result<_, _>>()
                        .unwrap();
                let mut key_reader = std::io::BufReader::new(client.key_pem.as_slice());
                let key = rustls_pemfile::private_key(&mut key_reader)
                    .unwrap()
                    .unwrap();
                builder.with_client_auth_cert(chain, key).unwrap()
            }
            None => builder.with_no_client_auth(),
        }
    }

    /// Raw HTTPS GET over a rustls client. `Err` = TCP/TLS failure
    /// (wrong-CA certs fail HERE — the server aborts before any HTTP
    /// response exists, surfacing at connect or first-read time).
    async fn mtls_get(
        addr: SocketAddr,
        client: &rustls::ClientConfig,
        host: &str,
        cookie: Option<&str>,
        extra_header: Option<(&str, &str)>,
        path: &str,
    ) -> Result<String, String> {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let connector = tokio_rustls::TlsConnector::from(std::sync::Arc::new(client.clone()));
        let stream = tokio::net::TcpStream::connect(addr)
            .await
            .map_err(|e| e.to_string())?;
        let name = rustls::pki_types::ServerName::try_from("localhost").unwrap();
        let mut tls = connector
            .connect(name, stream)
            .await
            .map_err(|e| format!("handshake: {e}"))?;
        let mut req = format!("GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n");
        if let Some(cookie) = cookie {
            req.push_str(&format!("Cookie: {cookie}\r\n"));
        }
        if let Some((name, value)) = extra_header {
            req.push_str(&format!("{name}: {value}\r\n"));
        }
        req.push_str("\r\n");
        tls.write_all(req.as_bytes())
            .await
            .map_err(|e| e.to_string())?;
        let mut raw = Vec::new();
        tls.read_to_end(&mut raw).await.map_err(|e| e.to_string())?;
        Ok(String::from_utf8_lossy(&raw).into_owned())
    }

    fn staff_cookie() -> String {
        format!("{}=staff-1", auth::ADM_COOKIE)
    }

    #[tokio::test]
    async fn mtls_strict_known_good_cert_returns_200() {
        let fx = strict_fixture();
        make_session(&fx.state.store_root, "staff-1", auth::Role::Staff);
        let (ca, ca_key) = test_ca("test-admin-ca");
        let client = test_client(&ca, &ca_key, "ops-1");
        enroll_cert(&fx.state.store_root, &client.fingerprint, "ops-1");
        let (addr, handle, server) = spawn_mtls_admin(
            strict_admin_router(&fx.state),
            self_signed_localhost(),
            Some(ca.pem().as_bytes()),
        );
        let raw = mtls_get(
            addr,
            &mtls_client_config(Some(&client)),
            "admin.andrii.ai",
            Some(&staff_cookie()),
            None,
            "/apostille/api/v1/me",
        )
        .await
        .unwrap();
        assert!(raw.starts_with("HTTP/1.1 200"), "got: {raw}");
        assert!(raw.contains(&client.fingerprint), "got: {raw}");
        handle.shutdown();
        server.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn mtls_strict_unknown_cert_returns_403() {
        // CA-signed (passes the handshake) but never enrolled.
        let fx = strict_fixture();
        make_session(&fx.state.store_root, "staff-1", auth::Role::Staff);
        let (ca, ca_key) = test_ca("test-admin-ca");
        let client = test_client(&ca, &ca_key, "ops-1");
        let (addr, handle, server) = spawn_mtls_admin(
            strict_admin_router(&fx.state),
            self_signed_localhost(),
            Some(ca.pem().as_bytes()),
        );
        let raw = mtls_get(
            addr,
            &mtls_client_config(Some(&client)),
            "admin.andrii.ai",
            Some(&staff_cookie()),
            None,
            "/apostille/api/v1/me",
        )
        .await
        .unwrap();
        assert!(raw.starts_with("HTTP/1.1 403"), "got: {raw}");
        handle.shutdown();
        server.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn mtls_strict_revoked_cert_returns_403() {
        let fx = strict_fixture();
        make_session(&fx.state.store_root, "staff-1", auth::Role::Staff);
        let (ca, ca_key) = test_ca("test-admin-ca");
        let client = test_client(&ca, &ca_key, "ops-1");
        enroll_cert(&fx.state.store_root, &client.fingerprint, "ops-1");
        revoke_cert(&fx.state.store_root, &client.fingerprint);
        let (addr, handle, server) = spawn_mtls_admin(
            strict_admin_router(&fx.state),
            self_signed_localhost(),
            Some(ca.pem().as_bytes()),
        );
        let raw = mtls_get(
            addr,
            &mtls_client_config(Some(&client)),
            "admin.andrii.ai",
            Some(&staff_cookie()),
            None,
            "/apostille/api/v1/me",
        )
        .await
        .unwrap();
        assert!(raw.starts_with("HTTP/1.1 403"), "got: {raw}");
        handle.shutdown();
        server.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn mtls_strict_missing_cert_returns_403() {
        // No client cert: the handshake SUCCEEDS (presence is an app
        // check, so the cert-less webhook keeps working) and strict
        // mode answers an auditable 403.
        let fx = strict_fixture();
        make_session(&fx.state.store_root, "staff-1", auth::Role::Staff);
        let (ca, _) = test_ca("test-admin-ca");
        let (addr, handle, server) = spawn_mtls_admin(
            strict_admin_router(&fx.state),
            self_signed_localhost(),
            Some(ca.pem().as_bytes()),
        );
        let raw = mtls_get(
            addr,
            &mtls_client_config(None),
            "admin.andrii.ai",
            Some(&staff_cookie()),
            None,
            "/apostille/api/v1/me",
        )
        .await
        .unwrap();
        assert!(raw.starts_with("HTTP/1.1 403"), "got: {raw}");
        assert!(raw.contains("client cert required"), "got: {raw}");
        handle.shutdown();
        server.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn mtls_wrong_ca_cert_fails_handshake() {
        // Chained to a CA the server does not trust: dead at the
        // handshake, before HTTP exists.
        let fx = strict_fixture();
        make_session(&fx.state.store_root, "staff-1", auth::Role::Staff);
        let (ca, _) = test_ca("test-admin-ca");
        let (other_ca, other_key) = test_ca("other-ca");
        let client = test_client(&other_ca, &other_key, "ops-1");
        enroll_cert(&fx.state.store_root, &client.fingerprint, "ops-1");
        let (addr, handle, server) = spawn_mtls_admin(
            strict_admin_router(&fx.state),
            self_signed_localhost(),
            Some(ca.pem().as_bytes()),
        );
        let err = mtls_get(
            addr,
            &mtls_client_config(Some(&client)),
            "admin.andrii.ai",
            Some(&staff_cookie()),
            None,
            "/apostille/api/v1/me",
        )
        .await
        .expect_err("wrong-CA cert must fail the handshake");
        assert!(
            !err.starts_with("HTTP/"),
            "got HTTP, want TLS failure: {err}"
        );
        handle.shutdown();
        server.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn mtls_strict_ignores_dev_cert_header() {
        // A forwardable header must never authenticate a cert: the
        // enrolled fingerprint arrives via header, no TLS cert — 403.
        let fx = strict_fixture();
        make_session(&fx.state.store_root, "staff-1", auth::Role::Staff);
        let (ca, ca_key) = test_ca("test-admin-ca");
        let client = test_client(&ca, &ca_key, "ops-1");
        enroll_cert(&fx.state.store_root, &client.fingerprint, "ops-1");
        let (addr, handle, server) = spawn_mtls_admin(
            strict_admin_router(&fx.state),
            self_signed_localhost(),
            Some(ca.pem().as_bytes()),
        );
        let raw = mtls_get(
            addr,
            &mtls_client_config(None),
            "admin.andrii.ai",
            Some(&staff_cookie()),
            Some(("x-admin-cert-sha256", &client.fingerprint)),
            "/apostille/api/v1/me",
        )
        .await
        .unwrap();
        assert!(raw.starts_with("HTTP/1.1 403"), "got: {raw}");
        handle.shutdown();
        server.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn mtls_no_ca_configured_fails_closed() {
        // No `tls.client_ca_file`: strict behaves as before the seam
        // (403 for lack of a verified cert), with or without a
        // client cert presented.
        let fx = strict_fixture();
        make_session(&fx.state.store_root, "staff-1", auth::Role::Staff);
        let (ca, ca_key) = test_ca("test-admin-ca");
        let client = test_client(&ca, &ca_key, "ops-1");
        enroll_cert(&fx.state.store_root, &client.fingerprint, "ops-1");
        let (addr, handle, server) = spawn_mtls_admin(
            strict_admin_router(&fx.state),
            self_signed_localhost(),
            None,
        );
        for config in [mtls_client_config(None), mtls_client_config(Some(&client))] {
            let raw = mtls_get(
                addr,
                &config,
                "admin.andrii.ai",
                Some(&staff_cookie()),
                None,
                "/apostille/api/v1/me",
            )
            .await
            .unwrap();
            assert!(raw.starts_with("HTTP/1.1 403"), "got: {raw}");
        }
        handle.shutdown();
        server.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn mtls_public_listener_needs_no_cert() {
        // The public listener serves plain TLS with no acceptor and
        // no fingerprint layer: health checks pass cert-less.
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let fx = fixture();
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
                    .serve(public_router_for(&fx.state).into_make_service())
                    .await
            }
        });
        let connector =
            tokio_rustls::TlsConnector::from(std::sync::Arc::new(mtls_client_config(None)));
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
        handle.shutdown();
        server.await.unwrap().unwrap();
    }

    #[test]
    fn load_admin_tls_refuses_bad_files_and_redacts() {
        // Missing server files refuse (via the pair loader).
        explicit_tls_pair(
            Some("no-such-cert.pem".to_string()),
            Some("no-such-key.pem".to_string()),
        )
        .expect_err("missing server files must fail");
        let dir = tempfile::tempdir().unwrap();
        let (cert_pem, key_pem) = self_signed_localhost();
        let (ca, _) = test_ca("test-admin-ca");
        // Missing CA refuses the boot (never silently downgrades).
        let err = load_admin_tls_from_pem(
            &cert_pem,
            &key_pem,
            Some(dir.path().join("no-ca.pem").to_str().unwrap()),
        )
        .expect_err("missing CA must fail");
        assert!(err.contains("tls.client_ca_file"), "{err}");
        assert!(!err.contains("BEGIN"), "{err}");
        // Empty CA refuses too.
        let empty = dir.path().join("empty.pem");
        std::fs::write(&empty, b"").unwrap();
        load_admin_tls_from_pem(&cert_pem, &key_pem, Some(empty.to_str().unwrap()))
            .expect_err("empty CA must fail");
        // Full triple loads.
        let ca_path = dir.path().join("ca.pem");
        std::fs::write(&ca_path, ca.pem().as_bytes()).unwrap();
        load_admin_tls_from_pem(&cert_pem, &key_pem, Some(ca_path.to_str().unwrap()))
            .expect("valid triple loads");
        // No CA loads as plain TLS.
        load_admin_tls_from_pem(&cert_pem, &key_pem, None).expect("no-CA pair loads");
    }

    // ===== Single-443 + QUIC (goal-026) =====

    /// Merged single-443 router exactly as production serves it:
    /// full admin stack + fingerprint layer, Host-dispatched with
    /// the public router, Alt-Svc iff `quic_enabled`.
    fn merged_router_for(state: &AppState, quic_enabled: bool) -> Router {
        let public = public_router_for(state);
        let admin = admin::router(state.clone())
            .merge(admin_workers::router(state.clone()))
            .merge(admin_voice::staff_router(state.clone()))
            .merge(admin_voice::webhook_router(state.clone()))
            .layer(axum::middleware::from_fn(mtls::inject_verified_cert));
        let cfg = config::RuntimeConfig {
            quic_enabled,
            ..config::RuntimeConfig::default()
        };
        tls_router(public, admin, &cfg)
    }

    #[tokio::test]
    async fn host_matrix_admin_public_unknown() {
        let fx = fixture();
        make_session(&fx.state.store_root, "staff-1", auth::Role::Staff);
        let router = merged_router_for(&fx.state, false);

        // Admin host + staff session (dev seam: no cert needed) → 200.
        let res = router
            .clone()
            .oneshot(admin_request(
                "GET",
                "/apostille/api/v1/me",
                Some("admin.andrii.ai"),
                Some("staff-1"),
                None,
                None,
                None,
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(json_body(res).await["user_id"], "staff-1");

        // Admin host without a session → 401 (auth untouched).
        let res = router
            .clone()
            .oneshot(admin_request(
                "GET",
                "/apostille/api/v1/me",
                Some("admin.andrii.ai"),
                None,
                None,
                None,
                None,
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);

        // Public, unknown, and missing hosts → public branch.
        for host in [Some("andrii.ai"), Some("unknown.test"), None] {
            let mut builder = Request::builder().uri("/apostille/healthz");
            if let Some(host) = host {
                builder = builder.header("host", host);
            }
            let res = router
                .clone()
                .oneshot(builder.body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(res.status(), StatusCode::OK, "host: {host:?}");
        }

        // Admin paths are unreachable via the public host: the
        // public branch answers 404, never the admin handler.
        let res = router
            .oneshot(admin_request(
                "GET",
                "/apostille/api/v1/queue",
                Some("andrii.ai"),
                Some("staff-1"),
                None,
                None,
                None,
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn host_matrix_strict_cert_semantics() {
        let fx = strict_fixture();
        make_session(&fx.state.store_root, "staff-1", auth::Role::Staff);
        let (ca, ca_key) = test_ca("strict-ca");
        let client = test_client(&ca, &ca_key, "staff-1");
        enroll_cert(&fx.state.store_root, &client.fingerprint, "staff-1");
        let router = merged_router_for(&fx.state, false);
        let admin_me = || {
            admin_request(
                "GET",
                "/apostille/api/v1/me",
                Some("admin.andrii.ai"),
                Some("staff-1"),
                None,
                None,
                None,
            )
        };

        // Session but no cert → 403.
        let res = router.clone().oneshot(admin_me()).await.unwrap();
        assert_eq!(res.status(), StatusCode::FORBIDDEN);

        // Session + registered cert (as the acceptor would capture,
        // via the branch's own fingerprint layer) → 200, echoed.
        let mut req = admin_me();
        let mut reader = std::io::BufReader::new(client.cert_pem.as_slice());
        let ders: Vec<Vec<u8>> = rustls_pemfile::certs(&mut reader)
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
            .iter()
            .map(|der| der.as_ref().to_vec())
            .collect();
        req.extensions_mut()
            .insert(mtls::PeerCertificates::from_ders(ders));
        let res = router.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(json_body(res).await["cert_fingerprint"], client.fingerprint);

        // Session + unknown cert → 403.
        let mut req = admin_me();
        req.extensions_mut()
            .insert(mtls::PeerCertificates::from_ders(vec![vec![0x30, 0x03]]));
        let res = router.oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::FORBIDDEN);
    }

    #[test]
    fn tls_listen_plan_has_no_444_and_gates_quic() {
        let mut cfg = config::RuntimeConfig {
            bind: std::net::IpAddr::from([127, 0, 0, 1]),
            ..config::RuntimeConfig::default()
        };
        // QUIC off: one TCP 443 + the port-80 redirector; 444 nowhere.
        let plan = tls_listen_plan(&cfg);
        assert_eq!(plan.len(), 2);
        assert_eq!(plan[0].kind, TlsListenerKind::Https);
        assert_eq!(plan[0].addr.port(), 443);
        assert_eq!(plan[1].kind, TlsListenerKind::Redirect);
        assert_eq!(plan[1].addr.port(), 80);
        assert!(
            plan.iter()
                .all(|l| l.addr.port() != cfg.admin_https_port && l.addr.port() != 444),
            "plan: {plan:?}"
        );
        // QUIC on: a UDP entry on the SAME port as TCP.
        cfg.quic_enabled = true;
        let plan = tls_listen_plan(&cfg);
        assert_eq!(plan.len(), 3);
        assert_eq!(plan[1].kind, TlsListenerKind::Quic);
        assert_eq!(plan[1].addr, plan[0].addr);
        // A v6 bind doubles every entry — still no 444.
        cfg.bind6 = Some(std::net::IpAddr::from(std::net::Ipv6Addr::LOCALHOST));
        let plan = tls_listen_plan(&cfg);
        assert_eq!(plan.len(), 6);
        assert!(plan.iter().all(|l| l.addr.port() != 444), "plan: {plan:?}");
    }

    #[tokio::test]
    async fn alt_svc_wired_iff_quic_enabled() {
        let fx = fixture();
        for enabled in [false, true] {
            let res = merged_router_for(&fx.state, enabled)
                .oneshot(
                    Request::builder()
                        .uri("/apostille/healthz")
                        .header("host", "andrii.ai")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(res.status(), StatusCode::OK);
            assert_eq!(
                res.headers().contains_key(axum::http::header::ALT_SVC),
                enabled,
                "quic: {enabled}"
            );
        }
        let res = merged_router_for(&fx.state, true)
            .oneshot(
                Request::builder()
                    .uri("/apostille/healthz")
                    .header("host", "andrii.ai")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            res.headers().get(axum::http::header::ALT_SVC).unwrap(),
            "h3=\":443\""
        );
    }

    /// QUIC+h3 test client config: verification skipped (rcgen
    /// certs), h3 ALPN, optional client identity. NEVER outside tests.
    struct QuicTestIdentity {
        certs: Vec<rustls::pki_types::CertificateDer<'static>>,
        key: rustls::pki_types::PrivateKeyDer<'static>,
    }

    fn quic_test_client_config(identity: Option<QuicTestIdentity>) -> rustls::ClientConfig {
        let builder = rustls::ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(std::sync::Arc::new(AcceptAny));
        let mut config = match identity {
            None => builder.with_no_client_auth(),
            Some(identity) => builder
                .with_client_auth_cert(identity.certs, identity.key)
                .expect("test client identity"),
        };
        config.alpn_protocols = vec![b"h3".to_vec()];
        config
    }

    fn quic_identity_from_pem(cert_pem: &[u8], key_pem: &[u8]) -> QuicTestIdentity {
        let mut cert_reader = std::io::BufReader::new(cert_pem);
        let certs = rustls_pemfile::certs(&mut cert_reader)
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        let mut key_reader = std::io::BufReader::new(key_pem);
        let key = rustls_pemfile::private_key(&mut key_reader)
            .unwrap()
            .unwrap();
        QuicTestIdentity { certs, key }
    }

    /// Bound QUIC endpoint on loopback port 0 serving `router` in
    /// the background. Returns the endpoint (close it at test end)
    /// and its bound address.
    async fn serve_quic_for_test(
        router: Router,
        cert_pem: &[u8],
        key_pem: &[u8],
        ca_pem: Option<&[u8]>,
    ) -> (quinn::Endpoint, SocketAddr) {
        let config = quic::server_config(cert_pem, key_pem, ca_pem).unwrap();
        let endpoint = quic::bind_endpoint("127.0.0.1:0".parse().unwrap(), &config).unwrap();
        let addr = endpoint.local_addr().unwrap();
        tokio::spawn(quic::serve(endpoint.clone(), router));
        (endpoint, addr)
    }

    /// One h3 GET over a fresh QUIC connection (fresh client
    /// endpoint on loopback port 0): returns status + full body.
    async fn h3_get(
        server: SocketAddr,
        client_tls: rustls::ClientConfig,
        host: &str,
        path: &str,
        cookie: Option<&str>,
    ) -> (StatusCode, Vec<u8>) {
        use bytes::Buf as _;
        let mut endpoint = quinn::Endpoint::client("127.0.0.1:0".parse().unwrap()).unwrap();
        let quic_client =
            quinn::crypto::rustls::QuicClientConfig::try_from(client_tls).expect("quic client");
        endpoint
            .set_default_client_config(quinn::ClientConfig::new(std::sync::Arc::new(quic_client)));
        let conn = endpoint
            .connect(server, "localhost")
            .unwrap()
            .await
            .unwrap();
        let (mut driver, mut send) = h3::client::new(h3_quinn::Connection::new(conn))
            .await
            .unwrap();
        tokio::spawn(async move {
            std::future::poll_fn(|cx| driver.poll_close(cx)).await;
        });
        let mut builder = axum::http::Request::builder()
            .method("GET")
            .uri(format!("https://{host}{path}"))
            .header("host", host);
        if let Some(cookie) = cookie {
            builder = builder.header("cookie", cookie);
        }
        let mut stream = send.send_request(builder.body(()).unwrap()).await.unwrap();
        stream.finish().await.unwrap();
        let resp = stream.recv_response().await.unwrap();
        let status = resp.status();
        let mut body = Vec::new();
        while let Some(chunk) = stream.recv_data().await.unwrap() {
            body.extend_from_slice(chunk.chunk());
        }
        endpoint.close(0u32.into(), b"done");
        (status, body)
    }

    #[tokio::test]
    async fn quic_serves_healthz_and_version_over_h3() {
        let fx = fixture();
        let (cert_pem, key_pem) = self_signed_localhost();
        let (endpoint, addr) = serve_quic_for_test(
            merged_router_for(&fx.state, true),
            &cert_pem,
            &key_pem,
            None,
        )
        .await;

        let (status, body) = h3_get(
            addr,
            quic_test_client_config(None),
            "andrii.ai",
            "/apostille/healthz",
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let health: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(health["status"], "ok");
        assert_eq!(health["version"], FULL_VERSION);

        let (status, body) = h3_get(
            addr,
            quic_test_client_config(None),
            "andrii.ai",
            "/apostille/version",
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let version: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(version["version"], env!("CARGO_PKG_VERSION"));
        assert_eq!(version["build"], env!("BUILD_SHA"));
        endpoint.close(0u32.into(), b"done");
    }

    #[tokio::test]
    async fn quic_bridge_injects_connect_info() {
        use axum::extract::ConnectInfo;
        // Every auth handler extracts `ConnectInfo` for rate-limit
        // buckets; without it the bridge 500s (staff login/start).
        let router = Router::new().route(
            "/echo-ip",
            get(|ConnectInfo(addr): ConnectInfo<SocketAddr>| async move { addr.ip().to_string() }),
        );
        let (cert_pem, key_pem) = self_signed_localhost();
        let (endpoint, addr) = serve_quic_for_test(router, &cert_pem, &key_pem, None).await;
        let (status, body) = h3_get(
            addr,
            quic_test_client_config(None),
            "andrii.ai",
            "/echo-ip",
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(String::from_utf8(body).unwrap(), "127.0.0.1");
        endpoint.close(0u32.into(), b"done");
    }

    #[tokio::test]
    async fn quic_bridge_marks_transport_quic() {
        let router = Router::new().route(
            "/echo-transport",
            get(|req: Request<Body>| async move {
                req.extensions()
                    .get::<RequestTransport>()
                    .map(|marked| marked.0)
                    .unwrap_or("missing")
                    .to_string()
            }),
        );
        let (cert_pem, key_pem) = self_signed_localhost();
        let (endpoint, addr) = serve_quic_for_test(router, &cert_pem, &key_pem, None).await;
        let (status, body) = h3_get(
            addr,
            quic_test_client_config(None),
            "andrii.ai",
            "/echo-transport",
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(String::from_utf8(body).unwrap(), "quic");
        endpoint.close(0u32.into(), b"done");
    }

    #[tokio::test]
    async fn quic_admin_branch_enforces_mtls_allow_and_deny() {
        let fx = strict_fixture();
        make_session(&fx.state.store_root, "staff-1", auth::Role::Staff);
        let (ca, ca_key) = test_ca("quic-admin-ca");
        let client = test_client(&ca, &ca_key, "quic-staff");
        enroll_cert(&fx.state.store_root, &client.fingerprint, "quic-staff");
        let (cert_pem, key_pem) = self_signed_localhost();
        let (endpoint, addr) = serve_quic_for_test(
            merged_router_for(&fx.state, true),
            &cert_pem,
            &key_pem,
            Some(&ca.pem().into_bytes()),
        )
        .await;
        let session_cookie = format!("{}=staff-1", auth::ADM_COOKIE);
        let admin_me = "/apostille/api/v1/me";

        // DENY: staff session but no client cert → 403.
        let (status, _) = h3_get(
            addr,
            quic_test_client_config(None),
            "admin.andrii.ai",
            admin_me,
            Some(&session_cookie),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);

        // ALLOW: same session + registered client cert → 200, echoed.
        let identity = quic_identity_from_pem(&client.cert_pem, &client.key_pem);
        let (status, body) = h3_get(
            addr,
            quic_test_client_config(Some(identity)),
            "admin.andrii.ai",
            admin_me,
            Some(&session_cookie),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let me: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(me["cert_fingerprint"], client.fingerprint);

        // Cert alone is not a session → 401.
        let identity = quic_identity_from_pem(&client.cert_pem, &client.key_pem);
        let (status, _) = h3_get(
            addr,
            quic_test_client_config(Some(identity)),
            "admin.andrii.ai",
            admin_me,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        endpoint.close(0u32.into(), b"done");
    }

    #[tokio::test]
    async fn quic_wrong_ca_handshake_fails() {
        let fx = strict_fixture();
        let (cert_pem, key_pem) = self_signed_localhost();
        let (server_ca, _) = test_ca("server-ca");
        let (endpoint, addr) = serve_quic_for_test(
            merged_router_for(&fx.state, true),
            &cert_pem,
            &key_pem,
            Some(&server_ca.pem().into_bytes()),
        )
        .await;
        // A client cert from a DIFFERENT CA → handshake failure, no HTTP.
        let (other_ca, other_key) = test_ca("other-ca");
        let rogue = test_client(&other_ca, &other_key, "rogue");
        let identity = quic_identity_from_pem(&rogue.cert_pem, &rogue.key_pem);
        let mut client_endpoint = quinn::Endpoint::client("127.0.0.1:0".parse().unwrap()).unwrap();
        let quic_client = quinn::crypto::rustls::QuicClientConfig::try_from(
            quic_test_client_config(Some(identity)),
        )
        .unwrap();
        client_endpoint
            .set_default_client_config(quinn::ClientConfig::new(std::sync::Arc::new(quic_client)));
        // quinn's `connect` resolves once the client flight is out —
        // the server's verdict arrives as a close, not a connect
        // error. What matters: the server aborts with a crypto
        // close before serving anything on this connection.
        let conn = client_endpoint
            .connect(addr, "localhost")
            .unwrap()
            .await
            .unwrap();
        let err = tokio::time::timeout(std::time::Duration::from_secs(10), conn.closed())
            .await
            .expect("server must close a wrong-CA connection, not serve it");
        assert!(
            matches!(err, quinn::ConnectionError::ConnectionClosed(_)),
            "got: {err:?}"
        );
        endpoint.close(0u32.into(), b"done");
        client_endpoint.close(0u32.into(), b"done");
    }

    #[tokio::test]
    async fn quic_ws_path_is_501_over_h3() {
        let fx = fixture();
        let (cert_pem, key_pem) = self_signed_localhost();
        let (endpoint, addr) = serve_quic_for_test(
            merged_router_for(&fx.state, true),
            &cert_pem,
            &key_pem,
            None,
        )
        .await;
        let (status, body) = h3_get(
            addr,
            quic_test_client_config(None),
            "andrii.ai",
            "/apostille/ws/echo",
            None,
        )
        .await;
        assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
        let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(
            value["error"].as_str().unwrap().contains("TCP"),
            "got: {value}"
        );
        endpoint.close(0u32.into(), b"done");
    }

    #[tokio::test]
    async fn quic_disabled_binds_no_udp_socket() {
        // Reserve a concrete loopback UDP port, then release it.
        let probe = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        let port = probe.local_addr().unwrap().port();
        drop(probe);
        // Disabled (the default): the plan holds no QUIC entry,
        // and a UDP bind on the would-be QUIC port succeeds
        // (nothing is there).
        let mut cfg = config::RuntimeConfig {
            bind: std::net::IpAddr::from([127, 0, 0, 1]),
            https_port: port,
            ..config::RuntimeConfig::default()
        };
        let plan = tls_listen_plan(&cfg);
        assert!(plan.iter().all(|l| l.kind != TlsListenerKind::Quic));
        let free = std::net::UdpSocket::bind(("127.0.0.1", port));
        assert!(free.is_ok(), "disabled QUIC must leave UDP {port} free");
        drop(free);

        // Enabled: binding the planned entry holds the port — the
        // same probe now fails with AddrInUse (which also proves the
        // probe above is a real negative).
        cfg.quic_enabled = true;
        let plan = tls_listen_plan(&cfg);
        let quic_addrs: Vec<SocketAddr> = plan
            .iter()
            .filter(|l| l.kind == TlsListenerKind::Quic)
            .map(|l| l.addr)
            .collect();
        assert_eq!(quic_addrs, vec![SocketAddr::new(cfg.bind, port)]);
        let (cert_pem, key_pem) = self_signed_localhost();
        let config = quic::server_config(&cert_pem, &key_pem, None).unwrap();
        let endpoint = quic::bind_endpoint(quic_addrs[0], &config).unwrap();
        let held = std::net::UdpSocket::bind(("127.0.0.1", port));
        assert!(
            matches!(held, Err(ref e) if e.kind() == std::io::ErrorKind::AddrInUse),
            "got: {held:?}"
        );
        endpoint.close(0u32.into(), b"done");
    }
}
