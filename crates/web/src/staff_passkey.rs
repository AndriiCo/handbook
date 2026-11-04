//! Staff passkey ceremonies (spec §5): invitation-gated
//! enrolment + UV-required login (usernameless: staff passkeys are
//! discoverable), admin branch only, plus the Staff page's
//! management routes (§5.6). Mirrors the customer handlers in
//! `passkey.rs` shape-for-shape; deltas are invite gating, `staff_*`
//! challenge kinds, `ADM_COOKIE`, and role-aware accounts. The
//! ceremony handlers must NOT take `AdminAuth` (enrolment precedes
//! any session) — they live behind the host and IP guards with
//! per-handler Origin/CSRF enforcement. The management routes do
//! take `AdminAuth` and sit behind the CSRF layer.

use axum::{
    Json,
    extract::{ConnectInfo, Extension, Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use base64::Engine as _;
use serde_json::json;
use std::net::SocketAddr;
use webauthn_rs::prelude::{
    Passkey, PasskeyAuthentication, PasskeyRegistration, PublicKeyCredential,
    RegisterPublicKeyCredential,
};

use super::admin::{AdminAuth, VerifiedClientCert};
use super::passkey;
use super::{AppState, store_unavailable, unix_now};
use auth::passkey as store;

/// Enrol-start challenge payload: the ceremony state plus the
/// invite hash the flow started with (finish compares — binds the
/// ceremony to its invite). Setup-mode ceremonies (§9.2) carry an
/// empty hash with `setup = true` instead.
#[derive(serde::Serialize, serde::Deserialize)]
struct EnrolState {
    reg: PasskeyRegistration,
    invite_hash: String,
    #[serde(default)]
    setup: bool,
    /// Labels the first passkey when enrolment gives no label, so the
    /// Staff page can tell accounts apart.
    #[serde(default)]
    display_name: String,
    /// Sign-in email (lowercased; empty when none was given): the
    /// passkey's account name, saved once the ceremony verifies.
    #[serde(default)]
    email: String,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EnrolStartBody {
    invite: Option<String>,
    /// First-boot path (§9.2): the CLI-minted setup key, accepted
    /// INSTEAD of invite + client cert while the window is open.
    setup_key: Option<String>,
    display_name: String,
    label: Option<String>,
    /// Sign-in email: names the passkey in the browser's passkey list
    /// and autofill instead of the 32-hex user id. Optional for older
    /// clients; the console always sends it.
    email: Option<String>,
}

/// `POST /auth/passkey/enrol/start` — validate invite +
/// cert, mint user, begin the UV ceremony.
pub async fn enrol_start(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    verified: Option<Extension<VerifiedClientCert>>,
    headers: HeaderMap,
    Json(body): Json<EnrolStartBody>,
) -> Response {
    let now = unix_now();
    if !passkey::origin_ok(&headers, state.admin_mtls_off) {
        return passkey::csrf_fail();
    }
    let ip = passkey::client_ip(addr);
    if !state.auth_limits.check(&format!("es:{ip}"), now, 30, 60) {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            Json(json!({"error": "rate limited"})),
        )
            .into_response();
    }
    let Some(conn) = passkey::open_conn(&state) else {
        return store_unavailable();
    };
    let setup_mode = match (body.invite.as_deref(), body.setup_key.as_deref()) {
        (Some(_), Some(_)) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "invite or setup_key, not both"})),
            )
                .into_response();
        }
        (None, None) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "invite or setup_key required"})),
            )
                .into_response();
        }
        (Some(_), None) => false,
        (None, Some(_)) => true,
    };
    // Setup-mode: the key alone replaces invite + cert (§9.2, D45).
    // The first staffer is always an admin (they own the server they
    // just configured). Refusals reuse the invite-gone shape exactly
    // — no oracle for whether the window is open.
    let (role, cn, fingerprint, invite_hash) = if setup_mode {
        let key = body.setup_key.as_deref().unwrap_or("");
        let ok = match state.kek.as_ref() {
            Some(kek) => {
                let hash = handbook_store::store::sha256_hex(key.as_bytes());
                config::verify_setup_key(&conn, kek, &hash, now).unwrap_or(false)
            }
            None => false,
        };
        if !ok {
            passkey::audit_auth(&conn, now, "anonymous", "setup.denied", &ip);
            return (
                StatusCode::GONE,
                Json(json!({"error": "invite invalid or expired"})),
            )
                .into_response();
        }
        (
            "admin".to_string(),
            "setup".to_string(),
            String::new(),
            String::new(),
        )
    } else {
        let invite = body.invite.as_deref().unwrap_or("");
        if invite.len() != 64 || !invite.bytes().all(|b| b.is_ascii_hexdigit()) {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "invite malformed"})),
            )
                .into_response();
        }
        let invite_hash = auth::login::token_hash(invite);
        let Some((role, cn)) = store::get_invite(&conn, &invite_hash, now).unwrap_or(None) else {
            return (
                StatusCode::GONE,
                Json(json!({"error": "invite invalid or expired"})),
            )
                .into_response();
        };
        // No presented cert is fine (the IP gate covers the route);
        // a presented one must still match the invitation.
        let (fingerprint, presented_cn) = super::admin::presented_cert_cn(
            &headers,
            verified.as_ref().map(|ext| &ext.0),
            &state,
            &conn,
            now,
        )
        .unwrap_or_default();
        if !presented_cn.is_empty() && presented_cn != cn {
            passkey::audit_auth(
                &conn,
                now,
                "anonymous",
                "staff_enrol_refused",
                "cn mismatch",
            );
            return (
                StatusCode::FORBIDDEN,
                Json(json!({"error": "certificate does not match invitation"})),
            )
                .into_response();
        }
        (role, cn, fingerprint, invite_hash)
    };
    if body.display_name.is_empty() || body.display_name.len() > 128 {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "display_name required (max 128)"})),
        )
            .into_response();
    }
    if body.label.as_deref().is_some_and(|label| label.len() > 64) {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "label too long (max 64)"})),
        )
            .into_response();
    }
    // Checked only after the invite or setup key passed: whether an
    // email is taken is never answered to an anonymous caller.
    let email = match body.email.as_deref() {
        None => String::new(),
        Some(raw) => match passkey::valid_email(raw) {
            Some(email) => email,
            None => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({"error": "email malformed"})),
                )
                    .into_response();
            }
        },
    };
    if !email.is_empty() && store::staff_email_taken(&conn, &email).unwrap_or(true) {
        return (
            StatusCode::CONFLICT,
            Json(json!({"error": "email already used by another staff account"})),
        )
            .into_response();
    }
    let user_id = store::mint_user_id();
    let uuid = match uuid::Uuid::parse_str(&user_id) {
        Ok(uuid) => uuid,
        Err(_) => return store_unavailable(),
    };
    // The passkey's account name is what the browser shows in its
    // passkey list and autofill: the email, not the hex id.
    let account_name = if email.is_empty() { &user_id } else { &email };
    let (options, reg_state) = match state.webauthn.start_passkey_registration(
        uuid,
        account_name,
        &body.display_name,
        None,
    ) {
        Ok(pair) => pair,
        Err(_) => return store_unavailable(),
    };
    let enrol = EnrolState {
        reg: reg_state,
        invite_hash,
        setup: setup_mode,
        display_name: body.display_name.clone(),
        email,
    };
    let Ok(state_json) = serde_json::to_string(&enrol) else {
        return store_unavailable();
    };
    let challenge_id = auth::csrf::mint_token();
    let _ = store::sweep_challenges(&conn, now);
    if store::store_challenge(
        &conn,
        &challenge_id,
        &store::NewChallenge {
            kind: store::KIND_STAFF_REGISTER,
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
    let secure = !state.admin_mtls_off;
    (
        StatusCode::OK,
        passkey::set_cookies(&[auth::csrf_cookie(&csrf, secure)]),
        Json(json!({
            "challenge_id": challenge_id,
            // Discoverable: staff sign in without typing a user ID.
            "options": passkey::discoverable_options(&options, true),
            "role": role,
            "cn": cn,
            "fp_suffix": fingerprint.get(fingerprint.len().saturating_sub(8)..).unwrap_or(&fingerprint),
        })),
    )
        .into_response()
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EnrolFinishBody {
    challenge_id: String,
    credential: RegisterPublicKeyCredential,
    invite: Option<String>,
    /// Setup-mode ceremonies re-present the key (burn-before-persist
    /// races fail closed here).
    setup_key: Option<String>,
    label: Option<String>,
}

/// `POST /auth/passkey/enrol/finish` — verify the UV
/// ceremony, consume the invite, create the staff account +
/// credential, and log the staffer in.
pub async fn enrol_finish(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    verified: Option<Extension<VerifiedClientCert>>,
    headers: HeaderMap,
    Json(body): Json<EnrolFinishBody>,
) -> Response {
    let now = unix_now();
    if !passkey::origin_ok(&headers, state.admin_mtls_off) || !passkey::csrf_echo_ok(&headers) {
        return passkey::csrf_fail();
    }
    let ip = passkey::client_ip(addr);
    if !state.auth_limits.check(&format!("ef:{ip}"), now, 60, 60) {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            Json(json!({"error": "rate limited"})),
        )
            .into_response();
    }
    let Some(conn) = passkey::open_conn(&state) else {
        return store_unavailable();
    };
    let Some((Some(user_id), state_json)) =
        store::take_challenge(&conn, &body.challenge_id, store::KIND_STAFF_REGISTER, now)
            .unwrap_or(None)
    else {
        return (
            StatusCode::GONE,
            Json(json!({"error": "challenge invalid or expired"})),
        )
            .into_response();
    };
    let Ok(enrol) = serde_json::from_str::<EnrolState>(&state_json) else {
        return store_unavailable();
    };
    // Setup-mode finish (§9.2): the key is re-presented here.
    // The burn happens only after the ceremony verifies (verify →
    // burn → persist): a failed ceremony must not brick a fresh
    // server. Invite-mode keeps the cert recheck + consume below,
    // untouched.
    let (role, cn, fingerprint) = if enrol.setup {
        let key = body.setup_key.as_deref().unwrap_or("");
        let ok = match state.kek.as_ref() {
            Some(kek) => {
                let hash = handbook_store::store::sha256_hex(key.as_bytes());
                config::verify_setup_key(&conn, kek, &hash, now).unwrap_or(false)
            }
            None => false,
        };
        if !ok {
            passkey::audit_auth(&conn, now, "anonymous", "setup.denied", &ip);
            return (
                StatusCode::GONE,
                Json(json!({"error": "invite invalid or expired"})),
            )
                .into_response();
        }
        (
            "admin".to_string(),
            "setup".to_string(),
            "setup-key".to_string(),
        )
    } else {
        let invite = body.invite.as_deref().unwrap_or("");
        let invite_hash = auth::login::token_hash(invite);
        if !auth::csrf::tokens_match(&invite_hash, &enrol.invite_hash) {
            return (
                StatusCode::GONE,
                Json(json!({"error": "invite invalid or expired"})),
            )
                .into_response();
        }
        if !state.auth_limits.check(
            &format!("ef:{}", &invite_hash[..16.min(invite_hash.len())]),
            now,
            10,
            60,
        ) {
            return (
                StatusCode::TOO_MANY_REQUESTS,
                Json(json!({"error": "rate limited"})),
            )
                .into_response();
        }
        // No presented cert is fine (the IP gate covers the route);
        // a presented one must still match before the invite burns.
        let (fingerprint, presented_cn) = super::admin::presented_cert_cn(
            &headers,
            verified.as_ref().map(|ext| &ext.0),
            &state,
            &conn,
            now,
        )
        .unwrap_or_default();
        // CN match BEFORE consume: a mismatch must not burn the
        // invite (an attacker holding another staffer's cert could
        // otherwise DoS the enrolment by burning it; without the
        // burn their retries can never succeed).
        let Some((role, cn)) = store::get_invite(&conn, &invite_hash, now).unwrap_or(None) else {
            passkey::audit_auth(&conn, now, "anonymous", "staff_enrol_refused", "invite");
            return (
                StatusCode::GONE,
                Json(json!({"error": "invite invalid or expired"})),
            )
                .into_response();
        };
        if !presented_cn.is_empty() && cn != presented_cn {
            passkey::audit_auth(
                &conn,
                now,
                "anonymous",
                "staff_enrol_refused",
                "cn mismatch",
            );
            return (
                StatusCode::FORBIDDEN,
                Json(json!({"error": "certificate does not match invitation"})),
            )
                .into_response();
        }
        // Consume BEFORE persisting (race-loss → 410): a burned
        // invite on later ceremony failure fails closed (fresh
        // invite is the recovery), while persisting first would
        // orphan credentials.
        if store::consume_invite(&conn, &invite_hash, &user_id, now)
            .unwrap_or(None)
            .is_none()
        {
            passkey::audit_auth(&conn, now, "anonymous", "staff_enrol_refused", "invite");
            return (
                StatusCode::GONE,
                Json(json!({"error": "invite invalid or expired"})),
            )
                .into_response();
        }
        (role, cn, fingerprint)
    };
    // UV is enforced inside `finish_passkey_registration` (the
    // crate requests UV Required at every start — a non-UV
    // ceremony fails closed here, identically for customer and
    // staff). Variants never leak, mirroring customer finish.
    let result = match state
        .webauthn
        .finish_passkey_registration(&body.credential, &enrol.reg)
    {
        Ok(passkey) => passkey,
        Err(_) => {
            return (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(json!({"error": "verification failed"})),
            )
                .into_response();
        }
    };
    let credential_id = passkey::cred_id_string(result.cred_id());
    if store::credential_known(&conn, &credential_id).unwrap_or(true) {
        passkey::audit_auth(
            &conn,
            now,
            &user_id,
            "passkey_reregister_refused",
            "credential already registered",
        );
        return (
            StatusCode::CONFLICT,
            Json(json!({"error": "credential already registered"})),
        )
            .into_response();
    }
    if body.label.as_deref().is_some_and(|label| label.len() > 64) {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "label too long (max 64)"})),
        )
            .into_response();
    }
    let role_parsed = match auth::Role::parse(&role) {
        Some(role) if role.meets(auth::Role::Staff) => role,
        _ => return store_unavailable(),
    };
    // First verified enrol closes the setup window in BOTH modes
    // (spec §9.1) — after the ceremony verifies AND the credential
    // is fresh, so failures never brick a fresh server. Setup mode
    // always burns; a lost burn fails closed (410) so concurrent
    // ceremonies yield a single winner. Invite mode burns only
    // when no staff credentials exist yet, so a break-glass
    // re-mint survives routine later enrols; best-effort (no 410
    // — the enrol won its race at consume_invite). The helper
    // no-ops when no row exists.
    if enrol.setup {
        if !crate::setup::burn_setup_key(&conn, &user_id, now) {
            return (
                StatusCode::GONE,
                Json(json!({"error": "invite invalid or expired"})),
            )
                .into_response();
        }
    } else if store::live_staff_passkey_count(&conn).unwrap_or(1) == 0 {
        crate::setup::burn_setup_key(&conn, &user_id, now);
    }
    if store::ensure_staff_account(&conn, &user_id, &role, now).is_err() {
        return store_unavailable();
    }
    // Best effort: the invite or setup key is already spent, so a lost
    // race for the same email must not fail the enrolment. The passkey
    // still carries the email as its account name.
    if !enrol.email.is_empty() {
        let _ = store::set_staff_email(&conn, &user_id, &enrol.email, now);
    }
    let passkey_json = match serde_json::to_string(&result) {
        Ok(json) => json,
        Err(_) => return store_unavailable(),
    };
    let label = body
        .label
        .as_deref()
        .filter(|label| !label.is_empty())
        .unwrap_or(&enrol.display_name);
    if store::store_credential(&conn, &user_id, &credential_id, &passkey_json, label, now).is_err()
    {
        return store_unavailable();
    }
    let _ = store::set_step_up(&conn, &user_id, now);
    let session_id = auth::mint_session_id();
    if auth::create_session(
        &conn,
        &session_id,
        &auth::NewSession {
            user_id: &user_id,
            role: role_parsed,
            customer_ref: None,
            now,
            ip: &ip,
            user_agent: passkey::user_agent(&headers),
        },
    )
    .is_err()
    {
        return store_unavailable();
    }
    let rotation = auth::csrf::mint_token();
    let secure = !state.admin_mtls_off;
    passkey::audit_auth(
        &conn,
        now,
        &user_id,
        "staff_enrolled",
        &format!("{role}:{cn}:{fingerprint}"),
    );
    (
        StatusCode::OK,
        passkey::set_cookies(&[
            auth::csrf_cookie(&rotation, secure),
            auth::session_cookie(auth::ADM_COOKIE, &session_id, secure),
        ]),
        Json(json!({"user_id": user_id, "role": role, "methods": ["passkey"]})),
    )
        .into_response()
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LoginStartBody {
    #[serde(default)]
    account: String,
}

