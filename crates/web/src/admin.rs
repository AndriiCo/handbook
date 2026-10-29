//! Admin branch: Host guard, session + mTLS-cert auth, CSRF/Origin.
//!
//! Served on the single-443 TLS listener's admin Host branch (and —
//! in prod — its own origin `https://admin.andrii.ai`, so no public
//! JS can touch the admin session); plain-HTTP dev keeps a separate
//! admin port. Every route here requires all of: the admin host, a
//! staff/admin session, and (unless `admin.mtls_off` in local dev) a
//! verified client cert. Mutations additionally require the Origin +
//! double-submit CSRF checks.

use axum::{
    Router,
    extract::{DefaultBodyLimit, FromRequestParts, Multipart, Path, State},
    http::{HeaderValue, Method, StatusCode, header, request::Parts},
    middleware::Next,
    response::{IntoResponse, Json},
    routing::{get, post, put},
};
use handbook_store::{db, store::Store};
use serde_json::json;
use tower_http::services::{ServeDir, ServeFile};

use crate::{AppState, approval, queue, store_unavailable, unix_now};

/// Verified mTLS client identity (SHA-256 hex), inserted as a request
/// extension by the TLS terminator.
///
/// Inserted by the [`mtls`](crate::mtls) fingerprint layer from the
/// peer chain the admin listener's [`MtlsAcceptor`](crate::mtls::MtlsAcceptor)
/// captured at handshake time. No chain (plain dev HTTP, no-CA TLS,
/// cert-less peer) means no extension, and strict mode fails closed
/// (403). Local dev sets `admin.mtls_off` and identifies test certs
/// with the `x-admin-cert-sha256` header instead (see [`AdminAuth`]).
#[derive(Debug, Clone)]
pub struct VerifiedClientCert(pub String);

/// Dev/test header carrying a client-cert fingerprint. Honored ONLY
/// when `ADMIN_MTLS=off`; strict mode ignores it (a forwardable
/// header must never authenticate a cert).
pub(crate) const DEV_CERT_HEADER: &str = "x-admin-cert-sha256";

/// Authenticated staff caller on the admin listener.
pub struct AdminAuth {
    pub user: auth::AuthenticatedUser,
}

impl FromRequestParts<AppState> for AdminAuth {
    type Rejection = axum::response::Response;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let forbidden =
            |msg: &str| (StatusCode::FORBIDDEN, Json(json!({"error": msg}))).into_response();
        let unauthorized = || {
            (
                StatusCode::UNAUTHORIZED,
                Json(json!({"error": "unauthorized"})),
            )
                .into_response()
        };
        let cookies = parts
            .headers
            .get(header::COOKIE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        let Some(token) = auth::cookie_value(cookies, auth::ADM_COOKIE) else {
            return Err(unauthorized());
        };
        let conn = match db::open(&Store::new(&state.store_root)) {
            Ok(conn) => conn,
            Err(_) => return Err(store_unavailable()),
        };
        let now = unix_now();
        let Ok(Some(session)) = auth::get_session(&conn, token) else {
            return Err(unauthorized());
        };
        if !session.valid(now) {
            return Err(unauthorized());
        }
        if !session.role.meets(auth::Role::Staff) {
            return Err(forbidden("staff role required"));
        }
        // Cert: defense in depth — login alone is not enough.
        let fingerprint = cert_fingerprint(parts, state);
        match (state.admin_mtls_off, fingerprint.as_deref()) {
            // Dev: no cert needed, but a presented one must validate.
            (true, None) => {}
            (true, Some(fp)) => {
                if !cert_known_good(&conn, fp) {
                    audit_cert(&conn, now, &session.user_id, fp, "admin_cert_rejected_dev");
                    return Err(forbidden("client cert rejected"));
                }
            }
            // Strict: the TLS seam must supply a known, live cert.
            (false, Some(fp)) => {
                if !cert_known_good(&conn, fp) {
                    audit_cert(&conn, now, &session.user_id, fp, "admin_cert_rejected");
                    return Err(forbidden("client cert required"));
                }
            }
            (false, None) => {
                audit_cert(&conn, now, &session.user_id, "", "admin_cert_missing");
                return Err(forbidden("client cert required"));
            }
        }
        let _ = auth::touch_session(&conn, &session.id, now);
        Ok(AdminAuth {
            user: auth::AuthenticatedUser {
                user_id: session.user_id,
                role: session.role,
                customer_ref: session.customer_ref,
                session_id: session.id,
                cert_fingerprint: fingerprint,
            },
        })
    }
}

