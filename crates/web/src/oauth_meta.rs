//! Meta OAuth handlers (spec `customer-auth.md` §11): start
//! builds the Meta auth URL; the callback (top-level GET from
//! Meta) exchanges, validates via `debug_token`, resolves
//! (sub-only — no email is ever fetched), sets the session cookie,
//! and 302s back into the SPA. Mirrors `oauth.rs` (Google)
//! handler-for-handler; Meta tokens are never stored.

use axum::{
    Form, Json,
    extract::{ConnectInfo, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{Html, IntoResponse, Response},
};
use rusqlite::OptionalExtension as _;
use serde_json::json;
use std::net::SocketAddr;

use super::oauth::UreqTransport;
use super::passkey;
use super::{AppState, store_unavailable, unix_now};
use auth::meta as flow;
use auth::oauth as core;
use auth::passkey as store;

/// Meta OAuth config: App ID (text) + App secret (redacting
/// `SecretText`) from sealed rows; base URLs default to Meta prod
/// (Graph API pin in `auth::meta`) and are overridden only by tests.
#[derive(Clone)]
pub struct MetaCfg {
    pub client_id: String,
    pub client_secret: config::SecretText,
    pub redirect_uri: String,
    pub auth_base: String,
    pub token_url: String,
    pub debug_url: String,
}

impl MetaCfg {
    pub fn from_values(client_id: &str, secret: config::SecretText, redirect_uri: &str) -> Self {
        Self {
            client_id: client_id.to_string(),
            client_secret: secret,
            redirect_uri: redirect_uri.to_string(),
            auth_base: flow::META_AUTH_URL.to_string(),
            token_url: flow::META_TOKEN_URL.to_string(),
            debug_url: flow::META_DEBUG_URL.to_string(),
        }
    }

    /// Both halves present and non-empty (endpoints fail closed).
    pub fn configured(&self) -> bool {
        !self.client_id.is_empty() && !self.client_secret.as_str().is_empty()
    }
}

/// `hb_meta` state cookie name (binds the state to this browser;
/// per-provider so concurrent Google/Apple/Meta flows coexist).
pub const META_COOKIE: &str = "hb_meta";

/// `hb_meta` cookie value: split binding, short-lived.
fn meta_cookie(binding: &str, secure: bool) -> String {
    let mut cookie =
        format!("{META_COOKIE}={binding}; HttpOnly; SameSite=Lax; Path=/; Max-Age=300");
    if secure {
        cookie.push_str("; Secure");
    }
    cookie
}

/// Authorization URL for the SPA top-level navigation (`url` crate
/// encodes every pair — no hand-rolled escaping). No `scope` param
/// by design: the default `public_profile` is all the sub-only
/// flow needs (spec §11.1).
fn auth_url(cfg: &MetaCfg, state: &str) -> Option<String> {
    let mut url = url::Url::parse(&cfg.auth_base).ok()?;
    url.query_pairs_mut()
        .append_pair("client_id", &cfg.client_id)
        .append_pair("redirect_uri", &cfg.redirect_uri)
        .append_pair("state", state);
    Some(url.into())
}

/// 302 to a fixed same-origin target (never reflected input — no
/// open redirect) with `Set-Cookie` lines attached.
fn redirect(target: &str, cookies: &[String]) -> Response {
    let mut headers = passkey::set_cookies(cookies);
    // Targets are fixed literals — a parse failure would be a bug.
    if let Ok(location) = HeaderValue::from_str(target) {
        headers.insert(header::LOCATION, location);
    }
    (StatusCode::FOUND, headers).into_response()
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StartBody {
    mode: Option<String>,
}

/// `POST /auth/meta/start` — mint state, return the Meta auth URL.
pub async fn start(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(body): Json<StartBody>,
) -> Response {
    let now = unix_now();
    if !passkey::origin_ok(&headers, state.admin_mtls_off) {
        return passkey::csrf_fail();
    }
    let ip = passkey::client_ip(addr);
    if !state.auth_limits.check(&format!("ms:{ip}"), now, 30, 60) {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            Json(json!({"error": "rate limited"})),
        )
            .into_response();
    }
    let Some(meta) = state.meta_cfg.as_ref().filter(|cfg| cfg.configured()) else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"error": "meta login unavailable"})),
        )
            .into_response();
    };
    let mode = match body.mode.as_deref() {
        None | Some("login") => core::OauthMode::Login,
        Some("link") => core::OauthMode::Link,
        Some(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "mode must be login or link"})),
            )
                .into_response();
        }
    };
    let Some(conn) = passkey::open_conn(&state) else {
        return store_unavailable();
    };
    let session = passkey::session_of(&conn, &headers, now);
    // Mode (b): explicit link (rule 4) — customer session + fresh
    // step-up, mirroring `register_start` mode b.
    let link_user = match (&session, mode) {
        (Some(active), core::OauthMode::Link) => {
            if active.role != auth::Role::Customer {
                return (
                    StatusCode::FORBIDDEN,
                    Json(json!({"error": "meta linking is for customer accounts"})),
                )
                    .into_response();
            }
            let fresh = store::get_account(&conn, &active.user_id)
                .ok()
                .flatten()
                .and_then(|account| account.step_up_at)
                .is_some_and(|at| now - at <= store::STEP_UP_WINDOW_SECS)
                || now - active.created_at <= store::STEP_UP_WINDOW_SECS;
            if !fresh {
                return (
                    StatusCode::FORBIDDEN,
                    Json(json!({"error": "step-up required: log in again first"})),
                )
                    .into_response();
            }
            Some(active.user_id.clone())
        }
        (None, core::OauthMode::Link) => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({"error": "unauthorized"})),
            )
                .into_response();
        }
        (_, core::OauthMode::Login) => None,
    };
    let handle = core::mint_state();
    let binding = core::mint_state();
    if core::store_state_kind(
        &conn,
        core::KIND_META,
        &handle,
        mode,
        link_user.as_deref(),
        &binding,
        now,
    )
    .is_err()
    {
        return store_unavailable();
    }
    let Some(url) = auth_url(meta, &handle) else {
        return store_unavailable();
    };
    let csrf = auth::csrf::mint_token();
    let secure = !state.admin_mtls_off;
    (
        StatusCode::OK,
        passkey::set_cookies(&[
            auth::csrf_cookie(&csrf, secure),
            meta_cookie(&binding, secure),
        ]),
        Json(json!({"auth_url": url})),
    )
        .into_response()
}

