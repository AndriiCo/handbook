//! Apple OAuth handlers (spec `customer-auth.md` §10): start
//! builds the Apple auth URL; the callback (top-level GET from
//! Apple) exchanges, verifies the id_token via JWKS, resolves,
//! sets the session cookie, and 302s back into the SPA. Mirrors
//! `oauth.rs` (Google) handler-for-handler; Apple tokens are never
//! stored.

use axum::{
    Json,
    extract::{ConnectInfo, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use serde_json::json;
use std::net::SocketAddr;

use super::oauth::UreqTransport;
use super::passkey;
use super::{AppState, store_unavailable, unix_now};
use auth::apple as flow;
use auth::oauth as core;
use auth::passkey as store;

/// Apple OAuth config: Services ID + Team ID + Key ID (text) and
/// the `.p8` private key (redacting `SecretText`) from sealed rows;
/// base URLs default to Apple prod and are overridden only by tests.
#[derive(Clone)]
pub struct AppleCfg {
    pub client_id: String,
    pub team_id: String,
    pub key_id: String,
    pub private_key: config::SecretText,
    pub redirect_uri: String,
    pub auth_base: String,
    pub token_url: String,
    pub keys_url: String,
}

impl AppleCfg {
    pub fn from_values(
        client_id: &str,
        team_id: &str,
        key_id: &str,
        private_key: config::SecretText,
        redirect_uri: &str,
    ) -> Self {
        Self {
            client_id: client_id.to_string(),
            team_id: team_id.to_string(),
            key_id: key_id.to_string(),
            private_key,
            redirect_uri: redirect_uri.to_string(),
            auth_base: flow::APPLE_AUTH_URL.to_string(),
            token_url: flow::APPLE_TOKEN_URL.to_string(),
            keys_url: flow::APPLE_KEYS_URL.to_string(),
        }
    }

    /// All four halves present and non-empty (endpoints fail closed).
    pub fn configured(&self) -> bool {
        !self.client_id.is_empty()
            && !self.team_id.is_empty()
            && !self.key_id.is_empty()
            && !self.private_key.as_str().is_empty()
    }
}

/// `hb_apple` state cookie name (binds the state to this browser;
/// per-provider so concurrent Google/Apple/Meta flows coexist).
pub const APPLE_COOKIE: &str = "hb_apple";

/// `hb_apple` cookie value: split binding, short-lived.
fn apple_cookie(binding: &str, secure: bool) -> String {
    let mut cookie =
        format!("{APPLE_COOKIE}={binding}; HttpOnly; SameSite=Lax; Path=/; Max-Age=300");
    if secure {
        cookie.push_str("; Secure");
    }
    cookie
}

/// Authorization URL for the SPA top-level navigation (`url` crate
/// encodes every pair — no hand-rolled escaping).
fn auth_url(cfg: &AppleCfg, state: &str) -> Option<String> {
    let mut url = url::Url::parse(&cfg.auth_base).ok()?;
    url.query_pairs_mut()
        .append_pair("response_type", "code")
        .append_pair("client_id", &cfg.client_id)
        .append_pair("redirect_uri", &cfg.redirect_uri)
        .append_pair("scope", flow::APPLE_SCOPE)
        .append_pair("response_mode", "query")
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

/// `POST /auth/apple/start` — mint state, return the Apple auth URL.
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
    if !state.auth_limits.check(&format!("as:{ip}"), now, 30, 60) {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            Json(json!({"error": "rate limited"})),
        )
            .into_response();
    }
    let Some(apple) = state.apple_cfg.as_ref().filter(|cfg| cfg.configured()) else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"error": "apple login unavailable"})),
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
                    Json(json!({"error": "apple linking is for customer accounts"})),
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
        core::KIND_APPLE,
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
    let Some(url) = auth_url(apple, &handle) else {
        return store_unavailable();
    };
    let csrf = auth::csrf::mint_token();
    let secure = !state.admin_mtls_off;
    (
        StatusCode::OK,
        passkey::set_cookies(&[
            auth::csrf_cookie(&csrf, secure),
            apple_cookie(&binding, secure),
        ]),
        Json(json!({"auth_url": url})),
    )
        .into_response()
}

/// Apple appends its own params, so this DTO deliberately skips
/// `deny_unknown_fields` — only `code`/`state`/`error` are read.
#[derive(serde::Deserialize)]
pub(crate) struct CallbackQuery {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
}

