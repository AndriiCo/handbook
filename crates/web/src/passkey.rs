//! Public passkey ceremonies (spec `customer-auth.md` §2).
//!
//! Registration (first key mints the account; logged-in adds a
//! key), login by `user_id` hex, linked email or no identifier at all
//! (discoverable credentials: conditional mediation / autofill, spec
//! §2.5), method list/unlink, logout, and `me`.

use axum::{
    Json,
    extract::{ConnectInfo, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use serde_json::json;
use std::collections::{HashMap, VecDeque};
use std::net::SocketAddr;
use std::sync::Mutex;
use webauthn_rs::prelude::*;

use super::{AppState, store_unavailable, unix_now};
use auth::passkey as store;

/// Build the crate handle from config. `None` refuses the boot
/// (fail closed on garbage RP values).
pub fn build_webauthn(rp_id: &str, rp_name: &str, origin: &str) -> Option<Webauthn> {
    let url = url::Url::parse(origin).ok()?;
    let admin_origin = url::Url::parse(&format!("https://{}", auth::admin_host())).ok()?;
    // `Webauthn` is 'static — builder borrows end at `build()`.
    WebauthnBuilder::new(rp_id, &url)
        .ok()?
        .rp_name(rp_name)
        .allow_subdomains(true)
        .append_allowed_origin(&admin_origin)
        .build()
        .ok()
}

/// Test handle (example.test RP; ceremonies against it fail
/// origin checks, which is what the negative tests assert).
#[cfg(test)]
pub fn test_webauthn() -> Webauthn {
    build_webauthn("example.test", "Example RP", "https://example.test").unwrap()
}

/// In-memory rate buckets (spec decision 0.6): single server, no
/// new table. Atomic check-and-decrement. A key already in the map
/// always keeps its own budget; when the map is full of live keys, a
/// *new* key gets the caller's [`WhenFull`] policy.
#[derive(Debug, Default)]
pub struct RateBuckets {
    inner: Mutex<HashMap<String, VecDeque<i64>>>,
}

/// What a full map does with a key it has not seen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WhenFull {
    /// Auth endpoints: fail closed for newcomers.
    RefuseNew,
    /// Model-free public reads: admit unrecorded, so a flood of
    /// distinct callers never locks every visitor out of a page.
    AdmitNew,
    /// Customer writes (behind a network bucket): evict the stalest
    /// key, so a flood of fresh refs never refuses every customer.
    EvictOldest,
}

const MAX_KEYS: usize = 8192;

impl RateBuckets {
    /// Fail-closed check for auth endpoints.
    pub fn check(&self, key: &str, now: i64, limit: u32, window_secs: i64) -> bool {
        self.check_with(key, now, limit, window_secs, WhenFull::RefuseNew)
    }

    /// Check for model-free public reads (see [`WhenFull::AdmitNew`]).
    pub fn check_read(&self, key: &str, now: i64, limit: u32, window_secs: i64) -> bool {
        self.check_with(key, now, limit, window_secs, WhenFull::AdmitNew)
    }

    /// Check for per-customer write buckets that sit behind a network
    /// bucket (see [`WhenFull::EvictOldest`]).
    pub fn check_write(&self, key: &str, now: i64, limit: u32, window_secs: i64) -> bool {
        self.check_with(key, now, limit, window_secs, WhenFull::EvictOldest)
    }

    fn check_with(
        &self,
        key: &str,
        now: i64,
        limit: u32,
        window_secs: i64,
        full: WhenFull,
    ) -> bool {
        let mut map = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if map.len() >= MAX_KEYS && !map.contains_key(key) {
            map.retain(|_, q: &mut VecDeque<i64>| {
                while q.front().is_some_and(|&t| now - t >= window_secs) {
                    q.pop_front();
                }
                !q.is_empty()
            });
            if map.len() >= MAX_KEYS {
                match full {
                    WhenFull::RefuseNew => return false,
                    WhenFull::AdmitNew => return true,
                    WhenFull::EvictOldest => {
                        let stalest = map
                            .iter()
                            .min_by_key(|(_, q)| q.back().copied().unwrap_or(i64::MIN))
                            .map(|(k, _)| k.clone());
                        if let Some(k) = stalest {
                            map.remove(&k);
                        }
                    }
                }
            }
        }
        let queue = map.entry(key.to_string()).or_default();
        while queue.front().is_some_and(|&t| now - t >= window_secs) {
            queue.pop_front();
        }
        if queue.len() as u32 >= limit {
            return false;
        }
        queue.push_back(now);
        true
    }
}

fn origin_of(headers: &HeaderMap) -> Option<&str> {
    headers.get(header::ORIGIN)?.to_str().ok()
}

pub(crate) fn origin_ok(headers: &HeaderMap, dev_off: bool) -> bool {
    auth::csrf::origin_allowed(origin_of(headers), dev_off)
}

pub(crate) fn csrf_echo_ok(headers: &HeaderMap) -> bool {
    let cookies = headers
        .get(header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let cookie_token = auth::cookie_value(cookies, auth::CSRF_COOKIE).unwrap_or("");
    let header_token = headers
        .get(auth::CSRF_HEADER)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    auth::csrf::tokens_match(cookie_token, header_token)
}

pub(crate) fn csrf_fail() -> Response {
    (
        StatusCode::FORBIDDEN,
        Json(json!({"error": "csrf check failed"})),
    )
        .into_response()
}

pub(crate) fn client_ip(addr: SocketAddr) -> String {
    addr.ip().to_string()
}

pub(crate) fn user_agent(headers: &HeaderMap) -> &str {
    headers
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
}

/// Valid session from the public cookie (touches `last_seen`;
/// honors the magic-link fallback window, spec §12.6).
/// True when the caller presents a non-empty session cookie (valid or
/// not). Absence means anonymous; presence with no live session means a
/// dead credential and stays 401.
fn presents_credential(headers: &HeaderMap) -> bool {
    headers
        .get(header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .and_then(|cookies| auth::cookie_value(cookies, auth::PUB_COOKIE))
        .is_some_and(|v| !v.is_empty())
}

pub(crate) fn session_of(
    conn: &rusqlite::Connection,
    headers: &HeaderMap,
    now: i64,
) -> Option<auth::Session> {
    let cookies = headers.get(header::COOKIE).and_then(|v| v.to_str().ok())?;
    let id = auth::cookie_value(cookies, auth::PUB_COOKIE)?;
    let session = auth::get_session(conn, id).ok()??;
    if !auth::session_usable(conn, &session, now) {
        return None;
    }
    let _ = auth::touch_session(conn, id, now);
    Some(session)
}

pub(crate) fn audit_auth(
    conn: &rusqlite::Connection,
    now: i64,
    actor: &str,
    action: &str,
    reason: &str,
) {
    let _ = conn.execute(
        "INSERT INTO audit(at, actor, action, entity, entity_id, reason)
         VALUES (?1, ?2, ?3, 'auth', 0, ?4)",
        rusqlite::params![now, actor, action, reason],
    );
}

pub(crate) fn cred_id_string(cred_id: &CredentialID) -> String {
    use base64::Engine as _;
    let bytes: &[u8] = cred_id.as_ref();
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

fn parse_cred_id(raw: &str) -> Option<CredentialID> {
    use base64::Engine as _;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(raw)
        .ok()?;
    if bytes.is_empty() || bytes.len() > 1024 {
        return None;
    }
    Some(CredentialID::from(bytes))
}

/// `user_id` hex (32 lowercase) or a linked email (lowercased).
pub(crate) fn split_account(account: &str) -> Option<(Option<&str>, Option<String>)> {
    let trimmed = account.trim();
    if trimmed.len() == 32
        && trimmed.bytes().all(|b| b.is_ascii_hexdigit())
        && trimmed == trimmed.to_lowercase()
    {
        return Some((Some(trimmed), None));
    }
    let email = trimmed.to_lowercase();
    if email.len() <= 254 && email.contains('@') && !email.contains(' ') {
        return Some((None, Some(email)));
    }
    None
}

/// Registration options as JSON, asking the authenticator for a
/// discoverable credential (a passkey that names its own user, so
/// sign-in needs no identifier). `required` refuses authenticators
/// that cannot store one; otherwise it is a preference. Finish never
/// checks this flag (webauthn-rs 0.5.5), so only the options change.
pub(crate) fn discoverable_options(
    options: &CreationChallengeResponse,
    required: bool,
) -> serde_json::Value {
    let mut value = serde_json::to_value(options).unwrap_or(serde_json::Value::Null);
    if let Some(selection) = value
        .pointer_mut("/publicKey/authenticatorSelection")
        .and_then(serde_json::Value::as_object_mut)
    {
        selection.insert(
            "residentKey".to_string(),
            json!(if required { "required" } else { "preferred" }),
        );
        selection.insert("requireResidentKey".to_string(), json!(required));
    }
    value
}

/// Options + serialised state for a discoverable (usernameless)
/// sign-in: no allow-list, UV required, conditional mediation.
pub(crate) fn start_discoverable(
    webauthn: &Webauthn,
) -> Option<(RequestChallengeResponse, String)> {
    let (options, state) = webauthn.start_discoverable_authentication().ok()?;
    let state_json = serde_json::to_string(&state).ok()?;
    Some((options, state_json))
}

/// Why a discoverable finish failed (callers keep their own strings).
pub(crate) enum DiscoverFail {
    /// The credential id is not on file (platform-side deletion):
    /// the `404 re-register` answer, never a dead end.
    Unknown,
    /// Bad user handle, owner mismatch or failed verification.
    Failed,
    Store,
}

/// Finish a discoverable sign-in: the credential's user handle must
/// name the account that owns the credential id, then the assertion
/// verifies against that one passkey. Returns the owner's `user_id`.
pub(crate) fn finish_discoverable(
    webauthn: &Webauthn,
    conn: &rusqlite::Connection,
    credential: &PublicKeyCredential,
    state_json: &str,
) -> Result<(String, AuthenticationResult), DiscoverFail> {
    use base64::Engine as _;
    let Ok(state) = serde_json::from_str::<DiscoverableAuthentication>(state_json) else {
        return Err(DiscoverFail::Store);
    };
    let (handle, cred_id) = webauthn
        .identify_discoverable_authentication(credential)
        .map_err(|_| DiscoverFail::Failed)?;
    let presented = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(cred_id);
    let (owner, blob) = match store::credential_owner(conn, &presented) {
        Ok(Some(row)) => row,
        Ok(None) => return Err(DiscoverFail::Unknown),
        Err(_) => return Err(DiscoverFail::Store),
    };
    // User handles are the account's 16-byte id (see `mint_user_id`).
    if handle.simple().to_string() != owner {
        return Err(DiscoverFail::Failed);
    }
    let Ok(passkey) = serde_json::from_str::<Passkey>(&blob) else {
        return Err(DiscoverFail::Store);
    };
    let result = webauthn
        .finish_discoverable_authentication(credential, state, &[DiscoverableKey::from(&passkey)])
        .map_err(|_| DiscoverFail::Failed)?;
    Ok((owner, result))
}

pub(crate) fn open_conn(state: &AppState) -> Option<rusqlite::Connection> {
    let store = handbook_store::store::Store::new(&state.store_root);
    handbook_store::db::open(&store).ok()
}

/// One `Set-Cookie` line per value (appended, never merged).
pub(crate) fn set_cookies(values: &[String]) -> HeaderMap {
    let mut map = HeaderMap::new();
    for cookie in values {
        // Cookie strings are internally built (hex tokens, fixed
        // attributes) — a parse failure would be a bug, not input.
        if let Ok(value) = HeaderValue::from_str(cookie) {
            map.append(header::SET_COOKIE, value);
        }
    }
    map
}

pub(crate) fn valid_email(raw: &str) -> Option<String> {
    let email = raw.trim().to_lowercase();
    if email.len() <= 254 && email.contains('@') && !email.contains(' ') {
        Some(email)
    } else {
        None
    }
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RegisterStartBody {
    display_name: Option<String>,
    label: Option<String>,
    email: Option<String>,
}

/// `POST /auth/passkey/register/start` — mode (a) mints the account
/// at finish; mode (b) (session present) links a new key.
pub async fn register_start(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(body): Json<RegisterStartBody>,
) -> Response {
    let now = unix_now();
    if !origin_ok(&headers, state.admin_mtls_off) {
        return csrf_fail();
    }
    let ip = client_ip(addr);
    if !state.auth_limits.check(&format!("rs:{ip}"), now, 30, 60) {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            Json(json!({"error": "rate limited"})),
        )
            .into_response();
    }
    let Some(conn) = open_conn(&state) else {
        return store_unavailable();
    };
    let session = session_of(&conn, &headers, now);
    let display_name = body.display_name.unwrap_or_default();
    let label = body.label.unwrap_or_default();
    if session.is_none() && (display_name.trim().is_empty() || display_name.len() > 128) {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "display_name required (1-128 chars)"})),
        )
            .into_response();
    }
    if label.len() > 64 {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "label too long (max 64)"})),
        )
            .into_response();
    }
    let email = body.email.as_deref().and_then(valid_email);
    if body.email.is_some() && email.is_none() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "email malformed"})),
        )
            .into_response();
    }
    // Mode (b): explicit link (rule 4) — session + fresh step-up.
    let (user_id, exclude) = match &session {
        Some(s) => {
            let fresh = store::get_account(&conn, &s.user_id)
                .ok()
                .flatten()
                .and_then(|a| a.step_up_at)
                .is_some_and(|at| now - at <= store::STEP_UP_WINDOW_SECS)
                || now - s.created_at <= store::STEP_UP_WINDOW_SECS;
            if !fresh {
                return (
                    StatusCode::FORBIDDEN,
                    Json(json!({"error": "step-up required: log in again first"})),
                )
                    .into_response();
            }
            // Device cap (decision 54): refuse before minting a
            // doomed challenge. The finish-side check is
            // authoritative (closes the two-tab race).
            if store::live_credential_count(&conn, &s.user_id).unwrap_or(0)
                >= store::MAX_CREDENTIALS_PER_ACCOUNT
            {
                return (
                    StatusCode::CONFLICT,
                    Json(
                        json!({"error": "device limit reached: remove a passkey to add a new one"}),
                    ),
                )
                    .into_response();
            }
            let ids = store::credentials_for_user(&conn, &s.user_id).unwrap_or_default();
            let exclude: Vec<CredentialID> =
                ids.iter().filter_map(|(id, _)| parse_cred_id(id)).collect();
            (s.user_id.clone(), Some(exclude))
        }
        None => (store::mint_user_id(), None),
    };
    let uuid = match session
        .as_ref()
        .map(|s| uuid::Uuid::parse_str(&s.user_id))
        .unwrap_or_else(|| uuid::Uuid::parse_str(&user_id))
    {
        Ok(uuid) => uuid,
        Err(_) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": "account id malformed"})),
            )
                .into_response();
        }
    };
    let shown = if display_name.trim().is_empty() {
        "Apostille Handbook user"
    } else {
        display_name.trim()
    };
    let (options, reg_state) = match state
        .webauthn
        .start_passkey_registration(uuid, &user_id, shown, exclude)
    {
        Ok(pair) => pair,
        Err(_) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": "ceremony unavailable"})),
            )
                .into_response();
        }
    };
    let state_json = match serde_json::to_string(&reg_state) {
        Ok(json) => json,
        Err(_) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": "ceremony unavailable"})),
            )
                .into_response();
        }
    };
    let _ = store::sweep_challenges(&conn, now);
    let challenge_id = store::mint_challenge_id();
    // The pending email rides back on finish (client echo); nothing
    // is stored until the ceremony completes.
    if store::store_challenge(
        &conn,
        &challenge_id,
        &store::NewChallenge {
            kind: store::KIND_REGISTER,
            user_id: Some(&user_id),
            state_json: &state_json,
            now,
        },
    )
    .is_err()
    {
        return store_unavailable();
    }
    let csrf = auth::csrf::mint_token();
    (
        StatusCode::OK,
        set_cookies(&[auth::csrf_cookie(&csrf, !state.admin_mtls_off)]),
        Json(json!({
            "challenge_id": challenge_id,
            "options": discoverable_options(&options, false),
        })),
    )
        .into_response()
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RegisterFinishBody {
    challenge_id: String,
    credential: RegisterPublicKeyCredential,
    email: Option<String>,
    label: Option<String>,
}