/// Meta appends its own params, so this DTO deliberately skips
/// `deny_unknown_fields` — only `code`/`state`/`error` are read.
#[derive(serde::Deserialize)]
pub(crate) struct CallbackQuery {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
}

/// `GET /auth/meta/callback` — exchange, verify, resolve, set the
/// session cookie, 302 back into the SPA. Every exit clears the
/// binding cookie.
pub async fn callback(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Query(query): Query<CallbackQuery>,
) -> Response {
    let now = unix_now();
    let secure = !state.admin_mtls_off;
    let clear = auth::clear_cookie(META_COOKIE, secure);
    let Some(meta) = state.meta_cfg.as_ref().filter(|cfg| cfg.configured()) else {
        return redirect(
            "/apostille/login?error=oauth_unavailable",
            std::slice::from_ref(&clear),
        );
    };
    let ip = passkey::client_ip(addr);
    if !state.auth_limits.check(&format!("mc:{ip}"), now, 60, 60) {
        return redirect(
            "/apostille/login?error=oauth_rate_limited",
            std::slice::from_ref(&clear),
        );
    }
    let Some(conn) = passkey::open_conn(&state) else {
        return redirect(
            "/apostille/login?error=oauth_failed",
            std::slice::from_ref(&clear),
        );
    };
    if query.error.is_some() {
        // User cancelled at Meta (or Meta refused). Best-effort
        // state burn so the handle cannot be replayed later.
        if let Some(handle) = query.state.as_deref() {
            let _ = core::take_state_kind(&conn, core::KIND_META, handle, now);
        }
        passkey::audit_auth(&conn, now, "anonymous", "meta_denied", "");
        return redirect(
            "/apostille/login?error=oauth_denied",
            std::slice::from_ref(&clear),
        );
    }
    let (Some(code), Some(handle)) = (query.code.as_deref(), query.state.as_deref()) else {
        passkey::audit_auth(&conn, now, "anonymous", "meta_failed", "missing params");
        return redirect(
            "/apostille/login?error=oauth_failed",
            std::slice::from_ref(&clear),
        );
    };
    let (link_user, mode, binding) =
        match core::take_state_kind(&conn, core::KIND_META, handle, now) {
            Ok(core::TakeOutcome::Taken {
                link_user,
                mode,
                binding,
            }) => (link_user, mode, binding),
            Ok(core::TakeOutcome::Corrupt) => {
                passkey::audit_auth(&conn, now, "anonymous", "meta_failed", "corrupt state");
                return redirect(
                    "/apostille/login?error=oauth_failed",
                    std::slice::from_ref(&clear),
                );
            }
            Ok(core::TakeOutcome::Missing) => {
                passkey::audit_auth(&conn, now, "anonymous", "meta_expired", "");
                return redirect(
                    "/apostille/login?error=oauth_expired",
                    std::slice::from_ref(&clear),
                );
            }
            Err(_) => {
                return redirect(
                    "/apostille/login?error=oauth_failed",
                    std::slice::from_ref(&clear),
                );
            }
        };
    let jar = headers
        .get(header::COOKIE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    let presented = auth::cookie_value(jar, META_COOKIE).unwrap_or("");
    // The binding is 256-bit random per flow, single-use, and 5-min
    // lived, and the row is already consumed — a mismatch oracle
    // buys an attacker nothing, so plain equality is fine.
    if presented.is_empty() || presented != binding {
        passkey::audit_auth(&conn, now, "anonymous", "meta_failed", "binding");
        return redirect(
            "/apostille/login?error=oauth_failed",
            std::slice::from_ref(&clear),
        );
    }
    let call = meta.clone();
    let code_owned = code.to_string();
    let outcome = tokio::task::spawn_blocking(move || {
        let transport = UreqTransport::new();
        let user_token = flow::exchange_code(
            &transport,
            &call.token_url,
            &code_owned,
            &call.client_id,
            call.client_secret.as_str(),
            &call.redirect_uri,
        )?;
        flow::debug_verify(
            &transport,
            &call.debug_url,
            &user_token,
            &call.client_id,
            call.client_secret.as_str(),
            now,
        )
    })
    .await;
    let verified = match outcome {
        Ok(Ok(verified)) => verified,
        Ok(Err(core::OAuthError::NotConfigured)) => {
            return redirect(
                "/apostille/login?error=oauth_unavailable",
                std::slice::from_ref(&clear),
            );
        }
        Ok(Err(core::OAuthError::Exchange(status))) => {
            passkey::audit_auth(
                &conn,
                now,
                "anonymous",
                "meta_failed",
                &format!("exchange:{status}"),
            );
            return redirect(
                "/apostille/login?error=oauth_failed",
                std::slice::from_ref(&clear),
            );
        }
        Ok(Err(core::OAuthError::InvalidToken(tag))) => {
            passkey::audit_auth(&conn, now, "anonymous", "meta_failed", tag);
            return redirect(
                "/apostille/login?error=oauth_failed",
                std::slice::from_ref(&clear),
            );
        }
        Ok(Err(core::OAuthError::Transport(_))) => {
            passkey::audit_auth(&conn, now, "anonymous", "meta_failed", "transport");
            return redirect(
                "/apostille/login?error=oauth_failed",
                std::slice::from_ref(&clear),
            );
        }
        Err(_) => {
            return redirect(
                "/apostille/login?error=oauth_failed",
                std::slice::from_ref(&clear),
            );
        }
    };
    if mode == core::OauthMode::Link {
        return link_callback(
            &conn,
            &headers,
            &clear,
            link_user.as_deref(),
            &verified,
            now,
        );
    }
    let outcome = match flow::resolve_meta(&conn, &verified.sub, now) {
        Ok(outcome) => outcome,
        Err(_) => {
            return redirect(
                "/apostille/login?error=oauth_failed",
                std::slice::from_ref(&clear),
            );
        }
    };
    let (user_id, fresh_account) = match outcome {
        // Defensive: `resolve_meta` never emits this (sub-only
        // flow always mints fresh) — the arm exists so a future
        // resolver change fails closed to the explicit-link page.
        core::ResolveOutcome::PromptExplicitLink => {
            passkey::audit_auth(
                &conn,
                now,
                &auth::hash_str(&verified.sub),
                "meta_failed",
                "link required",
            );
            return redirect(
                "/apostille/login?error=link_required",
                std::slice::from_ref(&clear),
            );
        }
        core::ResolveOutcome::Login { user_id } => (user_id, false),
        core::ResolveOutcome::Register { user_id } => (user_id, true),
    };
    // Per-account gate, now that the user is known.
    if !state
        .auth_limits
        .check(&format!("mc:{user_id}"), now, 10, 60)
    {
        return redirect(
            "/apostille/login?error=oauth_rate_limited",
            std::slice::from_ref(&clear),
        );
    }
    match store::get_account(&conn, &user_id).unwrap_or(None) {
        Some(account) if account.locked_at.is_some() => {
            passkey::audit_auth(&conn, now, &user_id, "account_locked", "login refused");
            return redirect(
                "/apostille/login?error=account_locked",
                std::slice::from_ref(&clear),
            );
        }
        Some(account) if account.role != auth::Role::Customer.as_str() => {
            // Staff log in with passkey+mTLS (spec §5), never Meta.
            passkey::audit_auth(&conn, now, &user_id, "meta_failed", "role");
            return redirect(
                "/apostille/login?error=oauth_failed",
                std::slice::from_ref(&clear),
            );
        }
        None => {
            return redirect(
                "/apostille/login?error=oauth_failed",
                std::slice::from_ref(&clear),
            );
        }
        Some(_) => {}
    }
    let session_id = auth::mint_session_id();
    if auth::create_session(
        &conn,
        &session_id,
        &auth::NewSession {
            user_id: &user_id,
            role: auth::Role::Customer,
            customer_ref: Some(&user_id),
            now,
            ip: &ip,
            user_agent: passkey::user_agent(&headers),
        },
    )
    .is_err()
    {
        return redirect(
            "/apostille/login?error=oauth_failed",
            std::slice::from_ref(&clear),
        );
    }
    let _ = store::clear_fallback(&conn, &user_id);
    let _ = core::stamp_state_user(&conn, handle, &user_id);
    let rotation = auth::csrf::mint_token();
    passkey::audit_auth(
        &conn,
        now,
        &user_id,
        "oauth_login",
        if fresh_account { "new_account" } else { "" },
    );
    redirect(
        "/apostille/login",
        &[
            clear.clone(),
            auth::csrf_cookie(&rotation, secure),
            auth::session_cookie(auth::PUB_COOKIE, &session_id, secure),
        ],
    )
}

/// Link-mode callback tail: the session must still own the flow,
/// and the sub must not belong to someone else (never a merge).
fn link_callback(
    conn: &rusqlite::Connection,
    headers: &HeaderMap,
    clear: &String,
    link_user: Option<&str>,
    verified: &flow::VerifiedMeta,
    now: i64,
) -> Response {
    let Some(user_id) = link_user else {
        passkey::audit_auth(conn, now, "anonymous", "meta_failed", "link session");
        return redirect(
            "/apostille/login?error=oauth_failed",
            std::slice::from_ref(clear),
        );
    };
    let session = passkey::session_of(conn, headers, now);
    let owns_flow = session
        .as_ref()
        .is_some_and(|active| active.user_id == user_id && active.role == auth::Role::Customer);
    if !owns_flow {
        passkey::audit_auth(conn, now, user_id, "meta_failed", "link session");
        return redirect(
            "/apostille/login?error=oauth_failed",
            std::slice::from_ref(clear),
        );
    }
    match flow::find_meta_owner(conn, &verified.sub).unwrap_or(None) {
        Some(owner) if owner != user_id => {
            passkey::audit_auth(conn, now, user_id, "meta_failed", "already linked");
            return redirect(
                "/apostille/login?error=already_linked",
                std::slice::from_ref(clear),
            );
        }
        Some(_) => {
            // Idempotent: this account already owns the sub.
            passkey::audit_auth(
                conn,
                now,
                user_id,
                "method_linked",
                &auth::hash_str(&verified.sub),
            );
            return redirect("/apostille/security", std::slice::from_ref(clear));
        }
        None => {}
    }
    if flow::insert_meta_link(conn, user_id, &verified.sub).is_err() {
        // Lost a double-submit race (two states, one sub): the
        // winner owns it — recheck before failing.
        if flow::find_meta_owner(conn, &verified.sub)
            .unwrap_or(None)
            .as_deref()
            == Some(user_id)
        {
            return redirect("/apostille/security", std::slice::from_ref(clear));
        }
        return redirect(
            "/apostille/login?error=oauth_failed",
            std::slice::from_ref(clear),
        );
    }
    passkey::audit_auth(
        conn,
        now,
        user_id,
        "method_linked",
        &auth::hash_str(&verified.sub),
    );
    redirect("/apostille/security", std::slice::from_ref(clear))
}

#[derive(serde::Deserialize)]
pub(crate) struct DeleteForm {
    signed_request: String,
}

#[derive(serde::Deserialize)]
pub(crate) struct DeleteStatusQuery {
    code: String,
}

/// `POST /auth/meta/delete-request` — Meta's Data Deletion Request
/// Callback (spec §11.9): form-encoded `signed_request`, HMAC'd
/// under the app secret. A verified request purges the linked
/// user's PII (identity rows, submissions, shipments, labels,
/// files) and mints a confirmation code; unknown subs get the
/// same success shape with status `unknown` (Meta: disregard —
/// nothing to delete). No session, no CSRF: server-to-server,
/// the signature IS the auth. The response is exactly Meta's
/// shape: `{url, confirmation_code}`.
pub async fn delete_request(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Form(form): Form<DeleteForm>,
) -> Response {
    let now = unix_now();
    let ip = passkey::client_ip(addr);
    if !state.auth_limits.check(&format!("md:{ip}"), now, 10, 60) {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            Json(json!({"error": "rate limited"})),
        )
            .into_response();
    }
    let Some(meta) = state.meta_cfg.as_ref().filter(|cfg| cfg.configured()) else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"error": "meta login unavailable"})),
        )
            .into_response();
    };
    let Some(conn) = passkey::open_conn(&state) else {
        return store_unavailable();
    };
    let Some(sub) = flow::parse_deletion_request(&form.signed_request, meta.client_secret.as_str())
    else {
        passkey::audit_auth(
            &conn,
            now,
            "anonymous",
            "meta_delete.rejected",
            "bad signature",
        );
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "invalid signed request"})),
        )
            .into_response();
    };
    // 128-bit hex — the confirmation code (Meta: alphanumeric).
    let code = auth::passkey::mint_user_id();
    let status = match flow::find_meta_owner(&conn, &sub).unwrap_or(None) {
        Some(user_id) => {
            let store = handbook_store::store::Store::new(&state.store_root);
            let p2 = match handbook_store::db::open_p2(&store) {
                Ok(conn) => conn,
                Err(_) => return store_unavailable(),
            };
            // Data first, identity last: if erasing data fails, Meta's
            // retry still finds the owner and can finish the job.
            if handbook_store::db::erase_customer_data(&conn, &p2, &store, &user_id, now).is_err() {
                return store_unavailable();
            }
            if store::erase_identity(&conn, &user_id).is_err() {
                return store_unavailable();
            }
            passkey::audit_auth(&conn, now, "system", "meta_data_deleted", &code);
            "completed"
        }
        None => {
            passkey::audit_auth(&conn, now, "system", "meta_delete.unknown", &code);
            "unknown"
        }
    };
    if conn
        .execute(
            "INSERT INTO meta_deletion(code, status, created_at, completed_at)
             VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![code, status, now, now],
        )
        .is_err()
    {
        return store_unavailable();
    }
    Json(json!({
        "url": format!("{}?code={code}", state.meta_delete_status_base),
        "confirmation_code": code,
    }))
    .into_response()
}