/// `POST /auth/passkey/login/start` — empty `account` (or none)
/// starts a discoverable ceremony: the authenticator offers the staff
/// passkeys it holds, no user ID typed (spec §5.3). Otherwise
/// `user_id` hex only (no emails on the admin branch; the config
/// step-up names the session's own id); no role oracle.
pub async fn login_start(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(body): Json<LoginStartBody>,
) -> Response {
    fn unknown() -> Response {
        (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error": "unknown account or no passkeys"})),
        )
            .into_response()
    }
    let now = unix_now();
    if !passkey::origin_ok(&headers, state.admin_mtls_off) {
        return passkey::csrf_fail();
    }
    let ip = passkey::client_ip(addr);
    if !state.auth_limits.check(&format!("sl:{ip}"), now, 30, 60) {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            Json(json!({"error": "rate limited"})),
        )
            .into_response();
    }
    let Some(conn) = passkey::open_conn(&state) else {
        return store_unavailable();
    };
    if body.account.trim().is_empty() {
        let Some((options, state_json)) = passkey::start_discoverable(&state.webauthn) else {
            return store_unavailable();
        };
        let challenge_id = auth::csrf::mint_token();
        let _ = store::sweep_challenges(&conn, now);
        if store::store_challenge(
            &conn,
            &challenge_id,
            &store::NewChallenge {
                kind: store::KIND_STAFF_DISCOVERABLE,
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
            passkey::set_cookies(&[auth::csrf_cookie(&csrf, !state.admin_mtls_off)]),
            Json(json!({"challenge_id": challenge_id, "options": options})),
        )
            .into_response();
    }
    // Staff accounts resolve by id only — emails aid enumeration.
    let Some((Some(user_id), _)) = passkey::split_account(&body.account) else {
        return unknown();
    };
    let account = match store::get_account(&conn, user_id).unwrap_or(None) {
        Some(account) if account.locked_at.is_none() => account,
        _ => return unknown(),
    };
    let stored = match store::credentials_for_user(&conn, &account.user_id) {
        Ok(stored) if !stored.is_empty() => stored,
        _ => return unknown(),
    };
    let mut passkeys = Vec::new();
    for (_, passkey_json) in &stored {
        let Ok(passkey) = serde_json::from_str::<Passkey>(passkey_json) else {
            return store_unavailable();
        };
        passkeys.push(passkey);
    }
    let (options, auth_state) = match state.webauthn.start_passkey_authentication(&passkeys) {
        Ok(pair) => pair,
        Err(_) => return store_unavailable(),
    };
    let Ok(state_json) = serde_json::to_string(&auth_state) else {
        return store_unavailable();
    };
    let challenge_id = auth::csrf::mint_token();
    let _ = store::sweep_challenges(&conn, now);
    if store::store_challenge(
        &conn,
        &challenge_id,
        &store::NewChallenge {
            kind: store::KIND_STAFF_LOGIN,
            user_id: Some(&account.user_id),
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
        passkey::set_cookies(&[auth::csrf_cookie(&csrf, !state.admin_mtls_off)]),
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

/// `POST /auth/passkey/login/finish` — verify the UV
/// ceremony against a presented live cert, mint a staff session.
pub async fn login_finish(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    verified: Option<Extension<VerifiedClientCert>>,
    headers: HeaderMap,
    Json(body): Json<LoginFinishBody>,
) -> Response {
    fn failed() -> Response {
        (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error": "authentication failed"})),
        )
            .into_response()
    }
    let now = unix_now();
    if !passkey::origin_ok(&headers, state.admin_mtls_off) || !passkey::csrf_echo_ok(&headers) {
        return passkey::csrf_fail();
    }
    let ip = passkey::client_ip(addr);
    if !state.auth_limits.check(&format!("sf:{ip}"), now, 60, 60) {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            Json(json!({"error": "rate limited"})),
        )
            .into_response();
    }
    let Some(conn) = passkey::open_conn(&state) else {
        return store_unavailable();
    };
    let limited = || {
        (
            StatusCode::TOO_MANY_REQUESTS,
            Json(json!({"error": "rate limited"})),
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
    // Presented cert (if any — user binding awaits OPEN-15);
    // recorded in the login audit below. No cert is required:
    // session + listed peer IP admit.
    let (fingerprint, _) = super::admin::presented_cert_cn(
        &headers,
        verified.as_ref().map(|ext| &ext.0),
        &state,
        &conn,
        now,
    )
    .unwrap_or_default();
    let presented = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(body.credential.get_credential_id());
    let named = store::take_challenge(&conn, &body.challenge_id, store::KIND_STAFF_LOGIN, now)
        .unwrap_or(None);
    let (user_id, result) = if let Some((Some(user_id), state_json)) = named {
        if !state
            .auth_limits
            .check(&format!("sf:{user_id}"), now, 10, 60)
        {
            return limited();
        }
        if store::credential_owner(&conn, &presented)
            .unwrap_or(None)
            .is_none()
        {
            return re_register();
        }
        let Ok(auth_state) = serde_json::from_str::<PasskeyAuthentication>(&state_json) else {
            return store_unavailable();
        };
        match state
            .webauthn
            .finish_passkey_authentication(&body.credential, &auth_state)
        {
            Ok(result) => (user_id, result),
            Err(_) => return failed(),
        }
    } else if let Some((None, state_json)) = store::take_challenge(
        &conn,
        &body.challenge_id,
        store::KIND_STAFF_DISCOVERABLE,
        now,
    )
    .unwrap_or(None)
    {
        match passkey::finish_discoverable(&state.webauthn, &conn, &body.credential, &state_json) {
            Ok((user_id, result)) => {
                if !state
                    .auth_limits
                    .check(&format!("sf:{user_id}"), now, 10, 60)
                {
                    return limited();
                }
                (user_id, result)
            }
            Err(passkey::DiscoverFail::Unknown) => return re_register(),
            Err(passkey::DiscoverFail::Failed) => return failed(),
            Err(passkey::DiscoverFail::Store) => return store_unavailable(),
        }
    } else {
        return (
            StatusCode::GONE,
            Json(json!({"error": "challenge invalid or expired"})),
        )
            .into_response();
    };
    let authed_id = passkey::cred_id_string(result.cred_id());
    let Some((owner_id, blob)) = store::credential_owner(&conn, &authed_id).unwrap_or(None) else {
        return failed();
    };
    if owner_id != user_id {
        return failed();
    }
    // Explicit UV assert with a paper trail (the crate also
    // enforces UV Required at finish — this is belt-and-braces;
    // the customer flow only records it for step-up).
    if !result.user_verified() {
        passkey::audit_auth(&conn, now, &user_id, "staff_login_refused", "uv missing");
        return failed();
    }
    let account = match store::get_account(&conn, &user_id).unwrap_or(None) {
        Some(account) if account.locked_at.is_none() => account,
        _ => return failed(),
    };
    let role = match auth::Role::parse(&account.role) {
        Some(role) if role.meets(auth::Role::Staff) => role,
        Some(_) => {
            return (
                StatusCode::FORBIDDEN,
                Json(json!({"error": "use the customer login"})),
            )
                .into_response();
        }
        None => return failed(),
    };
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
    let _ = store::set_step_up(&conn, &user_id, now);
    let session_id = auth::mint_session_id();
    if auth::create_session(
        &conn,
        &session_id,
        &auth::NewSession {
            user_id: &user_id,
            role,
            customer_ref: None,
            now,
            ip: &ip,
            user_agent: passkey::user_agent(&headers),
        },
    )
    .is_err()
    {
        return store_unavailable();
    }
    let rotation = auth::csrf::mint_token();
    let secure = !state.admin_mtls_off;
    passkey::audit_auth(
        &conn,
        now,
        &user_id,
        "staff_login",
        &format!("{}:{fingerprint}", auth::hash_str(&presented)),
    );
    (
        StatusCode::OK,
        passkey::set_cookies(&[
            auth::csrf_cookie(&rotation, secure),
            auth::session_cookie(auth::ADM_COOKIE, &session_id, secure),
        ]),
        Json(json!({"user_id": user_id, "methods": ["passkey"]})),
    )
        .into_response()
}

/// `POST /auth/logout` (admin branch) — idempotent staff sign-out:
/// deletes the session behind the admin cookie, audits it, and
/// clears the admin + CSRF cookies. Mirrors [`passkey::logout`] on
/// the staff cookie. The CSRF layer passes `/auth/*` through, so
/// the Origin check runs here: a cross-site page cannot sign staff
/// out.
pub async fn logout(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if !passkey::origin_ok(&headers, state.admin_mtls_off) {
        return passkey::csrf_fail();
    }
    let now = unix_now();
    let secure = !state.admin_mtls_off;
    let token = headers
        .get(axum::http::header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .and_then(|cookies| auth::cookie_value(cookies, auth::ADM_COOKIE));
    if let Some(token) = token
        && let Some(conn) = passkey::open_conn(&state)
        && let Ok(Some(session)) = auth::get_session(&conn, token)
    {
        let _ = auth::delete_session(&conn, &session.id);
        passkey::audit_auth(&conn, now, &session.user_id, "staff_logout", "");
    }
    (
        StatusCode::OK,
        passkey::set_cookies(&[
            auth::clear_cookie(auth::ADM_COOKIE, secure),
            auth::clear_cookie(auth::CSRF_COOKIE, secure),
        ]),
        Json(json!({"ok": true})),
    )
        .into_response()
}

// ---- Staff management (customer-auth.md §5.6) ----
//
// The console's Staff page. Every route takes `AdminAuth` (session +
// staff role + allowed IP) and sits behind the CSRF layer (Origin +
// double-submit on every POST). Mutations also need a fresh step-up;
// invites and other people's accounts need the admin role.

/// The 403 a staff-role caller gets on an admin-only change.
fn admin_required(user: &auth::AuthenticatedUser) -> Option<Response> {
    if user.role.meets(auth::Role::Admin) {
        return None;
    }
    Some(
        (
            StatusCode::FORBIDDEN,
            Json(json!({"error": "admin role required", "code": "admin_role_required"})),
        )
            .into_response(),
    )
}

/// `Some(403)` unless the caller confirmed a passkey in the last 10
/// minutes (spec §5.5).
fn stale_step_up(conn: &rusqlite::Connection, user: &auth::AuthenticatedUser) -> Option<Response> {
    let ages = crate::admin::step_up_ages(conn, user, unix_now());
    (!ages.fresh).then(|| crate::admin::step_up_refusal(user, &ages))
}

fn conflict(message: &str) -> Response {
    (StatusCode::CONFLICT, Json(json!({"error": message}))).into_response()
}

fn bad_request(message: &str) -> Response {
    (StatusCode::BAD_REQUEST, Json(json!({"error": message}))).into_response()
}

/// `GET /staff` — every staff/admin account with its live passkeys,
/// plus pending invites (admins only). No key material, no tokens.
pub async fn staff_list(auth: AdminAuth, State(state): State<AppState>) -> Response {
    let Some(conn) = passkey::open_conn(&state) else {
        return store_unavailable();
    };
    let now = unix_now();
    let Ok(accounts) = store::staff_accounts(&conn) else {
        return store_unavailable();
    };
    let mut rows = Vec::with_capacity(accounts.len());
    for account in accounts {
        let Ok(keys) = store::credential_summaries(&conn, &account.user_id) else {
            return store_unavailable();
        };
        rows.push(json!({
            "user_id": account.user_id,
            "role": account.role,
            "name": account.name,
            "email": account.email,
            "created_at": account.created_at,
            "locked_at": account.locked_at,
            "passkeys": keys.iter().map(|k| json!({
                "id": k.credential_id,
                "label": k.label,
                "created_at": k.created_at,
                "last_used_at": k.last_used_at,
            })).collect::<Vec<_>>(),
        }));
    }
    let invites = if auth.user.role.meets(auth::Role::Admin) {
        match store::pending_invites(&conn, now) {
            Ok(list) => list
                .into_iter()
                .map(|i| {
                    json!({
                        "id": i.token_hash,
                        "role": i.role,
                        "cn": i.cn,
                        "created_by": i.created_by,
                        "created_at": i.created_at,
                        "expires_at": i.expires_at,
                    })
                })
                .collect::<Vec<_>>(),
            Err(_) => return store_unavailable(),
        }
    } else {
        Vec::new()
    };
    Json(json!({
        "me": {"user_id": auth.user.user_id, "role": auth.user.role.as_str()},
        "accounts": rows,
        "invites": invites,
    }))
    .into_response()
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct InviteBody {
    role: String,
    cn: String,
    days: Option<i64>,
}

/// Invite names: ASCII word characters, dots and dashes, 1 to 64
/// (same rule as `xtask admin-invite`).
fn valid_cn(cn: &str) -> bool {
    !cn.is_empty()
        && cn.len() <= 64
        && cn
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

/// `POST /staff/invites` — mint a single-use enrolment link (admin +
/// step-up). The URL is built from the request's own admin origin
/// (the CSRF layer already checked it) and is shown once; only the
/// token hash is stored.
pub async fn invite_create(
    auth: AdminAuth,
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<InviteBody>,
) -> Response {
    if let Some(refused) = admin_required(&auth.user) {
        return refused;
    }
    if body.role != "staff" && body.role != "admin" {
        return bad_request("role must be staff or admin");
    }
    if !valid_cn(&body.cn) {
        return bad_request("invalid name (ascii word chars, dots, dashes; max 64)");
    }
    let days = body.days.unwrap_or(1);
    if !(1..=30).contains(&days) {
        return bad_request("days must be 1..30");
    }
    let origin = headers
        .get(axum::http::header::ORIGIN)
        .and_then(|v| v.to_str().ok())
        .filter(|o| auth::csrf::origin_allowed(Some(o), state.admin_mtls_off))
        .map(str::to_string)
        .unwrap_or_else(|| format!("https://{}", auth::admin_host()));
    let Some(conn) = passkey::open_conn(&state) else {
        return store_unavailable();
    };
    if let Some(refused) = stale_step_up(&conn, &auth.user) {
        return refused;
    }
    let now = unix_now();
    let token = auth::login::mint_magic_token();
    let ttl_secs = days.saturating_mul(86_400);
    if store::store_invite(
        &conn,
        &auth::login::token_hash(&token),
        &store::NewInvite {
            role: &body.role,
            cn: &body.cn,
            created_by: &auth.user.user_id,
            now,
            ttl_secs,
        },
    )
    .is_err()
    {
        return store_unavailable();
    }
    passkey::audit_auth(
        &conn,
        now,
        &auth.user.user_id,
        "staff_invite_issue",
        &format!("{}:{}", body.role, body.cn),
    );
    Json(json!({
        "url": format!("{}/enrol?invite={token}", origin.trim_end_matches('/')),
        "role": body.role,
        "cn": body.cn,
        "expires_at": now.saturating_add(ttl_secs),
    }))
    .into_response()
}

/// `POST /staff/invites/{id}/withdraw` — a pending invite expires now
/// (admin + step-up). `404` once used, expired or unknown.
pub async fn invite_withdraw(
    auth: AdminAuth,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    if let Some(refused) = admin_required(&auth.user) {
        return refused;
    }
    let Some(conn) = passkey::open_conn(&state) else {
        return store_unavailable();
    };
    if let Some(refused) = stale_step_up(&conn, &auth.user) {
        return refused;
    }
    let now = unix_now();
    match store::withdraw_invite(&conn, &id, now) {
        Ok(0) => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "invite not pending"})),
        )
            .into_response(),
        Ok(_) => {
            let prefix: String = id.chars().take(16).collect();
            passkey::audit_auth(
                &conn,
                now,
                &auth.user.user_id,
                "staff_invite_withdrawn",
                &prefix,
            );
            Json(json!({"ok": true})).into_response()
        }
        Err(_) => store_unavailable(),
    }
}

/// `POST /staff/{user_id}/revoke` — offboard one account (admin +
/// step-up): every session, every passkey, then the lock, each step
/// idempotent. Never your own account, never the last live admin.
pub async fn staff_revoke(
    auth: AdminAuth,
    State(state): State<AppState>,
    Path(user_id): Path<String>,
) -> Response {
    if let Some(refused) = admin_required(&auth.user) {
        return refused;
    }
    if user_id == auth.user.user_id {
        return conflict("you cannot revoke your own account: ask another admin");
    }
    let Some(conn) = passkey::open_conn(&state) else {
        return store_unavailable();
    };
    if let Some(refused) = stale_step_up(&conn, &auth.user) {
        return refused;
    }
    let account = match store::get_account(&conn, &user_id) {
        Ok(Some(account)) if account.role == "staff" || account.role == "admin" => account,
        Ok(_) => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({"error": "staff account not found"})),
            )
                .into_response();
        }
        Err(_) => return store_unavailable(),
    };
    if account.role == "admin"
        && account.locked_at.is_none()
        && store::live_admin_count(&conn).unwrap_or(0) <= 1
    {
        return conflict("the last admin cannot be revoked");
    }
    let now = unix_now();
    let Ok(sessions) = auth::revoke_user_sessions(&conn, &user_id) else {
        return store_unavailable();
    };
    let mut passkeys = 0;
    for (credential_id, _) in store::credentials_for_user(&conn, &user_id).unwrap_or_default() {
        passkeys += store::revoke_credential(&conn, &user_id, &credential_id, now).unwrap_or(0);
    }
    if store::lock_account(&conn, &user_id, now).is_err() {
        return store_unavailable();
    }
    passkey::audit_auth(
        &conn,
        now,
        &auth.user.user_id,
        "staff_revoked",
        &format!("{user_id}:{sessions}:{passkeys}"),
    );
    Json(json!({"ok": true, "sessions": sessions, "passkeys": passkeys})).into_response()
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PasskeyRevokeBody {
    credential_id: String,
}

/// `POST /staff/passkeys/revoke` — remove one passkey (step-up): your
/// own, or anyone's as an admin. An account's last live passkey stays
/// (`409`): revoke the account instead.
pub async fn passkey_revoke(
    auth: AdminAuth,
    State(state): State<AppState>,
    Json(body): Json<PasskeyRevokeBody>,
) -> Response {
    let Some(conn) = passkey::open_conn(&state) else {
        return store_unavailable();
    };
    if let Some(refused) = stale_step_up(&conn, &auth.user) {
        return refused;
    }
    let owner = match store::credential_owner(&conn, &body.credential_id) {
        Ok(Some((owner, _))) => owner,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({"error": "passkey not found"})),
            )
                .into_response();
        }
        Err(_) => return store_unavailable(),
    };
    if owner != auth.user.user_id
        && let Some(refused) = admin_required(&auth.user)
    {
        return refused;
    }
    // Staff routes never touch customer credentials.
    match store::get_account(&conn, &owner) {
        Ok(Some(account)) if account.role == "staff" || account.role == "admin" => {}
        Ok(_) => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({"error": "passkey not found"})),
            )
                .into_response();
        }
        Err(_) => return store_unavailable(),
    }
    if store::live_credential_count(&conn, &owner).unwrap_or(0) <= 1 {
        return conflict("the last passkey of an account cannot be removed: revoke the account");
    }
    let now = unix_now();
    if store::revoke_credential(&conn, &owner, &body.credential_id, now).is_err() {
        return store_unavailable();
    }
    passkey::audit_auth(
        &conn,
        now,
        &auth.user.user_id,
        "staff_passkey_revoked",
        &format!("{owner}:{}", auth::hash_str(&body.credential_id)),
    );
    Json(json!({"ok": true})).into_response()
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AddStartBody {
    label: Option<String>,
}

