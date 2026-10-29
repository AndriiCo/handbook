//! First-boot setup wizard gate (secure-config.md §9).
//!
//! Before any staff account exists, the CLI-minted setup key (presented
//! as an `x-setup-key` header, verified against the sealed
//! `setup.key_hash` row) authorizes exactly three things: reading
//! setup status/progress and writing wizard config keys. It NEVER
//! grants [`AdminAuth`](crate::admin::AdminAuth) powers. The first
//! staff passkey enrol burns the hash row ([`burn_setup_key`]),
//! closing the window permanently.

use axum::{
    Json,
    extract::{ConnectInfo, FromRequestParts, Path, State},
    http::{HeaderMap, StatusCode, request::Parts},
    response::{IntoResponse, Response},
};
use serde_json::json;
use std::net::SocketAddr;

use crate::{AppState, passkey, store_unavailable, unix_now};

/// Header carrying the setup key on every setup request. Custom
/// headers are CSRF-safe by construction (no CORS allowlist exists
/// to forge one cross-origin), so setup POSTs need no CSRF cookie.
pub const SETUP_KEY_HEADER: &str = "x-setup-key";

/// Setup attempts per IP bucket (matches the enrol-start budget:
/// a wizard finger clicks, it does not spray).
const SETUP_LIMIT: u32 = 30;
const SETUP_WINDOW_SECS: i64 = 60;

/// Setup-key authorization. Rejection vocabulary mirrors the admin
/// API: 401 missing key, 403 bad key / closed window, 429 limited,
/// 503 store or KEK unavailable.
pub struct SetupAuth;

impl FromRequestParts<AppState> for SetupAuth {
    type Rejection = Response;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let now = unix_now();
        let ip = parts
            .extensions
            .get::<ConnectInfo<SocketAddr>>()
            .map(|info| passkey::client_ip(info.0))
            .unwrap_or_else(|| "unknown".to_string());
        if !state
            .auth_limits
            .check(&format!("su:{ip}"), now, SETUP_LIMIT, SETUP_WINDOW_SECS)
        {
            if let Some(conn) = passkey::open_conn(state) {
                passkey::audit_auth(&conn, now, "anonymous", "setup.rate_limited", &ip);
            }
            return Err((
                StatusCode::TOO_MANY_REQUESTS,
                Json(json!({"error": "rate limited"})),
            )
                .into_response());
        }
        let Some(candidate) = parts
            .headers
            .get(SETUP_KEY_HEADER)
            .and_then(|v| v.to_str().ok())
        else {
            return Err((
                StatusCode::UNAUTHORIZED,
                Json(json!({"error": "setup key required"})),
            )
                .into_response());
        };
        let Some(kek) = state.kek.as_ref() else {
            return Err((
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({"error": "config unavailable"})),
            )
                .into_response());
        };
        let Some(conn) = passkey::open_conn(state) else {
            return Err(store_unavailable());
        };
        let hash = handbook_store::store::sha256_hex(candidate.as_bytes());
        match config::verify_setup_key(&conn, kek, &hash) {
            Ok(true) => Ok(SetupAuth),
            Ok(false) => {
                passkey::audit_auth(&conn, now, "anonymous", "setup.denied", &ip);
                Err((
                    StatusCode::FORBIDDEN,
                    Json(json!({"error": "setup key invalid or window closed"})),
                )
                    .into_response())
            }
            Err(_) => Err(store_unavailable()),
        }
    }
}

/// True while the `setup.key_hash` row exists. No KEK needed (the
/// row's presence is the signal; only verification opens it).
pub fn setup_required(conn: &rusqlite::Connection) -> bool {
    conn.query_row(
        "SELECT 1 FROM config WHERE key = ?1",
        [config::keys::SETUP_KEY_HASH],
        |_| Ok(()),
    )
    .is_ok()
}

/// Burn the setup window after the first staff enrol. Returns true
/// when a row actually died (that case audits `setup.burned`);
/// a second burn is a silent no-op false.
pub fn burn_setup_key(conn: &rusqlite::Connection, actor: &str, now: i64) -> bool {
    let burned = config::delete_value(conn, config::keys::SETUP_KEY_HASH).unwrap_or(false);
    if burned {
        passkey::audit_auth(
            conn,
            now,
            actor,
            "setup.burned",
            config::keys::SETUP_KEY_HASH,
        );
    }
    burned
}

/// `GET .../admin/setup/status` → `{setup_required}`. Public on the
/// admin host: a bare boolean leaks nothing beyond "fresh server".
pub async fn setup_status(State(state): State<AppState>) -> impl IntoResponse {
    let Some(conn) = passkey::open_conn(&state) else {
        return store_unavailable();
    };
    (
        StatusCode::OK,
        Json(json!({"setup_required": setup_required(&conn)})),
    )
        .into_response()
}