/// Fingerprint from the TLS extension (strict seam) or, in dev only,
/// the test header.
fn cert_fingerprint(parts: &Parts, state: &AppState) -> Option<String> {
    if let Some(verified) = parts.extensions.get::<VerifiedClientCert>() {
        return Some(verified.0.clone());
    }
    if state.admin_mtls_off {
        parts
            .headers
            .get(DEV_CERT_HEADER)
            .and_then(|v| v.to_str().ok())
            .filter(|v| !v.is_empty())
            .map(str::to_string)
    } else {
        None
    }
}

/// Known fingerprint whose cert is not revoked. Unknown DB errors
/// fail closed (`false`).
fn cert_known_good(conn: &rusqlite::Connection, fingerprint: &str) -> bool {
    auth::admin_cert_valid(conn, fingerprint).unwrap_or(false)
}

/// Presented client cert for the ceremony handlers (spec §5.2):
/// `(fingerprint, cn)` for a LIVE cert, else `None`. Strict mode
/// reads the TLS extension only; dev also accepts the test header
/// (validated, like `AdminAuth`). Unlike `cert_known_good` this
/// enforces expiry too — never bind a fresh enrolment to a dead
/// cert (the `AdminAuth` revoked-only gap is a SEC hardening item).
pub(crate) fn presented_cert_cn(
    headers: &axum::http::HeaderMap,
    verified: Option<&VerifiedClientCert>,
    state: &AppState,
    conn: &rusqlite::Connection,
    now: i64,
) -> Option<(String, String)> {
    let fingerprint = if let Some(cert) = verified {
        cert.0.clone()
    } else if state.admin_mtls_off {
        headers
            .get(DEV_CERT_HEADER)
            .and_then(|v| v.to_str().ok())
            .filter(|v| !v.is_empty())
            .map(str::to_string)?
    } else {
        return None;
    };
    let cn = auth::passkey::admin_cert_cn(conn, &fingerprint, now).unwrap_or(None)?;
    Some((fingerprint, cn))
}

/// Fresh step-up for destructive actions (spec §5.5): `step_up_at`
/// within 600s, or the session itself minted within 600s (same
/// freshness disjunct as customer mode-(b) link). Callers map
/// `false` to `403 step-up required: log in again first`.
pub(crate) fn require_step_up(
    conn: &rusqlite::Connection,
    user: &auth::AuthenticatedUser,
    now: i64,
) -> bool {
    let window = auth::passkey::STEP_UP_WINDOW_SECS;
    let stepped = auth::passkey::get_account(conn, &user.user_id)
        .ok()
        .flatten()
        .and_then(|account| account.step_up_at)
        .is_some_and(|at| now - at <= window);
    if stepped {
        return true;
    }
    auth::get_session(conn, &user.session_id)
        .ok()
        .flatten()
        .is_some_and(|session| now - session.created_at <= window)
}

/// Audit a cert decision (fingerprint only — never key bytes).
fn audit_cert(conn: &rusqlite::Connection, now: i64, actor: &str, fingerprint: &str, action: &str) {
    let _ = conn.execute(
        "INSERT INTO audit(at, actor, action, entity, entity_id, reason)
         VALUES (?1, ?2, ?3, 'admin_cert', 0, ?4)",
        rusqlite::params![now, actor, action, fingerprint],
    );
}

