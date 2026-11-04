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
mod admin_authorities;
mod admin_ip;
mod admin_orders;
mod admin_packs;
mod admin_restart;
mod admin_stats;
mod admin_voice;
mod admin_workers;
mod approval;
mod authority_path;
mod call_cache;
mod compile;
mod customer_labels;
mod demo_precheck;
mod draft;
mod factcheck;
mod fake;
mod fedex_live;
mod first_source;
mod follow;
mod forms_map;
mod fulfilment;
mod host;
mod label_autofill;
mod locations;
mod magic;
mod mtls;
mod oauth;
mod oauth_apple;
mod oauth_meta;
mod offices;
mod orders;
mod p2;
mod p2_labels;
mod passkey;
mod plan;
mod queue;
mod quic;
mod readyz;
mod reconcile;
mod reports;
mod setup;
mod source_history;
mod staff_passkey;
mod stripe;
mod telemetry;
mod tracking;
mod translate;
mod unfurl;
mod uploads;
mod usps;
mod worker;
mod ws_bus;

use axum::{
    Router,
    extract::{
        ConnectInfo, MatchedPath, State,
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

/// Unix seconds at boot (admin `/version` env badge).
static STARTED_AT: std::sync::OnceLock<i64> = std::sync::OnceLock::new();

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
pub(crate) struct ErrorKind(pub(crate) &'static str);

/// Tag a response with its failure kind for the access log (5xx at
/// ERROR, tagged 4xx at WARN), e.g. `step_up_stale` on a refused
/// config write, so a reported `x-request-id` explains itself.
pub(crate) fn tag_error(
    mut res: axum::response::Response,
    kind: &'static str,
) -> axum::response::Response {
    res.extensions_mut().insert(ErrorKind(kind));
    res
}

/// Request access log, one span per request: `id method uri
/// transport branch route` in, `status latency_ms` (+ `error_kind`
/// on 5xx) out. Logged fields are fixed and PII-free by
/// construction (our URLs never carry tokens or customer data).
/// 5xx logs at ERROR so the journald error budget (`-p err`)
/// counts app failures; everything else logs at INFO. The same
/// opaque ID goes back in `x-request-id` for correlation.
///
/// The whole tail runs under [`tracing::Instrument`], never a
/// bare `enter()` guard: an entered guard held across `await`
/// leaks the span onto whatever task the worker polls next and
/// loses it when this task migrates, so concurrent bursts log
/// spanless or cross-attributed response events.
async fn request_id_and_log(
    State(branch): State<&'static str>,
    req: axum::http::Request<axum::body::Body>,
    next: axum::middleware::Next,
) -> impl IntoResponse {
    use tracing::Instrument as _;
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
    // Path only, never the query: secrets ride in query strings
    // (the stable notify-unsubscribe token), and they must not
    // land in logs.
    let path = req.uri().path().to_owned();
    let span = tracing::info_span!(
        "request",
        id = %id,
        method = %req.method(),
        uri = %path,
        transport = %transport,
        branch = %branch,
        route = %route,
    );
    async move {
        let started = std::time::Instant::now();
        let mut res = next.run(req).await;
        if let Ok(value) = HeaderValue::from_str(&id) {
            res.headers_mut().insert("x-request-id", value);
        }
        // M2: baseline security headers on every response, both
        // branches (the admin branch additionally denies framing).
        for (name, value) in security_headers(branch) {
            res.headers_mut().insert(name, value);
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
        } else if let Some(kind) = res
            .status()
            .is_client_error()
            .then(|| res.extensions().get::<ErrorKind>().map(|kind| kind.0))
            .flatten()
        {
            tracing::warn!(
                status = %res.status(),
                latency_ms = started.elapsed().as_millis(),
                error_kind = %kind,
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
    .instrument(span)
    .await
}

/// Baseline security headers (M2): MIME-sniffing off, no
/// referrer leakage, and framing limited (denied on the admin
/// branch, same-origin on public). HSTS without preload: honored
/// on the TLS listeners, ignored by browsers over plain-HTTP dev.
/// No CSP here — the SvelteKit shells need script nonces first.
fn security_headers(branch: &str) -> [(axum::http::HeaderName, HeaderValue); 4] {
    let frame = if branch == "admin" {
        "DENY"
    } else {
        "SAMEORIGIN"
    };
    [
        (
            header::X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        ),
        (
            header::REFERRER_POLICY,
            HeaderValue::from_static("no-referrer"),
        ),
        (header::X_FRAME_OPTIONS, HeaderValue::from_static(frame)),
        (
            header::STRICT_TRANSPORT_SECURITY,
            HeaderValue::from_static("max-age=63072000; includeSubDomains"),
        ),
    ]
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
    /// `admin.mtls_off` dev seam (DB config): allows loopback
    /// admin hosts and honors the test cert header. Prod runs
    /// strict (false); true refuses release boot.
    admin_mtls_off: bool,
    /// `admin.allow_ips` (DB config): parsed CIDR/host allowlist for
    /// the admin listener. Session + staff role + listed IP, or 403.
    admin_allow_ips: Vec<config::IpNet>,
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
    /// Live FedEx buys (SA-19): gated staff purchase state built
    /// from `labels.*` DB rows at boot. Disabled until staff opts
    /// in; the endpoints 503 while any piece is missing.
    fedex_live: fedex_live::FedexLiveState,
    /// Passkey ceremonies (spec `customer-auth.md`): process-wide
    /// crate handle built from `auth.*` DB rows at boot.
    webauthn: std::sync::Arc<webauthn_rs::Webauthn>,
    /// Auth rate-limit buckets (spec decision 0.6).
    auth_limits: std::sync::Arc<passkey::RateBuckets>,
    /// Demo-precheck buckets (separate map: a flood of distinct demo
    /// IPs must never fill auth's 8192-key map and fail sign-in closed).
    demo_limits: std::sync::Arc<passkey::RateBuckets>,
    /// Live draft-run registry (operator finding 12): staff-triggered
    /// model drafts and their step state, keyed by server-allocated id.
    draft_runs: std::sync::Arc<draft::DraftRuns>,
    /// Runtime switches and caps (`orders.enabled`, `usps.live`, daily
    /// caps) read per request through a 30 s cache; the admin config
    /// PUT invalidates it, so installing keys needs no restart.
    config_cache: std::sync::Arc<config::CachedReader>,
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
    /// Follower-notification poke: approval pokes after each accept;
    /// the worker loop also ticks every 60 s for backoff rows.
    notify_poke: std::sync::Arc<tokio::sync::Notify>,
    /// `notify.*` rows (fail-closed: off, cap 3, empty postal).
    notify_enabled: bool,
    notify_daily_cap: u64,
    notify_postal: String,
    /// Follow rate-limit buckets (separate map: a flood of distinct
    /// targets must never fill auth's map and fail sign-in closed).
    notify_limits: std::sync::Arc<passkey::RateBuckets>,
    /// Notify link base (origin-derived, never configured):
    /// `{auth.origin}/apostille`.
    notify_base: String,
    /// Meta deletion-status base (origin-derived, never
    /// configured): `{auth.origin}/auth/meta/delete-status`.
    meta_delete_status_base: String,
    /// W-3 integration modes (`<area>.mode` rows) and the boot's fake
    /// guard: which outside services run their local fake.
    fakes: config::Modes,
}

/// Full-request budget, routing to response head: slow clients
/// and stuck handlers end in 408, never a hung connection.
/// Upgrade responses (WS 101s) resolve in milliseconds, so
/// long-lived streams are unaffected.
pub(crate) const REQUEST_TIMEOUT_SECS: u64 = 30;
/// Media-upload budget: multipart bodies stream inside the
/// handler future, so the 30 s default would 408 slow client
/// uploads. The route's byte cap still bounds the bytes.
pub(crate) const UPLOAD_TIMEOUT_SECS: u64 = 300;
/// The one path with the long budget (the admin upload route
/// and the selector below share it — no second spelling).
pub(crate) const MEDIA_UPLOAD_PATH: &str = "/apostille/api/v1/media/upload";
/// Slowloris guard at the hyper layer (both listeners): a client
/// that cannot finish its request head in this long is closed.
pub(crate) const HEADER_READ_TIMEOUT_SECS: u64 = 10;
/// In-flight request ceiling for the whole process. Overflow
/// waits inside the request budget above, then 408s like
/// anything slow.
pub(crate) const MAX_CONCURRENT_REQUESTS: usize = 512;

/// Per-path request budget in seconds (pure: unit-tested): the
/// media upload gets 300 s, everything else the 30 s default.
pub(crate) fn budget_secs_for_path(path: &str) -> u64 {
    if path == MEDIA_UPLOAD_PATH {
        UPLOAD_TIMEOUT_SECS
    } else {
        REQUEST_TIMEOUT_SECS
    }
}

/// The one in-flight ceiling for the whole process: public, the
/// fully-merged admin stack, and the QUIC bridge (which re-enters
/// the same guarded routers via `oneshot`) all draw permits from
/// this semaphore, so a flood on one listener cannot starve the
/// others past the shared cap. Clones share it — never rebuild it
/// per router.
pub(crate) fn shared_concurrency() -> tower::limit::GlobalConcurrencyLimitLayer {
    tower::limit::GlobalConcurrencyLimitLayer::with_semaphore(shared_semaphore())
}

/// The one semaphore behind [`shared_concurrency`]: owned here so
/// tests can read free permits without saturating the cap.
fn shared_semaphore() -> std::sync::Arc<tokio::sync::Semaphore> {
    static SHARED: std::sync::OnceLock<std::sync::Arc<tokio::sync::Semaphore>> =
        std::sync::OnceLock::new();
    SHARED
        .get_or_init(|| std::sync::Arc::new(tokio::sync::Semaphore::new(MAX_CONCURRENT_REQUESTS)))
        .clone()
}

/// Free permits on the shared ceiling (test seam: proves held
/// requests hold shared permits without saturating the cap).
#[cfg(test)]
pub(crate) fn shared_concurrency_permits() -> usize {
    shared_semaphore().available_permits()
}

/// Per-request budget middleware: the upload path gets 300 s,
/// everything else 30 s; overruns end in 408 with an empty body
/// (the same observable the old `HandleErrorLayer` produced).
async fn request_budget(
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let secs = budget_secs_for_path(req.uri().path());
    match tokio::time::timeout(std::time::Duration::from_secs(secs), next.run(req)).await {
        Ok(res) => res,
        Err(_) => StatusCode::REQUEST_TIMEOUT.into_response(),
    }
}

/// The same guardrail stack on every router: the shared
/// concurrency ceiling inside, the per-path budget outside, so
/// time spent queued for a permit counts toward the budget.
/// Logging middleware stays outermost (each router adds it after),
/// so timed-out requests still log. Applied once per router —
/// never nest it, or one request holds two shared permits.
pub(crate) fn guardrails<S>(router: Router<S>) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    // `Router::layer` wraps outside-in per call: budget first,
    // then the shared ceiling, then the routes.
    router
        .layer(shared_concurrency())
        .layer(axum::middleware::from_fn(request_budget))
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
            admin_allow_ips: config::loopback_allow_ips(),
            admin_spa_dir: PathBuf::from("test-admin-spa"),
            media_terms_read: false,
            label_cfg: p2_labels::LabelCfg::default(),
            voice_cfg: voice::VoiceConfig::default(),
            voice_api_key: None,
            fedex_live: fedex_live::FedexLiveState::default(),
            webauthn: std::sync::Arc::new(passkey::test_webauthn()),
            auth_limits: std::sync::Arc::new(passkey::RateBuckets::default()),
            demo_limits: std::sync::Arc::new(passkey::RateBuckets::default()),
            draft_runs: std::sync::Arc::new(draft::DraftRuns::default()),
            config_cache: std::sync::Arc::new(config::CachedReader::default()),
            google_cfg: None,
            apple_cfg: None,
            meta_cfg: None,
            mailer: None,
            magic_link_base: "https://example.test/apostille/login".to_string(),
            notify_poke: std::sync::Arc::new(tokio::sync::Notify::new()),
            notify_enabled: false,
            notify_daily_cap: 3,
            notify_postal: String::new(),
            notify_limits: std::sync::Arc::new(passkey::RateBuckets::default()),
            notify_base: "https://example.test/apostille".to_string(),
            meta_delete_status_base: "https://example.test/auth/meta/delete-status".to_string(),
            fakes: config::Modes::default(),
        },
        static_dir,
        spa_dir,
        acme_dir,
    )
}

// ===== Module hooks =====
// One place for every lane-owned module's DDL and routes, so lanes
// never edit this file and prod + test routers stay identical. The
// builder adds a line here when a lane needs a new hook.

/// Lane-owned metadata-DB DDL (`CREATE TABLE IF NOT EXISTS` only).
fn migrate_mission_tables(conn: &rusqlite::Connection) -> rusqlite::Result<()> {
    telemetry::migrate_tables(conn)?;
    plan::migrate_tables(conn)?;
    compile::migrate_tables(conn)?;
    factcheck::migrate_tables(conn)?;
    offices::migrate_tables(conn)?;
    first_source::migrate_tables(conn)?;
    authority_path::migrate_tables(conn)?;
    fake::migrate_tables(conn)?;
    forms_map::migrate_tables(conn)
}

/// Lane-owned P2-DB DDL (`CREATE TABLE IF NOT EXISTS` only).
fn migrate_mission_p2(p2: &rusqlite::Connection) -> rusqlite::Result<()> {
    orders::migrate_p2_tables(p2)?;
    forms_map::migrate_p2_tables(p2)?;
    p2_labels::migrate_p2_tables(p2)?;
    customer_labels::migrate_p2_tables(p2)?;
    stripe::migrate_p2_tables(p2)?;
    tracking::migrate_p2_tables(p2)
}

/// Lane-owned public routes (merged into the `/apostille` API).
fn mission_public_routes() -> Router<AppState> {
    plan::public_routes()
        .merge(stripe::public_routes())
        .merge(worker::public_routes())
        .merge(source_history::public_routes())
}

/// Lane-owned customer routes (merged into the `/api/v1/p2` nest).
fn mission_p2_routes() -> Router<AppState> {
    orders::p2_routes().merge(forms_map::p2_routes())
}

/// Lane-owned staff routers (ADMIN listener only). The admin guard
/// stack (host guard, IP allowlist, CSRF double-submit, request log)
/// wraps every lane router here, exactly as `admin_workers::router`
/// layers itself, so no lane route can ship without it; handlers still
/// take `AdminAuth` (session + staff role).
fn mission_admin_routers(state: &AppState) -> Router {
    mission_admin_routes(state)
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            admin::csrf_guard,
        ))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            admin::require_admin_ip,
        ))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            admin::admin_host_guard,
        ))
        .layer(axum::middleware::from_fn_with_state(
            "admin",
            request_id_and_log,
        ))
}

fn mission_admin_routes(state: &AppState) -> Router {
    telemetry::admin_router(state.clone())
        .merge(admin_stats::admin_router(state.clone()))
        .merge(uploads::admin_router(state.clone()))
        .merge(plan::admin_router(state.clone()))
        .merge(compile::admin_router(state.clone()))
        .merge(factcheck::admin_router(state.clone()))
        .merge(offices::admin_router(state.clone()))
        .merge(forms_map::admin_router(state.clone()))
        .merge(fulfilment::admin_router(state.clone()))
        .merge(admin_orders::admin_router(state.clone()))
        .merge(first_source::admin_router(state.clone()))
        .merge(admin_packs::admin_router(state.clone()))
        .merge(admin_authorities::admin_router(state.clone()))
        .merge(worker::admin_router(state.clone()))
        .merge(source_history::admin_router(state.clone()))
        .merge(fake::admin_router(state.clone()))
}