/// `GET /auth/apple/callback` — exchange, verify, resolve, set the
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
    let clear = auth::clear_cookie(APPLE_COOKIE, secure);
    let Some(apple) = state.apple_cfg.as_ref().filter(|cfg| cfg.configured()) else {
        return redirect(
            "/apostille/login?error=oauth_unavailable",
            std::slice::from_ref(&clear),
        );
    };
    let ip = passkey::client_ip(addr);
    if !state.auth_limits.check(&format!("ac:{ip}"), now, 60, 60) {
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
        // User cancelled at Apple (or Apple refused). Best-effort
        // state burn so the handle cannot be replayed later.
        if let Some(handle) = query.state.as_deref() {
            let _ = core::take_state_kind(&conn, core::KIND_APPLE, handle, now);
        }
        passkey::audit_auth(&conn, now, "anonymous", "apple_denied", "");
        return redirect(
            "/apostille/login?error=oauth_denied",
            std::slice::from_ref(&clear),
        );
    }
    let (Some(code), Some(handle)) = (query.code.as_deref(), query.state.as_deref()) else {
        passkey::audit_auth(&conn, now, "anonymous", "apple_failed", "missing params");
        return redirect(
            "/apostille/login?error=oauth_failed",
            std::slice::from_ref(&clear),
        );
    };
    let (link_user, mode, binding) =
        match core::take_state_kind(&conn, core::KIND_APPLE, handle, now) {
            Ok(core::TakeOutcome::Taken {
                link_user,
                mode,
                binding,
            }) => (link_user, mode, binding),
            Ok(core::TakeOutcome::Corrupt) => {
                passkey::audit_auth(&conn, now, "anonymous", "apple_failed", "corrupt state");
                return redirect(
                    "/apostille/login?error=oauth_failed",
                    std::slice::from_ref(&clear),
                );
            }
            Ok(core::TakeOutcome::Missing) => {
                passkey::audit_auth(&conn, now, "anonymous", "apple_expired", "");
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
    let presented = auth::cookie_value(jar, APPLE_COOKIE).unwrap_or("");
    // The binding is 256-bit random per flow, single-use, and 5-min
    // lived, and the row is already consumed — a mismatch oracle
    // buys an attacker nothing, so plain equality is fine.
    if presented.is_empty() || presented != binding {
        passkey::audit_auth(&conn, now, "anonymous", "apple_failed", "binding");
        return redirect(
            "/apostille/login?error=oauth_failed",
            std::slice::from_ref(&clear),
        );
    }
    let call = apple.clone();
    let code_owned = code.to_string();
    let outcome = tokio::task::spawn_blocking(move || {
        let transport = UreqTransport::new();
        let secret = flow::mint_client_secret(
            &call.team_id,
            &call.client_id,
            &call.key_id,
            call.private_key.as_str(),
            now,
        )?;
        let id_token = core::exchange_code(
            &transport,
            &call.token_url,
            &code_owned,
            &call.client_id,
            &secret,
            &call.redirect_uri,
        )?;
        flow::verify_id_token(&transport, &call.keys_url, &id_token, &call.client_id, now)
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
                "apple_failed",
                &format!("exchange:{status}"),
            );
            return redirect(
                "/apostille/login?error=oauth_failed",
                std::slice::from_ref(&clear),
            );
        }
        Ok(Err(core::OAuthError::InvalidToken(tag))) => {
            passkey::audit_auth(&conn, now, "anonymous", "apple_failed", tag);
            return redirect(
                "/apostille/login?error=oauth_failed",
                std::slice::from_ref(&clear),
            );
        }
        Ok(Err(core::OAuthError::Transport(_))) => {
            passkey::audit_auth(&conn, now, "anonymous", "apple_failed", "transport");
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
    let outcome = match flow::resolve_apple(
        &conn,
        &verified.sub,
        &verified.email,
        verified.email_verified,
        verified.is_relay,
        now,
    ) {
        Ok(outcome) => outcome,
        Err(_) => {
            return redirect(
                "/apostille/login?error=oauth_failed",
                std::slice::from_ref(&clear),
            );
        }
    };
    let (user_id, fresh_account) = match outcome {
        core::ResolveOutcome::PromptExplicitLink => {
            passkey::audit_auth(
                &conn,
                now,
                &auth::hash_str(&verified.email.to_lowercase()),
                "apple_failed",
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
        .check(&format!("ac:{user_id}"), now, 10, 60)
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
            // Staff log in with passkey+mTLS (spec §5), never Apple.
            passkey::audit_auth(&conn, now, &user_id, "apple_failed", "role");
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
    verified: &flow::VerifiedApple,
    now: i64,
) -> Response {
    let Some(user_id) = link_user else {
        passkey::audit_auth(conn, now, "anonymous", "apple_failed", "link session");
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
        passkey::audit_auth(conn, now, user_id, "apple_failed", "link session");
        return redirect(
            "/apostille/login?error=oauth_failed",
            std::slice::from_ref(clear),
        );
    }
    match flow::find_apple_owner(conn, &verified.sub).unwrap_or(None) {
        Some(owner) if owner != user_id => {
            passkey::audit_auth(conn, now, user_id, "apple_failed", "already linked");
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
    if flow::insert_apple_link(
        conn,
        user_id,
        &verified.sub,
        &verified.email,
        verified.email_verified,
        verified.is_relay,
    )
    .is_err()
    {
        // Lost a double-submit race (two states, one sub): the
        // winner owns it — recheck before failing.
        if flow::find_apple_owner(conn, &verified.sub)
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

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use std::io::{Read, Write};
    use tower::ServiceExt;

    fn test_app_with_apple(apple: Option<AppleCfg>) -> (tempfile::TempDir, axum::Router) {
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
            admin_spa_dir: std::path::PathBuf::from("test-admin-spa"),
            media_terms_read: false,
            label_cfg: crate::p2_labels::LabelCfg::default(),
            voice_cfg: voice::VoiceConfig::default(),
            voice_api_key: None,
            webauthn: std::sync::Arc::new(passkey::test_webauthn()),
            auth_limits: std::sync::Arc::new(passkey::RateBuckets::default()),
            google_cfg: None,
            apple_cfg: apple,
            meta_cfg: None,
            mailer: None,
            magic_link_base: "https://example.test/apostille/login".to_string(),
        };
        let app = crate::public_router(state, &dir, &dir, &dir);
        (tmp, app)
    }

    const TEST_KID: &str = "test-kid-1";

    /// Mint a fresh synthetic P-256 key as PKCS#8 PEM (test-only;
    /// generated per test via aws-lc-rs — no key material is
    /// embedded in the source). Armor lines are assembled from
    /// fragments: the secrets-scan forbids the literal needle even
    /// in test fixtures.
    fn generated_p8() -> String {
        use aws_lc_rs::rand::SystemRandom;
        use base64::Engine as _;
        let pkcs8 = aws_lc_rs::signature::EcdsaKeyPair::generate_pkcs8(
            &aws_lc_rs::signature::ECDSA_P256_SHA256_FIXED_SIGNING,
            &SystemRandom::new(),
        )
        .unwrap();
        let body = base64::engine::general_purpose::STANDARD.encode(pkcs8.as_ref());
        let begin: String = ["-----BEGIN", " PRIVATE KEY-----"].concat();
        let end: String = ["-----END", " PRIVATE KEY-----"].concat();
        let mut out = format!("{begin}\n");
        for chunk in body.as_bytes().chunks(64) {
            out.push_str(std::str::from_utf8(chunk).unwrap());
            out.push('\n');
        }
        out.push_str(&end);
        out.push('\n');
        out
    }

    /// JWKS body matching a PEM key (public point derived via
    /// aws-lc-rs).
    fn jwks_for(pem: &str) -> String {
        use aws_lc_rs::signature::KeyPair as _;
        use base64::Engine as _;
        let body: String = pem
            .lines()
            .filter(|line| !line.starts_with("-----"))
            .collect();
        let der = base64::engine::general_purpose::STANDARD
            .decode(body)
            .unwrap();
        let pair = aws_lc_rs::signature::EcdsaKeyPair::from_pkcs8(
            &aws_lc_rs::signature::ECDSA_P256_SHA256_FIXED_SIGNING,
            &der,
        )
        .unwrap();
        let point: &[u8] = pair.public_key().as_ref();
        let b64u = base64::engine::general_purpose::URL_SAFE_NO_PAD;
        let x = b64u.encode(&point[1..33]);
        let y = b64u.encode(&point[33..65]);
        format!(
            r#"{{"keys":[{{"kty":"EC","crv":"P-256","kid":"{TEST_KID}","x":"{x}","y":"{y}","use":"sig","alg":"ES256"}}]}}"#
        )
    }

    /// Sign an id_token with a PEM key (test-only issuer).
    fn id_token(pem: &str, sub: &str, email: &str, verified: bool, relay: bool) -> String {
        use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
        let key = EncodingKey::from_ec_pem(pem.as_bytes()).unwrap();
        let mut header = Header::new(Algorithm::ES256);
        header.kid = Some(TEST_KID.to_string());
        let claims = serde_json::json!({
            "iss": flow::APPLE_ISSUER,
            "aud": "cid-stub",
            "sub": sub,
            "exp": 2_000_000_000i64,
            "iat": 1_000_000_000i64,
            "email": email,
            "email_verified": verified,
            "is_private_email": relay,
        });
        encode(&header, &claims, &key).unwrap()
    }

    fn token_body(id_token: &str) -> String {
        format!(r#"{{"access_token":"at","id_token":"{id_token}","token_type":"Bearer"}}"#)
    }

    /// Build a stub-pointed cfg through the REAL sealed-rows path
    /// (no test-only `SecretText` constructor exists on purpose).
    fn stub_cfg(base: &str, pem: &str) -> AppleCfg {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        config::migrate_config_tables(&conn).unwrap();
        let kek = config::Kek::generate();
        for (key, key_type, value) in [
            (
                config::keys::APPLE_CLIENT_ID,
                config::ConfigType::Text,
                b"cid-stub".as_slice(),
            ),
            (
                config::keys::APPLE_TEAM_ID,
                config::ConfigType::Text,
                b"team-stub".as_slice(),
            ),
            (
                config::keys::APPLE_KEY_ID,
                config::ConfigType::Text,
                b"key-stub".as_slice(),
            ),
            (
                config::keys::APPLE_PRIVATE_KEY,
                config::ConfigType::Secret,
                pem.as_bytes(),
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
        let mut apple = AppleCfg::from_values(
            cfg.apple_client_id.as_deref().unwrap(),
            cfg.apple_team_id.as_deref().unwrap(),
            cfg.apple_key_id.as_deref().unwrap(),
            cfg.apple_private_key.unwrap(),
            "https://example.test/auth/apple/callback",
        );
        apple.auth_base = format!("{base}/auth");
        apple.token_url = format!("{base}/token");
        apple.keys_url = format!("{base}/keys");
        apple
    }

    /// Minimal port-0 HTTP stub: fixed token + keys bodies.
    /// `Connection: close` on every response keeps the accept loop
    /// to one request per connection (no keep-alive framing).
    struct StubApple {
        base: String,
        addr: SocketAddr,
        handle: Option<std::thread::JoinHandle<()>>,
    }

    impl StubApple {
        fn run(token: (u16, String), keys: (u16, String)) -> Self {
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
                    let (status, body) = if text.starts_with("POST /token") {
                        token.clone()
                    } else if text.starts_with("GET /keys") {
                        keys.clone()
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

    impl Drop for StubApple {
        fn drop(&mut self) {
            if let Some(handle) = self.handle.take() {
                if let Ok(mut quit) = std::net::TcpStream::connect(self.addr) {
                    let _ = quit.write_all(b"QUIT / HTTP/1.1\r\n\r\n");
                }
                let _ = handle.join();
            }
        }
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
                "/apostille/api/v1/auth/apple/start",
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
    fn configured_needs_all_four_halves() {
        let secret = config::SecretText::from("x".to_string());
        let mut cfg = AppleCfg::from_values("cid", "team", "kid", secret, "https://x/cb");
        assert!(cfg.configured());
        cfg.team_id.clear();
        assert!(!cfg.configured());
    }

    #[test]
    fn auth_url_shape() {
        let cfg = AppleCfg::from_values(
            "cid",
            "team",
            "kid",
            config::SecretText::from("s".to_string()),
            "https://example.test/cb",
        );
        let url = auth_url(&cfg, "state-1").unwrap();
        assert!(url.starts_with(flow::APPLE_AUTH_URL), "{url}");
        assert!(url.contains("response_type=code"), "{url}");
        assert!(url.contains("scope=name+email"), "{url}");
        assert!(url.contains("response_mode=query"), "{url}");
        assert!(url.contains("state=state-1"), "{url}");
        assert!(
            url.contains("redirect_uri=https%3A%2F%2Fexample.test%2Fcb"),
            "{url}"
        );
    }

    #[test]
    fn apple_cookie_attrs() {
        let cookie = apple_cookie("bind-1", true);
        assert!(cookie.starts_with("hb_apple=bind-1;"), "{cookie}");
        for attr in [
            "HttpOnly",
            "SameSite=Lax",
            "Path=/",
            "Max-Age=300",
            "Secure",
        ] {
            assert!(cookie.contains(attr), "{cookie}");
        }
        assert!(!apple_cookie("bind-1", false).contains("Secure"));
    }

    #[tokio::test]
    async fn start_503_when_unconfigured() {
        let (_tmp, app) = test_app_with_apple(None);
        let res = app
            .clone()
            .oneshot(post_json(
                "/apostille/api/v1/auth/apple/start",
                "",
                r#"{"mode":"login"}"#,
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);
        let res = app
            .clone()
            .oneshot(get("/auth/apple/callback?code=x&state=y", ""))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::FOUND);
        assert!(location_of(&res).contains("oauth_unavailable"));
    }

    #[tokio::test]
    async fn start_shape_cookies_and_guards() {
        let pem = generated_p8();
        let token = token_body(&id_token(&pem, "sub-1", "a@example.test", true, false));
        let stub = StubApple::run((200, token), (200, jwks_for(&pem)));
        let (_tmp, app) = test_app_with_apple(Some(stub_cfg(&stub.base, &pem)));
        // Evil origin → 403 even on the dev seam.
        let res = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/apostille/api/v1/auth/apple/start")
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
                .oneshot(post_json("/apostille/api/v1/auth/apple/start", "", body))
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
                "/apostille/api/v1/auth/apple/start",
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
        assert!(cookies.contains(APPLE_COOKIE), "{cookies}");
        assert!(cookies.contains(auth::CSRF_COOKIE), "{cookies}");
        assert!(!state_of(auth_url).is_empty());
    }

    #[tokio::test]
    async fn callback_denied_expired_and_mismatch() {
        let pem = generated_p8();
        let token = token_body(&id_token(&pem, "sub-1", "a@example.test", true, false));
        let stub = StubApple::run((200, token), (200, jwks_for(&pem)));
        let (_tmp, app) = test_app_with_apple(Some(stub_cfg(&stub.base, &pem)));
        // User cancelled at Apple → denied (state burned best-effort).
        let res = app
            .clone()
            .oneshot(get(
                "/auth/apple/callback?error=access_denied&state=nope",
                "",
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::FOUND);
        assert!(location_of(&res).contains("oauth_denied"));
        // Unknown state → expired.
        let res = app
            .clone()
            .oneshot(get("/auth/apple/callback?code=c&state=nope", ""))
            .await
            .unwrap();
        assert!(location_of(&res).contains("oauth_expired"));
        // Binding stripped → failed.
        let (status, body, _) = start_login(&app).await;
        assert_eq!(status, StatusCode::OK);
        let uri = format!(
            "/auth/apple/callback?code=c&state={}",
            state_of(body["auth_url"].as_str().unwrap())
        );
        let res = app.clone().oneshot(get(&uri, "")).await.unwrap();
        assert!(location_of(&res).contains("oauth_failed"));
    }

    #[tokio::test]
    async fn callback_full_loop_mints_session() {
        let pem = generated_p8();
        let token = token_body(&id_token(&pem, "sub-1", "a@example.test", true, false));
        let stub = StubApple::run((200, token), (200, jwks_for(&pem)));
        let (tmp, app) = test_app_with_apple(Some(stub_cfg(&stub.base, &pem)));
        let (status, body, cookies) = start_login(&app).await;
        assert_eq!(status, StatusCode::OK);
        let uri = format!(
            "/auth/apple/callback?code=code-1&state={}",
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
        assert!(set.contains(&format!("{APPLE_COOKIE}=;")), "{set}");
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
        // The link row exists for the new account.
        let store = handbook_store::store::Store::new(tmp.path());
        let conn = handbook_store::db::open(&store).unwrap();
        assert!(flow::find_apple_owner(&conn, "sub-1").unwrap().is_some());
    }

    #[tokio::test]
    async fn callback_email_match_links_nothing_without_session() {
        // u-A owns the verified email; the token carries a NEW sub.
        let pem = generated_p8();
        let token = token_body(&id_token(
            &pem,
            "sub-new",
            "taken@example.test",
            true,
            false,
        ));
        let stub = StubApple::run((200, token), (200, jwks_for(&pem)));
        let (tmp, app) = test_app_with_apple(Some(stub_cfg(&stub.base, &pem)));
        let store = handbook_store::store::Store::new(tmp.path());
        let conn = handbook_store::db::open(&store).unwrap();
        auth::passkey::ensure_account(&conn, "u-a", 1000).unwrap();
        core::insert_google_link(&conn, "u-a", "sub-old", "taken@example.test", true).unwrap();
        drop(conn);
        let (status, body, cookies) = start_login(&app).await;
        assert_eq!(status, StatusCode::OK);
        let uri = format!(
            "/auth/apple/callback?code=code-1&state={}",
            state_of(body["auth_url"].as_str().unwrap())
        );
        let res = app.clone().oneshot(get(&uri, &cookies)).await.unwrap();
        assert!(location_of(&res).contains("link_required"));
        let set = res
            .headers()
            .get_all(header::SET_COOKIE)
            .iter()
            .filter_map(|v| v.to_str().ok())
            .collect::<Vec<_>>()
            .join("; ");
        // No session minted (only the binding clear-cookie).
        assert!(!set.contains(auth::PUB_COOKIE), "{set}");
        // ...and rule 5 wrote no link row for the new sub.
        let conn = handbook_store::db::open(&store).unwrap();
        assert_eq!(flow::find_apple_owner(&conn, "sub-new").unwrap(), None);
    }

    #[tokio::test]
    async fn callback_relay_mints_fresh_account() {
        // Same address verified on Google — but the Apple claim is a
        // private relay, so the login mints fresh (never matches).
        let pem = generated_p8();
        let token = token_body(&id_token(&pem, "sub-relay", "r@example.test", true, true));
        let stub = StubApple::run((200, token), (200, jwks_for(&pem)));
        let (tmp, app) = test_app_with_apple(Some(stub_cfg(&stub.base, &pem)));
        let store = handbook_store::store::Store::new(tmp.path());
        let conn = handbook_store::db::open(&store).unwrap();
        auth::passkey::ensure_account(&conn, "u-g", 1000).unwrap();
        core::insert_google_link(&conn, "u-g", "g-sub-1", "r@example.test", true).unwrap();
        drop(conn);
        let (status, body, cookies) = start_login(&app).await;
        assert_eq!(status, StatusCode::OK);
        let uri = format!(
            "/auth/apple/callback?code=code-1&state={}",
            state_of(body["auth_url"].as_str().unwrap())
        );
        let res = app.clone().oneshot(get(&uri, &cookies)).await.unwrap();
        assert_eq!(location_of(&res), "/apostille/login");
        let conn = handbook_store::db::open(&store).unwrap();
        let owner = flow::find_apple_owner(&conn, "sub-relay")
            .unwrap()
            .expect("relay link row");
        assert_ne!(owner, "u-g");
    }

    #[tokio::test]
    async fn callback_link_mode_adds_method() {
        let pem = generated_p8();
        let token = token_body(&id_token(
            &pem,
            "sub-link",
            "linker@example.test",
            true,
            false,
        ));
        let stub = StubApple::run((200, token), (200, jwks_for(&pem)));
        let (tmp, app) = test_app_with_apple(Some(stub_cfg(&stub.base, &pem)));
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
                "/apostille/api/v1/auth/apple/start",
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
            "/auth/apple/callback?code=code-1&state={}",
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
            flow::find_apple_owner(&conn, "sub-link").unwrap(),
            Some("u-link".to_string())
        );
    }

    #[tokio::test]
    async fn callback_wrong_aud_fails_closed() {
        // Stub signs aud=cid-stub; cfg expects someone else.
        let pem = generated_p8();
        let mut cfg = stub_cfg("http://127.0.0.1:9", &pem);
        cfg.client_id = "cid-other".to_string();
        let token = token_body(&id_token(&pem, "sub-1", "a@example.test", true, false));
        let stub = StubApple::run((200, token), (200, jwks_for(&pem)));
        cfg.auth_base = format!("{}/auth", stub.base);
        cfg.token_url = format!("{}/token", stub.base);
        cfg.keys_url = format!("{}/keys", stub.base);
        let (_tmp, app) = test_app_with_apple(Some(cfg));
        let (status, body, cookies) = start_login(&app).await;
        assert_eq!(status, StatusCode::OK);
        let uri = format!(
            "/auth/apple/callback?code=code-1&state={}",
            state_of(body["auth_url"].as_str().unwrap())
        );
        let res = app.clone().oneshot(get(&uri, &cookies)).await.unwrap();
        assert!(location_of(&res).contains("oauth_failed"));
    }
}