pub async fn register_finish(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(body): Json<RegisterFinishBody>,
) -> Response {
    let now = unix_now();
    if !origin_ok(&headers, state.admin_mtls_off) || !csrf_echo_ok(&headers) {
        return csrf_fail();
    }
    let ip = client_ip(addr);
    if !state.auth_limits.check(&format!("rf:{ip}"), now, 60, 60) {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            Json(json!({"error": "rate limited"})),
        )
            .into_response();
    }
    let Some(conn) = open_conn(&state) else {
        return store_unavailable();
    };
    let session = session_of(&conn, &headers, now);
    let Some((owner, state_json)) =
        store::take_challenge(&conn, &body.challenge_id, store::KIND_REGISTER, now).unwrap_or(None)
    else {
        return (
            StatusCode::GONE,
            Json(json!({"error": "challenge invalid or expired"})),
        )
            .into_response();
    };
    let user_id = owner.unwrap_or_default();
    // Mode agreement: a session may only finish its own challenge.
    if let Some(s) = &session
        && s.user_id != user_id
    {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({"error": "challenge is not yours"})),
        )
            .into_response();
    }
    let reg_state: PasskeyRegistration = match serde_json::from_str(&state_json) {
        Ok(state) => state,
        Err(_) => return store_unavailable(),
    };
    let passkey = match state
        .webauthn
        .finish_passkey_registration(&body.credential, &reg_state)
    {
        Ok(passkey) => passkey,
        Err(_) => {
            // Generic on purpose: crate variants would fingerprint
            // the failure for attackers.
            return (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(json!({"error": "verification failed"})),
            )
                .into_response();
        }
    };
    let credential_id = cred_id_string(passkey.cred_id());
    if store::credential_known(&conn, &credential_id).unwrap_or(true) {
        audit_auth(&conn, now, &user_id, "passkey_reregister_refused", "");
        return (
            StatusCode::CONFLICT,
            Json(json!({"error": "credential already registered"})),
        )
            .into_response();
    }
    let passkey_json = match serde_json::to_string(&passkey) {
        Ok(json) => json,
        Err(_) => return store_unavailable(),
    };
    let label = body.label.as_deref().unwrap_or("");
    if label.len() > 64 {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "label too long (max 64)"})),
        )
            .into_response();
    }
    if store::ensure_account(&conn, &user_id, now).is_err() {
        return store_unavailable();
    }
    match store::store_credential_capped(
        &conn,
        &user_id,
        &credential_id,
        &passkey_json,
        label,
        now,
        store::MAX_CREDENTIALS_PER_ACCOUNT,
    ) {
        Ok(()) => {}
        Err(store::StoreCredentialError::CapReached) => {
            audit_auth(&conn, now, &user_id, "passkey_cap_refused", "");
            return (
                StatusCode::CONFLICT,
                Json(json!({"error": "device limit reached: remove a passkey to add a new one"})),
            )
                .into_response();
        }
        Err(store::StoreCredentialError::Db(_)) => return store_unavailable(),
    }
    if session.is_none()
        && let Some(email) = body.email.as_deref().and_then(valid_email)
    {
        let _ = conn.execute(
            "INSERT INTO identity_link(user_id, method, provider_sub, email_hash, verified_email, is_relay, disabled_at)
             VALUES (?1, 'email', ?2, ?3, 0, 0, NULL)",
            rusqlite::params![user_id, email, auth::hash_str(&email)],
        );
    }
    let rotation = auth::csrf::mint_token();
    let mut cookies = vec![auth::csrf_cookie(&rotation, !state.admin_mtls_off)];
    if session.is_none() {
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
                user_agent: user_agent(&headers),
            },
        )
        .is_err()
        {
            return store_unavailable();
        }
        cookies.push(auth::session_cookie(
            auth::PUB_COOKIE,
            &session_id,
            !state.admin_mtls_off,
        ));
        audit_auth(&conn, now, &user_id, "passkey_registered", "");
        (
            StatusCode::OK,
            set_cookies(&cookies),
            Json(json!({"user_id": user_id, "methods": ["passkey"]})),
        )
            .into_response()
    } else {
        audit_auth(&conn, now, &user_id, "method_linked", "passkey");
        (
            StatusCode::OK,
            set_cookies(&cookies),
            Json(json!({"linked": true, "credential_id": credential_id, "methods": ["passkey"]})),
        )
            .into_response()
    }
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LoginStartBody {
    #[serde(default)]
    account: String,
}