/// Host guard: the admin host, or loopback hosts in local dev.
pub async fn admin_host_guard(
    State(state): State<AppState>,
    req: axum::http::Request<axum::body::Body>,
    next: Next,
) -> impl IntoResponse {
    let host = req
        .headers()
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let bare = host.split_once(':').map_or(host, |(h, _)| h).to_lowercase();
    let ok = bare == auth::ADMIN_HOST
        || (state.admin_mtls_off && (bare == "localhost" || bare == "127.0.0.1"));
    if !ok {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({"error": "admin host required"})),
        )
            .into_response();
    }
    next.run(req).await
}

/// Whole-host client-cert gate (`admin.require_cert`): every
/// admin-branch request (API, HTML, JS) must carry a CA-verified
/// client cert ([`VerifiedClientCert`], stamped outside these
/// routers), except the setup surface (first boot has no certs:
/// the setup API, the `/config` wizard page, and the hashed
/// `/_app/` assets it needs) and the Telnyx webhook
/// (server-to-server, signature-authenticated — its router
/// carries no such layer by construction). Presence only:
/// registration/revocation stay enforced per API route by
/// [`AdminAuth`]. Denials are plain 403s (the access log keeps
/// the ID/route; no audit row per asset fetch).
pub async fn require_client_cert(
    State(state): State<AppState>,
    req: axum::http::Request<axum::body::Body>,
    next: Next,
) -> impl IntoResponse {
    if !state.admin_require_cert || state.admin_mtls_off {
        return next.run(req).await;
    }
    let path = req.uri().path();
    if path.starts_with("/apostille/api/v1/setup/")
        || path == "/config"
        || path.starts_with("/_app/")
    {
        return next.run(req).await;
    }
    let presented = req
        .extensions()
        .get::<VerifiedClientCert>()
        .is_some_and(|cert| !cert.0.is_empty());
    if !presented {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({"error": "client cert required"})),
        )
            .into_response();
    }
    next.run(req).await
}