/// Public router with explicit state (admin routes are NEVER here —
/// they live on the admin listener via [`admin::router`]).
fn public_router(state: AppState, static_dir: &str, spa_dir: &str, acme_dir: &str) -> Router {
    let spa_index = state.spa_index.clone();
    let api = Router::new()
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz::readyz))
        .route("/version", get(version))
        .route("/ws/echo", get(ws_echo_upgrade))
        .route("/ws/bus", get(ws_bus::ws_bus_upgrade))
        .route("/api/v1/demo/login", post(demo_login))
        .route("/api/v1/demo/precheck/samples", get(demo_precheck::samples))
        .route("/api/v1/demo/precheck", post(demo_precheck::run))
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
        .route("/api/v1/auth/providers", get(auth_providers))
        .route("/api/v1/quality", get(quality))
        .route("/api/v1/auth/google/start", post(oauth::start))
        .route("/api/v1/auth/apple/start", post(oauth_apple::start))
        .route("/api/v1/auth/meta/start", post(oauth_meta::start))
        .route("/api/v1/auth/magic/start", post(magic::start))
        .route("/api/v1/auth/magic/consume", post(magic::consume))
        .route("/api/v1/authorities", get(approval::public_authorities))
        .route(
            "/api/v1/authorities/{slug}",
            get(approval::public_authority),
        )
        // Readable addresses (usa/washington, us/wa) → internal slug.
        .route(
            "/api/v1/authorities/resolve",
            get(authority_path::resolve_route),
        )
        .route(
            "/api/v1/authorities/{slug}/facts",
            get(approval::public_facts),
        )
        .route(
            "/api/v1/authorities/{slug}/translations",
            get(translate::public_translations),
        )
        // SA-21 walk-in reports: signed-in submit (401 anonymous),
        // masked public list per authority.
        .route("/api/v1/reports", post(reports::submit))
        .route(
            "/api/v1/authorities/{slug}/reports",
            get(reports::list_for_authority),
        )
        // Crawler surface: shell with injected head (slug routes
        // sort before the SPA fallback below, so unfurls render
        // with no JS).
        .route("/authority/{*trail}", get(unfurl::dossier))
        // Place pages (P2-5/P4-1): `/in/usa`, `/in/usa/washington`.
        .route("/in/{*path}", get(locations::location_page))
        .route("/api/v1/locations/{*path}", get(locations::location_route))
        .route("/sitemap.xml", get(unfurl::sitemap))
        .route("/api/v1/notify/status", get(follow::status))
        .route("/api/v1/notify/subscribe", post(follow::subscribe))
        .route(
            "/api/v1/notify/confirm",
            post(follow::confirm_post).get(follow::confirm_get),
        )
        .route(
            "/api/v1/notify/unsubscribe",
            post(follow::unsubscribe_post).get(follow::unsubscribe_get),
        )
        .nest("/api/v1/p2", p2::router().merge(mission_p2_routes()))
        // MISSION WIN lanes (B0): public plan read, Stripe webhook.
        .merge(mission_public_routes())
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
    // Guardrails sit INSIDE the request-id log layer below, so
    // timed-out requests still log.
    guardrails(
        Router::new()
            // Domain root is not the handbook: the whole public surface
            // lives under `/apostille`, so `/` redirects there (a bare
            // 404 today). Static target — no open redirect.
            .route("/", get(root_to_apostille))
            .route("/apostille", get(serve_spa_index))
            .route("/apostille/", get(serve_spa_index))
            .route("/robots.txt", get(serve_robots))
            .route("/privacy", get(serve_privacy))
            .route("/auth/google/callback", get(oauth::callback))
            .route("/auth/google/risc", post(oauth::risc))
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
            )),
    )
    .with_state(state)
}

/// `GET /` on the public host is never content: redirect to the
/// handbook root. Static target — no open redirect.
async fn root_to_apostille() -> impl IntoResponse {
    (
        StatusCode::TEMPORARY_REDIRECT,
        [(header::LOCATION, HeaderValue::from_static("/apostille/"))],
    )
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

/// `demo_session` Set-Cookie value (H4: `Secure` outside local dev,
/// the same `secure = !admin_mtls_off` convention as the session and
/// CSRF cookies. The `__Host-` prefix stays deferred — it requires
/// `Path=/`, which would widen the cookie past `/apostille`).
fn demo_session_cookie(token: &str, secure: bool) -> String {
    // Built manually (no cookie crate): token is hex, so always valid.
    let mut cookie = format!("demo_session={token}; HttpOnly; SameSite=Lax; Path=/apostille");
    if secure {
        cookie.push_str("; Secure");
    }
    cookie
}

/// DEMO login: no password is checked (clearly-marked demo only).
/// Creates a `demo_session` row with 24h expiry and sets a
/// `demo_session` cookie. Only the token hash is stored server-side
/// (H4); the cookie carries the raw token.
async fn demo_login(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Json(body): Json<LoginBody>,
) -> impl IntoResponse {
    // H4: demo sessions read that ref's submissions, so only `demo-`
    // refs (which no real account can have) may mint one. A real
    // account id here would read a stranger's data.
    if body.customer_ref.is_empty()
        || body.customer_ref.len() > 64
        || !body.customer_ref.starts_with("demo-")
    {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"error": "invalid customer_ref"})),
        )
            .into_response();
    }
    let now = unix_now();
    // M1/H4: per-IP rate limit on mint-as-anyone (10/min). The sweep
    // below bounds table rows; this bounds the mint rate (and token
    // guessing through `me`, which shares the stored hash).
    // Keyed by network (IPv4 address or IPv6 /64), so rotating
    // addresses inside one /64 cannot mint refs freely (re-review #9).
    let ip = crate::admin_ip::rate_key(Some(addr.ip()));
    if !state.demo_limits.check(&format!("dl:{ip}"), now, 10, 60) {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            Json(json!({"error": "too many demo logins — wait a minute"})),
        )
            .into_response();
    }
    let bytes = rand::random::<[u8; 32]>();
    let mut token = String::with_capacity(64);
    for b in bytes {
        token.push_str(&format!("{b:02x}"));
    }
    let conn = match db::open(&Store::new(&state.store_root)) {
        Ok(conn) => conn,
        Err(_) => return store_unavailable(),
    };
    // M1: sweep expired rows on every login so the demo table cannot
    // grow unboundedly.
    let _ = db::delete_expired_demo_sessions(&conn, now);
    if db::create_demo_session(&conn, &token, &body.customer_ref, now, now + 86_400).is_err() {
        return store_unavailable();
    }
    let cookie = demo_session_cookie(&token, !state.admin_mtls_off);
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

/// `GET /api/v1/quality` — latest eval summary written by
/// `cargo xtask eval` to `<store>/quality.json`. Always 200: before the
/// first run the body says so explicitly (`run_at: null`), because the
/// home page probes this endpoint on every load and a 4xx/5xx there
/// would log a console error on GP1. The A5 row still fails on the
/// pending body (no `run_at`, `gold_count` 0) until the operator runs
/// xtask/ops/ops-a5-eval.sh.
async fn quality(State(state): State<AppState>) -> impl IntoResponse {
    let pending = || {
        (
            StatusCode::OK,
            Json(json!({
                "status": "no-quality-run",
                "run_at": null,
                "gold_count": 0,
                "quote_found_rate": null,
                "value_in_quote_rate": null,
                "topic_accuracy": null,
                "model": null,
            })),
        )
            .into_response()
    };
    let path = state.store_root.join("quality.json");
    let Ok(bytes) = std::fs::read(&path) else {
        return pending();
    };
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
        return pending();
    };
    (StatusCode::OK, Json(value)).into_response()
}

/// `GET /auth/providers` — OAuth provider ids configured in THIS
/// deployment (credentials present). The login page renders one button
/// per entry and nothing for unconfigured providers.
async fn auth_providers(State(state): State<AppState>) -> impl IntoResponse {
    let mut providers = Vec::new();
    if state.google_cfg.is_some() {
        providers.push("google");
    }
    if state.apple_cfg.is_some() {
        providers.push("apple");
    }
    if state.meta_cfg.is_some() {
        providers.push("meta");
    }
    (StatusCode::OK, Json(json!({"providers": providers}))).into_response()
}

/// DEMO session check: a valid token returns the customer ref. Callers
/// presenting no credential get 200 `{"user": null}` so the anonymous
/// probe in the public shell never logs a console error; a
/// presented-but-unknown/expired token is still 401.
async fn me(State(state): State<AppState>, headers: HeaderMap) -> impl IntoResponse {
    let anonymous = || (StatusCode::OK, Json(json!({"user": null}))).into_response();
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
        return anonymous();
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
    match mtls::cert_has_dns_san(cert_pem, auth::admin_host()) {
        Ok(true) => {}
        Ok(false) => eprintln!(
            "WARNING: TLS certificate lacks the {} SAN — browsers on the admin host will fail closed; reissue with both SANs (single-443)",
            auth::admin_host()
        ),
        Err(e) => eprintln!("WARNING: {e} — cannot confirm the admin SAN is present"),
    }
}

/// Single-443 listener TLS: plain server TLS, no client-cert
/// request (admin access is gated by `admin.allow_ips` and a staff
/// session).
fn load_admin_tls_from_pem(
    cert_pem: &[u8],
    key_pem: &[u8],
) -> Result<axum_server::tls_rustls::RustlsConfig, String> {
    let config = mtls::admin_server_config(cert_pem, key_pem, None)?;
    Ok(axum_server::tls_rustls::RustlsConfig::from_config(
        std::sync::Arc::new(config),
    ))
}