/// `POST /auth/passkey/login/start` — `account` is a `user_id` hex
/// or a linked email; empty (the autofill probe) starts a discoverable
/// ceremony instead (spec §2.5). Unknown/locked all answer the same
/// 401: no enumeration.
pub async fn login_start(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(body): Json<LoginStartBody>,
) -> Response {
    let now = unix_now();
    if !origin_ok(&headers, state.admin_mtls_off) {
        return csrf_fail();
    }
    let ip = client_ip(addr);
    if !state.auth_limits.check(&format!("ls:{ip}"), now, 30, 60) {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            Json(json!({"error": "rate limited"})),
        )
            .into_response();
    }
    let unknown = || {
        (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error": "unknown account or no passkeys"})),
        )
            .into_response()
    };
    let Some(conn) = open_conn(&state) else {
        return store_unavailable();
    };
    if body.account.trim().is_empty() {
        let Some((options, state_json)) = start_discoverable(&state.webauthn) else {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": "ceremony unavailable"})),
            )
                .into_response();
        };
        let _ = store::sweep_challenges(&conn, now);
        let challenge_id = store::mint_challenge_id();
        if store::store_challenge(
            &conn,
            &challenge_id,
            &store::NewChallenge {
                kind: store::KIND_DISCOVERABLE,
                user_id: None,
                state_json: &state_json,
                now,
            },
        )
        .is_err()
        {
            return store_unavailable();
        }
        let csrf = auth::csrf::mint_token();
        return (
            StatusCode::OK,
            set_cookies(&[auth::csrf_cookie(&csrf, !state.admin_mtls_off)]),
            Json(json!({"challenge_id": challenge_id, "options": options})),
        )
            .into_response();
    }
    let Some((by_id, by_email)) = split_account(&body.account) else {
        return unknown();
    };
    let user_id = match (by_id, by_email) {
        (Some(id), _) => id.to_string(),
        (None, Some(email)) => match store::account_by_any_email(&conn, &email).unwrap_or(None) {
            Some(uid) => uid,
            None => return unknown(),
        },
        (None, None) => return unknown(),
    };
    match store::get_account(&conn, &user_id).unwrap_or(None) {
        Some(account) if account.locked_at.is_none() => {}
        _ => return unknown(),
    }
    let rows = store::credentials_for_user(&conn, &user_id).unwrap_or_default();
    if rows.is_empty() {
        return unknown();
    }
    let mut passkeys = Vec::with_capacity(rows.len());
    for (_, blob) in &rows {
        match serde_json::from_str::<Passkey>(blob) {
            Ok(key) => passkeys.push(key),
            Err(_) => return store_unavailable(),
        }
    }
    let (options, auth_state) = match state.webauthn.start_passkey_authentication(&passkeys) {
        Ok(pair) => pair,
        Err(_) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": "ceremony unavailable"})),
            )
                .into_response();
        }
    };
    let state_json = match serde_json::to_string(&auth_state) {
        Ok(json) => json,
        Err(_) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": "ceremony unavailable"})),
            )
                .into_response();
        }
    };
    let _ = store::sweep_challenges(&conn, now);
    let challenge_id = store::mint_challenge_id();
    if store::store_challenge(
        &conn,
        &challenge_id,
        &store::NewChallenge {
            kind: store::KIND_LOGIN,
            user_id: Some(&user_id),
            state_json: &state_json,
            now,
        },
    )
    .is_err()
    {
        return store_unavailable();
    }
    let csrf = auth::csrf::mint_token();
    (
        StatusCode::OK,
        set_cookies(&[auth::csrf_cookie(&csrf, !state.admin_mtls_off)]),
        Json(json!({"challenge_id": challenge_id, "options": options})),
    )
        .into_response()
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LoginFinishBody {
    challenge_id: String,
    credential: PublicKeyCredential,
}