/// `POST /staff/passkeys/add/start` — begin registering another
/// passkey for the signed-in staffer (step-up). Their live passkeys are
/// excluded, so the same authenticator is not enrolled twice.
pub async fn add_start(
    auth: AdminAuth,
    State(state): State<AppState>,
    Json(body): Json<AddStartBody>,
) -> Response {
    if body.label.as_deref().is_some_and(|label| label.len() > 64) {
        return bad_request("label too long (max 64)");
    }
    let Some(conn) = passkey::open_conn(&state) else {
        return store_unavailable();
    };
    if let Some(refused) = stale_step_up(&conn, &auth.user) {
        return refused;
    }
    let user_id = auth.user.user_id.clone();
    let Ok(uuid) = uuid::Uuid::parse_str(&user_id) else {
        return store_unavailable();
    };
    let rows = store::credentials_for_user(&conn, &user_id).unwrap_or_default();
    if rows.len() >= store::MAX_CREDENTIALS_PER_ACCOUNT {
        return conflict("passkey limit reached (5): remove one first");
    }
    let mut exclude = Vec::with_capacity(rows.len());
    for (_, blob) in &rows {
        match serde_json::from_str::<Passkey>(blob) {
            Ok(key) => exclude.push(key.cred_id().clone()),
            Err(_) => return store_unavailable(),
        }
    }
    let display = body.label.clone().unwrap_or_default();
    // Same account name as the first passkey (the email when the
    // account has one), so the browser lists both under one name.
    let account_name = store::staff_email(&conn, &user_id)
        .ok()
        .flatten()
        .unwrap_or_else(|| user_id.clone());
    let (options, reg_state) = match state.webauthn.start_passkey_registration(
        uuid,
        &account_name,
        if display.is_empty() {
            &user_id
        } else {
            &display
        },
        Some(exclude),
    ) {
        Ok(pair) => pair,
        Err(_) => return store_unavailable(),
    };
    let Ok(state_json) = serde_json::to_string(&reg_state) else {
        return store_unavailable();
    };
    let now = unix_now();
    let challenge_id = auth::csrf::mint_token();
    let _ = store::sweep_challenges(&conn, now);
    if store::store_challenge(
        &conn,
        &challenge_id,
        &store::NewChallenge {
            kind: store::KIND_STAFF_ADD,
            user_id: Some(&user_id),
            state_json: &state_json,
            now,
        },
    )
    .is_err()
    {
        return store_unavailable();
    }
    Json(json!({
        "challenge_id": challenge_id,
        "options": passkey::discoverable_options(&options, true),
    }))
    .into_response()
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AddFinishBody {
    challenge_id: String,
    credential: RegisterPublicKeyCredential,
    label: Option<String>,
}

/// `POST /staff/passkeys/add/finish` — verify and store the new
/// passkey for the same signed-in staffer the ceremony started for.
pub async fn add_finish(
    auth: AdminAuth,
    State(state): State<AppState>,
    Json(body): Json<AddFinishBody>,
) -> Response {
    if body.label.as_deref().is_some_and(|label| label.len() > 64) {
        return bad_request("label too long (max 64)");
    }
    let Some(conn) = passkey::open_conn(&state) else {
        return store_unavailable();
    };
    let now = unix_now();
    let Some((Some(owner), state_json)) =
        store::take_challenge(&conn, &body.challenge_id, store::KIND_STAFF_ADD, now)
            .unwrap_or(None)
    else {
        return (
            StatusCode::GONE,
            Json(json!({"error": "challenge invalid or expired"})),
        )
            .into_response();
    };
    if owner != auth.user.user_id {
        return (
            StatusCode::GONE,
            Json(json!({"error": "challenge invalid or expired"})),
        )
            .into_response();
    }
    let Ok(reg_state) = serde_json::from_str::<PasskeyRegistration>(&state_json) else {
        return store_unavailable();
    };
    let Ok(result) = state
        .webauthn
        .finish_passkey_registration(&body.credential, &reg_state)
    else {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"error": "verification failed"})),
        )
            .into_response();
    };
    let credential_id = passkey::cred_id_string(result.cred_id());
    if store::credential_known(&conn, &credential_id).unwrap_or(true) {
        return conflict("credential already registered");
    }
    let Ok(passkey_json) = serde_json::to_string(&result) else {
        return store_unavailable();
    };
    match store::store_credential_capped(
        &conn,
        &owner,
        &credential_id,
        &passkey_json,
        body.label.as_deref().unwrap_or(""),
        now,
        store::MAX_CREDENTIALS_PER_ACCOUNT,
    ) {
        Ok(()) => {}
        Err(store::StoreCredentialError::CapReached) => {
            return conflict("passkey limit reached (5): remove one first");
        }
        Err(store::StoreCredentialError::Db(_)) => return store_unavailable(),
    }
    passkey::audit_auth(
        &conn,
        now,
        &owner,
        "staff_passkey_added",
        &auth::hash_str(&credential_id),
    );
    Json(json!({"ok": true, "id": credential_id})).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{HeaderValue, Request, header};
    use tower::ServiceExt;

    const BOGUS_REGISTER: &str = r#"{"id":"AA","rawId":"AA","response":{"attestationObject":"AA","clientDataJSON":"AA"},"type":"public-key"}"#;
    const BOGUS_LOGIN: &str = r#"{"id":"AA","rawId":"AA","response":{"authenticatorData":"AA","clientDataJSON":"AA","signature":"AA"},"type":"public-key"}"#;

    /// 64-hex invite token (valid shape; caller seeds its hash).
    const INVITE: &str = "ab00000000000000000000000000000000000000000000000000000000000000";
    const INVITE_B: &str = "cd00000000000000000000000000000000000000000000000000000000000000";
    const FP_JDOE: &str = "aa00000000000000000000000000000000000000000000000000000000000000";
    const FP_OTHER: &str = "bb00000000000000000000000000000000000000000000000000000000000000";

    fn test_state(tmp: &tempfile::TempDir) -> AppState {
        let store = handbook_store::store::Store::new(tmp.path());
        let conn = handbook_store::db::open(&store).unwrap();
        handbook_store::db::migrate(&conn).unwrap();
        auth::migrate_auth_tables(&conn).unwrap();
        config::migrate_config_tables(&conn).unwrap();
        drop(conn);
        let dir = tmp.path().to_str().unwrap().to_string();
        AppState {
            spa_index: std::path::Path::new(&dir).join("index.html"),
            static_dir: std::path::PathBuf::from(&dir),
            store_root: tmp.path().to_path_buf(),
            bus: crate::ws_bus::Bus::new(),
            kek: Some(std::sync::Arc::new(config::Kek::generate())),
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
            meta_cfg: None,
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
        }
    }

    fn open(store_root: &std::path::Path) -> rusqlite::Connection {
        handbook_store::db::open(&handbook_store::store::Store::new(store_root)).unwrap()
    }

    fn seed_cert(conn: &rusqlite::Connection, fp: &str, cn: &str, now: i64, exp: i64) {
        auth::issue_admin_cert(conn, fp, cn, now, exp, "").unwrap();
    }

    fn seed_invite(conn: &rusqlite::Connection, token: &str, role: &str, cn: &str, now: i64) {
        store::store_invite(
            conn,
            &auth::login::token_hash(token),
            &store::NewInvite {
                role,
                cn,
                created_by: "tester",
                now,
                ttl_secs: store::INVITE_TTL_SECS,
            },
        )
        .unwrap();
    }

    fn addr() -> ConnectInfo<SocketAddr> {
        ConnectInfo("127.0.0.1:9".parse().unwrap())
    }

    /// Admin-branch POST: localhost host (dev guard), Origin, JSON,
    /// optional cert header + cookies.
    fn post(uri: &str, body: &str) -> Request<Body> {
        Request::builder()
            .method("POST")
            .uri(uri)
            .header(header::HOST, "localhost")
            .header(header::ORIGIN, "http://localhost")
            .header(header::CONTENT_TYPE, "application/json")
            .extension(addr())
            .body(Body::from(body.to_string()))
            .unwrap()
    }

    fn with_cert(mut req: Request<Body>, fp: &str) -> Request<Body> {
        req.headers_mut().insert(
            crate::admin::DEV_CERT_HEADER,
            HeaderValue::from_str(fp).unwrap(),
        );
        req
    }

    fn with_cookies(mut req: Request<Body>, cookies: &str) -> Request<Body> {
        if !cookies.is_empty() {
            req.headers_mut()
                .insert(header::COOKIE, HeaderValue::from_str(cookies).unwrap());
        }
        req
    }

    fn with_csrf(req: Request<Body>, token: &str) -> Request<Body> {
        with_cookies(req, &format!("{}={token}", auth::CSRF_COOKIE)).tap(|req| {
            req.headers_mut()
                .insert(auth::CSRF_HEADER, HeaderValue::from_str(token).unwrap());
        })
    }

    trait Tap: Sized {
        fn tap(self, f: impl FnOnce(&mut Self)) -> Self {
            let mut this = self;
            f(&mut this);
            this
        }
    }
    impl Tap for Request<Body> {}

    async fn json_of(res: Response) -> (StatusCode, serde_json::Value, String) {
        let status = res.status();
        let cookies = res
            .headers()
            .get_all(header::SET_COOKIE)
            .iter()
            .filter_map(|v| v.to_str().ok())
            .collect::<Vec<_>>()
            .join("; ");
        let body = axum::body::to_bytes(res.into_body(), 65536).await.unwrap();
        let json = serde_json::from_slice(&body).unwrap_or(serde_json::Value::Null);
        (status, json, cookies)
    }

    fn csrf_of(cookies: &str) -> String {
        auth::cookie_value(cookies, auth::CSRF_COOKIE)
            .unwrap_or("")
            .to_string()
    }

    #[tokio::test]
    async fn enrol_start_guards() {
        let tmp = tempfile::tempdir().unwrap();
        let state = test_state(&tmp);
        let app = crate::admin::router(state.clone());
        // Evil origin → 403 (handler-level, through the middleware
        // exemption — same shape as customer).
        let res = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/apostille/api/v1/auth/passkey/enrol/start")
                    .header(header::HOST, "localhost")
                    .header(header::ORIGIN, "https://evil.test")
                    .header(header::CONTENT_TYPE, "application/json")
                    .extension(addr())
                    .body(Body::from(r#"{"invite":"x","display_name":"y"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::FORBIDDEN);
        // Non-admin host → 403 from the host guard.
        let res = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/apostille/api/v1/auth/passkey/enrol/start")
                    .header(header::HOST, "andrii.ai")
                    .header(header::ORIGIN, "https://andrii.ai")
                    .header(header::CONTENT_TYPE, "application/json")
                    .extension(addr())
                    .body(Body::from(r#"{"invite":"x","display_name":"y"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::FORBIDDEN);
        // Malformed invite → 400 (shape, not liveness).
        let (status, body, _) = json_of(
            app.clone()
                .oneshot(post(
                    "/apostille/api/v1/auth/passkey/enrol/start",
                    r#"{"invite":"short","display_name":"J"}"#,
                ))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error"], "invite malformed");
        // Unknown invite → 410.
        let (status, _, _) = json_of(
            app.clone()
                .oneshot(post(
                    "/apostille/api/v1/auth/passkey/enrol/start",
                    &format!(r#"{{"invite":"{INVITE}","display_name":"J"}}"#),
                ))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::GONE);
        // Unknown fields rejected.
        let (status, _, _) = json_of(
            app.clone()
                .oneshot(post(
                    "/apostille/api/v1/auth/passkey/enrol/start",
                    &format!(r#"{{"invite":"{INVITE}","display_name":"J","role":"admin"}}"#),
                ))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[tokio::test]
    async fn enrol_start_cert_optional() {
        let tmp = tempfile::tempdir().unwrap();
        let state = test_state(&tmp);
        let now = unix_now();
        let conn = open(&state.store_root);
        seed_cert(&conn, FP_JDOE, "jdoe", now, now + 86_400);
        seed_cert(&conn, FP_OTHER, "other", now, now + 86_400);
        seed_invite(&conn, INVITE, "staff", "jdoe", now);
        drop(conn);
        let app = crate::admin::router(state.clone());
        let body = format!(r#"{{"invite":"{INVITE}","display_name":"J"}}"#);
        let uri = "/apostille/api/v1/auth/passkey/enrol/start";
        // No cert → starts anyway (the IP gate covers the route).
        let (status, json, _) = json_of(app.clone().oneshot(post(uri, &body)).await.unwrap()).await;
        assert_eq!(status, StatusCode::OK);
        assert!(!json["challenge_id"].as_str().unwrap().is_empty());
        // Wrong-CN cert → 403 mismatch (presented certs still bind).
        let (status, json, _) = json_of(
            app.clone()
                .oneshot(with_cert(post(uri, &body), FP_OTHER))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(json["error"], "certificate does not match invitation");
        // Revoked cert is ignored (not live) → starts anyway.
        let conn = open(&state.store_root);
        auth::revoke_admin_cert(&conn, FP_JDOE, now).unwrap();
        drop(conn);
        let (status, _, _) = json_of(
            app.clone()
                .oneshot(with_cert(post(uri, &body), FP_JDOE))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
    }

    #[tokio::test]
    async fn enrol_start_happy_and_validation() {
        let tmp = tempfile::tempdir().unwrap();
        let state = test_state(&tmp);
        let now = unix_now();
        let conn = open(&state.store_root);
        seed_cert(&conn, FP_JDOE, "jdoe", now, now + 86_400);
        seed_invite(&conn, INVITE, "staff", "jdoe", now);
        drop(conn);
        let app = crate::admin::router(state.clone());
        let uri = "/apostille/api/v1/auth/passkey/enrol/start";
        // Empty display_name / long label → 400.
        for body in [
            format!(r#"{{"invite":"{INVITE}","display_name":""}}"#),
            format!(
                r#"{{"invite":"{INVITE}","display_name":"J","label":"{}"}}"#,
                "x".repeat(65)
            ),
        ] {
            let (status, _, _) = json_of(
                app.clone()
                    .oneshot(with_cert(post(uri, &body), FP_JDOE))
                    .await
                    .unwrap(),
            )
            .await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        }
        // Happy: challenge + options + grant echo + CSRF cookie.
        let (status, json, cookies) = json_of(
            app.clone()
                .oneshot(with_cert(
                    post(
                        uri,
                        &format!(r#"{{"invite":"{INVITE}","display_name":"J"}}"#),
                    ),
                    FP_JDOE,
                ))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(!json["challenge_id"].as_str().unwrap().is_empty());
        assert!(json["options"].is_object());
        assert_eq!(json["role"], "staff");
        assert_eq!(json["cn"], "jdoe");
        assert_eq!(
            json["fp_suffix"].as_str().unwrap(),
            &FP_JDOE[FP_JDOE.len() - 8..]
        );
        assert!(!csrf_of(&cookies).is_empty());
        // Row stored under the staff kind with the invite stashed.
        let conn = open(&state.store_root);
        let (kind, state_json): (String, String) = conn
            .query_row(
                "SELECT kind, state_json FROM webauthn_challenge WHERE id = ?1",
                [json["challenge_id"].as_str().unwrap()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(kind, store::KIND_STAFF_REGISTER);
        let stashed: serde_json::Value = serde_json::from_str(&state_json).unwrap();
        assert_eq!(stashed["invite_hash"], auth::login::token_hash(INVITE));
    }

    #[tokio::test]
    async fn enrol_finish_guards_and_no_burn() {
        let tmp = tempfile::tempdir().unwrap();
        let state = test_state(&tmp);
        let now = unix_now();
        let conn = open(&state.store_root);
        seed_cert(&conn, FP_JDOE, "jdoe", now, now + 86_400);
        seed_cert(&conn, FP_OTHER, "other", now, now + 86_400);
        seed_invite(&conn, INVITE, "staff", "jdoe", now);
        seed_invite(&conn, INVITE_B, "staff", "jdoe", now);
        drop(conn);
        let app = crate::admin::router(state.clone());
        let start_uri = "/apostille/api/v1/auth/passkey/enrol/start";
        let finish_uri = "/apostille/api/v1/auth/passkey/enrol/finish";
        // Fresh ceremony per leg (each finish consumes its row).
        let start = || {
            let app = app.clone();
            async move {
                let (_, json, cookies) = json_of(
                    app.clone()
                        .oneshot(with_cert(
                            post(
                                start_uri,
                                &format!(r#"{{"invite":"{INVITE}","display_name":"J"}}"#),
                            ),
                            FP_JDOE,
                        ))
                        .await
                        .unwrap(),
                )
                .await;
                (
                    json["challenge_id"].as_str().unwrap().to_string(),
                    csrf_of(&cookies),
                )
            }
        };
        let (challenge_id, csrf) = start().await;
        // No CSRF echo → 403 (handler-level enforcement through
        // the middleware exemption).
        let (status, _, _) = json_of(
            app.clone()
                .oneshot(with_cert(
                    post(
                        finish_uri,
                        &format!(
                            r#"{{"challenge_id":"{challenge_id}","credential":{BOGUS_REGISTER},"invite":"{INVITE}"}}"#
                        ),
                    ),
                    FP_JDOE,
                ))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        // Unknown challenge → 410.
        let (status, _, _) = json_of(
            app.clone()
                .oneshot(with_csrf(
                    with_cert(
                        post(
                            finish_uri,
                            &format!(
                                r#"{{"challenge_id":"nope","credential":{BOGUS_REGISTER},"invite":"{INVITE}"}}"#
                            ),
                        ),
                        FP_JDOE,
                    ),
                    &csrf,
                ))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::GONE);
        // Invite mismatch (stashed A, echoed B) → 410, nothing
        // consumed.
        let (challenge_id, csrf) = start().await;
        let (status, _, _) = json_of(
            app.clone()
                .oneshot(with_csrf(
                    with_cert(
                        post(
                            finish_uri,
                            &format!(
                                r#"{{"challenge_id":"{challenge_id}","credential":{BOGUS_REGISTER},"invite":"{INVITE_B}"}}"#
                            ),
                        ),
                        FP_JDOE,
                    ),
                    &csrf,
                ))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::GONE);
        // Wrong-CN cert at finish → 403, invite NOT burned.
        let (challenge_id, csrf) = start().await;
        let (status, _, _) = json_of(
            app.clone()
                .oneshot(with_csrf(
                    with_cert(
                        post(
                            finish_uri,
                            &format!(
                                r#"{{"challenge_id":"{challenge_id}","credential":{BOGUS_REGISTER},"invite":"{INVITE}"}}"#
                            ),
                        ),
                        FP_OTHER,
                    ),
                    &csrf,
                ))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        let conn = open(&state.store_root);
        assert!(
            store::get_invite(&conn, &auth::login::token_hash(INVITE), unix_now())
                .unwrap()
                .is_some(),
            "CN mismatch must not burn the invite"
        );
    }

    #[tokio::test]
    async fn enrol_finish_revoked_between_start_and_finish() {
        let tmp = tempfile::tempdir().unwrap();
        let state = test_state(&tmp);
        let now = unix_now();
        let conn = open(&state.store_root);
        seed_cert(&conn, FP_JDOE, "jdoe", now, now + 86_400);
        seed_invite(&conn, INVITE, "staff", "jdoe", now);
        drop(conn);
        let app = crate::admin::router(state.clone());
        let (_, start_json, start_cookies) = json_of(
            app.clone()
                .oneshot(with_cert(
                    post(
                        "/apostille/api/v1/auth/passkey/enrol/start",
                        &format!(r#"{{"invite":"{INVITE}","display_name":"J"}}"#),
                    ),
                    FP_JDOE,
                ))
                .await
                .unwrap(),
        )
        .await;
        let challenge_id = start_json["challenge_id"].as_str().unwrap().to_string();
        let csrf = csrf_of(&start_cookies);
        // Revoke between start and finish: the revoked cert is
        // ignored (not live), so finish proceeds past the gate —
        // never 403 — and the bogus credential fails the ceremony.
        let conn = open(&state.store_root);
        auth::revoke_admin_cert(&conn, FP_JDOE, now).unwrap();
        drop(conn);
        let (status, _, _) = json_of(
            app.clone()
                .oneshot(with_csrf(
                    with_cert(
                        post(
                            "/apostille/api/v1/auth/passkey/enrol/finish",
                            &format!(
                                r#"{{"challenge_id":"{challenge_id}","credential":{BOGUS_REGISTER},"invite":"{INVITE}"}}"#
                            ),
                        ),
                        FP_JDOE,
                    ),
                    &csrf,
                ))
                .await
                .unwrap(),
        )
        .await;
        assert_ne!(status, StatusCode::FORBIDDEN);
        // Invite burns on a past-the-gate attempt (consume precedes
        // ceremony verify; a fresh invite is the recovery).
        let conn = open(&state.store_root);
        assert!(
            store::get_invite(&conn, &auth::login::token_hash(INVITE), unix_now())
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn enrol_finish_race_burns_once() {
        let tmp = tempfile::tempdir().unwrap();
        let state = test_state(&tmp);
        let now = unix_now();
        let conn = open(&state.store_root);
        seed_cert(&conn, FP_JDOE, "jdoe", now, now + 86_400);
        seed_invite(&conn, INVITE, "staff", "jdoe", now);
        drop(conn);
        let app = crate::admin::router(state.clone());
        let start_uri = "/apostille/api/v1/auth/passkey/enrol/start";
        let finish_uri = "/apostille/api/v1/auth/passkey/enrol/finish";
        let mut challenges = Vec::new();
        let mut csrf = String::new();
        for _ in 0..2 {
            let (_, json, cookies) = json_of(
                app.clone()
                    .oneshot(with_cert(
                        post(
                            start_uri,
                            &format!(r#"{{"invite":"{INVITE}","display_name":"J"}}"#),
                        ),
                        FP_JDOE,
                    ))
                    .await
                    .unwrap(),
            )
            .await;
            challenges.push(json["challenge_id"].as_str().unwrap().to_string());
            csrf = csrf_of(&cookies);
        }
        // First finish: consume wins, then the bogus ceremony
        // fails closed (422) — no variants leak.
        let (status, json, _) = json_of(
            app.clone()
                .oneshot(with_csrf(
                    with_cert(
                        post(
                            finish_uri,
                            &format!(
                                r#"{{"challenge_id":"{}","credential":{BOGUS_REGISTER},"invite":"{INVITE}"}}"#,
                                challenges[0]
                            ),
                        ),
                        FP_JDOE,
                    ),
                    &csrf,
                ))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(json["error"], "verification failed");
        // Second finish: same invite already consumed → 410.
        let (status, _, _) = json_of(
            app.clone()
                .oneshot(with_csrf(
                    with_cert(
                        post(
                            finish_uri,
                            &format!(
                                r#"{{"challenge_id":"{}","credential":{BOGUS_REGISTER},"invite":"{INVITE}"}}"#,
                                challenges[1]
                            ),
                        ),
                        FP_JDOE,
                    ),
                    &csrf,
                ))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::GONE);
    }

    #[tokio::test]
    async fn challenge_kinds_isolate_branches() {
        let tmp = tempfile::tempdir().unwrap();
        let state = test_state(&tmp);
        let now = unix_now();
        let conn = open(&state.store_root);
        seed_cert(&conn, FP_JDOE, "jdoe", now, now + 86_400);
        seed_invite(&conn, INVITE, "staff", "jdoe", now);
        drop(conn);
        let admin_app = crate::admin::router(state.clone());
        let dir = tmp.path().to_str().unwrap().to_string();
        let public_app = crate::public_router(state.clone(), &dir, &dir, &dir);
        // Staff challenge → customer finish: 410 (kind boundary).
        let (_, start_json, _) = json_of(
            admin_app
                .clone()
                .oneshot(with_cert(
                    post(
                        "/apostille/api/v1/auth/passkey/enrol/start",
                        &format!(r#"{{"invite":"{INVITE}","display_name":"J"}}"#),
                    ),
                    FP_JDOE,
                ))
                .await
                .unwrap(),
        )
        .await;
        let staff_challenge = start_json["challenge_id"].as_str().unwrap().to_string();
        // Harvest a CSRF pair from a dummy customer start (the
        // cookie is not bound to the challenge).
        let dummy = public_app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/apostille/api/v1/auth/passkey/register/start")
                    .header(header::ORIGIN, "http://localhost")
                    .header(header::CONTENT_TYPE, "application/json")
                    .extension(addr())
                    .body(Body::from(r#"{"display_name":"D"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        let (_, _, dummy_cookies) = json_of(dummy).await;
        let dummy_csrf = csrf_of(&dummy_cookies);
        let res = public_app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/apostille/api/v1/auth/passkey/register/finish")
                    .header(header::ORIGIN, "http://localhost")
                    .header(header::CONTENT_TYPE, "application/json")
                    .header(
                        header::COOKIE,
                        format!("{}={dummy_csrf}", auth::CSRF_COOKIE),
                    )
                    .header(auth::CSRF_HEADER, &dummy_csrf)
                    .extension(addr())
                    .body(Body::from(format!(
                        r#"{{"challenge_id":"{staff_challenge}","credential":{BOGUS_REGISTER}}}"#
                    )))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::GONE);
        // Customer challenge → staff finish: 410 (and the staff
        // row survives — wrong-kind take consumes nothing).
        let res = public_app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/apostille/api/v1/auth/passkey/register/start")
                    .header(header::ORIGIN, "http://localhost")
                    .header(header::CONTENT_TYPE, "application/json")
                    .extension(addr())
                    .body(Body::from(r#"{"display_name":"C"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        let (_, customer_json, customer_cookies) = json_of(res).await;
        let customer_challenge = customer_json["challenge_id"].as_str().unwrap().to_string();
        let (status, _, _) = json_of(
            admin_app
                .clone()
                .oneshot(with_csrf(
                    with_cert(
                        post(
                            "/apostille/api/v1/auth/passkey/enrol/finish",
                            &format!(
                                r#"{{"challenge_id":"{customer_challenge}","credential":{BOGUS_REGISTER},"invite":"{INVITE}"}}"#
                            ),
                        ),
                        FP_JDOE,
                    ),
                    &csrf_of(&customer_cookies),
                ))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::GONE);
        // Both rows still takeable under their own kinds.
        let conn = open(&state.store_root);
        assert!(
            store::take_challenge(
                &conn,
                &staff_challenge,
                store::KIND_STAFF_REGISTER,
                unix_now()
            )
            .unwrap()
            .is_some()
        );
        assert!(
            store::take_challenge(&conn, &customer_challenge, store::KIND_REGISTER, unix_now())
                .unwrap()
                .is_some()
        );
    }

    #[tokio::test]
    async fn login_start_unknowns_share_one_shape() {
        let tmp = tempfile::tempdir().unwrap();
        let state = test_state(&tmp);
        let now = unix_now();
        let conn = open(&state.store_root);
        store::ensure_staff_account(&conn, "u-nocred", "staff", now).unwrap();
        store::ensure_staff_account(&conn, "u-locked", "staff", now).unwrap();
        store::lock_account(&conn, "u-locked", now).unwrap();
        drop(conn);
        let app = crate::admin::router(state.clone());
        let uri = "/apostille/api/v1/auth/passkey/login/start";
        // Unknown id, email arm, locked, cred-less: one shape, no
        // oracle (all 401 with the same body).
        for account in [
            "0123456789abcdef0123456789abcdef",
            "someone@example.test",
            "u-locked",
            "u-nocred",
        ] {
            let (status, json, _) = json_of(
                app.clone()
                    .oneshot(post(uri, &format!(r#"{{"account":"{account}"}}"#)))
                    .await
                    .unwrap(),
            )
            .await;
            assert_eq!(status, StatusCode::UNAUTHORIZED, "{account}");
            assert_eq!(json["error"], "unknown account or no passkeys", "{account}");
        }
    }

    #[tokio::test]
    async fn login_finish_guards() {
        let tmp = tempfile::tempdir().unwrap();
        let state = test_state(&tmp);
        let now = unix_now();
        let conn = open(&state.store_root);
        seed_cert(&conn, FP_JDOE, "jdoe", now, now + 86_400);
        store::ensure_staff_account(&conn, "u-s", "staff", now).unwrap();
        // Seed a staff-login challenge directly (no authenticator
        // in unit tests; the 403/404 guards precede verification).
        let _ = store::sweep_challenges(&conn, now);
        store::store_challenge(
            &conn,
            "staff-challenge-1",
            &store::NewChallenge {
                kind: store::KIND_STAFF_LOGIN,
                user_id: Some("u-s"),
                state_json: "{}",
                now,
            },
        )
        .unwrap();
        drop(conn);
        let app = crate::admin::router(state.clone());
        let uri = "/apostille/api/v1/auth/passkey/login/finish";
        // Unknown challenge → 410.
        let (status, _, _) = json_of(
            app.clone()
                .oneshot(with_csrf(
                    with_cert(
                        post(
                            uri,
                            &format!(r#"{{"challenge_id":"nope","credential":{BOGUS_LOGIN}}}"#),
                        ),
                        FP_JDOE,
                    ),
                    "csrf-token",
                ))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::GONE);
        // No cert → proceeds past the gate (certs decide nothing);
        // unknown credential id → 404 re-register. The attempt
        // consumed challenge-1 (take runs first), so re-seed below.
        let (status, _, _) = json_of(
            app.clone()
                .oneshot(with_csrf(
                    post(
                        uri,
                        &format!(
                            r#"{{"challenge_id":"staff-challenge-1","credential":{BOGUS_LOGIN}}}"#
                        ),
                    ),
                    "csrf-token",
                ))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        // Unknown credential id → 404 re-register (state_json
        // never parses — the 404 precedes verification). Re-seed
        // for this leg.
        let conn = open(&state.store_root);
        store::store_challenge(
            &conn,
            "staff-challenge-2",
            &store::NewChallenge {
                kind: store::KIND_STAFF_LOGIN,
                user_id: Some("u-s"),
                state_json: "{}",
                now,
            },
        )
        .unwrap();
        drop(conn);
        let (status, json, _) = json_of(
            app.clone()
                .oneshot(with_csrf(
                    with_cert(
                        post(
                            uri,
                            &format!(
                                r#"{{"challenge_id":"staff-challenge-2","credential":{BOGUS_LOGIN}}}"#
                            ),
                        ),
                        FP_JDOE,
                    ),
                    "csrf-token",
                ))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(json["action"], "re-register");
    }

    #[tokio::test]
    async fn set_config_step_up_pilot() {
        let tmp = tempfile::tempdir().unwrap();
        let state = test_state(&tmp);
        let now = unix_now();
        let conn = open(&state.store_root);
        store::ensure_staff_account(&conn, "u-fresh", "admin", now).unwrap();
        store::ensure_staff_account(&conn, "u-stale", "admin", now).unwrap();
        seed_cert(&conn, FP_JDOE, "jdoe", now, now + 86_400);
        // Fresh session (minted now): passes the gate.
        auth::create_session(
            &conn,
            "sess-fresh",
            &auth::NewSession {
                user_id: "u-fresh",
                role: auth::Role::Admin,
                customer_ref: None,
                now,
                ip: "127.0.0.1",
                user_agent: "test",
            },
        )
        .unwrap();
        // Stale session (minted 700s ago, no step-up): refused.
        auth::create_session(
            &conn,
            "sess-stale",
            &auth::NewSession {
                user_id: "u-stale",
                role: auth::Role::Admin,
                customer_ref: None,
                now: now - 700,
                ip: "127.0.0.1",
                user_agent: "test",
            },
        )
        .unwrap();
        drop(conn);
        let app = crate::admin::router(state.clone());
        let put = |session: &str| {
            Request::builder()
                .method("PUT")
                .uri("/apostille/api/v1/config/net.bind")
                .header(header::HOST, "localhost")
                .header(header::ORIGIN, "http://localhost")
                .header(header::CONTENT_TYPE, "application/json")
                .header(crate::admin::DEV_CERT_HEADER, FP_JDOE)
                .header(
                    header::COOKIE,
                    format!("{}={session}; {}=tok", auth::ADM_COOKIE, auth::CSRF_COOKIE),
                )
                .header(auth::CSRF_HEADER, "tok")
                .extension(addr())
                .body(Body::from(r#"{"type":"text","value":"0.0.0.0"}"#))
                .unwrap()
        };
        // Stale → 403 with the exact step-up string.
        let (status, json, _) =
            json_of(app.clone().oneshot(put("sess-stale")).await.unwrap()).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(json["error"], "step-up required: log in again first");
        // Machine-readable for the in-place passkey confirm (P409).
        assert_eq!(json["step_up"], "required");
        assert_eq!(json["window_s"], auth::passkey::STEP_UP_WINDOW_SECS);
        // Fresh → sails past the gate (409 here: KEK generation
        // never enrolled on this scratch store).
        let (status, _, _) = json_of(app.clone().oneshot(put("sess-fresh")).await.unwrap()).await;
        assert_eq!(status, StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn logout_ends_the_staff_session_and_clears_cookies() {
        let tmp = tempfile::tempdir().unwrap();
        let state = test_state(&tmp);
        let now = unix_now();
        let conn = open(&state.store_root);
        store::ensure_staff_account(&conn, "u-out", "admin", now).unwrap();
        auth::create_session(
            &conn,
            "sess-out",
            &auth::NewSession {
                user_id: "u-out",
                role: auth::Role::Admin,
                customer_ref: None,
                now,
                ip: "127.0.0.1",
                user_agent: "test",
            },
        )
        .unwrap();
        drop(conn);
        let app = crate::admin::router(state.clone());
        let me = |session: &str| {
            Request::builder()
                .uri("/apostille/api/v1/me")
                .header(header::HOST, "localhost")
                .header(header::COOKIE, format!("{}={session}", auth::ADM_COOKIE))
                .extension(addr())
                .body(Body::empty())
                .unwrap()
        };
        assert_eq!(
            app.clone().oneshot(me("sess-out")).await.unwrap().status(),
            StatusCode::OK
        );
        // Cross-site page → refused, session intact.
        let evil = Request::builder()
            .method("POST")
            .uri("/apostille/api/v1/auth/logout")
            .header(header::HOST, "localhost")
            .header(header::ORIGIN, "https://evil.test")
            .header(header::COOKIE, format!("{}=sess-out", auth::ADM_COOKIE))
            .extension(addr())
            .body(Body::empty())
            .unwrap();
        assert_eq!(
            app.clone().oneshot(evil).await.unwrap().status(),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            app.clone().oneshot(me("sess-out")).await.unwrap().status(),
            StatusCode::OK
        );
        // Same-origin sign-out: 200, both cookies cleared (`Secure`
        // even on the plain-HTTP boot, or the browser keeps them).
        let req = with_cookies(
            post("/apostille/api/v1/auth/logout", "{}"),
            &format!("{}=sess-out", auth::ADM_COOKIE),
        );
        let (status, json, cookies) = json_of(app.clone().oneshot(req).await.unwrap()).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["ok"], true);
        assert!(
            cookies.contains(&format!("{}=; Max-Age=0", auth::ADM_COOKIE)),
            "{cookies}"
        );
        assert!(cookies.contains("Secure"), "{cookies}");
        assert!(
            cookies.contains(&format!("{}=; Max-Age=0", auth::CSRF_COOKIE)),
            "{cookies}"
        );
        assert_eq!(
            app.clone().oneshot(me("sess-out")).await.unwrap().status(),
            StatusCode::UNAUTHORIZED
        );
        // Idempotent: no session, still 200.
        let (status, _, _) = json_of(
            app.clone()
                .oneshot(post("/apostille/api/v1/auth/logout", "{}"))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let conn = open(&state.store_root);
        let audited: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM audit WHERE action = 'staff_logout'",
                [],
                |r| r.get(0),
            )
            .unwrap_or(-1);
        assert_eq!(audited, 1);
    }

    /// One `webauthn_challenge` row: (kind, user_id).
    fn challenge_row(store_root: &std::path::Path, id: &str) -> (String, Option<String>) {
        open(store_root)
            .query_row(
                "SELECT kind, user_id FROM webauthn_challenge WHERE id = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap()
    }

    #[tokio::test]
    async fn discoverable_login_needs_no_user_id() {
        let tmp = tempfile::tempdir().unwrap();
        let state = test_state(&tmp);
        let app = crate::admin::router(state.clone());
        let uri = "/apostille/api/v1/auth/passkey/login/start";
        for body in ["{}", r#"{"account":""}"#, r#"{"account":"  "}"#] {
            let (status, json, cookies) =
                json_of(app.clone().oneshot(post(uri, body)).await.unwrap()).await;
            assert_eq!(status, StatusCode::OK, "{body}");
            let allow = &json["options"]["publicKey"]["allowCredentials"];
            assert!(
                allow.is_null() || allow.as_array().is_some_and(Vec::is_empty),
                "no allow-list: {allow}"
            );
            assert_eq!(json["options"]["publicKey"]["userVerification"], "required");
            let challenge = json["challenge_id"].as_str().unwrap().to_string();
            assert_eq!(
                challenge_row(&state.store_root, &challenge),
                (store::KIND_STAFF_DISCOVERABLE.to_string(), None)
            );
            // A forged assertion (no user handle) fails closed as a
            // failed check, and burns the challenge.
            let csrf = csrf_of(&cookies);
            let finish = |id: &str| {
                with_csrf(
                    post(
                        "/apostille/api/v1/auth/passkey/login/finish",
                        &format!(r#"{{"challenge_id":"{id}","credential":{BOGUS_LOGIN}}}"#),
                    ),
                    &csrf,
                )
            };
            let (status, json, cookies) =
                json_of(app.clone().oneshot(finish(&challenge)).await.unwrap()).await;
            assert_eq!(status, StatusCode::UNAUTHORIZED, "{json}");
            assert_eq!(json["error"], "authentication failed");
            assert!(!cookies.contains(auth::ADM_COOKIE), "no session: {cookies}");
            let (status, _, _) =
                json_of(app.clone().oneshot(finish(&challenge)).await.unwrap()).await;
            assert_eq!(status, StatusCode::GONE, "single use");
        }
    }

    #[tokio::test]
    async fn customer_discoverable_challenge_never_finishes_as_staff() {
        let tmp = tempfile::tempdir().unwrap();
        let state = test_state(&tmp);
        let now = unix_now();
        let conn = open(&state.store_root);
        store::store_challenge(
            &conn,
            "cust-disc",
            &store::NewChallenge {
                kind: store::KIND_DISCOVERABLE,
                user_id: None,
                state_json: "{}",
                now,
            },
        )
        .unwrap();
        drop(conn);
        let app = crate::admin::router(state.clone());
        let req = with_csrf(
            post(
                "/apostille/api/v1/auth/passkey/login/finish",
                &format!(r#"{{"challenge_id":"cust-disc","credential":{BOGUS_LOGIN}}}"#),
            ),
            "tok",
        );
        let (status, _, _) = json_of(app.oneshot(req).await.unwrap()).await;
        assert_eq!(status, StatusCode::GONE);
    }

    #[tokio::test]
    async fn enrolment_asks_for_a_discoverable_passkey() {
        let tmp = tempfile::tempdir().unwrap();
        let state = test_state(&tmp);
        let now = unix_now();
        let conn = open(&state.store_root);
        seed_invite(&conn, INVITE, "staff", "jdoe", now);
        drop(conn);
        let app = crate::admin::router(state.clone());
        let (status, json, _) = json_of(
            app.oneshot(post(
                "/apostille/api/v1/auth/passkey/enrol/start",
                &format!(r#"{{"invite":"{INVITE}","display_name":"Jordan"}}"#),
            ))
            .await
            .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{json}");
        let selection = &json["options"]["publicKey"]["authenticatorSelection"];
        assert_eq!(selection["residentKey"], "required", "{selection}");
        assert_eq!(selection["requireResidentKey"], true);
        assert_eq!(selection["userVerification"], "required");
    }

    /// W-1: the email names the passkey, so the browser lists and
    /// autofills it instead of the hex id. Malformed or taken emails are
    /// refused, but only once the invite passed (no anonymous oracle).
    #[tokio::test]
    async fn enrolment_names_the_passkey_by_email() {
        let tmp = tempfile::tempdir().unwrap();
        let state = test_state(&tmp);
        let now = unix_now();
        let conn = open(&state.store_root);
        seed_invite(&conn, INVITE, "staff", "jdoe", now);
        store::ensure_staff_account(&conn, "u-taken", "staff", now).unwrap();
        store::set_staff_email(&conn, "u-taken", "taken@example.test", now).unwrap();
        drop(conn);
        let app = crate::admin::router(state.clone());
        let start = |body: String| {
            let app = app.clone();
            async move {
                json_of(
                    app.oneshot(post("/apostille/api/v1/auth/passkey/enrol/start", &body))
                        .await
                        .unwrap(),
                )
                .await
            }
        };
        let body = |invite: &str, email: &str| {
            format!(r#"{{"invite":"{invite}","display_name":"Jordan","email":"{email}"}}"#)
        };
        // A taken email behind a bad invite answers like any bad invite.
        let (status, json, _) = start(body(INVITE_B, "taken@example.test")).await;
        assert_eq!(status, StatusCode::GONE, "{json}");
        let (status, json, _) = start(body(INVITE, "not-an-email")).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(json["error"], "email malformed");
        let (status, json, _) = start(body(INVITE, "Taken@Example.test")).await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(json["error"], "email already used by another staff account");
        let (status, json, _) = start(body(INVITE, " Jordan@Example.TEST ")).await;
        assert_eq!(status, StatusCode::OK, "{json}");
        let user = &json["options"]["publicKey"]["user"];
        assert_eq!(user["name"], "jordan@example.test", "{user}");
        assert_eq!(user["displayName"], "Jordan");
        // The email rides in the server-side ceremony state (saved at
        // finish), never in a client echo.
        let challenge = json["challenge_id"].as_str().unwrap();
        let conn = open(&state.store_root);
        let (_, state_json) =
            store::take_challenge(&conn, challenge, store::KIND_STAFF_REGISTER, unix_now())
                .unwrap()
                .unwrap();
        let enrol: EnrolState = serde_json::from_str(&state_json).unwrap();
        assert_eq!(enrol.email, "jordan@example.test");
        drop(conn);
        // Older clients send no email: the hex id still names it.
        let (status, json, _) = start(format!(
            r#"{{"invite":"{INVITE}","display_name":"Jordan"}}"#
        ))
        .await;
        assert_eq!(status, StatusCode::OK, "{json}");
        let name = json["options"]["publicKey"]["user"]["name"]
            .as_str()
            .unwrap();
        assert!(
            name.len() == 32 && name.bytes().all(|b| b.is_ascii_hexdigit()),
            "{name}"
        );
    }

    /// A second passkey gets the account's email as its name too, and
    /// the Staff page shows the email (empty for older accounts).
    #[tokio::test]
    async fn added_passkeys_and_the_staff_page_use_the_email() {
        let tmp = tempfile::tempdir().unwrap();
        let state = test_state(&tmp);
        let conn = open(&state.store_root);
        let jo = store::mint_user_id();
        let old = store::mint_user_id();
        seed_staffer(&conn, &jo, "admin", &[], 0);
        seed_staffer(&conn, &old, "staff", &[], 0);
        store::set_staff_email(&conn, &jo, "jo@example.test", unix_now()).unwrap();
        drop(conn);
        let app = crate::admin::router(state.clone());
        let add = |user: &str| {
            as_staff(
                "POST",
                "/apostille/api/v1/staff/passkeys/add/start",
                user,
                "{}",
            )
        };
        let (status, json, _) = json_of(app.clone().oneshot(add(&jo)).await.unwrap()).await;
        assert_eq!(status, StatusCode::OK, "{json}");
        assert_eq!(
            json["options"]["publicKey"]["user"]["name"],
            "jo@example.test"
        );
        let (status, json, _) = json_of(app.clone().oneshot(add(&old)).await.unwrap()).await;
        assert_eq!(status, StatusCode::OK, "{json}");
        assert_eq!(json["options"]["publicKey"]["user"]["name"], old.as_str());
        let (status, json, _) = json_of(
            app.clone()
                .oneshot(as_staff("GET", "/apostille/api/v1/staff", &jo, ""))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{json}");
        let email_of = |id: &str| {
            json["accounts"]
                .as_array()
                .unwrap()
                .iter()
                .find(|a| a["user_id"] == id)
                .unwrap()["email"]
                .clone()
        };
        assert_eq!(email_of(&jo), "jo@example.test");
        assert_eq!(email_of(&old), "");
    }

    /// Seed a staff/admin account with live passkeys (labels only, no
    /// key material) and a session minted `age` seconds ago.
    fn seed_staffer(
        conn: &rusqlite::Connection,
        user_id: &str,
        role: &str,
        keys: &[&str],
        session_age: i64,
    ) {
        let now = unix_now();
        store::ensure_staff_account(conn, user_id, role, now - 1000).unwrap();
        for key in keys {
            store::store_credential(conn, user_id, key, "{}", key, now - 900).unwrap();
        }
        auth::create_session(
            conn,
            &format!("sess-{user_id}"),
            &auth::NewSession {
                user_id,
                role: auth::Role::parse(role).unwrap(),
                customer_ref: None,
                now: now - session_age,
                ip: "127.0.0.1",
                user_agent: "test",
            },
        )
        .unwrap();
    }

    /// Admin-branch request as `user_id`'s session, with the CSRF pair.
    fn as_staff(method: &str, uri: &str, user_id: &str, body: &str) -> Request<Body> {
        Request::builder()
            .method(method)
            .uri(uri)
            .header(header::HOST, "localhost")
            .header(header::ORIGIN, "http://localhost")
            .header(header::CONTENT_TYPE, "application/json")
            .header(
                header::COOKIE,
                format!(
                    "{}=sess-{user_id}; {}=tok",
                    auth::ADM_COOKIE,
                    auth::CSRF_COOKIE
                ),
            )
            .header(auth::CSRF_HEADER, "tok")
            .extension(addr())
            .body(if body.is_empty() {
                Body::empty()
            } else {
                Body::from(body.to_string())
            })
            .unwrap()
    }

    #[tokio::test]
    async fn staff_page_lists_invites_and_revokes() {
        let tmp = tempfile::tempdir().unwrap();
        let state = test_state(&tmp);
        let conn = open(&state.store_root);
        seed_staffer(&conn, "u-admin", "admin", &["admin-mac"], 0);
        seed_staffer(&conn, "u-staff", "staff", &["staff-a", "staff-b"], 0);
        seed_staffer(&conn, "u-stale", "admin", &["stale-key"], 700);
        store::ensure_account(&conn, "u-cust", unix_now()).unwrap();
        store::store_credential(&conn, "u-cust", "cust-key", "{}", "", unix_now()).unwrap();
        drop(conn);
        let app = crate::admin::router(state.clone());
        let call = |method: &str, uri: &str, user: &str, body: &str| {
            let req = as_staff(method, uri, user, body);
            let app = app.clone();
            async move { json_of(app.oneshot(req).await.unwrap()).await }
        };

        // List: staff accounts only, live passkeys, no key material.
        let (status, json, _) = call("GET", "/apostille/api/v1/staff", "u-admin", "").await;
        assert_eq!(status, StatusCode::OK, "{json}");
        assert_eq!(json["me"]["user_id"], "u-admin");
        let ids: Vec<_> = json["accounts"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| a["user_id"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(ids, ["u-admin", "u-staff", "u-stale"]);
        assert_eq!(json["accounts"][1]["passkeys"].as_array().unwrap().len(), 2);
        assert!(!json.to_string().contains("passkey_json"));

        // Invite: admin only, the URL from the request's origin.
        let invite = r#"{"role":"staff","cn":"jdoe","days":2}"#;
        let (status, json, _) =
            call("POST", "/apostille/api/v1/staff/invites", "u-staff", invite).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(json["error"], "admin role required");
        let (status, json, _) =
            call("POST", "/apostille/api/v1/staff/invites", "u-stale", invite).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(json["step_up"], "required", "stale step-up: {json}");
        let (status, json, _) = call(
            "POST",
            "/apostille/api/v1/staff/invites",
            "u-admin",
            r#"{"role":"root","cn":"jdoe"}"#,
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{json}");
        let (status, json, _) =
            call("POST", "/apostille/api/v1/staff/invites", "u-admin", invite).await;
        assert_eq!(status, StatusCode::OK, "{json}");
        let url = json["url"].as_str().unwrap();
        let token = url
            .strip_prefix("http://localhost/enrol?invite=")
            .unwrap_or_else(|| panic!("origin-built url: {url}"));
        assert_eq!(token.len(), 64);
        // Hash at rest only; the pending list shows it, never the token.
        let (_, json, _) = call("GET", "/apostille/api/v1/staff", "u-admin", "").await;
        let pending = json["invites"].as_array().unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0]["cn"], "jdoe");
        assert!(!json.to_string().contains(token));
        let (_, json_staff, _) = call("GET", "/apostille/api/v1/staff", "u-staff", "").await;
        assert_eq!(
            json_staff["invites"].as_array().unwrap().len(),
            0,
            "admins only"
        );
        let id = pending[0]["id"].as_str().unwrap().to_string();
        let withdraw = format!("/apostille/api/v1/staff/invites/{id}/withdraw");
        let (status, _, _) = call("POST", &withdraw, "u-admin", "{}").await;
        assert_eq!(status, StatusCode::OK);
        let (status, _, _) = call("POST", &withdraw, "u-admin", "{}").await;
        assert_eq!(status, StatusCode::NOT_FOUND);

        // Passkeys: your own, or anyone's as an admin; never the last one.
        let remove = |key: &str| format!(r#"{{"credential_id":"{key}"}}"#);
        let uri = "/apostille/api/v1/staff/passkeys/revoke";
        let (status, _, _) = call("POST", uri, "u-staff", &remove("admin-mac")).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "staff cannot touch others");
        let (status, _, _) = call("POST", uri, "u-admin", &remove("cust-key")).await;
        assert_eq!(
            status,
            StatusCode::NOT_FOUND,
            "customer keys are off limits"
        );
        let (status, _, _) = call("POST", uri, "u-staff", &remove("staff-a")).await;
        assert_eq!(status, StatusCode::OK);
        let (status, json, _) = call("POST", uri, "u-admin", &remove("staff-b")).await;
        assert_eq!(status, StatusCode::CONFLICT, "last passkey: {json}");

        // Accounts: never yourself, never the last live admin.
        let (status, _, _) = call(
            "POST",
            "/apostille/api/v1/staff/u-admin/revoke",
            "u-admin",
            "{}",
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT);
        let (status, _, _) = call(
            "POST",
            "/apostille/api/v1/staff/u-admin/revoke",
            "u-staff",
            "{}",
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        let (status, json, _) = call(
            "POST",
            "/apostille/api/v1/staff/u-staff/revoke",
            "u-admin",
            "{}",
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{json}");
        assert_eq!(json["sessions"], 1);
        assert_eq!(json["passkeys"], 1);
        let (status, _, _) = call("GET", "/apostille/api/v1/me", "u-staff", "").await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "revoked = signed out");
        let conn = open(&state.store_root);
        let account = store::get_account(&conn, "u-staff").unwrap().unwrap();
        assert!(account.locked_at.is_some());
        assert!(
            store::credentials_for_user(&conn, "u-staff")
                .unwrap()
                .is_empty()
        );
        // u-stale is an admin with a live passkey: with u-admin there
        // are two, so u-admin may revoke u-stale…
        let (status, _, _) = call(
            "POST",
            "/apostille/api/v1/staff/u-stale/revoke",
            "u-admin",
            "{}",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        // …and a fresh second admin cannot then revoke u-admin, the
        // last live one.
        seed_staffer(&conn, "u-new", "admin", &[], 0);
        drop(conn);
        let (status, json, _) = call(
            "POST",
            "/apostille/api/v1/staff/u-admin/revoke",
            "u-new",
            "{}",
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "{json}");
        assert_eq!(json["error"], "the last admin cannot be revoked");
        let conn = open(&state.store_root);
        let audited: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM audit WHERE action IN
                 ('staff_invite_issue', 'staff_invite_withdrawn', 'staff_passkey_revoked', 'staff_revoked')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(audited, 5);
    }

    #[tokio::test]
    async fn add_passkey_needs_step_up_and_the_same_staffer() {
        let tmp = tempfile::tempdir().unwrap();
        let state = test_state(&tmp);
        let conn = open(&state.store_root);
        // Real ids: the add ceremony uses the user id as the handle.
        let fresh = store::mint_user_id();
        let other = store::mint_user_id();
        seed_staffer(&conn, &fresh, "staff", &[], 0);
        seed_staffer(&conn, &other, "staff", &[], 700);
        drop(conn);
        let app = crate::admin::router(state.clone());
        let start = "/apostille/api/v1/staff/passkeys/add/start";
        let (status, json, _) = json_of(
            app.clone()
                .oneshot(as_staff("POST", start, &other, "{}"))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(json["step_up"], "required");
        let (status, json, _) = json_of(
            app.clone()
                .oneshot(as_staff("POST", start, &fresh, r#"{"label":"Phone"}"#))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{json}");
        assert_eq!(
            json["options"]["publicKey"]["authenticatorSelection"]["residentKey"],
            "required"
        );
        let challenge = json["challenge_id"].as_str().unwrap().to_string();
        assert_eq!(
            challenge_row(&state.store_root, &challenge),
            (store::KIND_STAFF_ADD.to_string(), Some(fresh.clone()))
        );
        let finish = |user: &str| {
            as_staff(
                "POST",
                "/apostille/api/v1/staff/passkeys/add/finish",
                user,
                &format!(r#"{{"challenge_id":"{challenge}","credential":{BOGUS_REGISTER}}}"#),
            )
        };
        // Another staffer cannot finish (and burns nothing of theirs).
        let (status, _, _) = json_of(app.clone().oneshot(finish(&other)).await.unwrap()).await;
        assert_eq!(status, StatusCode::GONE);
        // The challenge was consumed by that attempt: single use.
        let (status, _, _) = json_of(app.clone().oneshot(finish(&fresh)).await.unwrap()).await;
        assert_eq!(status, StatusCode::GONE);
    }
}