/// Hot-reload the admin config after an ACME renewal: rebuild the
/// whole `ServerConfig` (rustls has no in-place cert swap on a
/// custom-built config) and swap it in.
fn reload_admin_tls(
    admin_tls: &axum_server::tls_rustls::RustlsConfig,
    cert_pem: &[u8],
    key_pem: &[u8],
) -> Result<(), String> {
    let config = mtls::admin_server_config(cert_pem, key_pem, None)?;
    admin_tls.reload_from_config(std::sync::Arc::new(config));
    // Renewals refresh the leaf `/readyz` watches, or it would
    // report the old expiry forever.
    readyz::record_serving_leaf(cert_pem);
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
/// Customer-facing origin: `https://{net.public_host}`. Unfurl
/// canonical/og/JSON-LD, the sitemap, OAuth callbacks, magic links,
/// notify links and the Meta delete-status base all build from this.
/// `auth.origin` stays the staff WebAuthn origin only.
fn public_origin(public_host: &str) -> String {
    format!("https://{}", public_host.trim_end_matches('/'))
}

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
    let _ = STARTED_AT.set(unix_now());
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
    // MISSION WIN lanes (B0): lane-owned P2 DDL, CREATE TABLE IF NOT
    // EXISTS only (orders, ledger, tracking). Must run before the drop.
    migrate_mission_p2(&p2conn)
        .unwrap_or_else(|e| panic!("STORE_DIR={store_dir}: migrate mission P2 tables: {e}"));
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
    notify::migrate(&conn)
        .unwrap_or_else(|e| panic!("STORE_DIR={store_dir}: migrate notify tables: {e}"));
    // SA-19 live FedEx: lane-owned DDL, CREATE TABLE IF NOT EXISTS
    // only (idempotency ledger + audit trail).
    for ddl in labels::live::ddl_statements() {
        conn.execute_batch(ddl)
            .unwrap_or_else(|e| panic!("STORE_DIR={store_dir}: migrate fedex-live tables: {e}"));
    }
    // SA-20 translate: lane-owned DDL, CREATE TABLE IF NOT EXISTS
    // only (fact_translation).
    crate::translate::migrate_translate_tables(&conn)
        .unwrap_or_else(|e| panic!("STORE_DIR={store_dir}: migrate translate tables: {e}"));
    // SA-21 walk-in reports: lane-owned DDL, CREATE TABLE IF NOT
    // EXISTS only (report, report_photo, report_promotion).
    crate::reports::migrate_report_tables(&conn)
        .unwrap_or_else(|e| panic!("STORE_DIR={store_dir}: migrate report tables: {e}"));
    // MISSION WIN lanes (B0): lane-owned DDL, CREATE TABLE IF NOT
    // EXISTS only (model_call, page_plan, office_record, …).
    migrate_mission_tables(&conn)
        .unwrap_or_else(|e| panic!("STORE_DIR={store_dir}: migrate mission tables: {e}"));
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
    // P393 staff restart: its preflight skips the ports we hold.
    admin_restart::remember_boot(&cfg);
    if cfg.admin_mtls_off {
        eprintln!(
            "WARNING: admin.mtls_off — local-dev seams on (loopback admin hosts, test cert header; local dev only)"
        );
    }
    // W-3: say loudly which outside services run their local fake
    // (the loader already refused fakes on a public, non-staging host).
    let fakes = cfg.modes.fakes();
    if !fakes.is_empty() {
        let names: Vec<&str> = fakes.iter().map(|i| i.label()).collect();
        eprintln!(
            "FAKE MODE — {}: demo stand-ins only; nothing is sent, shipped or charged",
            names.join(", ")
        );
    }
    // Host routing, CSRF origins and the admin passkey origin follow
    // `net.public_host` (admin is `admin.<public_host>`), fixed once here.
    if !auth::init_hosts(&cfg.public_host) {
        eprintln!(
            "WARNING: net.public_host {:?} is not a host name; serving as {} / {}",
            cfg.public_host,
            auth::public_host(),
            auth::admin_host()
        );
    }
    // B2: every customer-facing URL builds from the public origin,
    // never from the staff WebAuthn origin (`auth.origin` feeds only
    // `build_webauthn` below).
    let pub_origin = public_origin(&cfg.public_host);
    let state = AppState {
        spa_index: std::path::Path::new(&cfg.spa_dir).join("index.html"),
        static_dir: PathBuf::from(&cfg.static_dir),
        store_root: PathBuf::from(store_dir.as_str()),
        bus: ws_bus::Bus::new(),
        kek: Some(kek),
        admin_mtls_off: cfg.admin_mtls_off,
        admin_allow_ips: cfg.admin_allow_ips.clone(),
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
        // SA-19 live FedEx: secrets ride as `SecretText` (never
        // logged, never rendered); the routes 503 until all set.
        fedex_live: fedex_live::FedexLiveState {
            enabled: cfg.labels_live,
            daily_cap: cfg.labels_live_daily_cap,
            max_usd_cents: cfg.labels_live_max_usd_cents,
            ship_to_allow: cfg.labels_live_ship_to_allow.clone(),
            client_id: Option::clone(&cfg.labels_fedex_client_id),
            client_secret: Option::clone(&cfg.labels_fedex_client_secret),
            account: cfg.labels_fedex_account.clone(),
        },
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
        demo_limits: std::sync::Arc::new(passkey::RateBuckets::default()),
        draft_runs: std::sync::Arc::new(draft::DraftRuns::default()),
        config_cache: std::sync::Arc::new(config::CachedReader::default()),
        google_cfg: match (&cfg.google_client_id, &cfg.google_client_secret) {
            (Some(id), Some(secret)) => Some(oauth::GoogleCfg::from_values(
                id,
                secret.clone(),
                &format!("{pub_origin}/auth/google/callback"),
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
                    &format!("{pub_origin}/auth/apple/callback"),
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
                &format!("{pub_origin}/auth/meta/callback"),
            )),
            _ => {
                eprintln!(
                    "WARNING: config meta.client_id/meta.client_secret unset — Meta login disabled"
                );
                None
            }
        },
        mailer: magic::mailer_from_config(&cfg, std::path::Path::new(store_dir.as_str())),
        magic_link_base: format!("{pub_origin}/apostille/login"),
        notify_poke: std::sync::Arc::new(tokio::sync::Notify::new()),
        notify_enabled: cfg.notify_enabled,
        notify_daily_cap: cfg.notify_daily_cap,
        notify_postal: cfg.notify_postal_address.clone(),
        notify_limits: std::sync::Arc::new(passkey::RateBuckets::default()),
        notify_base: format!("{pub_origin}/apostille"),
        meta_delete_status_base: format!("{pub_origin}/auth/meta/delete-status"),
        fakes: cfg.modes,
    };
    // F2: the effective allowlist lands in the boot log at INFO, so
    // "which ranges are live" never needs a config dump to answer.
    // An explicitly-set loopback value reads the same as unset: in
    // both cases only loopback reaches the admin branch.
    let allow_summary = if state.admin_allow_ips == config::loopback_allow_ips() {
        "loopback only (unset)".to_string()
    } else {
        state
            .admin_allow_ips
            .iter()
            .map(std::string::ToString::to_string)
            .collect::<Vec<_>>()
            .join(", ")
    };
    tracing::info!(admin_allow_ips = %allow_summary, "admin.allow_ips effective");
    // Follower notifications (phase 1 email): in-process worker
    // (the ACME-renewal precedent), poke-woken plus a 60 s tick for
    // backoff rows. Disabled by default; sends need the master
    // switch, a mailer, and the operator's postal footer.
    follow::spawn_worker(
        follow::WorkerCfg {
            store_root: state.store_root.clone(),
            enabled: state.notify_enabled,
            daily_cap: state.notify_daily_cap,
            postal: state.notify_postal.clone(),
            mailer: state.mailer.clone(),
            kek: state.kek.clone(),
            base: state.notify_base.clone(),
            origin: pub_origin.clone(),
        },
        state.notify_poke.clone(),
    );
    // MISSION WIN lanes (B0): order reconciler + tracking poller
    // (each off unless its config switch is on).
    reconcile::spawn(&state);
    tracking::spawn(&state);
    customer_labels::spawn(&state);
    // P403: changed web captures draft from their added lines only.
    source_history::spawn(&state);
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
    // Guardrails land AFTER the merges: admin, workers, voice
    // staff and webhook share one concurrency ceiling and one
    // per-path budget (the media upload's 300 s survives because
    // the budget is per-path, not per-router).
    let admin = guardrails(
        admin::router(state.clone())
            .merge(admin_workers::router(state.clone()))
            .merge(fedex_live::router(state.clone()))
            .merge(admin_restart::router(state.clone()))
            .merge(admin_voice::staff_router(state.clone()))
            .merge(admin_voice::webhook_router(state.clone()))
            .merge(mission_admin_routers(&state)),
    )
    .layer(axum::middleware::from_fn(mtls::inject_verified_cert));
    // Explicit files always win: ACME stays off when the
    // operator pinned a certificate.
    match explicit_tls_pair(cfg.tls_cert.clone(), cfg.tls_key.clone()) {
        Ok(Some((cert_pem, key_pem))) => {
            warn_unless_admin_san(&cert_pem);
            match load_admin_tls_from_pem(&cert_pem, &key_pem) {
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
    let admin_tls = load_admin_tls_from_pem(&pair.cert_pem, &pair.key_pem).unwrap_or_else(|e| {
        eprintln!("refusing to start: {e}");
        std::process::exit(1);
    });
    let quic_reloader = quic::QuicReloader::default();
    let loop_admin_tls = admin_tls.clone();
    let loop_quic = quic_reloader.clone();
    let loop_quic_enabled = cfg.quic_enabled;
    tokio::spawn(async move {
        acme::renewal_loop(config, |cert, key| {
            let admin_tls = loop_admin_tls.clone();
            let loop_quic = loop_quic.clone();
            async move {
                reload_admin_tls(&admin_tls, &cert, &key)?;
                // QUIC serves the same pair: swap its crypto too so a
                // renewal never leaves h3 on a stale certificate.
                if loop_quic_enabled {
                    loop_quic.reload(&cert, &key, None)?;
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
                    let mut server = axum_server::bind(addr).handle(handle);
                    // Slowloris guard: the head must arrive fast. The
                    // timer is mandatory — hyper panics on a timeout
                    // without one.
                    server
                        .http_builder()
                        .http1()
                        .timer(hyper_util::rt::TokioTimer::new())
                        .header_read_timeout(std::time::Duration::from_secs(
                            HEADER_READ_TIMEOUT_SECS,
                        ));
                    server
                        .serve(router.into_make_service_with_connect_info::<SocketAddr>())
                        .await
                }
            });
        }
    }
    supervise(servers, handles, Vec::new()).await;
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
/// `net.https_port` (no separate admin port), UDP QUIC on the same port iff
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
/// Port 444 is gone: admin is Host-routed on the same listener.
///
/// The single TCP listener uses the ADMIN TLS shape for every
/// connection (no client-cert request; peer-chain capture via
/// [`mtls::MtlsAcceptor`] stays inert); the fingerprint layer runs
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
    // The leaf `/readyz` watches (both explicit and ACME pairs
    // arrive here; renewals re-record via `reload_admin_tls`).
    readyz::record_serving_leaf(&cert_pem);
    let router = tls_router(public, admin, cfg);
    let mut servers = tokio::task::JoinSet::new();
    let mut handles = Vec::new();
    let mut quic_drain = Vec::new();
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
                eprintln!("listening on https://{addr}/apostille (single-443 public+admin)");
                servers.spawn({
                    let router = router
                        .clone()
                        .layer(axum::middleware::from_fn(insert_tcp_transport));
                    let tls = admin_tls.clone();
                    async move {
                        let mut server = axum_server::bind(addr)
                            .acceptor(mtls::MtlsAcceptor::new(tls))
                            .handle(handle);
                        server
                            .http_builder()
                            .http1()
                            .timer(hyper_util::rt::TokioTimer::new())
                            .header_read_timeout(std::time::Duration::from_secs(
                                HEADER_READ_TIMEOUT_SECS,
                            ));
                        server
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
                let endpoint = bind_quic_listener(addr, &cert_pem, &key_pem);
                quic_reloader.register(&endpoint);
                quic_drain.push(endpoint.clone());
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
    supervise(servers, handles, quic_drain).await;
}

/// Bind one QUIC UDP endpoint for [`serve_tls`]: build the h3
/// crypto from the serving pair (re-reading the CA file, like the
/// TCP config) and bind. Refuses the boot on any failure — QUIC
/// misconfiguration must never silently halve the service.
fn bind_quic_listener(addr: SocketAddr, cert_pem: &[u8], key_pem: &[u8]) -> quinn::Endpoint {
    let config = quic::server_config(cert_pem, key_pem, None).unwrap_or_else(|e| {
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
/// Drain every listener: TCP handles get a 10s graceful stop,
/// QUIC endpoints get an immediate close (their accept loop ends
/// only on close — without this the serve task never completes
/// and the process hangs until SIGKILL). Module-level so the
/// regression test drives the production path directly.
fn drain(handles: &[axum_server::Handle], quic: &[quinn::Endpoint]) {
    eprintln!("shutting down");
    for handle in handles {
        handle.graceful_shutdown(Some(std::time::Duration::from_secs(10)));
    }
    for endpoint in quic {
        endpoint.close(0u32.into(), b"shutdown");
    }
}

async fn supervise(
    mut servers: tokio::task::JoinSet<std::io::Result<()>>,
    handles: Vec<axum_server::Handle>,
    quic: Vec<quinn::Endpoint>,
) -> ! {
    let ctrl_c = tokio::signal::ctrl_c();
    tokio::pin!(ctrl_c);
    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .expect("install SIGTERM handler");
    let mut draining = false;
    let mut failed = false;
    // Staff restart (P393): same drain, then exit 75 so systemd's
    // `Restart=on-failure` starts the new process.
    let mut exit_code = 0;
    loop {
        tokio::select! {
            _ = &mut ctrl_c, if !draining => {
                drain(&handles, &quic);
                draining = true;
            }
            _ = admin_restart::requested(), if !draining => {
                drain(&handles, &quic);
                draining = true;
                exit_code = admin_restart::EXIT_CODE;
            }
            _ = sigterm.recv(), if !draining => {
                drain(&handles, &quic);
                draining = true;
            }
            outcome = servers.join_next() => {
                match outcome {
                    None => std::process::exit(if failed { 1 } else { exit_code }),
                    Some(Ok(Ok(()))) => {}
                    Some(Ok(Err(e))) => {
                        eprintln!("listener failed: {e}");
                        failed = true;
                        if !draining {
                            drain(&handles, &quic);
                            draining = true;
                        }
                    }
                    Some(Err(e)) => {
                        eprintln!("listener task failed: {e}");
                        failed = true;
                        if !draining {
                            drain(&handles, &quic);
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
    use axum::BoxError;
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
        // `ConnectInfo` is a required extractor (same as the precheck
        // handler): oneshot requests carry it as an extension.
        Request::builder()
            .uri("/apostille/api/v1/demo/login")
            .method("POST")
            .header("content-type", "application/json")
            .extension(demo_addr())
            .body(Body::from(
                serde_json::json!({"customer_ref": customer_ref}).to_string(),
            ))
            .unwrap()
    }

    /// `/readyz` fails closed in tests (no serving leaf recorded,
    /// no KEK, no worker beats — statics stay unset outside boot),
    /// but still names every check honestly.
    #[tokio::test]
    async fn readyz_fails_closed_outside_boot() {
        let fx = fixture();
        let res = public_router_for(&fx.state)
            .oneshot(
                Request::builder()
                    .uri("/apostille/readyz")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);
        let body = axum::body::to_bytes(res.into_body(), 4096).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["status"], "not-ready");
        assert_eq!(v["checks"]["database"], true, "fixture DB opens");
        assert_eq!(v["checks"]["tls_certificate"], false, "no leaf recorded");
        assert_eq!(v["checks"]["kek"], true, "fixture mints a kek");
        assert_eq!(v["checks"]["timers"]["notify"], false, "no beats in tests");
    }

    #[tokio::test]
    async fn notify_status_reports_master_switch() {
        let fx = fixture();
        let res = public_router_for(&fx.state)
            .oneshot(
                Request::builder()
                    .uri("/apostille/api/v1/notify/status")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = axum::body::to_bytes(res.into_body(), 1024).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["enabled"], false, "fixtures run with notify off");
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
    async fn domain_root_redirects_to_handbook() {
        // `andrii.ai/` is not the handbook (a bare 404 today): 307 to
        // `/apostille/`, keeping the method for form replays.
        let res = test_app()
            .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::TEMPORARY_REDIRECT);
        assert_eq!(res.headers().get(header::LOCATION).unwrap(), "/apostille/");
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
    async fn me_without_cookie_is_anonymous() {
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
        assert_eq!(res.status(), StatusCode::OK);
        let body = axum::body::to_bytes(res.into_body(), 1024).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v, serde_json::json!({"user": null}));
    }

    #[tokio::test]
    async fn me_with_bogus_cookie_is_unauthorized() {
        // A presented-but-unknown token stays 401; only the absence of
        // a credential answers 200 {"user": null}.
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
    async fn auth_providers_lists_configured_only() {
        // Fixture state configures no OAuth provider: the list is empty
        // (the login page renders no provider buttons then).
        let fx = fixture();
        let res = public_router_for(&fx.state)
            .oneshot(
                Request::builder()
                    .uri("/apostille/api/v1/auth/providers")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(json_body(res).await, serde_json::json!({"providers": []}));
    }

    fn demo_addr() -> axum::extract::ConnectInfo<std::net::SocketAddr> {
        axum::extract::ConnectInfo("127.0.0.1:9".parse().unwrap())
    }

    fn demo_run_request(sample: &str) -> Request<Body> {
        demo_run_request_from(sample, demo_addr())
    }

    /// The precheck limiter is process-global: a test that drains a
    /// bucket uses its own peer address.
    fn demo_run_request_from(
        sample: &str,
        addr: axum::extract::ConnectInfo<std::net::SocketAddr>,
    ) -> Request<Body> {
        Request::builder()
            .uri("/apostille/api/v1/demo/precheck")
            .method("POST")
            .header("content-type", "application/json")
            .extension(addr)
            .body(Body::from(
                serde_json::json!({"sample": sample}).to_string(),
            ))
            .unwrap()
    }

    #[tokio::test]
    async fn demo_precheck_samples_lists_three() {
        let fx = fixture();
        let res = public_router_for(&fx.state)
            .oneshot(
                Request::builder()
                    .uri("/apostille/api/v1/demo/precheck/samples")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = json_body(res).await;
        let samples = body["samples"].as_array().unwrap();
        assert_eq!(samples.len(), 3);
        for s in samples {
            assert!(s["id"].as_str().is_some());
            assert!(s["label"].as_str().is_some());
            let thumb = s["thumb_url"].as_str().unwrap();
            assert!(thumb.starts_with("/apostille/demo/syn-doc-"));
            assert!(thumb.ends_with(".png"));
        }
    }

    #[tokio::test]
    async fn demo_precheck_rejects_unknown_sample() {
        let fx = fixture();
        let res = session_app(&fx.state.store_root)
            .oneshot(demo_run_request("nope"))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[tokio::test]
    async fn demo_precheck_unrecorded_is_unavailable() {
        let fx = fixture();
        let res = session_app(&fx.state.store_root)
            .oneshot(demo_run_request("syn-doc-clean"))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn demo_precheck_serves_recording_and_limits() {
        let fx = fixture();
        let own = || axum::extract::ConnectInfo("127.0.0.42:9".parse().unwrap());
        let dir = fx.state.store_root.join("demo_precheck");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("syn-doc-clean.json"),
            serde_json::json!({
                "verdict": "pass",
                "issues": [],
                "boxes": [{"label": "document page", "x": 0.08, "y": 0.06, "w": 0.84, "h": 0.88}],
                "models": ["sam-3.1", "muse-spark-1.3"],
                "recorded_at": "2026-10-30T00:00:00Z",
            })
            .to_string(),
        )
        .unwrap();
        let app = session_app(&fx.state.store_root);
        let res = app
            .clone()
            .oneshot(demo_run_request_from("syn-doc-clean", own()))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = json_body(res).await;
        assert_eq!(body["verdict"], "pass");
        assert_eq!(body["sample"], "syn-doc-clean");
        assert_eq!(
            body["models"],
            serde_json::json!(["sam-3.1", "muse-spark-1.3"])
        );
        assert_eq!(body["boxes"].as_array().unwrap().len(), 1);
        // 60 runs/min per network: 59 more pass, the 61st is limited.
        for _ in 0..59 {
            let res = app
                .clone()
                .oneshot(demo_run_request_from("syn-doc-clean", own()))
                .await
                .unwrap();
            assert_eq!(res.status(), StatusCode::OK);
        }
        let res = app
            .oneshot(demo_run_request_from("syn-doc-clean", own()))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::TOO_MANY_REQUESTS);
    }

    #[tokio::test]
    async fn quality_missing_is_pending_not_error() {
        // Always 200: the home page probes this on every load, so a
        // missing run must not log a console error. The pending body
        // still fails the A5 contract (no run_at, gold_count 0).
        let fx = fixture();
        let res = public_router_for(&fx.state)
            .oneshot(
                Request::builder()
                    .uri("/apostille/api/v1/quality")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = json_body(res).await;
        assert_eq!(body["status"], "no-quality-run");
        assert!(body["run_at"].is_null());
        assert_eq!(body["gold_count"], 0);
    }

    #[tokio::test]
    async fn quality_serves_eval_summary() {
        let fx = fixture();
        std::fs::write(
            fx.state.store_root.join("quality.json"),
            serde_json::json!({
                "run_at": "2026-10-30T13:00:00Z",
                "gold_count": 26,
                "quote_found_rate": 0.875,
                "value_in_quote_rate": 0.75,
                "topic_accuracy": 0.9,
                "model": "muse-spark-1.3",
                "drafted_total": 40,
            })
            .to_string(),
        )
        .unwrap();
        let res = public_router_for(&fx.state)
            .oneshot(
                Request::builder()
                    .uri("/apostille/api/v1/quality")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = json_body(res).await;
        assert_eq!(body["gold_count"], 26);
        assert_eq!(body["quote_found_rate"], 0.875);
        assert_eq!(body["model"], "muse-spark-1.3");
    }

    #[tokio::test]
    async fn demo_login_rejects_bad_ref() {
        let tmp = tempfile::tempdir().unwrap();
        for customer_ref in [String::new(), "x".repeat(65), "real-acct-7".to_string()] {
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
                    .extension(demo_addr())
                    .body(Body::from(
                        serde_json::json!({"customer_ref": "demo-1", "admin": true}).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[test]
    fn demo_session_cookie_secure_flag() {
        // H4: dev (plain HTTP loopback) omits Secure; strict sets it.
        let dev = demo_session_cookie("abc", false);
        assert!(dev.starts_with("demo_session=abc"));
        assert!(dev.contains("HttpOnly"));
        assert!(!dev.contains("Secure"));
        let prod = demo_session_cookie("abc", true);
        assert!(prod.contains("; Secure"));
    }

    #[test]
    fn security_headers_branch_values() {
        // M2: admin denies framing outright; public allows same-origin.
        let admin: std::collections::HashMap<_, _> =
            security_headers("admin").into_iter().collect();
        let public: std::collections::HashMap<_, _> =
            security_headers("public").into_iter().collect();
        for map in [&admin, &public] {
            assert_eq!(map[&header::X_CONTENT_TYPE_OPTIONS], "nosniff");
            assert_eq!(map[&header::REFERRER_POLICY], "no-referrer");
            assert_eq!(
                map[&header::STRICT_TRANSPORT_SECURITY],
                "max-age=63072000; includeSubDomains"
            );
        }
        assert_eq!(admin[&header::X_FRAME_OPTIONS], "DENY");
        assert_eq!(public[&header::X_FRAME_OPTIONS], "SAMEORIGIN");
    }

    #[tokio::test]
    async fn security_headers_on_public_responses() {
        // M2: the middleware stamps every public response, including
        // static fallbacks (not just JSON APIs).
        for uri in ["/apostille/healthz", "/apostille/no-such-page"] {
            let res = test_app()
                .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
                .await
                .unwrap();
            let headers = res.headers();
            assert_eq!(headers["x-content-type-options"], "nosniff");
            assert_eq!(headers["referrer-policy"], "no-referrer");
            assert_eq!(headers["x-frame-options"], "SAMEORIGIN");
            assert!(
                headers.contains_key("strict-transport-security"),
                "uri: {uri}"
            );
        }
    }

    fn login_request_from(ip: &str, customer_ref: &str) -> Request<Body> {
        Request::builder()
            .uri("/apostille/api/v1/demo/login")
            .method("POST")
            .header("content-type", "application/json")
            .extension(ConnectInfo(ip.parse::<SocketAddr>().unwrap()))
            .body(Body::from(
                serde_json::json!({"customer_ref": customer_ref}).to_string(),
            ))
            .unwrap()
    }

    #[tokio::test]
    async fn demo_login_limits_per_ip() {
        // M1/H4: 10 demo mints/min per IP; the eleventh is limited.
        let tmp = tempfile::tempdir().unwrap();
        let app = session_app(tmp.path());
        for _ in 0..10 {
            let res = app
                .clone()
                .oneshot(login_request_from("127.0.0.1:9", "demo-1"))
                .await
                .unwrap();
            assert_eq!(res.status(), StatusCode::OK);
        }
        let res = app
            .oneshot(login_request_from("127.0.0.1:9", "demo-1"))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::TOO_MANY_REQUESTS);
    }

    #[tokio::test]
    async fn ws_origin_gate_live() {
        // H1 over a real socket (oneshot has no upgrade support, so the
        // extractor 426s before the handler): a cross-site page riding
        // ambient cookies is denied, while native clients (no Origin)
        // and anonymous public reads are unaffected.
        use futures_util::StreamExt as _;

        let fx = fixture();
        make_session(&fx.state.store_root, "staff-1", auth::Role::Staff);
        {
            let store = Store::new(&fx.state.store_root);
            let conn = db::open(&store).unwrap();
            db::add_authority(
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
        }

        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(
            axum_server::from_tcp(listener).serve(
                public_router_for(&fx.state)
                    .into_make_service_with_connect_info::<std::net::SocketAddr>(),
            ),
        );

        async fn ws_connect(
            addr: &std::net::SocketAddr,
            topic: &str,
            cookie: Option<String>,
            origin: Option<&str>,
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
            if let Some(origin) = origin {
                builder = builder.with_header("origin", origin);
            }
            tokio_tungstenite::connect_async(builder).await
        }

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

        // Valid staff credential, disallowlisted Origin: 403 (without
        // the gate this exact request upgrades to a hello).
        let denied = ws_connect(
            &addr,
            "call:0123456789abcdef0123456789abcdef",
            Some(format!("{}=staff-1", auth::ADM_COOKIE)),
            Some("https://evil.test"),
        )
        .await;
        match denied {
            Err(tokio_tungstenite::tungstenite::Error::Http(res)) => {
                assert_eq!(res.status(), 403);
            }
            other => panic!("expected HTTP 403, got {other:?}"),
        }

        // Same credential from an allowlisted dev Origin: still 403
        // (M-A: the public branch ignores the staff cookie, so the
        // caller is anonymous and the staff topic is forbidden — the
        // hello lives on the admin branch, covered below).
        let denied = ws_connect(
            &addr,
            "call:0123456789abcdef0123456789abcdef",
            Some(format!("{}=staff-1", auth::ADM_COOKIE)),
            Some("http://localhost:3000"),
        )
        .await;
        match denied {
            Err(tokio_tungstenite::tungstenite::Error::Http(res)) => {
                assert_eq!(res.status(), 403);
            }
            other => panic!("expected HTTP 403, got {other:?}"),
        }

        // Anonymous public read from a cross-site Origin: unaffected
        // (no cookies to ride — public data, by design).
        let (mut anon, _) = ws_connect(
            &addr,
            "authority:usa-wa-sos",
            None,
            Some("https://evil.test"),
        )
        .await
        .unwrap();
        let hello: serde_json::Value = serde_json::from_str(&next_text(&mut anon).await).unwrap();
        assert_eq!(hello["topic"], "authority:usa-wa-sos");
        anon.close(None).await.unwrap();

        server.abort();
    }

    #[tokio::test]
    async fn admin_version_is_staff_only_and_names_the_build() {
        // B2 env badge: staff read the build + env label; no session
        // is refused, and the public host never serves the route.
        let fx = fixture();
        make_session(&fx.state.store_root, "staff-1", auth::Role::Staff);
        let get = |session: Option<&'static str>| {
            admin_request(
                "GET",
                "/apostille/api/v1/admin/version",
                Some("admin.andrii.ai"),
                session,
                None,
                None,
                None,
            )
        };
        let res = admin_router_for(&fx.state)
            .oneshot(get(Some("staff-1")))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(res.into_body(), 1 << 16)
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(v["version"], FULL_VERSION);
        assert_eq!(v["schema"], db::SCHEMA_VERSION);
        assert!(matches!(
            v["env"].as_str(),
            Some("local" | "staging" | "judge")
        ));
        let res = admin_router_for(&fx.state)
            .oneshot(get(None))
            .await
            .unwrap();
        assert!(matches!(
            res.status(),
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN
        ));
    }

    #[tokio::test]
    async fn lane_admin_routes_sit_behind_the_admin_guard_stack() {
        // B3 stats + M1 model-calls: staff on the admin host read them;
        // a wrong Host is refused by the guard layer, no session is
        // refused by AdminAuth, and the public router never serves them.
        let fx = fixture();
        make_session(&fx.state.store_root, "staff-1", auth::Role::Staff);
        for uri in [
            "/apostille/api/v1/admin/stats",
            "/apostille/api/v1/admin/model-calls",
            "/apostille/api/v1/admin/orders",
            "/apostille/api/v1/admin/plans",
            "/apostille/api/v1/admin/offices",
            "/apostille/api/v1/admin/form-maps",
            "/apostille/api/v1/admin/fact-checks?slug=usa-wa-sos",
            "/apostille/api/v1/admin/authorities",
            "/apostille/api/v1/admin/first-source/emails",
            "/apostille/api/v1/admin/fake/outbox",
            "/apostille/api/v1/admin/compile",
        ] {
            let get = |host: &'static str, session: Option<&'static str>| {
                admin_request("GET", uri, Some(host), session, None, None, None)
            };
            let full = || admin::router(fx.state.clone()).merge(mission_admin_routers(&fx.state));
            let res = full()
                .oneshot(get("admin.andrii.ai", Some("staff-1")))
                .await
                .unwrap();
            assert_eq!(res.status(), StatusCode::OK, "{uri}");
            let res = full()
                .oneshot(get("evil.test", Some("staff-1")))
                .await
                .unwrap();
            assert!(res.status().is_client_error(), "{uri}: wrong host refused");
            let res = full().oneshot(get("admin.andrii.ai", None)).await.unwrap();
            assert!(
                matches!(
                    res.status(),
                    StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN
                ),
                "{uri}: no session refused"
            );
            let res = public_router_for(&fx.state)
                .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(res.status(), StatusCode::NOT_FOUND, "{uri}: never public");
        }
    }

    #[tokio::test]
    async fn public_plan_read_is_always_200_and_model_free() {
        // M2: the dossier fetches /plan on every load, so an unknown slug
        // or a slug without an accepted plan reads as `plan: null` (a 404
        // would be a console error on GP2-page).
        let fx = fixture();
        for slug in ["usa-wa-sos", "no-such-authority"] {
            let res = public_router_for(&fx.state)
                .oneshot(
                    Request::builder()
                        .uri(format!("/apostille/api/v1/authorities/{slug}/plan"))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(res.status(), StatusCode::OK, "{slug}");
            let bytes = axum::body::to_bytes(res.into_body(), 1 << 20)
                .await
                .unwrap();
            let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(v["slug"], slug);
            assert!(v["plan"].is_null());
            for key in ["facts", "offices", "forms"] {
                assert!(v[key].is_array(), "{slug}: {key}");
            }
        }
    }

    #[tokio::test]
    async fn doomed_compose_runs_answer_synchronously() {
        // A run that cannot start answers 404/422 at once — never a 202
        // whose `*.failed` event fires before the console subscribes.
        let fx = fixture();
        make_session(&fx.state.store_root, "staff-1", auth::Role::Staff);
        {
            let conn = db::open(&Store::new(&fx.state.store_root)).unwrap();
            db::add_authority(
                &conn,
                &db::NewAuthority {
                    kind: "state",
                    country: "USA",
                    region: "TS",
                    name: "Empty SOS",
                    slug: "empty-sos",
                    actor: "t",
                    at: 1,
                },
            )
            .unwrap();
        }
        for (uri, slug, want) in [
            (
                "/apostille/api/v1/admin/plans/compose",
                "no-such",
                StatusCode::NOT_FOUND,
            ),
            (
                "/apostille/api/v1/admin/plans/compose",
                "empty-sos",
                StatusCode::UNPROCESSABLE_ENTITY,
            ),
            (
                "/apostille/api/v1/admin/offices/compose",
                "no-such",
                StatusCode::NOT_FOUND,
            ),
            (
                "/apostille/api/v1/admin/offices/compose",
                "empty-sos",
                StatusCode::UNPROCESSABLE_ENTITY,
            ),
        ] {
            let res = admin::router(fx.state.clone())
                .merge(mission_admin_routers(&fx.state))
                .oneshot(admin_request(
                    "POST",
                    uri,
                    Some("admin.andrii.ai"),
                    Some("staff-1"),
                    Some("csrf-token"),
                    Some("https://admin.andrii.ai"),
                    Some(serde_json::json!({"slug": slug})),
                ))
                .await
                .unwrap();
            assert_eq!(res.status(), want, "{uri} {slug}");
        }
    }

    /// Seal one config value in the fixture store and drop the cache.
    fn seal(fx: &Fixture, key: &str, key_type: config::ConfigType, value: &[u8]) {
        let conn = db::open(&Store::new(&fx.state.store_root)).unwrap();
        config::set_value(
            &conn,
            fx.state.kek.as_deref().unwrap(),
            &config::SetValue {
                version: 1,
                key,
                key_type,
                plaintext: value,
                now: unix_now(),
                actor: "test",
            },
        )
        .unwrap();
        fx.state.config_cache.invalidate();
    }

    fn add_test_authority(fx: &Fixture, slug: &str) -> i64 {
        let conn = db::open(&Store::new(&fx.state.store_root)).unwrap();
        db::add_authority(
            &conn,
            &db::NewAuthority {
                kind: "state",
                country: "USA",
                region: "TS",
                name: "Test SOS",
                slug,
                actor: "t",
                at: 1,
            },
        )
        .unwrap();
        conn.query_row("SELECT id FROM authority WHERE slug = ?1", [slug], |r| {
            r.get(0)
        })
        .unwrap()
    }

    #[tokio::test]
    async fn compile_runs_walk_the_pipeline_under_the_daily_cap() {
        // Never bill a real model from a unit test.
        if std::env::var("META_API_KEY").is_ok_and(|k| !k.is_empty()) {
            return;
        }
        let fx = fixture();
        make_session(&fx.state.store_root, "staff-1", auth::Role::Staff);
        add_test_authority(&fx, "compile-sos");
        let post = |body: serde_json::Value| {
            admin::router(fx.state.clone())
                .merge(mission_admin_routers(&fx.state))
                .oneshot(admin_request(
                    "POST",
                    "/apostille/api/v1/admin/compile",
                    Some("admin.andrii.ai"),
                    Some("staff-1"),
                    Some("csrf-token"),
                    Some("https://admin.andrii.ai"),
                    Some(body),
                ))
        };
        let res = post(serde_json::json!({"slug": "no-such"})).await.unwrap();
        assert_eq!(res.status(), StatusCode::NOT_FOUND);
        let res = post(serde_json::json!({"slug": "compile-sos", "seed_urls": ["http://x.test/"]}))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);

        seal(
            &fx,
            config::keys::COMPILE_DAILY_RUNS,
            config::ConfigType::Text,
            b"1",
        );
        let res = post(serde_json::json!({"slug": "compile-sos"}))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::ACCEPTED);
        let v: serde_json::Value = serde_json::from_str(&body_text(res).await).unwrap();
        let run_id = v["run_id"].as_i64().unwrap();
        assert_eq!(v["topic"], format!("run:{run_id}"));

        // Nothing to capture, draft or compose: every step is a skip and
        // the run still finishes as done, with its summary stored.
        let mut row = (String::new(), String::new());
        for _ in 0..200 {
            let conn = db::open(&Store::new(&fx.state.store_root)).unwrap();
            row = conn
                .query_row(
                    "SELECT status, summary_json FROM compile_run WHERE id = ?1",
                    [run_id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .unwrap();
            if row.0 != "running" {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        assert_eq!(row.0, "done", "summary: {}", row.1);
        let summary: serde_json::Value = serde_json::from_str(&row.1).unwrap();
        assert_eq!(summary["capture"]["pages"], 0);
        assert_eq!(summary["plan"]["status"], "skipped");

        // compile.daily_runs = 1: the second run today is refused.
        let res = post(serde_json::json!({"slug": "compile-sos"}))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::TOO_MANY_REQUESTS);

        // A running row blocks a second run for the same authority.
        seal(
            &fx,
            config::keys::COMPILE_DAILY_RUNS,
            config::ConfigType::Text,
            b"5",
        );
        {
            let conn = db::open(&Store::new(&fx.state.store_root)).unwrap();
            conn.execute(
                "INSERT INTO compile_run(id, slug, actor, status, started_at)
                 VALUES (900001, 'compile-sos', 'staff-1', 'running', ?1)",
                [unix_now()],
            )
            .unwrap();
        }
        let res = post(serde_json::json!({"slug": "compile-sos"}))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::CONFLICT);

        // compile.daily_runs = 0 stops every run.
        seal(
            &fx,
            config::keys::COMPILE_DAILY_RUNS,
            config::ConfigType::Text,
            b"0",
        );
        let res = post(serde_json::json!({"slug": "compile-sos"}))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::TOO_MANY_REQUESTS);
    }

    #[tokio::test]
    async fn auto_compose_is_off_by_default_and_coalesces_per_slug() {
        if std::env::var("META_API_KEY").is_ok_and(|k| !k.is_empty()) {
            return;
        }
        let fx = fixture();
        let authority = add_test_authority(&fx, "auto-sos");
        {
            let conn = db::open(&Store::new(&fx.state.store_root)).unwrap();
            conn.execute(
                "INSERT INTO capture(authority_id, captured_at, kind, path,
                                      bytes_raw, bytes_stored, sha256, trust)
                 VALUES (?1, 1761000000, 'file', 'p', 0, 0, 's', 'staff_verified')",
                [authority],
            )
            .unwrap();
            let cap = conn.last_insert_rowid();
            for topic in ["hours", "fees", "mailing"] {
                conn.execute(
                    "INSERT INTO fact(authority_id, capture_id, topic, statement,
                                       status, created_by, created_at,
                                       citation_start, citation_end)
                     VALUES (?1, ?2, ?3, 'synthetic', 'accepted', 't', 1, 0, 9)",
                    rusqlite::params![authority, cap, topic],
                )
                .unwrap();
            }
        }
        let mut ops = fx.state.bus.subscribe("ops");
        // Off by default: nothing starts.
        plan::auto_compose_after_accept(&fx.state, "auto-sos");
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert!(ops.try_recv().is_err(), "auto-compose ran while off");

        // On: three accepts in a burst run one compose plus one
        // follow-up (the burst folds), never three.
        seal(
            &fx,
            config::keys::PLAN_AUTO_COMPOSE,
            config::ConfigType::Bool,
            b"1",
        );
        for _ in 0..3 {
            plan::auto_compose_after_accept(&fx.state, "auto-sos");
        }
        let mut started = 0;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while std::time::Instant::now() < deadline {
            match tokio::time::timeout(std::time::Duration::from_millis(200), ops.recv()).await {
                Ok(Ok(msg)) => {
                    let v: serde_json::Value = serde_json::from_str(&msg).unwrap();
                    if v["type"] == "plan_auto_compose" {
                        assert_eq!(v["slug"], "auto-sos");
                        started += 1;
                    }
                }
                _ if started >= 2 => break,
                _ => {}
            }
        }
        assert_eq!(started, 2, "one run plus one folded follow-up");
    }

    /// A carrier link whose buy answers are scripted, counting calls.
    struct ScriptedLink {
        answers: std::cell::RefCell<Vec<Result<labels::BuyOutcome, labels::LabelError>>>,
        buys: std::cell::Cell<usize>,
    }

    impl customer_labels::LabelLink for ScriptedLink {
        fn carrier(&self) -> &'static str {
            "fedex"
        }
        fn quote(
            &self,
            _cfg: &labels::live::LiveConfig,
            _request: &labels::BuyRequest,
        ) -> Result<i64, String> {
            Ok(1_250)
        }
        fn buy(
            &self,
            _request: &labels::BuyRequest,
        ) -> Result<labels::BuyOutcome, labels::LabelError> {
            self.buys.set(self.buys.get() + 1);
            self.answers.borrow_mut().remove(0)
        }
        fn cancel(&self, _tracking_no: &str) -> Result<(), labels::LabelError> {
            Ok(())
        }
        fn payment_ref(&self) -> &str {
            "000000000"
        }
    }

    /// One paid FedEx label order waiting for the buyer.
    fn paid_label_order(fx: &Fixture) -> String {
        let p2 = db::open_p2(&Store::new(&fx.state.store_root)).unwrap();
        let dest = orders::destination(&[orders::FactText {
            id: 204,
            topic: "mailing".into(),
            text: "Mail to 801 Capitol Way S, Olympia, WA 98501.".into(),
        }])
        .unwrap();
        let body = orders::CreateBody {
            slug: "usa-wa-sos".into(),
            kind: "label_fedex".into(),
            path_id: None,
            form_fill_id: None,
            return_to: orders::ReturnTo {
                name: "SYNTHETIC Customer".into(),
                line1: "1 Example St".into(),
                line2: None,
                city: "Springfield".into(),
                region: "IL".into(),
                postal_code: "62701".into(),
                country: "US".into(),
                phone: Some("+12175550100".into()),
            },
        };
        let id = orders::create_order(&p2, "cust-1", (7, "S"), &body, &dest, "test", 1).unwrap();
        let quotes = serde_json::json!([
            {"leg": "outbound", "carrier": "fedex", "cents": 1250},
            {"leg": "return", "carrier": "fedex", "cents": 1250},
        ]);
        p2.execute(
            "UPDATE customer_order SET status = 'label_purchasing',
                    amount_json = json_object('total_cents', 2500, 'carrier_quotes', json(?1))
             WHERE id = ?2",
            rusqlite::params![quotes.to_string(), id],
        )
        .unwrap();
        id
    }

    fn order_status(fx: &Fixture, id: &str) -> String {
        let p2 = db::open_p2(&Store::new(&fx.state.store_root)).unwrap();
        p2.query_row(
            "SELECT status FROM customer_order WHERE id = ?1",
            [id],
            |r| r.get(0),
        )
        .unwrap()
    }

    fn ledger(fx: &Fixture, id: &str) -> Vec<(String, String)> {
        let p2 = db::open_p2(&Store::new(&fx.state.store_root)).unwrap();
        let mut stmt = p2
            .prepare("SELECT leg, status FROM label_purchase WHERE order_id = ?1 ORDER BY leg")
            .unwrap();
        stmt.query_map([id], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap()
    }

    #[tokio::test]
    async fn fake_labels_buy_a_paid_test_order_with_demo_pdfs_and_never_for_live_money() {
        // W-3 `labels.mode = fake`: no `labels.customer_live`, no FedEx
        // credentials — the real buyer and ledger run against the demo
        // carrier, and the customer's cabinet serves a stamped PDF.
        let mut fx = fixture();
        fx.state.fakes =
            config::Modes::default().with(config::Integration::Labels, config::Mode::Fake);
        let meta = db::open(&Store::new(&fx.state.store_root)).unwrap();
        assert!(customer_labels::ready(
            &fx.state,
            &meta,
            "label_fedex",
            false
        ));
        assert!(customer_labels::ready(
            &fx.state,
            &meta,
            "label_usps",
            false
        ));
        assert!(
            !customer_labels::ready(&fx.state, &meta, "label_fedex", true),
            "demo labels are never sold for live money"
        );
        drop(meta);
        let id = paid_label_order(&fx);
        let demo = |state: &AppState, meta: &rusqlite::Connection, kind: &str, live: bool| {
            customer_labels::connect(state, meta, kind, live, 0)
        };
        assert_eq!(
            customer_labels::buy_order(&fx.state, &id, &demo),
            customer_labels::BuyEnd::Ready
        );
        assert_eq!(order_status(&fx, &id), "labels_ready");
        let p2 = db::open_p2(&Store::new(&fx.state.store_root)).unwrap();
        let numbers: Vec<String> = p2
            .prepare("SELECT tracking_number FROM order_shipment WHERE order_id = ?1")
            .unwrap()
            .query_map([&id], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        drop(p2);
        assert_eq!(numbers.len(), 2);
        assert!(
            numbers.iter().all(|n| labels::demo::is_demo_tracking(n)),
            "{numbers:?}"
        );
        make_session(&fx.state.store_root, "cust-1", auth::Role::Customer);
        let res = public_router_for(&fx.state)
            .oneshot(
                Request::builder()
                    .uri(format!("/apostille/api/v1/p2/orders/{id}/labels/outbound"))
                    .header("cookie", format!("{}=cust-1", auth::PUB_COOKIE))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let pdf = body_text(res).await;
        assert!(pdf.starts_with("%PDF-"));
        assert!(pdf.contains("(DEMO \\227 NOT POSTAGE)"), "stamped");
        assert!(pdf.contains("98501"), "addressed to the office");
        // A real-money order never reaches the demo carrier.
        assert!(
            customer_labels::connect(
                &fx.state,
                &db::open(&Store::new(&fx.state.store_root)).unwrap(),
                "label_fedex",
                true,
                0
            )
            .is_err()
        );
    }

    #[tokio::test]
    async fn fake_stripe_checkout_pays_through_the_real_signed_webhook_with_a_demo_receipt() {
        // W-3 `stripe.mode = fake`: no key, no network. The DEMO
        // checkout's Pay button delivers a signed event to the real
        // webhook handler, which verifies it and settles against the
        // fake session; the receipt is marked DEMO.
        let mut fx = fixture();
        fx.state.fakes =
            config::Modes::default().with(config::Integration::Stripe, config::Mode::Fake);
        let id = paid_label_order(&fx);
        let p2 = db::open_p2(&Store::new(&fx.state.store_root)).unwrap();
        p2.execute(
            "UPDATE customer_order SET status = 'awaiting_payment' WHERE id = ?1",
            [&id],
        )
        .unwrap();
        let meta = db::open(&Store::new(&fx.state.store_root)).unwrap();
        let keys = stripe::keys(&fx.state, &meta).expect("fake keys need no sealed key");
        assert!(!keys.live());
        let (session, url) = stripe::create_checkout(
            &keys,
            &stripe::CheckoutRequest {
                order_id: id.clone(),
                expires_at: unix_now() + 1800,
                bucket: 0,
                currency: "usd".into(),
                lines: vec![
                    ("FedEx 2Day label to the office".into(), 1250),
                    ("FedEx 2Day return label to you".into(), 1250),
                ],
                success_url: format!("https://andrii.ai/apostille/orders/{id}?checkout=success"),
                cancel_url: format!("https://andrii.ai/apostille/order?order={id}"),
            },
        )
        .unwrap();
        assert!(session.starts_with(stripe::FAKE_SESSION_PREFIX));
        assert_eq!(
            url,
            format!("/apostille/api/v1/stripe/fake/checkout/{session}")
        );
        p2.execute(
            "INSERT INTO order_payment(order_id, session_id, checkout_url, amount_cents, currency, livemode, status, created_at)
             VALUES (?1, ?2, ?3, 2500, 'usd', 0, 'open', 1)",
            rusqlite::params![id, session, url],
        )
        .unwrap();
        let get = |uri: String| Request::builder().uri(uri).body(Body::empty()).unwrap();
        let res = public_router_for(&fx.state)
            .oneshot(get(url.clone()))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let page = body_text(res).await;
        assert!(page.contains("DEMO — no card is charged"), "{page}");
        assert!(page.contains("Pay 25.00 USD (demo)"), "{page}");
        // A forged event (wrong secret) is refused by the real handler.
        let forged = r#"{"id":"evt_x","type":"checkout.session.completed","data":{"object":{"id":"cs_fake_x"}}}"#;
        let res = public_router_for(&fx.state)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/apostille/api/v1/stripe/webhook")
                    .header(
                        "stripe-signature",
                        format!("t={},v1={}", unix_now(), "00".repeat(32)),
                    )
                    .body(Body::from(forged))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
        // Pay: a same-origin form post.
        let res = public_router_for(&fx.state)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("{url}/pay"))
                    .header("origin", "https://andrii.ai")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::SEE_OTHER);
        assert_eq!(
            res.headers()[header::LOCATION],
            format!("/apostille/orders/{id}?checkout=success").as_str()
        );
        let (pay_status, intent): (String, String) = p2
            .query_row(
                "SELECT status, payment_intent FROM order_payment WHERE order_id = ?1",
                [&id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(pay_status, "paid");
        assert!(intent.starts_with("pi_fake_"), "{intent}");
        let event: String = p2
            .query_row("SELECT type FROM stripe_event", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            event, "checkout.session.completed",
            "the real webhook recorded it"
        );
        let paid_text: String = p2
            .query_row(
                "SELECT text FROM order_event WHERE order_id = ?1 AND kind = 'order.paid'",
                [&id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(paid_text, "Paid $25.00 (DEMO, no card charged)");
        drop(p2);
        // The owner's order shows a DEMO receipt link; the page says DEMO.
        make_session(&fx.state.store_root, "cust-1", auth::Role::Customer);
        let res = public_router_for(&fx.state)
            .oneshot(
                Request::builder()
                    .uri(format!("/apostille/api/v1/p2/orders/{id}"))
                    .header("cookie", format!("{}=cust-1", auth::PUB_COOKIE))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let order: serde_json::Value = serde_json::from_str(&body_text(res).await).unwrap();
        let receipt = &order["receipt"];
        assert_eq!(receipt["demo"], true, "{order}");
        let receipt_url = receipt["receipt_url"].as_str().unwrap().to_string();
        let res = public_router_for(&fx.state)
            .oneshot(get(receipt_url))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        assert!(body_text(res).await.contains("DEMO receipt"));
        // Refunds move nothing either.
        assert!(
            stripe::refund(&keys, &id, &intent)
                .unwrap()
                .starts_with("re_fake_")
        );
        // Without the fake, the fake pages do not exist.
        let res = public_router_for(&fixture().state)
            .oneshot(get(url))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn customer_labels_buy_each_leg_once_and_fail_safe() {
        let fx = fixture();
        seal(
            &fx,
            config::keys::LABELS_CUSTOMER_LIVE,
            config::ConfigType::Bool,
            b"1",
        );

        // 1. Both legs buy over the recorded FedEx sandbox answers.
        let id = paid_label_order(&fx);
        let sandbox = |_: &AppState, _: &rusqlite::Connection, _: &str, _: bool| {
            Ok(Box::new(customer_labels::FedexLink {
                base: labels::FEDEX_BASE_SANDBOX.to_string(),
                bearer: "synthetic".to_string(),
                account: "000000000".to_string(),
                transport: labels::live::SandboxTransport::new(
                    labels::live::FixtureSet::synthetic(),
                ),
            }) as Box<dyn customer_labels::LabelLink>)
        };
        assert_eq!(
            customer_labels::buy_order(&fx.state, &id, &sandbox),
            customer_labels::BuyEnd::Ready
        );
        assert_eq!(order_status(&fx, &id), "labels_ready");
        assert_eq!(
            ledger(&fx, &id),
            vec![
                ("outbound".to_string(), "bought".to_string()),
                ("return".to_string(), "bought".to_string())
            ]
        );
        // A second pass never buys again.
        assert_eq!(
            customer_labels::buy_order(&fx.state, &id, &sandbox),
            customer_labels::BuyEnd::Skipped
        );
        // The owner downloads the PDF; nobody else does.
        let label = |who: &str| {
            Request::builder()
                .uri(format!("/apostille/api/v1/p2/orders/{id}/labels/return"))
                .header("cookie", format!("{}={who}", auth::PUB_COOKIE))
                .body(Body::empty())
                .unwrap()
        };
        make_session(&fx.state.store_root, "cust-1", auth::Role::Customer);
        let res = public_router_for(&fx.state)
            .oneshot(label("cust-1"))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        assert!(body_text(res).await.starts_with("%PDF-"));

        // 2. The carrier refuses the shipment (400): nothing is retried;
        //    with no Stripe key configured the refund goes to staff.
        let id = paid_label_order(&fx);
        let refusing = ScriptedLink {
            answers: std::cell::RefCell::new(vec![Err(labels::LabelError::Api {
                status: 400,
                message: "invalid recipient".into(),
            })]),
            buys: std::cell::Cell::new(0),
        };
        let end = customer_labels::buy_order(&fx.state, &id, &|_, _, _, _| {
            Ok(Box::new(ScriptedLink {
                answers: std::cell::RefCell::new(refusing.answers.borrow_mut().drain(..).collect()),
                buys: std::cell::Cell::new(0),
            }) as Box<dyn customer_labels::LabelLink>)
        });
        assert!(matches!(end, customer_labels::BuyEnd::Staff(_)), "{end:?}");
        assert_eq!(order_status(&fx, &id), "needs_staff");
        assert_eq!(
            ledger(&fx, &id),
            vec![("outbound".to_string(), "failed".to_string())]
        );

        // 3. A timeout may have bought: `unknown`, staff, and a later pass
        //    (staff sent it back) never calls the carrier again.
        let id = paid_label_order(&fx);
        let calls = std::rc::Rc::new(std::cell::Cell::new(0usize));
        let timeouts = {
            let calls = calls.clone();
            move |_: &AppState, _: &rusqlite::Connection, _: &str, _: bool| {
                calls.set(calls.get() + 1);
                Ok(Box::new(ScriptedLink {
                    answers: std::cell::RefCell::new(vec![Err(labels::LabelError::Transport(
                        labels::TransportError::new("timed out"),
                    ))]),
                    buys: std::cell::Cell::new(0),
                }) as Box<dyn customer_labels::LabelLink>)
            }
        };
        let end = customer_labels::buy_order(&fx.state, &id, &timeouts);
        assert!(matches!(end, customer_labels::BuyEnd::Staff(_)), "{end:?}");
        assert_eq!(
            ledger(&fx, &id),
            vec![("outbound".to_string(), "unknown".to_string())]
        );
        {
            let p2 = db::open_p2(&Store::new(&fx.state.store_root)).unwrap();
            p2.execute(
                "UPDATE customer_order SET status = 'label_purchasing' WHERE id = ?1",
                [&id],
            )
            .unwrap();
        }
        assert_eq!(
            customer_labels::buy_order(&fx.state, &id, &timeouts),
            customer_labels::BuyEnd::Staff("A label purchase needs a check by our team.")
        );
        assert_eq!(calls.get(), 1, "the unknown leg is never re-bought");

        // 4. The daily cap counts every buy that may have happened: 2 bought
        //    + 1 unknown today, so a cap of 3 sends the next order to staff
        //    before any carrier call.
        seal(
            &fx,
            config::keys::LABELS_CUSTOMER_DAILY_CAP,
            config::ConfigType::Text,
            b"3",
        );
        let id = paid_label_order(&fx);
        let end = customer_labels::buy_order(&fx.state, &id, &|_, _, _, _| {
            panic!("no carrier call past the daily cap")
        });
        assert!(matches!(end, customer_labels::BuyEnd::Staff(_)), "{end:?}");
        assert!(ledger(&fx, &id).is_empty());

        // 5. The switch off: staff, no ledger row.
        seal(
            &fx,
            config::keys::LABELS_CUSTOMER_DAILY_CAP,
            config::ConfigType::Text,
            b"10",
        );
        seal(
            &fx,
            config::keys::LABELS_CUSTOMER_LIVE,
            config::ConfigType::Bool,
            b"0",
        );
        let id = paid_label_order(&fx);
        let end = customer_labels::buy_order(&fx.state, &id, &|_, _, _, _| {
            panic!("no carrier call while customer buys are off")
        });
        assert!(matches!(end, customer_labels::BuyEnd::Staff(_)), "{end:?}");
        assert!(ledger(&fx, &id).is_empty());
    }

    #[tokio::test]
    async fn tracking_moves_orders_on_the_carriers_word_and_stops_when_delivered() {
        use labels::track::{TrackingScan, TrackingStatus};
        let fx = fixture();
        seal(
            &fx,
            config::keys::LABELS_CUSTOMER_LIVE,
            config::ConfigType::Bool,
            b"1",
        );
        let id = paid_label_order(&fx);
        let sandbox = |_: &AppState, _: &rusqlite::Connection, _: &str, _: bool| {
            Ok(Box::new(customer_labels::FedexLink {
                base: labels::FEDEX_BASE_SANDBOX.to_string(),
                bearer: "synthetic".to_string(),
                account: "000000000".to_string(),
                transport: labels::live::SandboxTransport::new(
                    labels::live::FixtureSet::synthetic(),
                ),
            }) as Box<dyn customer_labels::LabelLink>)
        };
        assert_eq!(
            customer_labels::buy_order(&fx.state, &id, &sandbox),
            customer_labels::BuyEnd::Ready
        );
        {
            // The recorded fixture reuses one number; give each leg its own.
            let p2 = db::open_p2(&Store::new(&fx.state.store_root)).unwrap();
            p2.execute(
                "UPDATE order_shipment SET tracking_number = 'SYNTH-' || role WHERE order_id = ?1",
                [&id],
            )
            .unwrap();
        }
        let mut billing = fx.state.bus.subscribe("billing:cust-1");
        let scan = |code: &str, text: &str, at: &str| TrackingScan {
            code: code.into(),
            text: text.into(),
            at: at.into(),
            place: "OLYMPIA, WA".into(),
        };
        let calls = std::cell::RefCell::new(Vec::<String>::new());
        let moving = |_: &str, tn: &str| -> Result<tracking::Fetched, String> {
            calls.borrow_mut().push(tn.to_string());
            Ok((
                Some(TrackingStatus::InTransit),
                vec![scan("IT", "In transit", "2026-11-04T08:00:00")],
            ))
        };
        let now = 1_900_000_000;
        assert_eq!(tracking::poll_once(&fx.state, now, &moving), 2);
        assert_eq!(order_status(&fx, &id), "in_transit");
        let msg: serde_json::Value =
            serde_json::from_str(&billing.try_recv().expect("tracking.update")).unwrap();
        assert_eq!(msg["type"], "tracking.update");
        assert_eq!(msg["order_id"], id);
        // Polled again too soon: nothing is fetched.
        assert_eq!(tracking::poll_once(&fx.state, now + 60, &moving), 0);
        assert_eq!(calls.borrow().len(), 2);
        // An hour later the same scans are duplicates: no new events.
        assert_eq!(tracking::poll_once(&fx.state, now + 3_700, &moving), 0);
        // The return leg delivered: the order is done, and a delivered
        // number is never polled again.
        let delivered = |_: &str, tn: &str| -> Result<tracking::Fetched, String> {
            calls.borrow_mut().push(tn.to_string());
            let status = if tn.ends_with("return") {
                TrackingStatus::Delivered
            } else {
                TrackingStatus::InTransit
            };
            Ok((
                Some(status),
                vec![scan("DL", "Delivered", "2026-11-06T10:00:00")],
            ))
        };
        assert_eq!(tracking::poll_once(&fx.state, now + 7_400, &delivered), 2);
        assert_eq!(order_status(&fx, &id), "delivered");
        let before = calls.borrow().len();
        tracking::poll_once(&fx.state, now + 20_000, &delivered);
        assert!(
            calls.borrow()[before..].is_empty(),
            "a delivered order is never polled again"
        );
        // A carrier error is recorded and skipped, never fatal.
        let id2 = paid_label_order(&fx);
        assert_eq!(
            customer_labels::buy_order(&fx.state, &id2, &sandbox),
            customer_labels::BuyEnd::Ready
        );
        {
            let p2 = db::open_p2(&Store::new(&fx.state.store_root)).unwrap();
            p2.execute(
                "UPDATE order_shipment SET tracking_number = 'SYNTH2-' || role WHERE order_id = ?1",
                [&id2],
            )
            .unwrap();
        }
        let failing =
            |_: &str, _: &str| -> Result<tracking::Fetched, String> { Err("http 503".into()) };
        assert_eq!(tracking::poll_once(&fx.state, now + 30_000, &failing), 0);
        assert_eq!(order_status(&fx, &id2), "labels_ready");
    }

    #[tokio::test]
    async fn staff_advance_never_moves_money_states() {
        // Review #12/#28: advance is for fulfilment; paid/refunded/
        // canceled-after-payment go through settle and refund only.
        let fx = fixture();
        make_session(&fx.state.store_root, "staff-1", auth::Role::Staff);
        let store = Store::new(&fx.state.store_root);
        let p2 = db::open_p2(&store).unwrap();
        let dest = orders::destination(&[orders::FactText {
            id: 203,
            topic: "po_box".into(),
            text: "Send to PO Box 4000, Olympia, WA 98504.".into(),
        }])
        .unwrap();
        let body = |kind: &str| orders::CreateBody {
            slug: "usa-wa-sos".into(),
            kind: kind.into(),
            path_id: None,
            form_fill_id: None,
            return_to: orders::ReturnTo {
                name: "SYNTHETIC Customer".into(),
                line1: "1 Example St".into(),
                line2: None,
                city: "Springfield".into(),
                region: "IL".into(),
                postal_code: "62701".into(),
                country: "US".into(),
                phone: None,
            },
        };
        let free = orders::create_order(
            &p2,
            "cust-1",
            (7, "S"),
            &body("address_label"),
            &dest,
            "test",
            1,
        )
        .unwrap();
        let kit =
            orders::create_order(&p2, "cust-1", (7, "S"), &body("mail_kit"), &dest, "test", 1)
                .unwrap();
        let advance = |id: &str, to: &str| {
            admin_request(
                "POST",
                &format!("/apostille/api/v1/admin/orders/{id}/advance"),
                Some("admin.andrii.ai"),
                Some("staff-1"),
                Some("csrf-token"),
                Some("https://admin.andrii.ai"),
                Some(serde_json::json!({"to": to})),
            )
        };
        let full = || admin::router(fx.state.clone()).merge(mission_admin_routers(&fx.state));
        for (id, to, want) in [
            (&free, "refunded", StatusCode::CONFLICT),
            (&free, "paid", StatusCode::CONFLICT),
            (&kit, "labels_ready", StatusCode::CONFLICT), // unpaid kit
            (&kit, "canceled", StatusCode::OK),           // unpaid → may cancel
            (&free, "in_transit", StatusCode::OK),        // free label, customer mails it
        ] {
            let res = full().oneshot(advance(id, to)).await.unwrap();
            assert_eq!(res.status(), want, "{id} → {to}");
        }
    }

    #[tokio::test]
    async fn order_create_rejects_bad_ids_before_any_work() {
        // Review #16: path_id and form_fill_id are validated (and a fill
        // must belong to the caller) before an order row exists.
        let fx = fixture();
        make_session(&fx.state.store_root, "cust-sess", auth::Role::Customer);
        for (path, fill) in [
            (serde_json::json!("x".repeat(500)), serde_json::Value::Null),
            (serde_json::json!("Bad Path!"), serde_json::Value::Null),
            (serde_json::Value::Null, serde_json::json!("fil_../../etc")),
        ] {
            let body = serde_json::json!({
                "slug": "usa-wa-sos", "kind": "address_label", "path_id": path, "form_fill_id": fill,
                "return_to": {"name": "A", "line1": "1 A St", "city": "B", "region": "WA", "postal_code": "98501"}
            });
            let req = Request::builder()
                .method("POST")
                .uri("/apostille/api/v1/p2/orders")
                .header("origin", "https://andrii.ai")
                .header("content-type", "application/json")
                .header(auth::CSRF_HEADER, "tok")
                .header(
                    "cookie",
                    format!("{}=cust-sess; {}=tok", auth::PUB_COOKIE, auth::CSRF_COOKIE),
                )
                .body(Body::from(body.to_string()))
                .unwrap();
            let res = public_router_for(&fx.state).oneshot(req).await.unwrap();
            assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY, "{body}");
        }
    }

    #[tokio::test]
    async fn publish_rejects_oversized_body() {
        // M1: arbitrary-JSON fan-out is capped; normal events flow.
        let fx = fixture();
        make_session(&fx.state.store_root, "staff-1", auth::Role::Staff);
        let publish = |body: serde_json::Value| {
            admin_request(
                "POST",
                "/apostille/api/v1/bus/publish",
                Some("admin.andrii.ai"),
                Some("staff-1"),
                Some("csrf-token"),
                Some("http://localhost:3001"),
                Some(serde_json::json!({
                    "topic": "call:0123456789abcdef0123456789abcdef",
                    "kind": "note",
                    "body": body,
                })),
            )
        };
        let res = admin_router_for(&fx.state)
            .oneshot(publish(serde_json::json!({"text": "hello"})))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let big = "x".repeat(20_000);
        let res = admin_router_for(&fx.state)
            .oneshot(publish(serde_json::json!({"text": big})))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::PAYLOAD_TOO_LARGE);
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

    /// B2: with `auth.origin=https://admin.andrii.ai` (staff WebAuthn)
    /// and `net.public_host=andrii.ai`, every customer-facing URL the
    /// state build composes must land on the public host — never the
    /// IP-gated admin host.
    #[test]
    fn public_urls_use_public_host() {
        let pub_origin = public_origin("andrii.ai");
        assert_eq!(pub_origin, "https://andrii.ai");
        for url in [
            format!("{pub_origin}/auth/google/callback"),
            format!("{pub_origin}/auth/apple/callback"),
            format!("{pub_origin}/auth/meta/callback"),
            format!("{pub_origin}/auth/meta/delete-status"),
            format!("{pub_origin}/apostille/login"),
            format!("{pub_origin}/apostille"),
            format!("{pub_origin}/apostille/authority/vietnam"),
        ] {
            assert!(
                url.starts_with("https://andrii.ai/"),
                "public URL on admin host: {url}"
            );
            assert!(!url.contains("admin.andrii.ai"), "admin leak: {url}");
        }
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

    #[tokio::test(flavor = "multi_thread", worker_threads = 8)]
    async fn concurrent_requests_log_exactly_once_with_span_context() {
        // The response event must keep its span (id/route/transport)
        // even when the runtime migrates the task across threads
        // mid-request: `enter()` + await loses the span, so the
        // middleware must `instrument()` the whole tail instead.
        // Uses the global default: thread-local capture would drop
        // events emitted on other workers.
        let app = Router::new()
            .route(
                "/burst",
                get(|| async {
                    tokio::time::sleep(std::time::Duration::from_millis(2)).await;
                    tokio::task::yield_now().await;
                    "ok"
                }),
            )
            .layer(axum::middleware::from_fn_with_state(
                "public",
                request_id_and_log,
            ));
        let logs = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let subscriber = tracing_subscriber::fmt()
            .json()
            .with_writer({
                let logs = logs.clone();
                move || BufWriter(logs.clone())
            })
            .finish();
        tracing::subscriber::set_global_default(subscriber).expect("global default already set");
        // Independent tasks (not one join_all): the work-stealer
        // spreads them across workers, migrating futures mid-await
        // the way a real connection burst does.
        let mut handles = Vec::new();
        for _ in 0..128 {
            let app = app.clone();
            handles.push(tokio::spawn(async move {
                app.oneshot(
                    Request::builder()
                        .uri("/burst")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap()
            }));
        }
        for handle in handles {
            assert_eq!(handle.await.unwrap().status(), StatusCode::OK);
        }
        let text = String::from_utf8(logs.lock().unwrap().clone()).unwrap();
        // Only this test serves /burst; a response event that lost
        // its span has no route and drops out of the count.
        let contextual = text
            .lines()
            .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
            .filter(|event| {
                event["fields"]["message"] == "response" && event["span"]["route"] == "/burst"
            })
            .count();
        // Exactly one contextual event per request: fewer means the
        // span was lost to thread migration, more means double-logging.
        assert_eq!(contextual, 128, "got: {text}");
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
        // Lane DDL runs at boot; the fixture mirrors it so lane routes
        // test against the same tables production has.
        migrate_mission_tables(&conn).unwrap();
        drop(conn);
        if let Ok(p2) = db::open_p2(&store) {
            let _ = db::migrate_p2(&p2);
            migrate_mission_p2(&p2).unwrap();
        }
        let state = AppState {
            spa_index: PathBuf::from("test-spa/index.html"),
            static_dir: PathBuf::from("static"),
            store_root: tmp.path().to_path_buf(),
            bus: ws_bus::Bus::new(),
            kek: Some(std::sync::Arc::new(config::Kek::generate())),
            admin_mtls_off: true,
            admin_allow_ips: config::loopback_allow_ips(),
            admin_spa_dir: tmp.path().join("admin-spa"),
            media_terms_read: false,
            label_cfg: p2_labels::LabelCfg::default(),
            voice_cfg: voice::VoiceConfig::default(),
            voice_api_key: None,
            fedex_live: fedex_live::FedexLiveState::default(),
            webauthn: std::sync::Arc::new(passkey::test_webauthn()),
            auth_limits: std::sync::Arc::new(passkey::RateBuckets::default()),
            demo_limits: std::sync::Arc::new(passkey::RateBuckets::default()),
            draft_runs: std::sync::Arc::new(draft::DraftRuns::default()),
            config_cache: std::sync::Arc::new(config::CachedReader::default()),
            google_cfg: None,
            apple_cfg: None,
            meta_cfg: None,
            mailer: None,
            magic_link_base: "https://example.test/apostille/login".to_string(),
            notify_poke: std::sync::Arc::new(tokio::sync::Notify::new()),
            notify_enabled: false,
            notify_daily_cap: 3,
            notify_postal: String::new(),
            notify_limits: std::sync::Arc::new(passkey::RateBuckets::default()),
            notify_base: "https://example.test/apostille".to_string(),
            meta_delete_status_base: "https://example.test/auth/meta/delete-status".to_string(),
            fakes: config::Modes::default(),
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
    /// Unfurl dossier: accepted fact with a hostile statement, so
    /// the crawler surface proves its escaping end to end.
    fn seed_unfurl_dossier(store_root: &std::path::Path) {
        let store = Store::new(store_root);
        let conn = db::open(&store).unwrap();
        let authority = db::add_authority(
            &conn,
            &db::NewAuthority {
                kind: "state",
                country: "USA",
                region: "TS",
                name: "Test \"SOS\" & Co",
                slug: "unfurl-sos",
                actor: "t",
                at: 1,
            },
        )
        .unwrap();
        conn.execute(
            "INSERT INTO source(authority_id, kind, uri)
             VALUES (?1, 'website_official', 'https://example.test/office')",
            [authority],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO capture(authority_id, captured_at, kind, path,
                                  bytes_raw, bytes_stored, sha256, trust)
             VALUES (?1, 1761000000, 'file', 'p', 0, 0, 's', 'staff_verified')",
            [authority],
        )
        .unwrap();
        let cap = conn.last_insert_rowid();
        std::fs::create_dir_all(store_root.join("text")).unwrap();
        std::fs::write(
            store_root.join("text").join(format!("{cap}.txt")),
            "open 9 to 5 weekdays",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO fact(authority_id, capture_id, topic, statement,
                               status, created_by, created_at,
                               citation_start, citation_end)
             VALUES (?1, ?2, 'hours', 'open 9 to 5 </script>', 'accepted', 't', 1, 0, 20)",
            rusqlite::params![authority, cap],
        )
        .unwrap();
        // Accepted but staff-only: must never surface publicly.
        conn.execute(
            "INSERT INTO fact(authority_id, capture_id, topic, statement,
                               status, created_by, created_at,
                               citation_start, citation_end)
             VALUES (?1, ?2, 'phone_script', 'say hello, then ask', 'accepted', 't', 1, 0, 20)",
            rusqlite::params![authority, cap],
        )
        .unwrap();
    }

    async fn body_text(res: axum::response::Response) -> String {
        String::from_utf8(
            axum::body::to_bytes(res.into_body(), 1 << 20)
                .await
                .unwrap()
                .to_vec(),
        )
        .unwrap()
    }

    #[tokio::test]
    async fn unfurl_unknown_slug_serves_shell_as_404() {
        let fx = fixture();
        let body = body_text(
            public_router_for(&fx.state)
                .oneshot(
                    Request::builder()
                        .uri("/apostille/authority/nope")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap(),
        )
        .await;
        // Status asserted via a second identical request (body_text consumes).
        let res = public_router_for(&fx.state)
            .oneshot(
                Request::builder()
                    .uri("/apostille/authority/nope")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::NOT_FOUND);
        assert!(body.contains("spa-fixture"), "unknown slug keeps the shell");
    }

    #[tokio::test]
    async fn unfurl_dossier_escapes_and_lists_accepted_facts() {
        let fx = fixture();
        seed_unfurl_dossier(&fx.state.store_root);
        let res = public_router_for(&fx.state)
            .oneshot(
                Request::builder()
                    .uri("/apostille/authority/unfurl-sos")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(
            res.headers()["cache-control"],
            "public, max-age=300",
            "dossier edge-cached"
        );
        let body = body_text(res).await;
        assert!(
            body.contains("&lt;/script&gt;"),
            "hostile statement escaped"
        );
        assert!(!body.contains("open 9 to 5 </script>"), "no raw breakout");
        assert!(
            !body.contains("say hello, then ask"),
            "phone_script stays staff-only"
        );
        assert!(body.contains("<noscript>"), "crawler summary present");
        assert!(body.contains("/authority/unfurl-sos"), "canonical + links");
        assert!(body.contains("GovernmentService"), "json-ld present");
        assert!(
            body.contains("https://example.test/office"),
            "isBasedOn source"
        );
    }

    #[tokio::test]
    async fn outreach_email_is_drafted_reviewed_and_sent_with_our_message_id() {
        if std::env::var("META_API_KEY").is_ok_and(|k| !k.is_empty()) {
            eprintln!("skip: META_API_KEY is set; this test must not call the model");
            return;
        }
        let fx = fixture();
        make_session(&fx.state.store_root, "staff-1", auth::Role::Staff);
        let conn = db::open(&Store::new(&fx.state.store_root)).unwrap();
        db::add_authority(
            &conn,
            &db::NewAuthority {
                kind: "state",
                country: "USA",
                region: "New York",
                name: "Synthetic NY DoS",
                slug: "synthetic-ny-dos",
                actor: "t",
                at: 1,
            },
        )
        .unwrap();
        drop(conn);
        let mail_dir = fx.state.store_root.join("mail-out");
        std::fs::create_dir_all(&mail_dir).unwrap();
        let mut state = fx.state.clone();
        state.mailer = Some(magic::Mailer::File(magic::FileMailer {
            dir: mail_dir.clone(),
        }));
        let call = |method: &str, uri: &str, body: Option<serde_json::Value>| {
            admin_request(
                method,
                uri,
                Some("admin.andrii.ai"),
                Some("staff-1"),
                Some("tok"),
                Some("https://admin.andrii.ai"),
                body,
            )
        };
        let app = || admin::router(state.clone()).merge(mission_admin_routers(&state));
        let read = |res: axum::response::Response| async move {
            let status = res.status();
            let v: serde_json::Value = serde_json::from_str(&body_text(res).await).unwrap();
            (status, v)
        };

        // No model key here: the fixed template drafts, and says so.
        let (status, v) = read(
            app()
                .oneshot(call(
                    "POST",
                    "/apostille/api/v1/admin/first-source/emails",
                    Some(
                        json!({"slug": "synthetic-ny-dos", "to": "Apostille@Example.gov",
                                "questions": ["What is the fee per document?",
                                              "Which form should applicants use?"]}),
                    ),
                ))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{v}");
        let email = &v["email"];
        let id = email["id"].as_i64().unwrap();
        assert_eq!(email["status"], "draft");
        assert_eq!(email["model"], "template");
        assert_eq!(email["to"], "apostille@example.gov");
        assert!(
            email["body"]
                .as_str()
                .unwrap()
                .contains("1. What is the fee per document?\n2. Which form should applicants use?")
        );

        // Staff edit, then send: one file in the outbox with our Message-ID.
        let (status, v) = read(
            app()
                .oneshot(call(
                    "PUT",
                    &format!("/apostille/api/v1/admin/first-source/emails/{id}"),
                    Some(
                        json!({"to": "apostille@example.gov", "subject": "Two questions",
                                "body": "Hello,\n\n1. What is the fee per document?\n\nThanks."}),
                    ),
                ))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{v}");
        let (status, v) = read(
            app()
                .oneshot(call(
                    "POST",
                    &format!("/apostille/api/v1/admin/first-source/emails/{id}/send"),
                    None,
                ))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{v}");
        assert_eq!(v["email"]["status"], "sent");
        let files: Vec<_> = std::fs::read_dir(&mail_dir).unwrap().collect();
        assert_eq!(files.len(), 1);
        let sent = std::fs::read_to_string(files[0].as_ref().unwrap().path()).unwrap();
        assert!(sent.contains("Subject: Two questions"), "{sent}");
        assert!(
            sent.contains(&format!("Message-ID: <outreach-{id}-")),
            "{sent}"
        );

        // A sent email is final: no second send, no edit.
        let res = app()
            .oneshot(call(
                "POST",
                &format!("/apostille/api/v1/admin/first-source/emails/{id}/send"),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::CONFLICT);
        // The address we wrote to is now on file as a contact.
        let (_, v) = read(
            app()
                .oneshot(call(
                    "GET",
                    "/apostille/api/v1/admin/first-source/synthetic-ny-dos/contacts",
                    None,
                ))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(v["contacts"][0]["value"], "apostille@example.gov");
    }

    #[tokio::test]
    async fn fake_mail_loop_outbox_then_composed_reply_threads_with_dmarc_trust() {
        // W-3: with no Gmail and no SMTP, outreach lands in the fake
        // outbox, and a reply composed in the console is captured by the
        // poller's own step: DMARC decides the trust, In-Reply-To threads
        // it to the outreach email, and the capture waits for the
        // instruct worker like any polled mail.
        if std::env::var("META_API_KEY").is_ok_and(|k| !k.is_empty()) {
            eprintln!("skip: META_API_KEY is set; this test must not call the model");
            return;
        }
        let fx = fixture();
        make_session(&fx.state.store_root, "staff-1", auth::Role::Staff);
        let conn = db::open(&Store::new(&fx.state.store_root)).unwrap();
        let authority = db::add_authority(
            &conn,
            &db::NewAuthority {
                kind: "state",
                country: "USA",
                region: "New York",
                name: "Synthetic NY DoS",
                slug: "synthetic-ny-dos",
                actor: "t",
                at: 1,
            },
        )
        .unwrap();
        conn.execute(
            "INSERT INTO domain(name, authority_id, status) VALUES ('example.gov', ?1, 'official')",
            [authority],
        )
        .unwrap();
        drop(conn);
        let mut state = fx.state.clone();
        state.fakes = config::Modes::default()
            .with(config::Integration::MailOut, config::Mode::Fake)
            .with(config::Integration::MailIn, config::Mode::Fake);
        let cfg = config::RuntimeConfig {
            modes: state.fakes,
            mailer_provider: "smtp".to_string(),
            ..config::RuntimeConfig::default()
        };
        // `mailer.mode = fake` wins over the configured provider.
        state.mailer = magic::mailer_from_config(&cfg, &state.store_root);
        assert!(matches!(state.mailer, Some(magic::Mailer::Outbox(_))));
        let call = |method: &str, uri: &str, body: Option<serde_json::Value>| {
            admin_request(
                method,
                uri,
                Some("admin.andrii.ai"),
                Some("staff-1"),
                Some("tok"),
                Some("https://admin.andrii.ai"),
                body,
            )
        };
        let app =
            |state: &AppState| admin::router(state.clone()).merge(mission_admin_routers(state));
        let read = |res: axum::response::Response| async move {
            let status = res.status();
            let v: serde_json::Value = serde_json::from_str(&body_text(res).await).unwrap();
            (status, v)
        };
        let (status, v) = read(
            app(&state)
                .oneshot(call(
                    "POST",
                    "/apostille/api/v1/admin/first-source/emails",
                    Some(
                        json!({"slug": "synthetic-ny-dos", "to": "apostille@example.gov",
                                "questions": ["What is the fee per document?"]}),
                    ),
                ))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{v}");
        let id = v["email"]["id"].as_i64().unwrap();
        let (status, v) = read(
            app(&state)
                .oneshot(call(
                    "POST",
                    &format!("/apostille/api/v1/admin/first-source/emails/{id}/send"),
                    None,
                ))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{v}");
        assert_eq!(v["email"]["status"], "sent");
        // The outbox holds it, with our Message-ID; nothing was dialled.
        let (status, v) = read(
            app(&state)
                .oneshot(call("GET", "/apostille/api/v1/admin/fake/outbox", None))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{v}");
        assert_eq!(
            (v["outbox"].clone(), v["inbox"].clone()),
            (json!(true), json!(true))
        );
        let kept = &v["messages"][0];
        assert_eq!(kept["kind"], "outreach");
        assert_eq!(kept["to"], "apostille@example.gov");
        let message_id = kept["headers"]["Message-ID"].as_str().unwrap().to_string();
        assert!(
            message_id.starts_with(&format!("<outreach-{id}-")),
            "{message_id}"
        );
        // The office answers (DMARC pass): official trust, its own
        // authority, threaded to the outreach email.
        let reply = |dmarc: &str, body: &str| {
            json!({"from": "Apostille@Example.gov", "subject": "Re: your questions",
                   "body": body, "in_reply_to": message_id, "dmarc": dmarc})
        };
        let (status, v) = read(
            app(&state)
                .oneshot(call(
                    "POST",
                    "/apostille/api/v1/admin/fake/inbox",
                    Some(reply("pass", "The fee is $15 per document.")),
                ))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{v}");
        assert_eq!(v["trust"], "official");
        assert_eq!(v["verified_by"], "dmarc");
        assert_eq!(v["authority"], "synthetic-ny-dos");
        assert_eq!(v["outreach_email"], id);
        let capture_id = v["capture_id"].as_i64().unwrap();
        let conn = db::open(&Store::new(&state.store_root)).unwrap();
        let (status_now, linked): (String, i64) = conn
            .query_row(
                "SELECT status, reply_capture_id FROM outreach_email WHERE id = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!((status_now.as_str(), linked), ("replied", capture_id));
        let (kind, authenticity): (String, String) = conn
            .query_row(
                "SELECT kind, authenticity FROM capture WHERE id = ?1",
                [capture_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(kind, "email");
        assert!(
            authenticity.contains("\"received_via\":\"fake-inbox\""),
            "{authenticity}"
        );
        drop(conn);
        // DMARC fail from the same domain: kept, capped at community.
        let (status, v) = read(
            app(&state)
                .oneshot(call(
                    "POST",
                    "/apostille/api/v1/admin/fake/inbox",
                    Some(reply("fail", "Spoofed: the fee is $1.")),
                ))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{v}");
        assert_eq!(
            (v["trust"].clone(), v["verified_by"].clone()),
            (json!("community"), json!("none"))
        );
        // Bad input refuses; a boot without the fake inbox has none.
        let res = app(&state)
            .oneshot(call(
                "POST",
                "/apostille/api/v1/admin/fake/inbox",
                Some(reply("maybe", "x")),
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let res = app(&fx.state)
            .oneshot(call(
                "POST",
                "/apostille/api/v1/admin/fake/inbox",
                Some(reply("pass", "x")),
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn readable_addresses_serve_the_dossier_with_one_canonical_path() {
        let fx = fixture();
        let conn = db::open(&Store::new(&fx.state.store_root)).unwrap();
        db::add_authority(
            &conn,
            &db::NewAuthority {
                kind: "state",
                country: "USA",
                region: "Washington",
                name: "Synthetic WA SoS",
                slug: "synthetic-wa-sos",
                actor: "t",
                at: 1,
            },
        )
        .unwrap();
        drop(conn);
        for (uri, status) in [
            ("/apostille/authority/usa/washington", StatusCode::OK),
            ("/apostille/authority/us/wa", StatusCode::OK),
            ("/apostille/authority/synthetic-wa-sos", StatusCode::OK),
            ("/apostille/authority/usa/oregon", StatusCode::NOT_FOUND),
        ] {
            let res = public_router_for(&fx.state)
                .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(res.status(), status, "{uri}");
            let body = body_text(res).await;
            assert!(body.contains("spa-fixture"), "{uri}: the SPA shell");
            if status == StatusCode::OK {
                assert!(
                    body.contains("<link rel=\"canonical\" href=\"https://example.test/apostille/authority/usa/washington\""),
                    "{uri}: canonical is the primary path"
                );
            }
        }
        let res = public_router_for(&fx.state)
            .oneshot(
                Request::builder()
                    .uri("/apostille/api/v1/authorities/resolve?path=US/WA")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let v: serde_json::Value = serde_json::from_str(&body_text(res).await).unwrap();
        assert_eq!(v["slug"], "synthetic-wa-sos");
        assert_eq!(v["canonical"], "usa/washington");
        assert_eq!(v["crumbs"][0]["label"], "United States");
        let res = public_router_for(&fx.state)
            .oneshot(
                Request::builder()
                    .uri("/apostille/api/v1/authorities")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(&body_text(res).await).unwrap();
        let row = v["authorities"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["slug"] == "synthetic-wa-sos")
            .unwrap();
        assert_eq!(row["path"], "usa/washington");
        assert!(row["aliases"].as_array().unwrap().contains(&json!("us/wa")));
    }

    #[tokio::test]
    async fn public_facts_hide_phone_script() {
        let fx = fixture();
        seed_unfurl_dossier(&fx.state.store_root);
        let res = public_router_for(&fx.state)
            .oneshot(
                Request::builder()
                    .uri("/apostille/api/v1/authorities/unfurl-sos/facts")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let public = json_body(res).await;
        let facts = public["facts"].as_array().unwrap();
        assert_eq!(facts.len(), 1, "only the hours fact is public");
        assert!(facts.iter().all(|f| f["topic"] != "phone_script"));
    }

    #[tokio::test]
    async fn sitemap_lists_authorities_with_lastmod() {
        let fx = fixture();
        seed_unfurl_dossier(&fx.state.store_root);
        let res = public_router_for(&fx.state)
            .oneshot(
                Request::builder()
                    .uri("/apostille/sitemap.xml")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(
            res.headers()["content-type"],
            "application/xml; charset=utf-8"
        );
        assert_eq!(
            res.headers()["cache-control"],
            "public, max-age=300",
            "sitemap edge-cached"
        );
        let body = body_text(res).await;
        assert!(body.starts_with("<?xml"), "declaration first");
        assert!(!body.contains("sentinel"), "id-0 sentinel never listed");
        assert!(
            body.contains(
                "<loc>https://example.test/apostille/authority/unfurl-sos</loc><lastmod>"
            ),
            "authority row with lastmod, got: {body}"
        );
        assert!(body.ends_with("</urlset>\n"), "closed urlset");
    }

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
        // Official https source + sidecar: accepts require a verified
        // citation (quote 8-400 chars over https).
        conn.execute(
            "INSERT INTO source(authority_id, kind, uri)
             VALUES (?1, 'website_official', 'https://example.gov/office')",
            [authority],
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
        std::fs::create_dir_all(store_root.join("text")).unwrap();
        std::fs::write(
            store_root.join("text").join(format!("{cap}.txt")),
            "open 9 to 5; open 8 to 6 daily",
        )
        .unwrap();
        let mut ids = Vec::new();
        for (statement, end) in [("open 9 to 5", 11), ("open 8 to 6", 23)] {
            conn.execute(
                "INSERT INTO fact(authority_id, capture_id, topic, statement,
                                   status, created_by, created_at,
                                   citation_start, citation_end)
                 VALUES (?1, ?2, 'hours', ?3, 'proposed', 't', 1, 0, ?4)",
                rusqlite::params![authority, cap, statement, end],
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
        // Test peers are loopback (the fixture allowlist holds it).
        builder = builder.extension(ConnectInfo("127.0.0.1:443".parse::<SocketAddr>().unwrap()));
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
            ("usa-wa-sos", "WA", "USA", "Washington"),
            ("deu-berlin", "Berlin", "Germany", ""),
            ("fra-paris", "Paris", "France", ""),
        ];
        let mut ids = std::collections::HashMap::new();
        for (slug, name, country, region) in specs {
            let id = db::add_authority(
                &conn,
                &db::NewAuthority {
                    kind: "state",
                    country,
                    region,
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
        // The dossier CTA reads the region when present.
        assert_eq!(authorities[2]["region"], "Washington");
        assert_eq!(authorities[0]["region"], "");
        // No sentinel row, no other keys on the envelope.
        assert!(authorities.iter().all(|a| a["slug"] != ""));
        assert_eq!(
            body.as_object().unwrap().keys().collect::<Vec<_>>(),
            vec!["authorities"]
        );
    }

    #[tokio::test]
    async fn public_facts_carry_quote_verified_citations() {
        let fx = fixture();
        let store = Store::new(&fx.state.store_root);
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
            "INSERT INTO source(authority_id, kind, uri)
             VALUES (?1, 'website_official', 'https://www.sos.wa.gov/about-office/contact-us')",
            [authority],
        )
        .unwrap();
        let source_id = conn.last_insert_rowid();
        // 1800000050 -> 2027-01-15T08:00:50Z.
        conn.execute(
            "INSERT INTO capture(authority_id, source_id, captured_at, kind, path,
                                  bytes_raw, bytes_stored, sha256, trust)
             VALUES (?1, ?2, 1800000050, 'file', 'p', 56, 56, 's', 'official')",
            rusqlite::params![authority, source_id],
        )
        .unwrap();
        let cap = conn.last_insert_rowid();
        let text = "Walk-in service moved to Tumwater; mail to PO Box 40228.";
        std::fs::create_dir_all(fx.state.store_root.join("text")).unwrap();
        std::fs::write(
            fx.state.store_root.join("text").join(format!("{cap}.txt")),
            text,
        )
        .unwrap();
        // Cited accepted fact: quote is bytes [0..33).
        conn.execute(
            "INSERT INTO fact(authority_id, capture_id, topic, statement, status,
                               created_by, created_at, citation_start, citation_end)
             VALUES (?1, ?2, 'walkin', 'walkin: Walk-in service moved to Tumwater',
                     'accepted', 'muse-spark-1.3 via analysis-agent/aa-prompt-v1',
                     2, 0, 33)",
            rusqlite::params![authority, cap],
        )
        .unwrap();
        // Uncited accepted fact: honestly carries zero citations.
        conn.execute(
            "INSERT INTO fact(authority_id, capture_id, topic, statement, status,
                               created_by, created_at, citation_start, citation_end)
             VALUES (?1, 0, 'hours', 'weekday: Mon-Fri 9-5',
                     'accepted', 'muse-spark-1.3 via analysis-agent/aa-prompt-v1',
                     3, -1, -1)",
            [authority],
        )
        .unwrap();
        drop(conn);

        let res = public_router_for(&fx.state)
            .oneshot(
                Request::builder()
                    .uri("/apostille/api/v1/authorities/usa-wa-sos/facts")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let public = json_body(res).await;
        let facts = public["facts"].as_array().unwrap();
        assert_eq!(facts.len(), 2);
        let cited = &facts[0];
        assert!(cited["drafted_by"].as_str().unwrap().contains("muse-spark"));
        let citations = cited["citations"].as_array().unwrap();
        assert_eq!(citations.len(), 1);
        assert_eq!(citations[0]["quote"], "Walk-in service moved to Tumwater");
        assert_eq!(
            citations[0]["source_url"],
            "https://www.sos.wa.gov/about-office/contact-us"
        );
        assert_eq!(citations[0]["captured_at"], "2027-01-15T08:00:50Z");
        assert_eq!(facts[1]["citations"].as_array().unwrap().len(), 0);
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

        // Revoked dev cert is ignored → 200 (certs decide nothing now).
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
        assert_eq!(res.status(), StatusCode::OK);

        // Strict mode without a cert → 200 for a listed peer (fail open
        // on certs is gone; the IP gate decides).
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
        assert_eq!(res.status(), StatusCode::OK);
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
        let body = Some(
            serde_json::json!({"topic": "call:0123456789abcdef0123456789abcdef", "kind": "note", "body": {}}),
        );

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

        // Authority channels carry the accepted-content kinds only
        // (`ws_bus::AUTHORITY_KINDS`): a draft is refused …
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
        // … and an accepted page plan passes.
        let res = admin_router_for(&fx.state)
            .oneshot(admin_request(
                "POST",
                uri,
                Some("admin.andrii.ai"),
                Some("staff-1"),
                Some("csrf-token"),
                Some("https://admin.andrii.ai"),
                Some(serde_json::json!({"topic": "authority:x", "kind": "plan_accepted", "body": {}})),
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn fake_modes_show_on_the_badge_and_never_save_on_a_closed_guard() {
        // W-3: the version badge names every fake, and a host whose
        // guard is closed (public bind, not staging) refuses to save a
        // `fake` mode row; any other value saves as before.
        let mut fx = fixture();
        make_session(&fx.state.store_root, "staff-1", auth::Role::Staff);
        let conn = db::open(&Store::new(&fx.state.store_root)).unwrap();
        config::record_kek(&conn, 1, "test-cred", 100).unwrap();
        drop(conn);
        fx.state.fakes = config::Modes::default()
            .with(config::Integration::MailOut, config::Mode::Fake)
            .with(config::Integration::Labels, config::Mode::Fake)
            .with(config::Integration::Stripe, config::Mode::Sandbox);
        let res = admin_router_for(&fx.state)
            .oneshot(admin_request(
                "GET",
                "/apostille/api/v1/admin/version",
                Some("admin.andrii.ai"),
                Some("staff-1"),
                None,
                None,
                None,
            ))
            .await
            .unwrap();
        let v = json_body(res).await;
        assert_eq!(
            v["fakes"],
            serde_json::json!([
                {"slug": "mail_out", "label": "Outgoing email"},
                {"slug": "labels", "label": "FedEx/USPS labels"},
            ])
        );
        let put = |state: &AppState, value: &str| {
            admin_router_for(state).oneshot(admin_request(
                "PUT",
                "/apostille/api/v1/config/labels.mode",
                Some("admin.andrii.ai"),
                Some("staff-1"),
                Some("csrf-token"),
                Some("https://admin.andrii.ai"),
                Some(serde_json::json!({"type": "text", "value": value})),
            ))
        };
        assert!(!fx.state.fakes.fakes_allowed);
        let res = put(&fx.state, "fake").await.unwrap();
        assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert!(
            json_body(res).await["error"]
                .as_str()
                .unwrap()
                .contains("loopback")
        );
        assert_eq!(
            put(&fx.state, "sandbox").await.unwrap().status(),
            StatusCode::OK
        );
        fx.state.fakes.fakes_allowed = true;
        assert_eq!(
            put(&fx.state, " fake").await.unwrap().status(),
            StatusCode::OK
        );
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
        // P393: each row says when it was saved, and the list says when
        // this process started (the page's "restart needed" chip).
        assert!(by_key("mailer.host")["updated_at"].as_i64().unwrap() > 0);
        assert!(listed["started_at"].is_i64());

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
            .extension(ConnectInfo("127.0.0.1:443".parse::<SocketAddr>().unwrap()))
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
            .extension(ConnectInfo("127.0.0.1:443".parse::<SocketAddr>().unwrap()))
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
        // Anonymous authority subscribe requires a known slug.
        {
            let store = Store::new(&fx.state.store_root);
            let conn = db::open(&store).unwrap();
            db::add_authority(
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
        }

        // Real server on an ephemeral port (never a fixed one).
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(
            axum_server::from_tcp(listener).serve(
                public_router_for(&fx.state)
                    .into_make_service_with_connect_info::<std::net::SocketAddr>(),
            ),
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
        let denied = ws_connect(&addr, "call:0123456789abcdef0123456789abcdef", None).await;
        match denied {
            Err(tokio_tungstenite::tungstenite::Error::Http(res)) => {
                assert_eq!(res.status(), 403);
            }
            other => panic!("expected HTTP 403, got {other:?}"),
        }

        // Unknown authority slug: 422, no channel even anonymously.
        let denied = ws_connect(&addr, "authority:no-such-slug", None).await;
        match denied {
            Err(tokio_tungstenite::tungstenite::Error::Http(res)) => {
                assert_eq!(res.status(), 422);
            }
            other => panic!("expected HTTP 422, got {other:?}"),
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

        // Staff cookie on the public branch: 403 even for a live
        // staff session (M-A — the admin branch below upgrades it).
        let denied = ws_connect(
            &addr,
            "call:0123456789abcdef0123456789abcdef",
            Some(format!("{}=staff-1", auth::ADM_COOKIE)),
        )
        .await;
        match denied {
            Err(tokio_tungstenite::tungstenite::Error::Http(res)) => {
                assert_eq!(res.status(), 403);
            }
            other => panic!("expected HTTP 403, got {other:?}"),
        }

        server.abort();
    }

    #[tokio::test]
    async fn ws_admin_branch_upgrades_staff_topics() {
        // M-A: staff topics (`proposals`, `run:{id}`) upgrade on the
        // admin router with the staff cookie, and stay closed without
        // one. Served on an ephemeral port like the public-branch
        // tests above (loopback peer passes the fixture allowlist;
        // the Host guard admits loopback in `admin_mtls_off` dev).
        use futures_util::StreamExt as _;

        let fx = fixture();
        make_session(&fx.state.store_root, "staff-1", auth::Role::Staff);

        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(
            axum_server::from_tcp(listener).serve(
                crate::admin::router(fx.state.clone())
                    .into_make_service_with_connect_info::<std::net::SocketAddr>(),
            ),
        );

        async fn ws_connect(
            addr: &std::net::SocketAddr,
            topic: &str,
            cookie: Option<String>,
            origin: Option<String>,
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
            if let Some(origin) = origin {
                builder = builder.with_header("origin", origin);
            }
            tokio_tungstenite::connect_async(builder).await
        }

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

        let staff_cookie = || Some(format!("{}=staff-1", auth::ADM_COOKIE));
        // Dev-seam origin (fixture runs with `admin_mtls_off`): the
        // origin gate admits loopback http here, exactly the admin
        // origin in production.
        let dev_origin = || Some(format!("http://127.0.0.1:{}", addr.port()));

        // `proposals` with the staff cookie: hello + live fan-out.
        let (mut proposals, _) = ws_connect(&addr, "proposals", staff_cookie(), dev_origin())
            .await
            .unwrap();
        let hello: serde_json::Value =
            serde_json::from_str(&next_text(&mut proposals).await).unwrap();
        assert_eq!(hello["type"], "subscribed");
        assert_eq!(hello["topic"], "proposals");
        fx.state.bus.publish(
            "proposals",
            r#"{"type":"proposal.created","topic":"proposals","fact_id":7}"#.to_string(),
        );
        let event = next_text(&mut proposals).await;
        assert!(event.contains("proposal.created"), "got: {event}");
        proposals.close(None).await.unwrap();

        // `run:{id}` with the staff cookie: hello.
        let (mut run, _) = ws_connect(&addr, "run:9", staff_cookie(), dev_origin())
            .await
            .unwrap();
        let hello: serde_json::Value = serde_json::from_str(&next_text(&mut run).await).unwrap();
        assert_eq!(hello["topic"], "run:9");
        run.close(None).await.unwrap();

        // Same staff topics with no cookie: 403, no upgrade.
        for topic in ["proposals", "run:9"] {
            let denied = ws_connect(&addr, topic, None, dev_origin()).await;
            match denied {
                Err(tokio_tungstenite::tungstenite::Error::Http(res)) => {
                    assert_eq!(res.status(), 403, "topic: {topic}");
                }
                other => panic!("expected HTTP 403, got {other:?}"),
            }
        }

        // Origin gate: a valid staff cookie from any other origin —
        // or none at all — never upgrades on the admin branch.
        for origin in [
            None,
            Some("https://evil.test".to_string()),
            Some("https://andrii.ai".to_string()),
        ] {
            let denied = ws_connect(&addr, "proposals", staff_cookie(), origin.clone()).await;
            match denied {
                Err(tokio_tungstenite::tungstenite::Error::Http(res)) => {
                    assert_eq!(res.status(), 403, "origin: {origin:?}");
                }
                other => panic!("expected HTTP 403, got {other:?}"),
            }
        }

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
                    .serve(router.into_make_service_with_connect_info::<SocketAddr>())
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
    async fn ip_gate_known_good_cert_returns_200() {
        // A presented cert is simply ignored: session + listed IP admit.
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
    async fn ip_gate_unknown_cert_returns_200() {
        // CA-signed (passes the handshake) but never enrolled:
        // enrolment is no longer consulted, session + IP decide.
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
        assert!(raw.starts_with("HTTP/1.1 200"), "got: {raw}");
        handle.shutdown();
        server.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn ip_gate_revoked_cert_returns_200() {
        // Revocation no longer gates access: session + listed IP decide.
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
        assert!(raw.starts_with("HTTP/1.1 200"), "got: {raw}");
        handle.shutdown();
        server.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn ip_gate_missing_cert_returns_200() {
        // No client cert: session + listed loopback peer are enough.
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
        assert!(raw.starts_with("HTTP/1.1 200"), "got: {raw}");
        handle.shutdown();
        server.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn ip_gate_unlisted_peer_returns_403_naming_ip() {
        // Same session from an unlisted range: 403 names the peer IP
        // and the config section that owns the range.
        let fx = strict_fixture();
        make_session(&fx.state.store_root, "staff-1", auth::Role::Staff);
        let mut state = fx.state.clone();
        state.admin_allow_ips =
            config::parse_allow_ips("203.0.113.0/24").expect("test allowlist parses");
        let (ca, _) = test_ca("test-admin-ca");
        let (addr, handle, server) = spawn_mtls_admin(
            strict_admin_router(&state),
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
        assert!(raw.contains("127.0.0.1"), "names the peer: {raw}");
        assert!(raw.contains("admin.allow_ips"), "points at the key: {raw}");
        assert!(
            raw.contains("docs/specs/config-db.md"),
            "points at the docs: {raw}"
        );
        handle.shutdown();
        server.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn ip_gate_wrong_ca_cert_ignored_returns_200() {
        // Production no longer requests client certs (no CA): even a
        // cert chained to an unknown CA is ignored, session + IP decide.
        let fx = strict_fixture();
        make_session(&fx.state.store_root, "staff-1", auth::Role::Staff);
        let (other_ca, other_key) = test_ca("other-ca");
        let client = test_client(&other_ca, &other_key, "ops-1");
        enroll_cert(&fx.state.store_root, &client.fingerprint, "ops-1");
        let (addr, handle, server) = spawn_mtls_admin(
            strict_admin_router(&fx.state),
            self_signed_localhost(),
            None,
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
        handle.shutdown();
        server.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn ip_gate_ignores_dev_cert_header_returns_200() {
        // The dev cert header never decides access: session + listed
        // IP admit, with or without it.
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
        assert!(raw.starts_with("HTTP/1.1 200"), "got: {raw}");
        handle.shutdown();
        server.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn ip_gate_no_ca_configured_returns_200() {
        // No client CA (the production shape): session
        // + listed IP admit, with or without a client cert presented.
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
            assert!(raw.starts_with("HTTP/1.1 200"), "got: {raw}");
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
                    .serve(
                        public_router_for(&fx.state)
                            .into_make_service_with_connect_info::<std::net::SocketAddr>(),
                    )
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
        let (cert_pem, key_pem) = self_signed_localhost();
        load_admin_tls_from_pem(&cert_pem, &key_pem).expect("pair loads as plain TLS");
        load_admin_tls_from_pem(b"not a cert", &key_pem).expect_err("bad cert must fail");
    }

    // ===== Single-443 + QUIC (goal-026) =====

    /// Merged single-443 router exactly as production serves it:
    /// full admin stack + fingerprint layer, Host-dispatched with
    /// the public router, Alt-Svc iff `quic_enabled`.
    fn merged_router_for(state: &AppState, quic_enabled: bool) -> Router {
        let public = public_router_for(state);
        let admin = guardrails(
            admin::router(state.clone())
                .merge(admin_workers::router(state.clone()))
                .merge(fedex_live::router(state.clone()))
                .merge(admin_restart::router(state.clone()))
                .merge(admin_voice::staff_router(state.clone()))
                .merge(admin_voice::webhook_router(state.clone()))
                .merge(mission_admin_routers(state)),
        )
        .layer(axum::middleware::from_fn(mtls::inject_verified_cert));
        let cfg = config::RuntimeConfig {
            quic_enabled,
            ..config::RuntimeConfig::default()
        };
        tls_router(public, admin, &cfg)
    }

    /// The guardrail stack shape both routers carry: a stuck
    /// handler ends in 408 (not 500, not a hang) while a fast one
    /// passes through untouched. Production uses the 30 s
    /// [`REQUEST_TIMEOUT_SECS`]; the short budget here proves the
    /// mapping, not the constant.
    #[tokio::test]
    async fn guardrail_stack_maps_stuck_handlers_to_408() {
        use axum::error_handling::HandleErrorLayer;
        use tower::ServiceBuilder;
        use tower::limit::ConcurrencyLimitLayer;
        use tower::timeout::TimeoutLayer;
        async fn stuck() {
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        }
        let app = Router::new()
            .route("/stuck", axum::routing::get(stuck))
            .route("/ok", axum::routing::get(|| async { "fine" }))
            .layer(
                ServiceBuilder::new()
                    .layer(HandleErrorLayer::new(|_: BoxError| async {
                        StatusCode::REQUEST_TIMEOUT
                    }))
                    .layer(TimeoutLayer::new(std::time::Duration::from_millis(20)))
                    .layer(ConcurrencyLimitLayer::new(1)),
            );
        let res = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/stuck")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::REQUEST_TIMEOUT);
        let res = app
            .oneshot(Request::builder().uri("/ok").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
    }

    /// The media upload is the one path with the 300 s budget;
    /// everything else stays on the 30 s default (pure selector,
    /// so the budgets are pinned without waiting them out).
    #[test]
    fn upload_path_gets_long_budget_everything_else_default() {
        assert_eq!(
            budget_secs_for_path(MEDIA_UPLOAD_PATH),
            UPLOAD_TIMEOUT_SECS,
            "upload must outlast slow client bodies"
        );
        for path in [
            "/",
            "/apostille/",
            "/apostille/api/v1/me",
            "/apostille/api/v1/workers/status",
            "/apostille/api/v1/calls/webhook",
            "/apostille/ws/bus",
        ] {
            assert_eq!(
                budget_secs_for_path(path),
                REQUEST_TIMEOUT_SECS,
                "path: {path}"
            );
        }
    }

    /// Fast requests pass through the real [`guardrails`] stack
    /// untouched, on both the default budget and the upload path.
    #[tokio::test]
    async fn guardrails_pass_fast_requests_through() {
        let app = guardrails(
            Router::new()
                .route("/ok", get(|| async { "fine" }))
                .route(MEDIA_UPLOAD_PATH, get(|| async { "upload-slot" })),
        );
        for uri in ["/ok", MEDIA_UPLOAD_PATH] {
            let res = app
                .clone()
                .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(res.status(), StatusCode::OK, "uri: {uri}");
        }
    }

    /// The guardrail ceiling is one process-wide semaphore: blocked
    /// requests visibly hold its permits, so a flood on one router
    /// (public, admin, QUIC bridge) counts against every other.
    #[tokio::test]
    async fn guardrails_draw_from_one_shared_ceiling() {
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };
        let entered = Arc::new(AtomicUsize::new(0));
        let gate = Arc::new(tokio::sync::Notify::new());
        let app = guardrails(Router::new().route(
            "/hold",
            get({
                let entered = entered.clone();
                let gate = gate.clone();
                move || {
                    let entered = entered.clone();
                    let gate = gate.clone();
                    async move {
                        entered.fetch_add(1, Ordering::SeqCst);
                        gate.notified().await;
                        "released"
                    }
                }
            }),
        ));
        let in_flight: Vec<_> = (0..4)
            .map(|_| {
                tokio::spawn(
                    app.clone()
                        .oneshot(Request::builder().uri("/hold").body(Body::empty()).unwrap()),
                )
            })
            .collect();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while entered.load(Ordering::SeqCst) < 4 && std::time::Instant::now() < deadline {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert_eq!(
            entered.load(Ordering::SeqCst),
            4,
            "handlers must block inside"
        );
        assert!(
            shared_concurrency_permits() <= MAX_CONCURRENT_REQUESTS - 4,
            "four held requests must hold four shared permits"
        );
        gate.notify_waiters();
        for task in in_flight {
            let res = task.await.expect("request task").expect("oneshot");
            assert_eq!(res.status(), StatusCode::OK);
        }
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
    async fn host_matrix_ip_semantics() {
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

        // Session but no cert → 200 (certs decide nothing; the
        // loopback peer is listed).
        let res = router.clone().oneshot(admin_me()).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);

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

        // Session + unknown cert → 200 (certs decide nothing).
        let mut req = admin_me();
        req.extensions_mut()
            .insert(mtls::PeerCertificates::from_ders(vec![vec![0x30, 0x03]]));
        let res = router.oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
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
        assert!(plan.iter().all(|l| l.addr.port() != 444), "plan: {plan:?}");
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
    async fn quic_drain_ends_serve_loop() {
        // P0 (prompt-076): SIGINT/SIGTERM drained only the TCP
        // handles, so the QUIC accept loop never ended and every
        // prod stop hung 90s into SIGKILL. The production drain
        // must end the serve task promptly — process exit waits
        // on it, and SIGKILL is what happens when it doesn't.
        // Manual bind+spawn (not serve_quic_for_test) is
        // deliberate: the test needs the serve JoinHandle.
        let router = Router::new().route("/ping", get(|| async { "pong" }));
        let (cert_pem, key_pem) = self_signed_localhost();
        let config = quic::server_config(&cert_pem, &key_pem, None).unwrap();
        let endpoint = quic::bind_endpoint("127.0.0.1:0".parse().unwrap(), &config).unwrap();
        let serve = tokio::spawn(quic::serve(endpoint.clone(), router));
        let addr = endpoint.local_addr().unwrap();
        let (status, _) = h3_get(
            addr,
            quic_test_client_config(None),
            "andrii.ai",
            "/ping",
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        drain(&[], &[endpoint]);
        tokio::time::timeout(std::time::Duration::from_secs(5), serve)
            .await
            .expect("quic serve exits within 5s of drain")
            .unwrap();
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
    async fn quic_admin_branch_enforces_ip_allow_and_deny() {
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

        // ALLOW: staff session, no client cert — the loopback QUIC
        // peer is listed, and certs decide nothing now.
        let (status, _) = h3_get(
            addr,
            quic_test_client_config(None),
            "admin.andrii.ai",
            admin_me,
            Some(&session_cookie),
        )
        .await;
        assert_eq!(status, StatusCode::OK);

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