pub async fn login_finish(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(body): Json<LoginFinishBody>,
) -> Response {
    let now = unix_now();
    if !origin_ok(&headers, state.admin_mtls_off) || !csrf_echo_ok(&headers) {
        return csrf_fail();
    }
    let ip = client_ip(addr);
    if !state.auth_limits.check(&format!("lf:{ip}"), now, 60, 60) {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            Json(json!({"error": "rate limited"})),
        )
            .into_response();
    }
    let Some(conn) = open_conn(&state) else {
        return store_unavailable();
    };
    let gone = || {
        (
            StatusCode::GONE,
            Json(json!({"error": "challenge invalid or expired"})),
        )
            .into_response()
    };
    let limited = || {
        (
            StatusCode::TOO_MANY_REQUESTS,
            Json(json!({"error": "rate limited"})),
        )
            .into_response()
    };
    let failed = || {
        (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error": "verification failed"})),
        )
            .into_response()
    };
    let re_register = || {
        (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "credential unknown", "action": "re-register"})),
        )
            .into_response()
    };
    let named =
        store::take_challenge(&conn, &body.challenge_id, store::KIND_LOGIN, now).unwrap_or(None);
    let (user_id, result) = if let Some((owner, state_json)) = named {
        let user_id = owner.unwrap_or_default();
        if !state
            .auth_limits
            .check(&format!("lf:{user_id}"), now, 10, 60)
        {
            return limited();
        }
        // Platform-side credential deletion lands here (not a dead
        // end): the id is well-formed but unknown to us.
        let presented = {
            use base64::Engine as _;
            base64::engine::general_purpose::URL_SAFE_NO_PAD
                .encode(body.credential.get_credential_id())
        };
        if store::credential_owner(&conn, &presented)
            .unwrap_or(None)
            .is_none()
        {
            return re_register();
        }
        let auth_state: PasskeyAuthentication = match serde_json::from_str(&state_json) {
            Ok(state) => state,
            Err(_) => return store_unavailable(),
        };
        match state
            .webauthn
            .finish_passkey_authentication(&body.credential, &auth_state)
        {
            Ok(result) => (user_id, result),
            Err(_) => return failed(),
        }
    } else if let Some((None, state_json)) =
        store::take_challenge(&conn, &body.challenge_id, store::KIND_DISCOVERABLE, now)
            .unwrap_or(None)
    {
        match finish_discoverable(&state.webauthn, &conn, &body.credential, &state_json) {
            Ok((user_id, result)) => {
                if !state
                    .auth_limits
                    .check(&format!("lf:{user_id}"), now, 10, 60)
                {
                    return limited();
                }
                (user_id, result)
            }
            Err(DiscoverFail::Unknown) => return re_register(),
            Err(DiscoverFail::Failed) => return failed(),
            Err(DiscoverFail::Store) => return store_unavailable(),
        }
    } else {
        return gone();
    };
    let authed_id = cred_id_string(result.cred_id());
    let Some((owner_id, blob)) = store::credential_owner(&conn, &authed_id).unwrap_or(None) else {
        return failed();
    };
    if owner_id != user_id {
        return failed();
    }
    match store::get_account(&conn, &user_id).unwrap_or(None) {
        Some(account) if account.locked_at.is_none() => {
            if account.role != auth::Role::Customer.as_str() {
                // Staff log in on the admin branch (Phase 4).
                return (
                    StatusCode::FORBIDDEN,
                    Json(json!({"error": "use the admin login"})),
                )
                    .into_response();
            }
        }
        _ => return failed(),
    }
    // Crate-blessed counter flow: re-persist only when changed.
    if result.needs_update() {
        let mut passkey: Passkey = match serde_json::from_str(&blob) {
            Ok(key) => key,
            Err(_) => return store_unavailable(),
        };
        if passkey.update_credential(&result) == Some(true) {
            match serde_json::to_string(&passkey) {
                Ok(updated) => {
                    let _ = store::update_passkey_json(&conn, &authed_id, &updated, now);
                }
                Err(_) => return store_unavailable(),
            }
        }
    }
    let _ = store::touch_credential(&conn, &authed_id, now);
    if result.user_verified() {
        let _ = store::set_step_up(&conn, &user_id, now);
    }
    let _ = store::clear_fallback(&conn, &user_id);
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
            user_agent: user_agent(&headers),
        },
    )
    .is_err()
    {
        return store_unavailable();
    }
    let rotation = auth::csrf::mint_token();
    audit_auth(&conn, now, &user_id, "passkey_login", "");
    (
        StatusCode::OK,
        set_cookies(&[
            auth::csrf_cookie(&rotation, !state.admin_mtls_off),
            auth::session_cookie(auth::PUB_COOKIE, &session_id, !state.admin_mtls_off),
        ]),
        Json(json!({"user_id": user_id, "methods": ["passkey"]})),
    )
        .into_response()
}

