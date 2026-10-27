//! Admin listener: Host guard, session + mTLS-cert auth, CSRF/Origin.
//!
//! Served on its own listener (`ADMIN_PORT`, default 3001) and — in
//! prod — its own origin (`https://admin.andrii.ai`, no public JS can
//! touch the admin session). Every route here requires all of: the
//! admin host, a staff/admin session, and (unless `ADMIN_MTLS=off` in
//! local dev) a verified client cert. Mutations additionally require
//! the Origin + double-submit CSRF checks.

use axum::{
    Router,
    extract::{FromRequestParts, Path, State},
    http::{Method, StatusCode, header, request::Parts},
    middleware::Next,
    response::{IntoResponse, Json},
    routing::{get, post, put},
};
use handbook_store::{db, store::Store};
use serde_json::json;

use crate::{AppState, approval, store_unavailable, unix_now};

/// Verified mTLS client identity (SHA-256 hex), inserted as a request
/// extension by the TLS terminator.
///
/// SEAM: axum-server exposes no per-connection client cert yet, so no
/// production layer inserts this today — strict mode therefore fails
/// closed (403) until the rustls client-cert verifier lands. Local
/// dev sets `ADMIN_MTLS=off` and identifies test certs with the
/// `x-admin-cert-sha256` header instead (see [`AdminAuth`]).
#[derive(Debug, Clone)]
pub struct VerifiedClientCert(pub String);

/// Dev/test header carrying a client-cert fingerprint. Honored ONLY
/// when `ADMIN_MTLS=off`; strict mode ignores it (a forwardable
/// header must never authenticate a cert).
const DEV_CERT_HEADER: &str = "x-admin-cert-sha256";

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

/// CSRF guard for admin mutations: allowed `Origin` plus the
/// double-submit token (`hb_csrf` cookie echoed in `x-csrf`).
/// Safe methods pass through.
pub async fn csrf_guard(
    State(state): State<AppState>,
    req: axum::http::Request<axum::body::Body>,
    next: Next,
) -> impl IntoResponse {
    if matches!(*req.method(), Method::GET | Method::HEAD | Method::OPTIONS) {
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

/// The admin-only router (own listener; never mounted on public).
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/apostille/api/v1/admin/me", get(admin_me))
        .route("/apostille/api/v1/admin/facts", get(approval::list_facts))
        .route(
            "/apostille/api/v1/admin/facts/{id}/accept",
            post(approval::accept_fact),
        )
        .route(
            "/apostille/api/v1/admin/facts/{id}/reject",
            post(approval::reject_fact),
        )
        .route("/apostille/api/v1/admin/config", get(get_config))
        .route("/apostille/api/v1/admin/config/{key}", put(set_config))
        .route("/apostille/api/v1/admin/bus/publish", post(publish))
        .fallback(admin_not_found)
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            csrf_guard,
        ))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            admin_host_guard,
        ))
        .layer(axum::middleware::from_fn(crate::request_id_and_log))
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
/// Requires an enrolled KEK generation; step-up re-auth gates this
/// in prod (staff passkey/TOTP story still open — see playbook).
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