/// CSRF guard for admin mutations: allowed `Origin` plus the
/// double-submit token (`hb_csrf` cookie echoed in `x-csrf`).
/// Safe methods pass through. The ceremony routes below pass
/// through too: start POSTs have no token to echo yet, so those
/// handlers enforce Origin (start) / Origin+echo (finish)
/// per-handler exactly like customer §2 — there is exactly one
/// enforcement site per route either way.
pub async fn csrf_guard(
    State(state): State<AppState>,
    req: axum::http::Request<axum::body::Body>,
    next: Next,
) -> impl IntoResponse {
    if matches!(*req.method(), Method::GET | Method::HEAD | Method::OPTIONS) {
        return next.run(req).await;
    }
    if req.uri().path().starts_with("/apostille/api/v1/auth/") {
        return next.run(req).await;
    }
    // Setup wizard (§9): no cookies exist before the first enrol, so
    // the double-submit check cannot apply. The setup POST handler
    // does its own Origin check; the setup GETs are safe methods and
    // the custom `x-setup-key` header is CSRF-safe by construction
    // (no CORS allowlist to forge it).
    if req.uri().path().starts_with("/apostille/api/v1/setup/") {
        return next.run(req).await;
    }
    let fail = || {
        (
            StatusCode::FORBIDDEN,
            Json(json!({"error": "csrf check failed"})),
        )
            .into_response()
    };
    let origin = req
        .headers()
        .get(header::ORIGIN)
        .and_then(|v| v.to_str().ok());
    if !auth::csrf::origin_allowed(origin, state.admin_mtls_off) {
        return fail();
    }
    let cookies = req
        .headers()
        .get(header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let cookie_token = auth::cookie_value(cookies, auth::CSRF_COOKIE).unwrap_or("");
    let header_token = req
        .headers()
        .get(auth::CSRF_HEADER)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if !auth::csrf::tokens_match(cookie_token, header_token) {
        return fail();
    }
    next.run(req).await
}

/// The admin-only router (admin Host branch of single-443; never
/// on the public branch). Serves the admin SPA shell
/// (`ADMIN_SPA_DIR`) at every non-API path with SPA fallback;
/// unknown API paths stay JSON 404s (never the shell), so API
/// clients cannot mistake HTML for data. The host
/// differentiates the console: page paths carry no `/admin`
/// segment.
pub fn router(state: AppState) -> Router {
    let spa_index = state.admin_spa_dir.join("index.html");
    Router::new()
        // Domain root is staff-only: the admin build also registers the
        // (public) homepage at `/`, so serve no shell here — send
        // browsers to the console (its layout gate forwards to
        // /staff-login when unauthenticated).
        .route("/", get(root_to_console))
        // Demo routes never serve on the staff origin: (mock) ships in
        // the admin JS bundle (client-routable) but must 404 server-side.
        .route("/mock/{*rest}", get(admin_not_found))
        .route("/apostille/api/v1/me", get(admin_me))
        .route("/apostille/api/v1/facts", get(approval::list_facts))
        .route(
            "/apostille/api/v1/facts/{id}/accept",
            post(approval::accept_fact),
        )
        .route(
            "/apostille/api/v1/facts/{id}/reject",
            post(approval::reject_fact),
        )
        .route("/apostille/api/v1/queue", get(queue::queue_items))
        .route("/apostille/api/v1/queue/{id}", get(queue::queue_item))
        .route(
            "/apostille/api/v1/suppress-rules",
            get(queue::list_rules).post(queue::create_rule),
        )
        .route(
            "/apostille/api/v1/suppress-rules/{id}/revoke",
            post(queue::revoke_rule),
        )
        .route(
            "/apostille/api/v1/suppressions",
            get(queue::list_suppressions),
        )
        .route("/apostille/api/v1/config", get(get_config))
        .route("/apostille/api/v1/config/{key}", put(set_config))
        .route(
            "/apostille/api/v1/setup/status",
            get(crate::setup::setup_status),
        )
        .route(
            "/apostille/api/v1/setup/steps",
            get(crate::setup::setup_steps),
        )
        .route(
            "/apostille/api/v1/setup/keys/{key}",
            post(crate::setup::setup_set_key),
        )
        .route(
            "/apostille/api/v1/auth/passkey/enrol/start",
            post(crate::staff_passkey::enrol_start),
        )
        .route(
            "/apostille/api/v1/auth/passkey/enrol/finish",
            post(crate::staff_passkey::enrol_finish),
        )
        .route(
            "/apostille/api/v1/auth/passkey/login/start",
            post(crate::staff_passkey::login_start),
        )
        .route(
            "/apostille/api/v1/auth/passkey/login/finish",
            post(crate::staff_passkey::login_finish),
        )
        .route("/apostille/api/v1/bus/publish", post(publish))
        .route(
            "/apostille/api/v1/media/upload",
            post(upload_media).layer(DefaultBodyLimit::disable()),
        )
        .route(
            "/apostille/api/v1/{*path}",
            get(admin_not_found)
                .post(admin_not_found)
                .put(admin_not_found),
        )
        .fallback_service(ServeDir::new(&state.admin_spa_dir).fallback(ServeFile::new(spa_index)))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            csrf_guard,
        ))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            require_client_cert,
        ))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            admin_host_guard,
        ))
        .layer(axum::middleware::from_fn_with_state(
            "admin",
            crate::request_id_and_log,
        ))
        .with_state(state)
}

/// Session + cert visibility for the admin shell.
async fn admin_me(auth: AdminAuth) -> impl IntoResponse {
    Json(json!({
        "user_id": auth.user.user_id,
        "role": auth.user.role.as_str(),
        "customer_ref": auth.user.customer_ref,
        "cert_fingerprint": auth.user.cert_fingerprint,
    }))
}

async fn admin_not_found() -> impl IntoResponse {
    (StatusCode::NOT_FOUND, Json(json!({"error": "not found"})))
}

/// `GET /` on the admin host is never the public homepage (the admin
/// build registers `(public)/+page` at `/` too): redirect to the
/// staff console. Static target — no open redirect.
async fn root_to_console() -> impl IntoResponse {
    (
        StatusCode::SEE_OTHER,
        [(header::LOCATION, HeaderValue::from_static("/console"))],
    )
}