/// `GET /auth/profile/methods` — live methods + passkey labels.
pub async fn list_methods(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let now = unix_now();
    let Some(conn) = open_conn(&state) else {
        return store_unavailable();
    };
    let Some(session) = session_of(&conn, &headers, now) else {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error": "unauthorized"})),
        )
            .into_response();
    };
    let links = store::links_for_user(&conn, &session.user_id).unwrap_or_default();
    let mut methods = Vec::with_capacity(links.len());
    for (method, id) in links {
        let label = if method == "passkey" {
            store::credential_label(&conn, &id).unwrap_or_default()
        } else {
            String::new()
        };
        methods.push(json!({"method": method, "id": id, "label": label, "disabled": false}));
    }
    (StatusCode::OK, Json(json!({"methods": methods}))).into_response()
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct UnlinkBody {
    method: String,
    id: String,
    reauth_proof: String,
}

/// `POST /auth/profile/unlink` — rule 6: re-auth proof + a
/// remaining live method required; last method is never removed.
pub async fn unlink(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<UnlinkBody>,
) -> Response {
    let now = unix_now();
    if !origin_ok(&headers, state.admin_mtls_off) || !csrf_echo_ok(&headers) {
        return csrf_fail();
    }
    let Some(conn) = open_conn(&state) else {
        return store_unavailable();
    };
    let Some(session) = session_of(&conn, &headers, now) else {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error": "unauthorized"})),
        )
            .into_response();
    };
    if body.method != "passkey"
        && body.method != "google"
        && body.method != "apple"
        && body.method != "meta"
    {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "unknown method"})),
        )
            .into_response();
    }
    if !state
        .auth_limits
        .check(&format!("un:{}", session.user_id), now, 10, 60)
    {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            Json(json!({"error": "rate limited"})),
        )
            .into_response();
    }
    // Re-auth proof: a fresh consumed challenge (passkey ceremony
    // or stamped oauth row) — or, for OAuth-only users who cannot
    // run a passkey ceremony, the literal "session" while the
    // current session is step-up fresh (spec §9.4 tradeoff,
    // generalized to all OAuth methods).
    let reauthed = store::proof_fresh(&conn, &body.reauth_proof, &session.user_id, now)
        .unwrap_or(false)
        || (body.method != "passkey"
            && body.reauth_proof == "session"
            && now - session.created_at <= store::STEP_UP_WINDOW_SECS);
    if !reauthed {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({"error": "re-authentication required"})),
        )
            .into_response();
    }
    let live = store::live_method_count(&conn, &session.user_id).unwrap_or(0);
    let remaining = live.saturating_sub(1);
    if !auth::login::unlink_permitted(remaining, true) {
        return (
            StatusCode::CONFLICT,
            Json(json!({"error": "last method cannot be removed"})),
        )
            .into_response();
    }
    let revoked = if body.method == "passkey" {
        store::revoke_credential(&conn, &session.user_id, &body.id, now).unwrap_or(0)
    } else {
        store::disable_link(&conn, &session.user_id, &body.method, &body.id, now).unwrap_or(0)
    };
    if revoked == 0 {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "method unknown"})),
        )
            .into_response();
    }
    // Rule 6 tail (defensive; the guard above normally prevents it).
    if store::live_method_count(&conn, &session.user_id).unwrap_or(1) == 0 {
        let _ = store::lock_account(&conn, &session.user_id, now);
        audit_auth(
            &conn,
            now,
            &session.user_id,
            "account_locked",
            "last link revoked",
        );
    }
    audit_auth(
        &conn,
        now,
        &session.user_id,
        "method_unlinked",
        &body.method,
    );
    let rotation = auth::csrf::mint_token();
    (
        StatusCode::OK,
        set_cookies(&[auth::csrf_cookie(&rotation, !state.admin_mtls_off)]),
        Json(json!({"unlinked": true})),
    )
        .into_response()
}

/// `POST /auth/logout` — idempotent; clears both cookies.
pub async fn logout(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let now = unix_now();
    let secure = !state.admin_mtls_off;
    if let Some(conn) = open_conn(&state)
        && let Some(session) = session_of(&conn, &headers, now)
    {
        let _ = auth::delete_session(&conn, &session.id);
        audit_auth(&conn, now, &session.user_id, "logout", "");
    }
    (
        StatusCode::OK,
        set_cookies(&[
            auth::clear_cookie(auth::PUB_COOKIE, secure),
            auth::clear_cookie(auth::CSRF_COOKIE, secure),
        ]),
        Json(json!({"ok": true})),
    )
        .into_response()
}