/// `GET /auth/meta/delete-status?code=` — the human-readable
/// deletion receipt Meta requires the confirmation `url` to
/// serve: status in prose plus what was deleted and what the
/// law keeps. The code is an unguessable bearer; no login.
/// Unknown codes 404 with the same explanatory shape.
pub async fn delete_status(
    State(state): State<AppState>,
    Query(query): Query<DeleteStatusQuery>,
) -> Response {
    let shaped = query.code.len() == 32 && query.code.bytes().all(|b| b.is_ascii_hexdigit());
    let row: Option<(String, i64)> = if !shaped {
        None
    } else {
        passkey::open_conn(&state).and_then(|conn| {
            conn.query_row(
                "SELECT status, completed_at FROM meta_deletion WHERE code = ?1",
                [&query.code],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .unwrap_or(None)
        })
    };
    // Never echo an unshaped code (reflected XSS); shaped but
    // unknown codes are safe hex by construction.
    let shown = if shaped {
        query.code.as_str()
    } else {
        "(malformed)"
    };
    match row {
        Some((status, at)) => Html(delete_status_page(shown, &status, at)).into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Html(delete_status_page(shown, "unknown code", -1)),
        )
            .into_response(),
    }
}

/// Server-rendered receipt (no JS, crawler-safe): status first,
/// then the deletion/retention account in plain language.
fn delete_status_page(code: &str, status: &str, completed_at: i64) -> String {
    let body = match status {
        "completed" => format!(
            "<p>Completed (unix time {completed_at}). All personal data held for this request \
             has been deleted: the account, login methods (passkeys and linked OAuth \
             identities), sessions, submitted forms and documents, shipments and shipping \
             labels, and stored files.</p><p>What remains: append-only security-audit entries \
             under an opaque identifier that can no longer be linked to any person, and \
             payment records where tax law requires retention (none are held today).</p>"
        ),
        "unknown" => "<p>No data is held for this request — either it was already deleted or \
             the identifier was never linked to an account. Nothing further is required.</p>"
            .to_string(),
        _ => "<p>This code matches no deletion request. Codes are issued by the Meta data \
             deletion callback; check the link and try again.</p>"
            .to_string(),
    };
    format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
         <title>Data deletion status</title></head><body><main>\
         <h1>Data deletion status</h1>\
         <p>Request <code>{code}</code>: <strong>{status}</strong>.</p>{body}\
         </main></body></html>"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use std::io::{Read, Write};
    use tower::ServiceExt;

    fn test_app_with_meta(meta: Option<MetaCfg>) -> (tempfile::TempDir, axum::Router) {
        let tmp = tempfile::tempdir().unwrap();
        let store = handbook_store::store::Store::new(tmp.path());
        let conn = handbook_store::db::open(&store).unwrap();
        handbook_store::db::migrate(&conn).unwrap();
        auth::migrate_auth_tables(&conn).unwrap();
        config::migrate_config_tables(&conn).unwrap();
        drop(conn);
        let dir = tmp.path().to_str().unwrap().to_string();
        let state = AppState {
            spa_index: std::path::Path::new(&dir).join("index.html"),
            static_dir: std::path::PathBuf::from(&dir),
            store_root: tmp.path().to_path_buf(),
            bus: crate::ws_bus::Bus::new(),
            kek: None,
            admin_mtls_off: true,
            admin_allow_ips: config::loopback_allow_ips(),
            admin_spa_dir: std::path::PathBuf::from("test-admin-spa"),
            media_terms_read: false,
            label_cfg: crate::p2_labels::LabelCfg::default(),
            voice_cfg: voice::VoiceConfig::default(),
            voice_api_key: None,
            webauthn: std::sync::Arc::new(passkey::test_webauthn()),
            auth_limits: std::sync::Arc::new(passkey::RateBuckets::default()),
            demo_limits: std::sync::Arc::new(passkey::RateBuckets::default()),
            draft_runs: std::sync::Arc::new(crate::draft::DraftRuns::default()),
            config_cache: std::sync::Arc::new(config::CachedReader::default()),
            google_cfg: None,
            apple_cfg: None,
            meta_cfg: meta,
            mailer: None,
            magic_link_base: "https://example.test/apostille/login".to_string(),
            notify_poke: std::sync::Arc::new(tokio::sync::Notify::new()),
            notify_enabled: false,
            notify_daily_cap: 3,
            notify_postal: String::new(),
            notify_limits: std::sync::Arc::new(crate::passkey::RateBuckets::default()),
            notify_base: "https://example.test/apostille".to_string(),
            meta_delete_status_base: "https://example.test/auth/meta/delete-status".to_string(),
            fakes: config::Modes::default(),
            fedex_live: crate::fedex_live::FedexLiveState::default(),
        };
        let app = crate::public_router(state, &dir, &dir, &dir);
        (tmp, app)
    }

    /// Build a stub-pointed cfg through the REAL sealed-rows path
    /// (no test-only `SecretText` constructor exists on purpose).
    fn stub_cfg(base: &str, client_id: &str) -> MetaCfg {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        config::migrate_config_tables(&conn).unwrap();
        let kek = config::Kek::generate();
        for (key, key_type, value) in [
            (
                config::keys::META_CLIENT_ID,
                config::ConfigType::Text,
                client_id.as_bytes(),
            ),
            (
                config::keys::META_CLIENT_SECRET,
                config::ConfigType::Secret,
                b"secret-stub",
            ),
        ] {
            config::set_value(
                &conn,
                &kek,
                &config::SetValue {
                    version: 1,
                    key,
                    key_type,
                    plaintext: value,
                    now: 1000,
                    actor: "test",
                },
            )
            .unwrap();
        }
        let (cfg, warnings) = config::RuntimeConfig::load(&conn, &kek).unwrap();
        assert!(warnings.is_empty());
        let mut meta = MetaCfg::from_values(
            cfg.meta_client_id.as_deref().unwrap(),
            cfg.meta_client_secret.unwrap(),
            "https://example.test/auth/meta/callback",
        );
        meta.auth_base = format!("{base}/auth");
        meta.token_url = format!("{base}/oauth/access_token");
        meta.debug_url = format!("{base}/debug_token");
        meta
    }

    /// Minimal port-0 HTTP stub: fixed token + debug bodies.
    /// `Connection: close` on every response keeps the accept loop
    /// to one request per connection (no keep-alive framing).
    struct StubMeta {
        base: String,
        addr: SocketAddr,
        handle: Option<std::thread::JoinHandle<()>>,
    }

    impl StubMeta {
        fn run(token: (u16, String), debug: (u16, String)) -> Self {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let addr = listener.local_addr().unwrap();
            let base = format!("http://{addr}");
            let handle = std::thread::spawn(move || {
                for _ in 0..16 {
                    let Ok((mut stream, _)) = listener.accept() else {
                        break;
                    };
                    let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(5)));
                    let mut head = Vec::new();
                    let mut chunk = [0u8; 4096];
                    while let Ok(n) = stream.read(&mut chunk) {
                        if n == 0 {
                            break;
                        }
                        head.extend_from_slice(&chunk[..n]);
                        if head.windows(4).any(|w| w == b"\r\n\r\n") || head.len() > 16384 {
                            break;
                        }
                    }
                    let header_end = head
                        .windows(4)
                        .position(|w| w == b"\r\n\r\n")
                        .map(|i| i + 4)
                        .unwrap_or(head.len());
                    // Consume the body (if any) before responding:
                    // closing with unread received bytes RSTs the
                    // connection and flakes the client read.
                    let content_len: usize = String::from_utf8_lossy(&head[..header_end])
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .and_then(|v| v.trim().parse().ok())
                        })
                        .unwrap_or(0);
                    let mut remaining =
                        content_len.saturating_sub(head.len().saturating_sub(header_end));
                    while remaining > 0 {
                        let mut body_buf = [0u8; 4096];
                        let take = remaining.min(body_buf.len());
                        match stream.read(&mut body_buf[..take]) {
                            Ok(0) | Err(_) => break,
                            Ok(n) => remaining -= n,
                        }
                    }
                    let text = String::from_utf8_lossy(&head);
                    if text.starts_with("QUIT") {
                        break;
                    }
                    let (status, body) = if text.starts_with("GET /oauth/access_token") {
                        token.clone()
                    } else if text.starts_with("GET /debug_token") {
                        debug.clone()
                    } else {
                        (404u16, "{}".to_string())
                    };
                    let _ = write!(
                        stream,
                        "HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = stream.flush();
                }
            });
            Self {
                base,
                addr,
                handle: Some(handle),
            }
        }
    }

    impl Drop for StubMeta {
        fn drop(&mut self) {
            if let Some(handle) = self.handle.take() {
                if let Ok(mut quit) = std::net::TcpStream::connect(self.addr) {
                    let _ = quit.write_all(b"QUIT / HTTP/1.1\r\n\r\n");
                }
                let _ = handle.join();
            }
        }
    }

    const TOKEN_OK: &str = r#"{"access_token":"user-token-1","token_type":"bearer"}"#;

    fn debug(sub: &str, app_id: &str) -> String {
        format!(
            r#"{{"data":{{"app_id":"{app_id}","is_valid":true,"user_id":"{sub}","expires_at":2000000000,"scopes":["public_profile"]}}}}"#
        )
    }

    fn addr() -> ConnectInfo<SocketAddr> {
        ConnectInfo("127.0.0.1:9".parse().unwrap())
    }

    fn post_json(uri: &str, cookies: &str, body: &str) -> Request<Body> {
        let mut req = Request::builder()
            .method("POST")
            .uri(uri)
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::ORIGIN, "http://localhost")
            .extension(addr());
        if !cookies.is_empty() {
            req = req.header(header::COOKIE, cookies);
        }
        req.body(Body::from(body.to_string())).unwrap()
    }

    fn get(uri: &str, cookies: &str) -> Request<Body> {
        let mut req = Request::builder().method("GET").uri(uri).extension(addr());
        if !cookies.is_empty() {
            req = req.header(header::COOKIE, cookies);
        }
        req.body(Body::empty()).unwrap()
    }

    async fn start_login(app: &axum::Router) -> (StatusCode, serde_json::Value, String) {
        let res = app
            .clone()
            .oneshot(post_json(
                "/apostille/api/v1/auth/meta/start",
                "",
                r#"{"mode":"login"}"#,
            ))
            .await
            .unwrap();
        let status = res.status();
        let cookies = res
            .headers()
            .get_all(header::SET_COOKIE)
            .iter()
            .filter_map(|v| v.to_str().ok())
            .collect::<Vec<_>>()
            .join("; ");
        let body = axum::body::to_bytes(res.into_body(), 65536).await.unwrap();
        (status, serde_json::from_slice(&body).unwrap(), cookies)
    }

    fn state_of(auth_url: &str) -> String {
        url::Url::parse(auth_url)
            .unwrap()
            .query_pairs()
            .find(|(key, _)| key == "state")
            .map(|(_, value)| value.to_string())
            .unwrap()
    }

    fn location_of(res: &Response) -> String {
        res.headers()
            .get(header::LOCATION)
            .unwrap()
            .to_str()
            .unwrap()
            .to_string()
    }

    #[test]
    fn configured_needs_both_halves() {
        let secret = config::SecretText::from("x".to_string());
        let mut cfg = MetaCfg::from_values("cid", secret, "https://x/cb");
        assert!(cfg.configured());
        cfg.client_id.clear();
        assert!(!cfg.configured());
    }

    #[test]
    fn auth_url_shape() {
        let cfg = MetaCfg::from_values(
            "cid",
            config::SecretText::from("s".to_string()),
            "https://example.test/cb",
        );
        let url = auth_url(&cfg, "state-1").unwrap();
        assert!(url.starts_with(flow::META_AUTH_URL), "{url}");
        assert!(!url.contains("scope="), "{url}");
        assert!(url.contains("state=state-1"), "{url}");
        assert!(
            url.contains("redirect_uri=https%3A%2F%2Fexample.test%2Fcb"),
            "{url}"
        );
    }

    #[test]
    fn meta_cookie_attrs() {
        let cookie = meta_cookie("bind-1", true);
        assert!(cookie.starts_with("hb_meta=bind-1;"), "{cookie}");
        for attr in [
            "HttpOnly",
            "SameSite=Lax",
            "Path=/",
            "Max-Age=300",
            "Secure",
        ] {
            assert!(cookie.contains(attr), "{cookie}");
        }
        assert!(!meta_cookie("bind-1", false).contains("Secure"));
    }

    #[tokio::test]
    async fn start_503_when_unconfigured() {
        let (_tmp, app) = test_app_with_meta(None);
        let res = app
            .clone()
            .oneshot(post_json(
                "/apostille/api/v1/auth/meta/start",
                "",
                r#"{"mode":"login"}"#,
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);
        let res = app
            .clone()
            .oneshot(get("/auth/meta/callback?code=x&state=y", ""))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::FOUND);
        assert!(location_of(&res).contains("oauth_unavailable"));
    }

    #[tokio::test]
    async fn start_shape_cookies_and_guards() {
        let stub = StubMeta::run((200, TOKEN_OK.to_string()), (200, "unused".to_string()));
        let (_tmp, app) = test_app_with_meta(Some(stub_cfg(&stub.base, "cid-stub")));
        // Evil origin → 403 even on the dev seam.
        let res = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/apostille/api/v1/auth/meta/start")
                    .header(header::CONTENT_TYPE, "application/json")
                    .header(header::ORIGIN, "https://evil.test")
                    .extension(addr())
                    .body(Body::from(r#"{"mode":"login"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::FORBIDDEN);
        // Bad mode → 400; unknown fields rejected.
        for body in [r#"{"mode":"sso"}"#, r#"{"mode":"login","role":"admin"}"#] {
            let res = app
                .clone()
                .oneshot(post_json("/apostille/api/v1/auth/meta/start", "", body))
                .await
                .unwrap();
            assert!(
                res.status() == StatusCode::BAD_REQUEST
                    || res.status() == StatusCode::UNPROCESSABLE_ENTITY,
                "{body}"
            );
        }
        // Link without a session → 401.
        let res = app
            .clone()
            .oneshot(post_json(
                "/apostille/api/v1/auth/meta/start",
                "",
                r#"{"mode":"link"}"#,
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
        // Happy start: auth URL + binding + CSRF cookies.
        let (status, body, cookies) = start_login(&app).await;
        assert_eq!(status, StatusCode::OK);
        let auth_url = body["auth_url"].as_str().unwrap();
        assert!(auth_url.contains("cid-stub"), "{auth_url}");
        assert!(auth_url.contains("state="), "{auth_url}");
        assert!(cookies.contains(META_COOKIE), "{cookies}");
        assert!(cookies.contains(auth::CSRF_COOKIE), "{cookies}");
        assert!(!state_of(auth_url).is_empty());
    }

    #[tokio::test]
    async fn callback_denied_expired_and_mismatch() {
        let stub = StubMeta::run((200, TOKEN_OK.to_string()), (200, "unused".to_string()));
        let (_tmp, app) = test_app_with_meta(Some(stub_cfg(&stub.base, "cid-stub")));
        // User cancelled at Meta → denied (state burned best-effort).
        let res = app
            .clone()
            .oneshot(get(
                "/auth/meta/callback?error=access_denied&state=nope",
                "",
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::FOUND);
        assert!(location_of(&res).contains("oauth_denied"));
        // Unknown state → expired.
        let res = app
            .clone()
            .oneshot(get("/auth/meta/callback?code=c&state=nope", ""))
            .await
            .unwrap();
        assert!(location_of(&res).contains("oauth_expired"));
        // Binding stripped → failed.
        let (status, body, _) = start_login(&app).await;
        assert_eq!(status, StatusCode::OK);
        let uri = format!(
            "/auth/meta/callback?code=c&state={}",
            state_of(body["auth_url"].as_str().unwrap())
        );
        let res = app.clone().oneshot(get(&uri, "")).await.unwrap();
        assert!(location_of(&res).contains("oauth_failed"));
    }

    #[tokio::test]
    async fn callback_full_loop_mints_session() {
        let stub = StubMeta::run(
            (200, TOKEN_OK.to_string()),
            (200, debug("scoped-1", "cid-stub")),
        );
        let (tmp, app) = test_app_with_meta(Some(stub_cfg(&stub.base, "cid-stub")));
        let (status, body, cookies) = start_login(&app).await;
        assert_eq!(status, StatusCode::OK);
        let uri = format!(
            "/auth/meta/callback?code=code-1&state={}",
            state_of(body["auth_url"].as_str().unwrap())
        );
        let res = app.clone().oneshot(get(&uri, &cookies)).await.unwrap();
        assert_eq!(res.status(), StatusCode::FOUND);
        let headers = res.headers().clone();
        assert_eq!(location_of(&res), "/apostille/login");
        let set = headers
            .get_all(header::SET_COOKIE)
            .iter()
            .filter_map(|v| v.to_str().ok())
            .collect::<Vec<_>>()
            .join("; ");
        assert!(set.contains(auth::PUB_COOKIE), "{set}");
        assert!(set.contains(&format!("{META_COOKIE}=;")), "{set}");
        // The minted session works.
        let session = auth::cookie_value(&set, auth::PUB_COOKIE).unwrap();
        let res = app
            .clone()
            .oneshot(get(
                "/apostille/api/v1/auth/me",
                &format!("{}={session}", auth::PUB_COOKIE),
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        // The link row exists for the new account (NULL email hash).
        let store = handbook_store::store::Store::new(tmp.path());
        let conn = handbook_store::db::open(&store).unwrap();
        let owner = flow::find_meta_owner(&conn, "scoped-1")
            .unwrap()
            .expect("meta link row");
        let hash: Option<String> = conn
            .query_row(
                "SELECT email_hash FROM identity_link WHERE user_id = ?1 AND method = 'meta'",
                [&owner],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(hash, None);
    }

    #[tokio::test]
    async fn callback_always_fresh_despite_same_address() {
        // A same-address Google account exists — but Meta never
        // fetches email, so the login mints fresh (link_required
        // NEVER fires for Meta).
        let stub = StubMeta::run(
            (200, TOKEN_OK.to_string()),
            (200, debug("scoped-9", "cid-stub")),
        );
        let (tmp, app) = test_app_with_meta(Some(stub_cfg(&stub.base, "cid-stub")));
        let store = handbook_store::store::Store::new(tmp.path());
        let conn = handbook_store::db::open(&store).unwrap();
        auth::passkey::ensure_account(&conn, "u-g", 1000).unwrap();
        core::insert_google_link(&conn, "u-g", "g-sub-1", "m@example.test", true).unwrap();
        drop(conn);
        let (status, body, cookies) = start_login(&app).await;
        assert_eq!(status, StatusCode::OK);
        let uri = format!(
            "/auth/meta/callback?code=code-1&state={}",
            state_of(body["auth_url"].as_str().unwrap())
        );
        let res = app.clone().oneshot(get(&uri, &cookies)).await.unwrap();
        assert_eq!(location_of(&res), "/apostille/login");
        let conn = handbook_store::db::open(&store).unwrap();
        let owner = flow::find_meta_owner(&conn, "scoped-9")
            .unwrap()
            .expect("meta link row");
        assert_ne!(owner, "u-g");
    }

    #[tokio::test]
    async fn callback_link_mode_adds_method() {
        let stub = StubMeta::run(
            (200, TOKEN_OK.to_string()),
            (200, debug("scoped-link", "cid-stub")),
        );
        let (tmp, app) = test_app_with_meta(Some(stub_cfg(&stub.base, "cid-stub")));
        // Seed account + fresh session directly.
        let store = handbook_store::store::Store::new(tmp.path());
        let conn = handbook_store::db::open(&store).unwrap();
        let now = unix_now();
        auth::passkey::ensure_account(&conn, "u-link", now).unwrap();
        auth::create_session(
            &conn,
            "sess-link",
            &auth::NewSession {
                user_id: "u-link",
                role: auth::Role::Customer,
                customer_ref: Some("u-link"),
                now,
                ip: "127.0.0.1",
                user_agent: "test",
            },
        )
        .unwrap();
        drop(conn);
        let session_cookie = format!("{}=sess-link", auth::PUB_COOKIE);
        let res = app
            .clone()
            .oneshot(post_json(
                "/apostille/api/v1/auth/meta/start",
                &session_cookie,
                r#"{"mode":"link"}"#,
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let cookies = res
            .headers()
            .get_all(header::SET_COOKIE)
            .iter()
            .filter_map(|v| v.to_str().ok())
            .collect::<Vec<_>>()
            .join("; ");
        let body = axum::body::to_bytes(res.into_body(), 65536).await.unwrap();
        let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let uri = format!(
            "/auth/meta/callback?code=code-1&state={}",
            state_of(body["auth_url"].as_str().unwrap())
        );
        let res = app
            .clone()
            .oneshot(get(&uri, &format!("{cookies}; {session_cookie}")))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::FOUND);
        assert_eq!(location_of(&res), "/apostille/security");
        let conn = handbook_store::db::open(&store).unwrap();
        assert_eq!(
            flow::find_meta_owner(&conn, "scoped-link").unwrap(),
            Some("u-link".to_string())
        );
    }

    #[tokio::test]
    async fn callback_debug_app_mismatch_fails_closed() {
        // Stub answers app_id=cid-stub; cfg expects someone else.
        let stub = StubMeta::run(
            (200, TOKEN_OK.to_string()),
            (200, debug("scoped-1", "cid-stub")),
        );
        let (_tmp, app) = test_app_with_meta(Some(stub_cfg(&stub.base, "cid-other")));
        let (status, body, cookies) = start_login(&app).await;
        assert_eq!(status, StatusCode::OK);
        let uri = format!(
            "/auth/meta/callback?code=code-1&state={}",
            state_of(body["auth_url"].as_str().unwrap())
        );
        let res = app.clone().oneshot(get(&uri, &cookies)).await.unwrap();
        assert!(location_of(&res).contains("oauth_failed"));
    }

    fn post_form(uri: &str, body: &str) -> Request<Body> {
        Request::builder()
            .method("POST")
            .uri(uri)
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .extension(addr())
            .body(Body::from(body.to_string()))
            .unwrap()
    }

    fn deletion_signed(sub: &str, secret: &str) -> String {
        use aws_lc_rs::hmac;
        use base64::Engine as _;
        let engine = base64::engine::general_purpose::URL_SAFE_NO_PAD;
        let payload = engine.encode(format!(
            r#"{{"algorithm":"HMAC-SHA256","user_id":"{sub}"}}"#
        ));
        let key = hmac::Key::new(hmac::HMAC_SHA256, secret.as_bytes());
        let sig = engine.encode(hmac::sign(&key, payload.as_bytes()).as_ref());
        format!("signed_request={sig}.{payload}")
    }

    fn seed_meta_user(tmp: &tempfile::TempDir, sub: &str) {
        use handbook_store::store::Store;
        let store = Store::new(tmp.path());
        let conn = handbook_store::db::open(&store).unwrap();
        auth::passkey::ensure_account(&conn, "u-9", 1000).unwrap();
        auth::meta::insert_meta_link(&conn, "u-9", sub).unwrap();
        let p2 = handbook_store::db::open_p2(&store).unwrap();
        handbook_store::db::migrate_p2(&p2).unwrap();
        p2.execute(
            "INSERT INTO form_submission(form_template_id, authority_id, customer_ref,
                 values_json, status, created_at)
             VALUES (1, 0, 'u-9', '{\"name\":\"Meta\"}', 'draft', 1000)",
            [],
        )
        .unwrap();
    }

    #[tokio::test]
    async fn delete_request_purges_and_mints_receipt() {
        let stub = StubMeta::run((200, String::new()), (200, String::new()));
        let (tmp, app) = test_app_with_meta(Some(stub_cfg(&stub.base, "cid-x")));
        seed_meta_user(&tmp, "asid-1");
        let res = app
            .clone()
            .oneshot(post_form(
                "/auth/meta/delete-request",
                &deletion_signed("asid-1", "secret-stub"),
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = axum::body::to_bytes(res.into_body(), 4096).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let code = json["confirmation_code"].as_str().unwrap();
        assert_eq!(code.len(), 32);
        assert_eq!(
            json["url"],
            format!("https://example.test/auth/meta/delete-status?code={code}")
        );
        // Status page renders the completed receipt as HTML.
        let res = app
            .clone()
            .oneshot(get(&format!("/auth/meta/delete-status?code={code}"), ""))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = axum::body::to_bytes(res.into_body(), 8192).await.unwrap();
        let html = String::from_utf8(body.to_vec()).unwrap();
        assert!(html.contains("completed"), "{html}");
        assert!(html.contains("security-audit"), "{html}");
        // Identity + PII rows are gone.
        let store = handbook_store::store::Store::new(tmp.path());
        let conn = handbook_store::db::open(&store).unwrap();
        let left: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM account WHERE user_id = 'u-9'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(left, 0);
        let p2 = handbook_store::db::open_p2(&store).unwrap();
        let subs: i64 = p2
            .query_row(
                "SELECT COUNT(*) FROM form_submission WHERE customer_ref = 'u-9'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(subs, 0);
    }

    #[tokio::test]
    async fn delete_request_unknown_and_guards() {
        let stub = StubMeta::run((200, String::new()), (200, String::new()));
        let (tmp, app) = test_app_with_meta(Some(stub_cfg(&stub.base, "cid-x")));
        // Unknown sub: success shape, status unknown (Meta: disregard).
        let res = app
            .clone()
            .oneshot(post_form(
                "/auth/meta/delete-request",
                &deletion_signed("asid-ghost", "secret-stub"),
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = axum::body::to_bytes(res.into_body(), 4096).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let code = json["confirmation_code"].as_str().unwrap();
        let store = handbook_store::store::Store::new(tmp.path());
        let conn = handbook_store::db::open(&store).unwrap();
        let status: String = conn
            .query_row(
                "SELECT status FROM meta_deletion WHERE code = ?1",
                [code],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(status, "unknown");
        // Bad signature: 400, no row.
        let res = app
            .clone()
            .oneshot(post_form(
                "/auth/meta/delete-request",
                &deletion_signed("asid-ghost", "wrong-secret"),
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
        // Unknown code: 404 HTML, no echo of malicious input.
        let res = app
            .clone()
            .oneshot(get("/auth/meta/delete-status?code=%3Cscript%3E", ""))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::NOT_FOUND);
        let body = axum::body::to_bytes(res.into_body(), 8192).await.unwrap();
        let html = String::from_utf8(body.to_vec()).unwrap();
        assert!(!html.contains("<script>"), "{html}");
        // Unconfigured Meta: 503.
        let (_tmp2, bare) = test_app_with_meta(None);
        let res = bare
            .oneshot(post_form(
                "/auth/meta/delete-request",
                &deletion_signed("asid-1", "secret-stub"),
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);
    }
}