async fn get_config(auth: AdminAuth, State(state): State<AppState>) -> impl IntoResponse {
    let _ = auth;
    let Some(kek) = state.kek.as_ref() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"error": "config unavailable"})),
        )
            .into_response();
    };
    let conn = match db::open(&Store::new(&state.store_root)) {
        Ok(conn) => conn,
        Err(_) => return store_unavailable(),
    };
    match config::list_display(&conn, kek) {
        Ok(keys) => {
            let rendered: Vec<serde_json::Value> = keys
                .into_iter()
                .map(|(key, value)| match value {
                    config::DisplayValue::Bool(b) => {
                        json!({"key": key, "type": "bool", "value": b})
                    }
                    config::DisplayValue::Text(s) => {
                        json!({"key": key, "type": "text", "value": s})
                    }
                    config::DisplayValue::Secret { set } => {
                        json!({"key": key, "type": "secret", "set": set})
                    }
                })
                .collect();
            (StatusCode::OK, Json(json!({"keys": rendered}))).into_response()
        }
        Err(_) => store_unavailable(),
    }
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct SetConfigBody {
    #[serde(rename = "type")]
    key_type: String,
    value: serde_json::Value,
}

/// Set one config key (secret: set-new-only, never echoed).
/// Requires an enrolled KEK generation + fresh step-up (spec
/// §5.5 pilot: staff login always steps up, so expiry forces
/// re-login; the full destructive-action list is OPEN-17).
async fn set_config(
    auth: AdminAuth,
    State(state): State<AppState>,
    Path(key): Path<String>,
    Json(body): Json<SetConfigBody>,
) -> impl IntoResponse {
    let Some(kek) = state.kek.as_ref() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"error": "config unavailable"})),
        )
            .into_response();
    };
    // The setup gate is mint/burn only (§9.1): an admin PUT must
    // never reopen or clobber the setup window.
    if key.starts_with("setup.") {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"error": "setup keys are mint/burn only"})),
        )
            .into_response();
    }
    let Some(key_type) = config::ConfigType::parse(&body.key_type) else {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"error": "unknown type (bool|text|secret)"})),
        )
            .into_response();
    };
    let plaintext: Vec<u8> = match (key_type, &body.value) {
        (config::ConfigType::Bool, serde_json::Value::Bool(true)) => vec![b'1'],
        (config::ConfigType::Bool, serde_json::Value::Bool(false)) => vec![b'0'],
        (config::ConfigType::Bool, serde_json::Value::String(s)) if s == "0" || s == "1" => {
            vec![s.as_bytes()[0]]
        }
        (config::ConfigType::Text | config::ConfigType::Secret, serde_json::Value::String(s)) => {
            s.as_bytes().to_vec()
        }
        _ => {
            return (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(json!({"error": "value shape does not match type"})),
            )
                .into_response();
        }
    };
    let conn = match db::open(&Store::new(&state.store_root)) {
        Ok(conn) => conn,
        Err(_) => return store_unavailable(),
    };
    if !require_step_up(&conn, &auth.user, unix_now()) {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({"error": "step-up required: log in again first"})),
        )
            .into_response();
    }
    let version = match config::active_kek_version(&conn) {
        Ok(Some(v)) => v,
        Ok(None) => {
            return (
                StatusCode::CONFLICT,
                Json(json!({"error": "KEK not enrolled"})),
            )
                .into_response();
        }
        Err(_) => return store_unavailable(),
    };
    let now = unix_now();
    let write = config::SetValue {
        version,
        key: &key,
        key_type,
        plaintext: &plaintext,
        now,
        actor: &auth.user.user_id,
    };
    match config::set_value(&conn, kek, &write) {
        Ok(()) => {}
        Err(config::ConfigError::BadKeyName | config::ConfigError::BadValue(_)) => {
            return (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(json!({"error": "invalid key or value"})),
            )
                .into_response();
        }
        Err(_) => return store_unavailable(),
    }
    // Audit carries key+version only, never values.
    let _ = conn.execute(
        "INSERT INTO audit(at, actor, action, entity, entity_id,
                            before_ref, after_ref, reason)
         VALUES (?1, ?2, 'config_set', 'config', ?3, ?4, ?5, ?6)",
        rusqlite::params![
            now,
            auth.user.user_id,
            version,
            key,
            format!("{key}:v{version}"),
            key_type.as_str(),
        ],
    );
    (
        StatusCode::OK,
        Json(json!({"key": key, "type": key_type.as_str(), "set": true})),
    )
        .into_response()
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct PublishBody {
    topic: String,
    kind: String,
    body: serde_json::Value,
}