/// `GET .../admin/setup/steps` (SetupAuth) → wizard progress: the
/// admin-list row shape, registry keys except `setup.*`. Unset keys
/// are absent — the SPA marks them missing. No Origin check: safe
/// method + key-gated, and browsers omit `Origin` on GET — the
/// `x-setup-key` header is the CSRF protection (§9.1).
pub async fn setup_steps(_auth: SetupAuth, State(state): State<AppState>) -> impl IntoResponse {
    let Some(kek) = state.kek.as_ref() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"error": "config unavailable"})),
        )
            .into_response();
    };
    let Some(conn) = passkey::open_conn(&state) else {
        return store_unavailable();
    };
    match config::list_display(&conn, kek) {
        Ok(keys) => {
            let rendered: Vec<serde_json::Value> = keys
                .into_iter()
                .filter(|(key, _)| !key.starts_with("setup."))
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
pub struct SetupSetBody {
    #[serde(rename = "type")]
    key_type: String,
    value: serde_json::Value,
}

/// `POST .../admin/setup/keys/{key}` (SetupAuth) → sealed put +
/// audit (actor `setup`). Same validation as admin PUT minus
/// step-up (no session exists — the key IS the auth), restricted
/// to registered wizard keys (`setup.*` and unknown keys 422).
pub async fn setup_set_key(
    _auth: SetupAuth,
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(key): Path<String>,
    Json(body): Json<SetupSetBody>,
) -> impl IntoResponse {
    if !passkey::origin_ok(&headers, state.admin_mtls_off) {
        return passkey::csrf_fail();
    }
    if key.starts_with("setup.") {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"error": "setup keys are mint/burn only"})),
        )
            .into_response();
    }
    if !config::keys::ALL.iter().any(|(name, _)| *name == key) {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"error": "unknown wizard key"})),
        )
            .into_response();
    }
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
    let Some(conn) = passkey::open_conn(&state) else {
        return store_unavailable();
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
        actor: "setup",
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
    // Audit carries key+version only, never values (admin PUT shape).
    let _ = conn.execute(
        "INSERT INTO audit(at, actor, action, entity, entity_id,
                            before_ref, after_ref, reason)
         VALUES (?1, 'setup', 'config_set', 'config', ?2, ?3, ?4, ?5)",
        rusqlite::params![
            now,
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

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Router, body::Body, extract::ConnectInfo, http::Request, routing::get};
    use handbook_store::{db, store::Store};
    use std::path::PathBuf;
    use tower::ServiceExt as _;

    const KEY: &str = "setup_testkey0000000000000000000000000000000000000000000000000001";

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
        let kek = config::Kek::generate();
        config::record_kek(&conn, 1, "test", 1).unwrap();
        let hash = handbook_store::store::sha256_hex(KEY.as_bytes());
        config::set_value(
            &conn,
            &kek,
            &config::SetValue {
                version: 1,
                key: config::keys::SETUP_KEY_HASH,
                key_type: config::ConfigType::Secret,
                plaintext: hash.as_bytes(),
                now: 1,
                actor: "test",
            },
        )
        .unwrap();
        drop(conn);
        let state = AppState {
            spa_index: PathBuf::from("test-spa/index.html"),
            static_dir: PathBuf::from("static"),
            store_root: tmp.path().to_path_buf(),
            bus: crate::ws_bus::Bus::new(),
            kek: Some(std::sync::Arc::new(kek)),
            admin_mtls_off: true,
            admin_spa_dir: tmp.path().join("admin-spa"),
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
        };
        Fixture { _tmp: tmp, state }
    }

    fn router(state: AppState) -> Router {
        Router::new()
            .route("/apostille/api/v1/admin/setup/status", get(setup_status))
            .route("/apostille/api/v1/admin/setup/steps", get(setup_steps))
            .route(
                "/apostille/api/v1/admin/setup/keys/{key}",
                axum::routing::post(setup_set_key),
            )
            .route(
                "/apostille/api/v1/admin/auth/passkey/enrol/start",
                axum::routing::post(crate::staff_passkey::enrol_start),
            )
            .route(
                "/apostille/api/v1/admin/auth/passkey/enrol/finish",
                axum::routing::post(crate::staff_passkey::enrol_finish),
            )
            .layer(axum::middleware::from_fn_with_state(
                state.clone(),
                crate::admin::csrf_guard,
            ))
            .layer(axum::middleware::from_fn_with_state(
                state.clone(),
                crate::admin::admin_host_guard,
            ))
            .with_state(state)
    }

    fn request(
        method: &str,
        uri: &str,
        key: Option<&str>,
        body: Option<serde_json::Value>,
    ) -> Request<Body> {
        let mut builder = Request::builder()
            .uri(uri)
            .method(method)
            .header("host", "admin.andrii.ai")
            .header("origin", "https://admin.andrii.ai")
            .header("cookie", format!("{}=t", auth::CSRF_COOKIE))
            .header(auth::CSRF_HEADER, "t");
        if let Some(key) = key {
            builder = builder.header(SETUP_KEY_HEADER, key);
        }
        if method == "POST" {
            builder = builder.header("content-type", "application/json");
        }
        // Enrol handlers extract ConnectInfo (rate buckets): oneshot
        // has no listener, so plant loopback directly.
        builder = builder.extension(ConnectInfo(std::net::SocketAddr::from(([127, 0, 0, 1], 9))));
        builder
            .body(Body::from(body.map(|v| v.to_string()).unwrap_or_default()))
            .unwrap()
    }

    /// Browser-faithful GET: same-origin fetches send no `Origin`
    /// and the wizard mints no CSRF cookie — only Host + key.
    fn browser_get(uri: &str, key: Option<&str>) -> Request<Body> {
        let mut builder = Request::builder()
            .uri(uri)
            .method("GET")
            .header("host", "admin.andrii.ai");
        if let Some(key) = key {
            builder = builder.header(SETUP_KEY_HEADER, key);
        }
        builder = builder.extension(ConnectInfo(std::net::SocketAddr::from(([127, 0, 0, 1], 9))));
        builder.body(Body::empty()).unwrap()
    }

    async fn json_body(res: Response) -> serde_json::Value {
        let bytes = axum::body::to_bytes(res.into_body(), 1024 * 1024)
            .await
            .unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    fn audit_count(state: &AppState, action: &str) -> i64 {
        let store = Store::new(&state.store_root);
        let conn = db::open(&store).unwrap();
        conn.query_row(
            "SELECT COUNT(*) FROM audit WHERE action = ?1",
            [action],
            |row| row.get(0),
        )
        .unwrap()
    }

    #[tokio::test]
    async fn status_tracks_the_hash_row() {
        let fix = fixture();
        let app = router(fix.state.clone());
        let res = app
            .oneshot(request(
                "GET",
                "/apostille/api/v1/admin/setup/status",
                None,
                None,
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(json_body(res).await["setup_required"], true);
        // Burn flips it with no key involved.
        let store = Store::new(&fix.state.store_root);
        let conn = db::open(&store).unwrap();
        assert!(burn_setup_key(&conn, "test", 2));
        assert!(!setup_required(&conn));
        drop(conn);
        let app = router(fix.state.clone());
        let res = app
            .oneshot(request(
                "GET",
                "/apostille/api/v1/admin/setup/status",
                None,
                None,
            ))
            .await
            .unwrap();
        assert_eq!(json_body(res).await["setup_required"], false);
    }

    #[tokio::test]
    async fn steps_gate_and_shape() {
        let fix = fixture();
        let app = router(fix.state.clone());
        // Missing key → 401.
        let res = app
            .oneshot(request(
                "GET",
                "/apostille/api/v1/admin/setup/steps",
                None,
                None,
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
        // Wrong key → 403 + denied audit.
        let app = router(fix.state.clone());
        let res = app
            .oneshot(request(
                "GET",
                "/apostille/api/v1/admin/setup/steps",
                Some("setup_wrong"),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::FORBIDDEN);
        assert_eq!(audit_count(&fix.state, "setup.denied"), 1);
        // Right key → keys list without setup.* rows.
        let app = router(fix.state.clone());
        let res = app
            .oneshot(request(
                "GET",
                "/apostille/api/v1/admin/setup/steps",
                Some(KEY),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = json_body(res).await;
        assert!(body["keys"].is_array());
        for row in body["keys"].as_array().unwrap() {
            assert!(!row["key"].as_str().unwrap().starts_with("setup."));
        }
    }

    #[tokio::test]
    async fn steps_load_without_origin_like_a_browser() {
        let fix = fixture();
        let app = router(fix.state.clone());
        let res = app
            .oneshot(browser_get(
                "/apostille/api/v1/admin/setup/steps",
                Some(KEY),
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        assert!(json_body(res).await["keys"].is_array());
    }

    #[tokio::test]
    async fn set_key_roundtrip_and_guards() {
        let fix = fixture();
        // Bool + text + secret all seal (oneshot consumes the router,
        // so rebuild per request).
        for (key, key_type, value) in [
            ("acme.enabled", "bool", json!(true)),
            ("net.public_host", "text", json!("example.test")),
            ("telnyx.api_key", "secret", json!("s3cr3t")),
        ] {
            let app = router(fix.state.clone());
            let res = app
                .oneshot(request(
                    "POST",
                    &format!("/apostille/api/v1/admin/setup/keys/{key}"),
                    Some(KEY),
                    Some(json!({"type": key_type, "value": value})),
                ))
                .await
                .unwrap();
            assert_eq!(res.status(), StatusCode::OK, "key {key}");
        }
        // setup.* is mint/burn only.
        let app = router(fix.state.clone());
        let res = app
            .oneshot(request(
                "POST",
                "/apostille/api/v1/admin/setup/keys/setup.key_hash",
                Some(KEY),
                Some(json!({"type": "secret", "value": "evil"})),
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
        // Unregistered keys 422 (admin PUT allows them; setup does not).
        let app = router(fix.state.clone());
        let res = app
            .oneshot(request(
                "POST",
                "/apostille/api/v1/admin/setup/keys/made.up",
                Some(KEY),
                Some(json!({"type": "text", "value": "x"})),
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
        // Type mismatch 422s.
        let app = router(fix.state.clone());
        let res = app
            .oneshot(request(
                "POST",
                "/apostille/api/v1/admin/setup/keys/acme.enabled",
                Some(KEY),
                Some(json!({"type": "bool", "value": "yes-please"})),
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
        // The sealed rows read back (bool + text decrypt; secret hidden).
        let app = router(fix.state.clone());
        let res = app
            .oneshot(request(
                "GET",
                "/apostille/api/v1/admin/setup/steps",
                Some(KEY),
                None,
            ))
            .await
            .unwrap();
        let body = json_body(res).await;
        let rows = body["keys"].as_array().unwrap();
        let find = |key: &str| {
            rows.iter()
                .find(|r| r["key"].as_str().unwrap() == key)
                .cloned()
        };
        assert_eq!(find("acme.enabled").unwrap()["value"], true);
        assert_eq!(find("net.public_host").unwrap()["value"], "example.test");
        assert_eq!(find("telnyx.api_key").unwrap()["set"], true);
        assert!(find("telnyx.api_key").unwrap().get("value").is_none());
    }

    #[tokio::test]
    async fn enrol_start_setup_mode() {
        let fix = fixture();
        // Both invite and key → 400.
        let app = router(fix.state.clone());
        let res = app
            .oneshot(request(
                "POST",
                "/apostille/api/v1/admin/auth/passkey/enrol/start",
                None,
                Some(json!({"invite": "i", "setup_key": KEY, "display_name": "Op"})),
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
        // Neither → 400.
        let app = router(fix.state.clone());
        let res = app
            .oneshot(request(
                "POST",
                "/apostille/api/v1/admin/auth/passkey/enrol/start",
                None,
                Some(json!({"display_name": "Op"})),
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
        // Wrong key → gone shape (no oracle), denied audited.
        let app = router(fix.state.clone());
        let res = app
            .oneshot(request(
                "POST",
                "/apostille/api/v1/admin/auth/passkey/enrol/start",
                None,
                Some(json!({"setup_key": "setup_wrong", "display_name": "Op"})),
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::GONE);
        assert_eq!(json_body(res).await["error"], "invite invalid or expired");
        // Right key, no cert, no invite → ceremony starts as admin.
        let app = router(fix.state.clone());
        let res = app
            .oneshot(request(
                "POST",
                "/apostille/api/v1/admin/auth/passkey/enrol/start",
                None,
                Some(json!({"setup_key": KEY, "display_name": "Op"})),
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = json_body(res).await;
        assert_eq!(body["role"], "admin");
        assert_eq!(body["cn"], "setup");
        assert!(body["challenge_id"].is_string());
    }

    #[tokio::test]
    async fn enrol_finish_setup_mode_rejects_unknown_challenge() {
        let fix = fixture();
        let app = router(fix.state.clone());
        let res = app
            .oneshot(request(
                "POST",
                "/apostille/api/v1/admin/auth/passkey/enrol/finish",
                None,
                Some(json!({
                    "challenge_id": "nope",
                    "credential": {
                        "id": "AA",
                        "rawId": "AA",
                        "type": "public-key",
                        "response": {
                            "clientDataJSON": "AA",
                            "attestationObject": "AA",
                        },
                    },
                    "setup_key": KEY,
                })),
            ))
            .await
            .unwrap();
        // Challenge lookup fails before any key handling: no leak.
        assert_eq!(res.status(), StatusCode::GONE);
    }
}