/// `GET /auth/me` — session check + method list. Callers presenting no
/// credential get 200 `{"user": null}` so the login page's anonymous
/// probe never logs a console error; a presented-but-invalid session is
/// still 401, and only logged-in callers see `user_id`/`methods`.
pub async fn me(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let now = unix_now();
    let Some(conn) = open_conn(&state) else {
        return store_unavailable();
    };
    if !presents_credential(&headers) {
        return (StatusCode::OK, Json(json!({"user": null}))).into_response();
    }
    let Some(session) = session_of(&conn, &headers, now) else {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error": "unauthorized"})),
        )
            .into_response();
    };
    let links = store::links_for_user(&conn, &session.user_id).unwrap_or_default();
    let methods: Vec<String> = links.into_iter().map(|(method, _)| method).collect();
    (
        StatusCode::OK,
        Json(
            json!({"user_id": session.user_id, "role": session.role.as_str(), "methods": methods}),
        ),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;

    const BOGUS_REGISTER: &str = r#"{"id":"AA","rawId":"AA","response":{"attestationObject":"AA","clientDataJSON":"AA"},"type":"public-key"}"#;
    const BOGUS_LOGIN: &str = r#"{"id":"AA","rawId":"AA","response":{"authenticatorData":"AA","clientDataJSON":"AA","signature":"AA"},"type":"public-key"}"#;

    fn test_app() -> (tempfile::TempDir, axum::Router) {
        let tmp = tempfile::tempdir().unwrap();
        let store = handbook_store::store::Store::new(tmp.path());
        let conn = handbook_store::db::open(&store).unwrap();
        handbook_store::db::migrate(&conn).unwrap();
        auth::migrate_auth_tables(&conn).unwrap();
        config::migrate_config_tables(&conn).unwrap();
        drop(conn);
        let dir = tmp.path().to_str().unwrap().to_string();
        let app = crate::app(&dir, &dir, tmp.path().to_path_buf(), &dir);
        (tmp, app)
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

    fn csrf_pair(set_cookie: &str) -> (String, String) {
        // "hb_csrf=<token>; ..." → cookie header + echo header value.
        let token = auth::cookie_value(set_cookie, auth::CSRF_COOKIE).unwrap_or("");
        (format!("{}={token}", auth::CSRF_COOKIE), token.to_string())
    }

    async fn start_register(
        app: &axum::Router,
        display: &str,
        extra_cookies: &str,
    ) -> (StatusCode, serde_json::Value, String) {
        let res = app
            .clone()
            .oneshot(post_json(
                "/apostille/api/v1/auth/passkey/register/start",
                extra_cookies,
                &format!(r#"{{"display_name":"{display}"}}"#),
            ))
            .await
            .unwrap();
        let status = res.status();
        let set_cookie = res
            .headers()
            .get_all(header::SET_COOKIE)
            .iter()
            .filter_map(|v| v.to_str().ok())
            .collect::<Vec<_>>()
            .join("; ");
        let body = axum::body::to_bytes(res.into_body(), 65536).await.unwrap();
        (status, serde_json::from_slice(&body).unwrap(), set_cookie)
    }

    #[test]
    fn builder_accepts_prod_shape_rejects_garbage() {
        assert!(build_webauthn("andrii.ai", "Apostille Handbook", "https://andrii.ai").is_some());
        assert!(build_webauthn("", "x", "https://andrii.ai").is_none());
        assert!(build_webauthn("andrii.ai", "x", "not a url").is_none());
        assert!(build_webauthn("andrii.ai", "x", "https://evil.test").is_none());
    }

    #[test]
    fn rate_buckets_allow_block_expire() {
        let buckets = RateBuckets::default();
        assert!(buckets.check("k", 1000, 2, 60));
        assert!(buckets.check("k", 1001, 2, 60));
        assert!(!buckets.check("k", 1002, 2, 60));
        assert!(buckets.check("k", 1061, 2, 60));
        assert!(buckets.check("other", 1002, 2, 60));
    }

    #[test]
    fn rate_buckets_fail_closed_when_huge() {
        let buckets = RateBuckets::default();
        for i in 0..8193 {
            let _ = buckets.check(&format!("k-{i}"), 1000, 1_000_000, 60);
        }
        assert!(!buckets.check("victim", 1001, 1_000_000, 60));
        // After the window passes the shed entries expire and the
        // gate opens again.
        assert!(buckets.check("victim", 2000, 1_000_000, 60));
    }

    #[test]
    fn full_maps_keep_known_callers_and_apply_their_policy() {
        // Re-review #34 / #9: a full map used to refuse every key,
        // including callers it already held.
        let fill = |b: &RateBuckets| {
            for i in 0..MAX_KEYS {
                let _ = b.check_read(&format!("k-{i}"), 1000, 1_000_000, 60);
            }
        };
        let auth = RateBuckets::default();
        fill(&auth);
        assert!(
            auth.check("k-7", 1001, 1_000_000, 60),
            "a known caller keeps its budget"
        );
        assert!(
            !auth.check("newcomer", 1001, 1_000_000, 60),
            "auth stays fail-closed"
        );

        let read = RateBuckets::default();
        fill(&read);
        assert!(read.check_read("k-7", 1001, 1_000_000, 60));
        assert!(
            read.check_read("newcomer", 1001, 1_000_000, 60),
            "reads admit newcomers"
        );

        let write = RateBuckets::default();
        fill(&write);
        assert!(
            write.check_write("customer", 1001, 2, 60),
            "writes evict the stalest key"
        );
        assert!(write.check_write("customer", 1002, 2, 60));
        assert!(
            !write.check_write("customer", 1003, 2, 60),
            "and still enforce the limit"
        );
    }

    #[test]
    fn account_split_and_cred_roundtrip() {
        let hex = "ab".repeat(16);
        assert_eq!(split_account(&hex), Some((Some(hex.as_str()), None)));
        assert_eq!(
            split_account("A@Example.test "),
            Some((None, Some("a@example.test".to_string())))
        );
        assert_eq!(split_account("not-an-id"), None);
        assert_eq!(split_account(""), None);
        let id = parse_cred_id("AAEC").unwrap();
        assert_eq!(cred_id_string(&id), "AAEC");
        assert!(parse_cred_id("!!!").is_none());
        assert!(parse_cred_id("").is_none());
    }

    #[tokio::test]
    async fn register_start_guards_and_shape() {
        let (_tmp, app) = test_app();
        // Evil origin → 403 even on the dev seam.
        let res = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/apostille/api/v1/auth/passkey/register/start")
                    .header(header::CONTENT_TYPE, "application/json")
                    .header(header::ORIGIN, "https://evil.test")
                    .extension(addr())
                    .body(Body::from(r#"{"display_name":"A"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::FORBIDDEN);
        // Missing display_name (logged-out) → 400.
        let res = app
            .clone()
            .oneshot(post_json(
                "/apostille/api/v1/auth/passkey/register/start",
                "",
                "{}",
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
        // Unknown fields rejected (mass-assignment hygiene).
        let res = app
            .clone()
            .oneshot(post_json(
                "/apostille/api/v1/auth/passkey/register/start",
                "",
                r#"{"display_name":"A","role":"admin"}"#,
            ))
            .await
            .unwrap();
        assert!(
            res.status() == StatusCode::BAD_REQUEST
                || res.status() == StatusCode::UNPROCESSABLE_ENTITY
        );
        // Happy start: options + challenge id + CSRF cookie.
        let (status, body, set_cookie) = start_register(&app, "Andrii", "").await;
        assert_eq!(status, StatusCode::OK);
        assert!(body["challenge_id"].as_str().is_some_and(|s| !s.is_empty()));
        assert!(body["options"].is_object());
        assert!(set_cookie.contains(auth::CSRF_COOKIE));
    }

    #[tokio::test]
    async fn register_finish_rejects_forged_ceremony() {
        let (_tmp, app) = test_app();
        let (_, started, set_cookie) = start_register(&app, "Andrii", "").await;
        let challenge_id = started["challenge_id"].as_str().unwrap().to_string();
        let (cookie, token) = csrf_pair(&set_cookie);
        // No CSRF echo → 403.
        let res = app
            .clone()
            .oneshot(post_json(
                "/apostille/api/v1/auth/passkey/register/finish",
                &cookie,
                &format!(r#"{{"challenge_id":"{challenge_id}","credential":{BOGUS_REGISTER}}}"#),
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::FORBIDDEN);
        // Unknown challenge → 410.
        let res = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/apostille/api/v1/auth/passkey/register/finish")
                    .header(header::CONTENT_TYPE, "application/json")
                    .header(header::ORIGIN, "http://localhost")
                    .header(header::COOKIE, &cookie)
                    .header(auth::CSRF_HEADER, &token)
                    .extension(addr())
                    .body(Body::from(format!(
                        r#"{{"challenge_id":"nope","credential":{BOGUS_REGISTER}}}"#
                    )))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::GONE);
        // Well-formed but forged attestation → 422, generic body.
        let res = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/apostille/api/v1/auth/passkey/register/finish")
                    .header(header::CONTENT_TYPE, "application/json")
                    .header(header::ORIGIN, "http://localhost")
                    .header(header::COOKIE, &cookie)
                    .header(auth::CSRF_HEADER, &token)
                    .extension(addr())
                    .body(Body::from(format!(
                        r#"{{"challenge_id":"{challenge_id}","credential":{BOGUS_REGISTER}}}"#
                    )))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let body = axum::body::to_bytes(res.into_body(), 4096).await.unwrap();
        let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["error"], "verification failed");
    }

    #[tokio::test]
    async fn login_start_hides_everything_behind_401() {
        let (_tmp, app) = test_app();
        for account in ["unknown-user-id", "nobody@example.test"] {
            let res = app
                .clone()
                .oneshot(post_json(
                    "/apostille/api/v1/auth/passkey/login/start",
                    "",
                    &format!(r#"{{"account":"{account}"}}"#),
                ))
                .await
                .unwrap();
            assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
        }
    }

    #[tokio::test]
    async fn autofill_probe_starts_a_discoverable_login() {
        let (tmp, app) = test_app();
        // The login page's conditional-mediation probe sends "".
        let res = app
            .clone()
            .oneshot(post_json(
                "/apostille/api/v1/auth/passkey/login/start",
                "",
                r#"{"account":""}"#,
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let set_cookie = res
            .headers()
            .get_all(header::SET_COOKIE)
            .iter()
            .filter_map(|v| v.to_str().ok())
            .collect::<Vec<_>>()
            .join("; ");
        let body = axum::body::to_bytes(res.into_body(), 65536).await.unwrap();
        let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["options"]["mediation"], "conditional");
        let challenge = value["challenge_id"].as_str().unwrap().to_string();
        let conn =
            handbook_store::db::open(&handbook_store::store::Store::new(tmp.path())).unwrap();
        let (kind, user): (String, Option<String>) = conn
            .query_row(
                "SELECT kind, user_id FROM webauthn_challenge WHERE id = ?1",
                [&challenge],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(kind, store::KIND_DISCOVERABLE);
        assert_eq!(user, None);
        // A forged assertion fails closed, without a session.
        let (cookie, token) = csrf_pair(&set_cookie);
        let res = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/apostille/api/v1/auth/passkey/login/finish")
                    .header(header::CONTENT_TYPE, "application/json")
                    .header(header::ORIGIN, "http://localhost")
                    .header(header::COOKIE, &cookie)
                    .header(auth::CSRF_HEADER, &token)
                    .extension(addr())
                    .body(Body::from(format!(
                        r#"{{"challenge_id":"{challenge}","credential":{BOGUS_LOGIN}}}"#
                    )))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
        assert!(
            res.headers()
                .get_all(header::SET_COOKIE)
                .iter()
                .all(|v| { !v.to_str().unwrap_or("").starts_with(auth::PUB_COOKIE) })
        );
    }

    #[tokio::test]
    async fn registration_prefers_a_discoverable_passkey() {
        let (_tmp, app) = test_app();
        let (status, body, _) = start_register(&app, "Andrii", "").await;
        assert_eq!(status, StatusCode::OK);
        let selection = &body["options"]["publicKey"]["authenticatorSelection"];
        assert_eq!(selection["residentKey"], "preferred", "{selection}");
        assert_eq!(selection["requireResidentKey"], false);
    }

    #[tokio::test]
    async fn login_finish_unknown_challenge_and_cred() {
        let (_tmp, app) = test_app();
        // Unknown challenge id → 410 (CSRF pair needed to reach it).
        let (_, _, set_cookie) = start_register(&app, "Andrii", "").await;
        let (cookie, token) = csrf_pair(&set_cookie);
        let res = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/apostille/api/v1/auth/passkey/login/finish")
                    .header(header::CONTENT_TYPE, "application/json")
                    .header(header::ORIGIN, "http://localhost")
                    .header(header::COOKIE, &cookie)
                    .header(auth::CSRF_HEADER, &token)
                    .extension(addr())
                    .body(Body::from(format!(
                        r#"{{"challenge_id":"nope","credential":{BOGUS_LOGIN}}}"#
                    )))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::GONE);
    }

    #[tokio::test]
    async fn me_logout_methods_roundtrip() {
        let (tmp, app) = test_app();
        // Anonymous /auth/me answers 200 {"user": null} (no console
        // error for the login probe); methods stays 401.
        let res = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/apostille/api/v1/auth/me")
                    .extension(addr())
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = axum::body::to_bytes(res.into_body(), 4096).await.unwrap();
        let anon: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(anon, serde_json::json!({"user": null}));
        let res = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/apostille/api/v1/auth/profile/methods")
                    .extension(addr())
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
        // Seed session + one passkey row directly.
        let store = handbook_store::store::Store::new(tmp.path());
        let conn = handbook_store::db::open(&store).unwrap();
        let now = unix_now();
        auth::passkey::ensure_account(&conn, "u-9", now).unwrap();
        auth::create_session(
            &conn,
            "sess-9",
            &auth::NewSession {
                user_id: "u-9",
                role: auth::Role::Customer,
                customer_ref: Some("u-9"),
                now,
                ip: "127.0.0.1",
                user_agent: "test",
            },
        )
        .unwrap();
        auth::passkey::store_credential(&conn, "u-9", "cred-9", "{}", "Phone", now).unwrap();
        drop(conn);
        let cookies = format!("{}=sess-9", auth::PUB_COOKIE);
        let res = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/apostille/api/v1/auth/me")
                    .header(header::COOKIE, &cookies)
                    .extension(addr())
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = axum::body::to_bytes(res.into_body(), 4096).await.unwrap();
        let me: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(me["user_id"], "u-9");
        assert_eq!(me["methods"], serde_json::json!(["passkey"]));
        let res = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/apostille/api/v1/auth/profile/methods")
                    .header(header::COOKIE, &cookies)
                    .extension(addr())
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = axum::body::to_bytes(res.into_body(), 4096).await.unwrap();
        let methods: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(methods["methods"][0]["label"], "Phone");
        // Logout clears the session (idempotent 200).
        let res = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/apostille/api/v1/auth/logout")
                    .header(header::COOKIE, &cookies)
                    .extension(addr())
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let cleared: Vec<_> = res
            .headers()
            .get_all(header::SET_COOKIE)
            .iter()
            .filter_map(|v| v.to_str().ok().map(str::to_string))
            .collect();
        assert!(
            cleared
                .iter()
                .any(|c| c.starts_with(&format!("{}=;", auth::PUB_COOKIE))
                    || c.contains("Max-Age=0"))
        );
        let res = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/apostille/api/v1/auth/me")
                    .header(header::COOKIE, &cookies)
                    .extension(addr())
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn unlink_enforces_proof_and_last_method() {
        let (tmp, app) = test_app();
        let store = handbook_store::store::Store::new(tmp.path());
        let conn = handbook_store::db::open(&store).unwrap();
        let now = unix_now();
        auth::passkey::ensure_account(&conn, "u-9", now).unwrap();
        auth::create_session(
            &conn,
            "sess-9",
            &auth::NewSession {
                user_id: "u-9",
                role: auth::Role::Customer,
                customer_ref: Some("u-9"),
                now,
                ip: "127.0.0.1",
                user_agent: "test",
            },
        )
        .unwrap();
        auth::passkey::store_credential(&conn, "u-9", "cred-1", "{}", "", now).unwrap();
        auth::passkey::store_challenge(
            &conn,
            "proof-1",
            &auth::passkey::NewChallenge {
                kind: auth::passkey::KIND_LOGIN,
                user_id: Some("u-9"),
                state_json: "{}",
                now,
            },
        )
        .unwrap();
        // Consume the challenge to mint a fresh proof.
        auth::passkey::take_challenge(&conn, "proof-1", auth::passkey::KIND_LOGIN, now).unwrap();
        drop(conn);
        let cookies = format!("{}=sess-9", auth::PUB_COOKIE);
        let (_, _, set_cookie) = start_register(&app, "Andrii", "").await;
        let (_, token) = csrf_pair(&set_cookie);
        async fn call(
            app: &axum::Router,
            cookies: &str,
            token: &str,
            proof: &str,
            id: &str,
        ) -> StatusCode {
            let jar = format!("{cookies}; {}={token}", auth::CSRF_COOKIE);
            app.clone()
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri("/apostille/api/v1/auth/profile/unlink")
                        .header(header::CONTENT_TYPE, "application/json")
                        .header(header::ORIGIN, "http://localhost")
                        .header(header::COOKIE, jar)
                        .header(auth::CSRF_HEADER, token)
                        .extension(addr())
                        .body(Body::from(format!(
                            r#"{{"method":"passkey","id":"{id}","reauth_proof":"{proof}"}}"#
                        )))
                        .unwrap(),
                )
                .await
                .unwrap()
                .status()
        }
        // Bad proof → 403 even for a real credential.
        assert_eq!(
            call(&app, &cookies, &token, "nope", "cred-1").await,
            StatusCode::FORBIDDEN
        );
        // Good proof but last method → 409.
        assert_eq!(
            call(&app, &cookies, &token, "proof-1", "cred-1").await,
            StatusCode::CONFLICT
        );
    }

    #[tokio::test]
    async fn unlink_google_accepts_fresh_session_proof() {
        let (tmp, app) = test_app();
        let store = handbook_store::store::Store::new(tmp.path());
        let conn = handbook_store::db::open(&store).unwrap();
        let now = unix_now();
        auth::passkey::ensure_account(&conn, "u-g", now).unwrap();
        auth::create_session(
            &conn,
            "sess-g",
            &auth::NewSession {
                user_id: "u-g",
                role: auth::Role::Customer,
                customer_ref: Some("u-g"),
                now,
                ip: "127.0.0.1",
                user_agent: "test",
            },
        )
        .unwrap();
        auth::oauth::insert_google_link(&conn, "u-g", "sub-1", "g@example.test", true).unwrap();
        auth::oauth::insert_google_link(&conn, "u-g", "sub-2", "g@example.test", true).unwrap();
        drop(conn);
        let cookies = format!("{}=sess-g", auth::PUB_COOKIE);
        let (_, _, set_cookie) = start_register(&app, "Andrii", "").await;
        let (_, token) = csrf_pair(&set_cookie);
        async fn call(
            app: &axum::Router,
            cookies: &str,
            token: &str,
            proof: &str,
            id: &str,
        ) -> StatusCode {
            let jar = format!("{cookies}; {}={token}", auth::CSRF_COOKIE);
            app.clone()
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri("/apostille/api/v1/auth/profile/unlink")
                        .header(header::CONTENT_TYPE, "application/json")
                        .header(header::ORIGIN, "http://localhost")
                        .header(header::COOKIE, jar)
                        .header(auth::CSRF_HEADER, token)
                        .extension(addr())
                        .body(Body::from(format!(
                            r#"{{"method":"google","id":"{id}","reauth_proof":"{proof}"}}"#
                        )))
                        .unwrap(),
                )
                .await
                .unwrap()
                .status()
        }
        // Bogus proof → 403.
        assert_eq!(
            call(&app, &cookies, &token, "nope", "sub-1").await,
            StatusCode::FORBIDDEN
        );
        // Fresh session proof → 200 (a method remains).
        assert_eq!(
            call(&app, &cookies, &token, "session", "sub-1").await,
            StatusCode::OK
        );
        // Last method → 409 even with a good proof.
        assert_eq!(
            call(&app, &cookies, &token, "session", "sub-2").await,
            StatusCode::CONFLICT
        );
    }

    #[tokio::test]
    async fn unlink_apple_meta_accept_session_proof() {
        let (tmp, app) = test_app();
        let store = handbook_store::store::Store::new(tmp.path());
        let conn = handbook_store::db::open(&store).unwrap();
        let now = unix_now();
        auth::passkey::ensure_account(&conn, "u-am", now).unwrap();
        auth::create_session(
            &conn,
            "sess-am",
            &auth::NewSession {
                user_id: "u-am",
                role: auth::Role::Customer,
                customer_ref: Some("u-am"),
                now,
                ip: "127.0.0.1",
                user_agent: "test",
            },
        )
        .unwrap();
        auth::apple::insert_apple_link(&conn, "u-am", "sub-a", "a@example.test", true, false)
            .unwrap();
        auth::meta::insert_meta_link(&conn, "u-am", "sub-m").unwrap();
        drop(conn);
        let cookies = format!("{}=sess-am", auth::PUB_COOKIE);
        let (_, _, set_cookie) = start_register(&app, "Andrii", "").await;
        let (_, token) = csrf_pair(&set_cookie);
        async fn call(
            app: &axum::Router,
            cookies: &str,
            token: &str,
            method: &str,
            id: &str,
        ) -> StatusCode {
            let jar = format!("{cookies}; {}={token}", auth::CSRF_COOKIE);
            app.clone()
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri("/apostille/api/v1/auth/profile/unlink")
                        .header(header::CONTENT_TYPE, "application/json")
                        .header(header::ORIGIN, "http://localhost")
                        .header(header::COOKIE, jar)
                        .header(auth::CSRF_HEADER, token)
                        .extension(addr())
                        .body(Body::from(format!(
                            r#"{{"method":"{method}","id":"{id}","reauth_proof":"session"}}"#
                        )))
                        .unwrap(),
                )
                .await
                .unwrap()
                .status()
        }
        // Unknown methods still 400.
        assert_eq!(
            call(&app, &cookies, &token, "sso", "sub-a").await,
            StatusCode::BAD_REQUEST
        );
        // Apple unlink with a fresh session proof → 200.
        assert_eq!(
            call(&app, &cookies, &token, "apple", "sub-a").await,
            StatusCode::OK
        );
        // Last method (meta) → 409 even with a good proof.
        assert_eq!(
            call(&app, &cookies, &token, "meta", "sub-m").await,
            StatusCode::CONFLICT
        );
    }

    #[tokio::test]
    async fn register_start_mode_b_enforces_device_cap() {
        let (tmp, app) = test_app();
        let store = handbook_store::store::Store::new(tmp.path());
        let conn = handbook_store::db::open(&store).unwrap();
        let now = unix_now();
        let user_id = auth::passkey::mint_user_id();
        auth::passkey::ensure_account(&conn, &user_id, now).unwrap();
        auth::create_session(
            &conn,
            "sess-cap",
            &auth::NewSession {
                user_id: &user_id,
                role: auth::Role::Customer,
                customer_ref: Some(&user_id),
                now,
                ip: "127.0.0.1",
                user_agent: "test",
            },
        )
        .unwrap();
        for n in 0..store::MAX_CREDENTIALS_PER_ACCOUNT {
            auth::passkey::store_credential(&conn, &user_id, &format!("cred-{n}"), "{}", "", now)
                .unwrap();
        }
        drop(conn);
        let cookies = format!("{}=sess-cap", auth::PUB_COOKIE);
        // At cap: 409 with the remove-a-device copy (fresh session
        // passes the step-up gate, so the cap is what fires).
        let res = app
            .clone()
            .oneshot(post_json(
                "/apostille/api/v1/auth/passkey/register/start",
                &cookies,
                r#"{"label":"extra"}"#,
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::CONFLICT);
        let body = axum::body::to_bytes(res.into_body(), 4096).await.unwrap();
        let denied: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(
            denied["error"],
            "device limit reached: remove a passkey to add a new one"
        );
        // Remove one → a slot frees and start succeeds.
        let conn = handbook_store::db::open(&store).unwrap();
        auth::passkey::revoke_credential(&conn, &user_id, "cred-0", unix_now()).unwrap();
        drop(conn);
        let (status, _, _) = start_register(&app, "", &cookies).await;
        assert_eq!(status, StatusCode::OK);
    }
}