/// Staff publish to one bus channel. Authority channels accept only
/// `fact_accepted` events — this is what keeps anonymous subscribers
/// on accepted facts only.
async fn publish(
    _auth: AdminAuth,
    State(state): State<AppState>,
    Json(body): Json<PublishBody>,
) -> impl IntoResponse {
    let Some(topic) = auth::ws::WsTopic::parse(&body.topic) else {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"error": "unknown topic"})),
        )
            .into_response();
    };
    if matches!(topic, auth::ws::WsTopic::Authority { .. }) && body.kind != "fact_accepted" {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"error": "authority channels carry fact_accepted only"})),
        )
            .into_response();
    }
    if body.kind.is_empty() || body.kind.len() > 64 {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"error": "bad event kind"})),
        )
            .into_response();
    }
    state.bus.publish(
        &topic.as_channel(),
        json!({"type": body.kind, "topic": topic.as_channel(),
               "body": body.body, "at": unix_now()})
        .to_string(),
    );
    (StatusCode::OK, Json(json!({"published": true}))).into_response()
}

/// Staff media upload (F8 intake): multipart `file` (+ `authority`
/// slug, `grant_version`, `grant_accepted_at`, optional
/// `synthetic=true`). Validates through the media crate, persists a
/// quarantined `community` capture, audits. Real uploads fail closed
/// until `MEDIA_TERMS_READ=1`; synthetic fixtures always flow.
async fn upload_media(
    auth: AdminAuth,
    State(state): State<AppState>,
    mut multipart: Multipart,
) -> impl IntoResponse {
    let bad = |msg: String| {
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"error": msg})),
        )
            .into_response()
    };
    let mut file_bytes: Option<Vec<u8>> = None;
    let mut filename: Option<String> = None;
    let mut content_type: Option<String> = None;
    let mut authority: Option<String> = None;
    let mut grant_version: Option<String> = None;
    let mut grant_accepted_at: Option<i64> = None;
    let mut synthetic = false;
    while let Some(field) = multipart.next_field().await.unwrap_or(None) {
        let name = field.name().unwrap_or("").to_string();
        match name.as_str() {
            "file" => {
                filename = field.file_name().map(str::to_string);
                content_type = field.content_type().map(|mime| mime.to_string());
                match field.bytes().await {
                    Ok(bytes) => file_bytes = Some(bytes.to_vec()),
                    Err(_) => return bad("unreadable file part".to_string()),
                }
            }
            "authority" => authority = field.text().await.ok(),
            "grant_version" => grant_version = field.text().await.ok(),
            "grant_accepted_at" => {
                grant_accepted_at = field.text().await.ok().and_then(|t| t.parse().ok());
            }
            "synthetic" => {
                synthetic = field.text().await.is_ok_and(|t| t == "true");
            }
            _ => {}
        }
    }
    let (Some(bytes), Some(filename), Some(content_type), Some(authority)) =
        (file_bytes, filename, content_type, authority)
    else {
        return bad("multipart needs file + authority parts".to_string());
    };
    let store = Store::new(&state.store_root);
    let conn = match db::open(&store) {
        Ok(conn) => conn,
        Err(_) => return store_unavailable(),
    };
    let authority_id: i64 = match conn.query_row(
        "SELECT id FROM authority WHERE slug = ?1",
        [&authority],
        |row| row.get(0),
    ) {
        Ok(id) => id,
        Err(_) => return bad(format!("unknown authority slug: {authority}")),
    };
    let policy = if state.media_terms_read {
        media::policy::MediaPolicy::terms_read()
    } else {
        media::policy::MediaPolicy::synthetic_only()
    };
    let source = if synthetic {
        media::policy::MediaSource::SyntheticFixture
    } else {
        media::policy::MediaSource::RealUpload
    };
    let grant = grant_version.map(|version| media::upload::RightsGrant {
        version,
        accepted_at_unix: grant_accepted_at.unwrap_or(0),
    });
    let intake = match media::upload::intake_upload(
        &policy,
        source,
        &filename,
        &content_type,
        &bytes,
        grant.as_ref(),
        unix_now(),
    ) {
        Ok(intake) => intake,
        Err(e) => return bad(e.to_string()),
    };
    let now = unix_now();
    let relative = match store
        .write_blob(&bytes)
        .map_err(|_| ())
        .and_then(|_| store.blob_path(&intake.sha256_hex).map_err(|_| ()))
        .ok()
        .and_then(|path| {
            path.strip_prefix(store.root())
                .ok()
                .and_then(|p| p.to_str().map(str::to_string))
        }) {
        Some(relative) => relative,
        None => return store_unavailable(),
    };
    let kind = match intake.media_kind {
        media::upload::MediaKind::Photo => "photo",
        media::upload::MediaKind::Video => "video",
        media::upload::MediaKind::Comment => "comment",
        _ => "file",
    };
    let capture_id: i64 = match conn.execute(
        "INSERT INTO capture(authority_id, source_id, captured_at, kind,
                              path, bytes_raw, bytes_stored, sha256,
                              authenticity, trust, notes)
         VALUES (?1, NULL, ?2, ?3, ?4, ?5, ?5, ?6, '', 'community', 'quarantine:intake')",
        rusqlite::params![
            authority_id,
            now,
            kind,
            relative,
            bytes.len() as i64,
            intake.sha256_hex,
        ],
    ) {
        Ok(_) => conn.last_insert_rowid(),
        Err(_) => return store_unavailable(),
    };
    let _ = conn.execute(
        "INSERT INTO audit(at, actor, action, entity, entity_id, reason)
         VALUES (?1, ?2, 'media_upload', 'capture', ?3, ?4)",
        rusqlite::params![now, auth.user.user_id, capture_id, intake.filename],
    );
    (
        StatusCode::OK,
        Json(json!({
            "capture_id": capture_id,
            "sha256": intake.sha256_hex,
            "quarantined": true,
        })),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt as _;

    fn state_with_spa(tmp: &tempfile::TempDir) -> AppState {
        let store = Store::new(tmp.path());
        store.init().unwrap();
        let conn = db::open(&store).unwrap();
        db::migrate(&conn).unwrap();
        auth::migrate_auth_tables(&conn).unwrap();
        // Prove the redirect wins over the SPA fallback: a shell exists.
        std::fs::write(tmp.path().join("index.html"), "<html>admin-shell</html>").unwrap();
        AppState {
            spa_index: tmp.path().join("index.html"),
            static_dir: tmp.path().to_path_buf(),
            store_root: tmp.path().to_path_buf(),
            bus: crate::ws_bus::Bus::new(),
            kek: None,
            admin_mtls_off: true,
            admin_require_cert: false,
            admin_spa_dir: tmp.path().to_path_buf(),
            media_terms_read: false,
            label_cfg: crate::p2_labels::LabelCfg::default(),
            voice_cfg: voice::VoiceConfig::default(),
            voice_api_key: None,
            webauthn: std::sync::Arc::new(crate::passkey::test_webauthn()),
            auth_limits: std::sync::Arc::new(crate::passkey::RateBuckets::default()),
            google_cfg: None,
            apple_cfg: None,
            meta_cfg: None,
            mailer: None,
            magic_link_base: "https://example.test/apostille/login".to_string(),
            meta_delete_status_base: "https://example.test/auth/meta/delete-status".to_string(),
        }
    }

    #[tokio::test]
    async fn root_redirects_to_console_not_public_shell() {
        let tmp = tempfile::tempdir().unwrap();
        let app = router(state_with_spa(&tmp));
        let res = app
            .oneshot(
                Request::builder()
                    .uri("/")
                    .header("host", "admin.andrii.ai")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::SEE_OTHER);
        assert_eq!(res.headers().get(header::LOCATION).unwrap(), "/console");
    }

    #[tokio::test]
    async fn root_redirect_still_requires_admin_host() {
        let tmp = tempfile::tempdir().unwrap();
        let app = router(state_with_spa(&tmp));
        // Wrong host → host guard 403s before any redirect (dev loopback
        // would pass; a public hostname must not).
        let res = app
            .oneshot(
                Request::builder()
                    .uri("/")
                    .header("host", "andrii.ai")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn deep_staff_path_still_serves_shell() {
        let tmp = tempfile::tempdir().unwrap();
        let app = router(state_with_spa(&tmp));
        let res = app
            .oneshot(
                Request::builder()
                    .uri("/console")
                    .header("host", "admin.andrii.ai")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = axum::body::to_bytes(res.into_body(), 4096).await.unwrap();
        assert!(body.windows(11).any(|w| w == b"admin-shell"));
    }

    #[tokio::test]
    async fn mock_routes_404_on_staff_origin() {
        let tmp = tempfile::tempdir().unwrap();
        let app = router(state_with_spa(&tmp));
        let res = app
            .oneshot(
                Request::builder()
                    .uri("/mock/admin/config")
                    .header("host", "admin.andrii.ai")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::NOT_FOUND);
    }

    fn strict_cert_state(tmp: &tempfile::TempDir) -> AppState {
        let mut state = state_with_spa(tmp);
        state.admin_mtls_off = false;
        state.admin_require_cert = true;
        state
    }

    fn cert_request(uri: &str, cert: Option<&str>) -> Request<Body> {
        let mut req = Request::builder()
            .uri(uri)
            .header("host", "admin.andrii.ai")
            .body(Body::empty())
            .unwrap();
        if let Some(fingerprint) = cert {
            req.extensions_mut()
                .insert(VerifiedClientCert(fingerprint.to_string()));
        }
        req
    }

    #[tokio::test]
    async fn require_cert_denies_uncertified_page_and_api() {
        let tmp = tempfile::tempdir().unwrap();
        let app = router(strict_cert_state(&tmp));
        for uri in ["/console", "/apostille/api/v1/me"] {
            let res = app.clone().oneshot(cert_request(uri, None)).await.unwrap();
            assert_eq!(res.status(), StatusCode::FORBIDDEN, "uri: {uri}");
            let body = axum::body::to_bytes(res.into_body(), 1024).await.unwrap();
            let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
            assert_eq!(v["error"], "client cert required");
        }
    }

    #[tokio::test]
    async fn require_cert_allows_verified_client() {
        let tmp = tempfile::tempdir().unwrap();
        let app = router(strict_cert_state(&tmp));
        // Page serves the shell once a cert is presented.
        let res = app
            .clone()
            .oneshot(cert_request("/console", Some("fingerprint")))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        // API passes the gate too (then AdminAuth 401s the missing session).
        let res = app
            .oneshot(cert_request("/apostille/api/v1/me", Some("fingerprint")))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn require_cert_exempts_setup_and_assets() {
        let tmp = tempfile::tempdir().unwrap();
        let app = router(strict_cert_state(&tmp));
        for uri in [
            "/apostille/api/v1/setup/status",
            "/config",
            "/_app/immutable/x.js",
        ] {
            let res = app.clone().oneshot(cert_request(uri, None)).await.unwrap();
            assert_eq!(res.status(), StatusCode::OK, "uri: {uri}");
        }
    }
}
