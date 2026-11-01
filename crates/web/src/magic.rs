//! Magic-link handlers (spec `customer-auth.md` §12): request is
//! always-200 (anti-enumeration); consume POSTs the fragment
//! token as JSON and mints a fallback-window session. The live
//! SMTP mailer and the dev/test file mailer live here next to the
//! handlers (the `oauth.rs` precedent).

use axum::{
    Json,
    extract::{ConnectInfo, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use serde_json::json;
use std::net::SocketAddr;

use super::passkey;
use super::{AppState, store_unavailable, unix_now};
use auth::magic as flow;
use auth::passkey as store;

/// Live SMTP mailer (spec §12.8): Gmail STARTTLS today, any
/// STARTTLS host tomorrow (host/port/creds/From are config rows).
/// Holds the password as a redacting `SecretText` and builds the
/// transport per send (fallback-only volume — no pool needed).
#[derive(Clone)]
pub struct SmtpMailer {
    host: String,
    port: u16,
    username: String,
    password: config::SecretText,
    from: String,
}

impl SmtpMailer {
    pub fn new(
        host: &str,
        port: u16,
        username: &str,
        password: config::SecretText,
        from: &str,
    ) -> Result<Self, &'static str> {
        if host.trim().is_empty() || username.trim().is_empty() || from.trim().is_empty() {
            return Err("smtp mailer needs host, username, and from");
        }
        if port == 0 {
            return Err("smtp mailer needs a nonzero port");
        }
        // Fail closed at boot: the From must parse as a mailbox.
        let _: lettre::message::Mailbox = from.parse().map_err(|_| "mailer.from does not parse")?;
        // lettre runs with `rustls-no-provider`, like ureq — the
        // workspace installs its single provider explicitly.
        super::oauth::ensure_provider();
        Ok(Self {
            host: host.to_string(),
            port,
            username: username.to_string(),
            password,
            from: from.to_string(),
        })
    }

    fn transport(&self) -> Result<lettre::SmtpTransport, flow::MailError> {
        let creds = lettre::transport::smtp::authentication::Credentials::new(
            self.username.clone(),
            self.password.as_str().to_string(),
        );
        Ok(lettre::SmtpTransport::starttls_relay(&self.host)
            .map_err(|_| flow::MailError::Transport("smtp setup failed".into()))?
            .port(self.port)
            .credentials(creds)
            .timeout(Some(std::time::Duration::from_secs(30)))
            .build())
    }
}

/// Static-shape SMTP error mapping: the 3-digit code is real
/// (proves e.g. 535 bad-auth vs a 5xx refusal); the response
/// TEXT is dropped — it can echo the link or the address.
fn map_smtp_error(error: lettre::transport::smtp::Error) -> flow::MailError {
    match error.status() {
        Some(code) => {
            let n = code.to_string().parse::<u16>().unwrap_or(0);
            flow::MailError::Rejected(n)
        }
        None => flow::MailError::Transport("smtp send failed".into()),
    }
}

impl flow::Mailer for SmtpMailer {
    fn send(&self, to: &str, subject: &str, text_body: &str) -> Result<(), flow::MailError> {
        use lettre::Transport as _;
        let message = lettre::Message::builder()
            .from(
                self.from
                    .parse()
                    .map_err(|_| flow::MailError::Transport("bad sender".into()))?,
            )
            .to(to
                .parse()
                .map_err(|_| flow::MailError::Transport("bad recipient".into()))?)
            .subject(subject)
            .body(text_body.to_string())
            .map_err(|_| flow::MailError::Transport("message build failed".into()))?;
        self.transport()?
            .send(&message)
            .map(|_| ())
            .map_err(map_smtp_error)
    }
}

/// Dev/test mailer: one 0600 file per message under the store
/// `mail/` dir (`{nanos}-{seq}-{email_hash16}.txt`). CI never
/// sends real mail; handler tests parse these files.
#[derive(Clone)]
pub struct FileMailer {
    pub dir: std::path::PathBuf,
}

static FILE_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// 0600 file write (the `config::seal` `write_private` shape):
/// delivered links are live login grants, even on a dev box.
#[cfg(unix)]
fn write_private(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::os::unix::fs::OpenOptionsExt as _;
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true).mode(0o600);
    use std::io::Write as _;
    opts.open(path)?.write_all(bytes)
}

#[cfg(not(unix))]
fn write_private(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    std::fs::write(path, bytes)
}

impl flow::Mailer for FileMailer {
    fn send(&self, to: &str, subject: &str, text_body: &str) -> Result<(), flow::MailError> {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let seq = FILE_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let hash16 = &auth::hash_str(&to.to_lowercase())[..16];
        let path = self.dir.join(format!("{nanos}-{seq}-{hash16}.txt"));
        let content = format!("To: {to}\nSubject: {subject}\n\n{text_body}");
        write_private(&path, content.as_bytes())
            .map_err(|_| flow::MailError::Transport("mail file write failed".into()))
    }
}

/// Configured mailer (boot-built; `None` in `AppState` means the
/// endpoints 503 and the boot never refuses).
#[derive(Clone)]
pub enum Mailer {
    Smtp(SmtpMailer),
    File(FileMailer),
}

impl flow::Mailer for Mailer {
    fn send(&self, to: &str, subject: &str, text_body: &str) -> Result<(), flow::MailError> {
        match self {
            Mailer::Smtp(inner) => inner.send(to, subject, text_body),
            Mailer::File(inner) => inner.send(to, subject, text_body),
        }
    }
}

/// Build the boot mailer from sealed rows (`mailer.provider` is
/// the discriminator; unknown/partial → warn + `None`, fail
/// closed). The file dir is created at boot so dev "just works".
pub fn mailer_from_config(
    cfg: &config::RuntimeConfig,
    store_root: &std::path::Path,
) -> Option<Mailer> {
    match cfg.mailer_provider.as_str() {
        "file" => {
            let dir = store_root.join(&cfg.mailer_dir);
            if let Err(e) = std::fs::create_dir_all(&dir) {
                eprintln!(
                    "WARNING: config mailer.dir {} not writable ({e}) — magic login disabled",
                    dir.display()
                );
                return None;
            }
            Some(Mailer::File(FileMailer { dir }))
        }
        "smtp" => {
            match (
                &cfg.mailer_host,
                &cfg.mailer_port,
                &cfg.mailer_username,
                &cfg.mailer_password,
                &cfg.mailer_from,
            ) {
                (Some(host), Some(port), Some(user), Some(pass), Some(from)) => {
                    match port.parse::<u16>() {
                        Ok(parsed) => match SmtpMailer::new(host, parsed, user, pass.clone(), from)
                        {
                            Ok(mailer) => Some(Mailer::Smtp(mailer)),
                            Err(why) => {
                                eprintln!(
                                    "WARNING: config mailer.* ({why}) — magic login disabled"
                                );
                                None
                            }
                        },
                        Err(_) => {
                            eprintln!(
                                "WARNING: config mailer.port is not a port — magic login disabled"
                            );
                            None
                        }
                    }
                }
                _ => {
                    eprintln!(
                        "WARNING: config mailer.host/mailer.port/mailer.username/mailer.password/mailer.from incomplete — magic login disabled"
                    );
                    None
                }
            }
        }
        "" => {
            eprintln!("WARNING: config mailer.provider unset — magic login disabled");
            None
        }
        other => {
            eprintln!(
                "WARNING: config mailer.provider {other:?} unknown (want file|smtp) — magic login disabled"
            );
            None
        }
    }
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StartBody {
    email: String,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ConsumeBody {
    token: String,
}

fn rate_limited() -> Response {
    (
        StatusCode::TOO_MANY_REQUESTS,
        Json(json!({"error": "rate limited"})),
    )
        .into_response()
}

fn gone() -> Response {
    (
        StatusCode::GONE,
        Json(json!({"error": "link invalid or expired"})),
    )
        .into_response()
}

/// `POST /auth/magic/start` — always-200 anti-enumeration core
/// (spec §12.3). Mints `hb_csrf` like every §2 start.
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
    if !state.auth_limits.check(&format!("mr:{ip}"), now, 10, 60) {
        return rate_limited();
    }
    let Some(mailer) = state.mailer.clone() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"error": "magic login unavailable"})),
        )
            .into_response();
    };
    let Some(email) = passkey::valid_email(&body.email) else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "email malformed"})),
        )
            .into_response();
    };
    let hash = auth::hash_str(&email);
    if !state
        .auth_limits
        .check(&format!("mr:{}", &hash[..16]), now, 3, 3600)
    {
        return rate_limited();
    }
    let Some(conn) = passkey::open_conn(&state) else {
        return store_unavailable();
    };
    let _ = flow::sweep_magic_tokens(&conn, now);
    // Exactly one live customer owner → store + send. Every other
    // shape (unknown, ambiguous, locked, non-customer) → identical
    // silence (no useless email for ambiguous addresses either).
    let owners = flow::owners_by_email_hash(&conn, &hash).unwrap_or_default();
    let single_live = match owners.as_slice() {
        [user] => store::get_account(&conn, user)
            .ok()
            .flatten()
            .is_some_and(|a| a.locked_at.is_none() && a.role == "customer"),
        _ => false,
    };
    if single_live {
        let _ = flow::invalidate_email_tokens(&conn, &email, now);
        let token = auth::login::mint_magic_token();
        let token_hash = auth::login::token_hash(&token);
        if flow::store_magic_token(&conn, &token_hash, &email, now).is_ok() {
            let base = state.magic_link_base.clone();
            let to = email.clone();
            let send = tokio::task::spawn_blocking(move || {
                flow::send_magic_link(&mailer, &to, &base, &token)
            })
            .await;
            // Send failure: row deleted + audit, still 200 (a
            // failure is indistinguishable from unknown-address
            // silence by design).
            if !matches!(send, Ok(Ok(()))) {
                let tag = match send {
                    Ok(Err(flow::MailError::Rejected(_))) => "rejected",
                    _ => "transport",
                };
                let _ = flow::delete_magic_token(&conn, &token_hash);
                passkey::audit_auth(&conn, now, "anonymous", "magic_send_failed", tag);
            }
        }
    }
    let csrf = auth::csrf::mint_token();
    let secure = !state.admin_mtls_off;
    (
        StatusCode::OK,
        passkey::set_cookies(&[auth::csrf_cookie(&csrf, secure)]),
        Json(json!({"ok": true})),
    )
        .into_response()
}

/// `POST /auth/magic/consume` — consume-before-session (spec
/// §12.4). Origin check only, no CSRF echo: cross-browser
/// consume is the point, so no double-submit cookie can exist on
/// first touch.
pub async fn consume(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(body): Json<ConsumeBody>,
) -> Response {
    let now = unix_now();
    if !passkey::origin_ok(&headers, state.admin_mtls_off) {
        return passkey::csrf_fail();
    }
    let ip = passkey::client_ip(addr);
    if !state.auth_limits.check(&format!("mx:{ip}"), now, 60, 60) {
        return rate_limited();
    }
    let token = body.token.trim();
    let Some(conn) = passkey::open_conn(&state) else {
        return store_unavailable();
    };
    if !(token.len() == 64 && token.bytes().all(|b| b.is_ascii_hexdigit())) {
        passkey::audit_auth(&conn, now, "anonymous", "magic_failed", "shape");
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "link invalid"})),
        )
            .into_response();
    }
    let token_hash = auth::login::token_hash(token);
    let Some(email_hash) = flow::take_magic_token(&conn, &token_hash, now).unwrap_or(None) else {
        // Honest audit reason via a peek (the take is
        // uniform-None by design — the response stays a single
        // 410 with no replay-vs-expired oracle).
        let state =
            flow::inspect_magic_token(&conn, &token_hash, now).unwrap_or(flow::TokenState::Missing);
        match state {
            flow::TokenState::Expired => {
                passkey::audit_auth(&conn, now, "anonymous", "magic_expired", "expired");
            }
            flow::TokenState::Consumed => {
                passkey::audit_auth(&conn, now, "anonymous", "magic_failed", "replay");
            }
            _ => {
                passkey::audit_auth(&conn, now, "anonymous", "magic_failed", "invalid");
            }
        }
        return gone();
    };
    let owners = flow::owners_by_email_hash(&conn, &email_hash).unwrap_or_default();
    if owners.len() != 1 {
        let reason = if owners.is_empty() {
            "orphaned"
        } else {
            "ambiguous"
        };
        passkey::audit_auth(&conn, now, "anonymous", "magic_failed", reason);
        return gone();
    }
    let user_id = &owners[0];
    if !state
        .auth_limits
        .check(&format!("mx:{user_id}"), now, 10, 60)
    {
        return rate_limited();
    }
    match store::get_account(&conn, user_id).unwrap_or(None) {
        Some(account) if account.locked_at.is_some() => {
            passkey::audit_auth(&conn, now, user_id, "account_locked", "login refused");
            return gone();
        }
        Some(account) if account.role == "customer" => account,
        _ => {
            passkey::audit_auth(&conn, now, "anonymous", "magic_failed", "role");
            return gone();
        }
    };
    // Rule-2 flip: inbox control proven by this consume.
    let _ = flow::verify_email_rows(&conn, user_id, &email_hash);
    let session_id = auth::mint_session_id();
    if auth::create_session(
        &conn,
        &session_id,
        &auth::NewSession {
            user_id,
            role: auth::Role::Customer,
            customer_ref: Some(user_id),
            now,
            ip: &ip,
            user_agent: passkey::user_agent(&headers),
        },
    )
    .is_err()
        || flow::set_fallback(
            &conn,
            user_id,
            now.saturating_add(auth::FALLBACK_WINDOW_SECS),
        )
        .is_err()
    {
        passkey::audit_auth(&conn, now, "anonymous", "magic_failed", "store");
        return store_unavailable();
    }
    // NEVER set step_up_at here (no UV signal — rule 6). A later
    // passkey/OAuth login graduates via `clear_fallback`.
    let rotation = auth::csrf::mint_token();
    let secure = !state.admin_mtls_off;
    passkey::audit_auth(&conn, now, user_id, "magic_login", "fallback");
    (
        StatusCode::OK,
        passkey::set_cookies(&[
            auth::csrf_cookie(&rotation, secure),
            auth::session_cookie(auth::PUB_COOKIE, &session_id, secure),
        ]),
        Json(json!({"user_id": user_id, "fallback": true})),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, header};
    use tower::ServiceExt as _;

    fn addr() -> ConnectInfo<SocketAddr> {
        ConnectInfo("127.0.0.1:9".parse().unwrap())
    }

    fn open_store(root: &std::path::Path) -> rusqlite::Connection {
        let store = handbook_store::store::Store::new(root);
        handbook_store::db::open(&store).unwrap()
    }

    fn test_app_with_mailer(mailer: Option<Mailer>) -> (tempfile::TempDir, axum::Router) {
        let tmp = tempfile::tempdir().unwrap();
        let conn = open_store(tmp.path());
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
            admin_require_cert: false,
            admin_spa_dir: std::path::PathBuf::from("test-admin-spa"),
            media_terms_read: false,
            label_cfg: crate::p2_labels::LabelCfg::default(),
            voice_cfg: voice::VoiceConfig::default(),
            voice_api_key: None,
            webauthn: std::sync::Arc::new(passkey::test_webauthn()),
            auth_limits: std::sync::Arc::new(passkey::RateBuckets::default()),
            demo_limits: std::sync::Arc::new(passkey::RateBuckets::default()),
            draft_runs: std::sync::Arc::new(crate::draft::DraftRuns::default()),
            google_cfg: None,
            apple_cfg: None,
            meta_cfg: None,
            mailer,
            magic_link_base: "https://example.test/apostille/login".to_string(),
            meta_delete_status_base: "https://example.test/auth/meta/delete-status".to_string(),
        };
        let app = crate::public_router(state, &dir, &dir, &dir);
        (tmp, app)
    }

    fn file_app() -> (tempfile::TempDir, axum::Router, std::path::PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let mail_dir = tmp.path().join("mail");
        std::fs::create_dir_all(&mail_dir).unwrap();
        let conn = open_store(tmp.path());
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
            admin_require_cert: false,
            admin_spa_dir: std::path::PathBuf::from("test-admin-spa"),
            media_terms_read: false,
            label_cfg: crate::p2_labels::LabelCfg::default(),
            voice_cfg: voice::VoiceConfig::default(),
            voice_api_key: None,
            webauthn: std::sync::Arc::new(passkey::test_webauthn()),
            auth_limits: std::sync::Arc::new(passkey::RateBuckets::default()),
            demo_limits: std::sync::Arc::new(passkey::RateBuckets::default()),
            draft_runs: std::sync::Arc::new(crate::draft::DraftRuns::default()),
            google_cfg: None,
            apple_cfg: None,
            meta_cfg: None,
            mailer: Some(Mailer::File(FileMailer {
                dir: mail_dir.clone(),
            })),
            magic_link_base: "https://example.test/apostille/login".to_string(),
            meta_delete_status_base: "https://example.test/auth/meta/delete-status".to_string(),
        };
        let app = crate::public_router(state, &dir, &dir, &dir);
        (tmp, app, mail_dir)
    }

    fn post_json(uri: &str, body: &str) -> Request<Body> {
        Request::builder()
            .method("POST")
            .uri(uri)
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::ORIGIN, "http://localhost")
            .extension(addr())
            .body(Body::from(body.to_string()))
            .unwrap()
    }

    async fn roundtrip(
        app: &axum::Router,
        req: Request<Body>,
    ) -> (StatusCode, serde_json::Value, Vec<String>) {
        let res = app.clone().oneshot(req).await.unwrap();
        let status = res.status();
        let cookies = res
            .headers()
            .get_all(header::SET_COOKIE)
            .iter()
            .filter_map(|v| v.to_str().ok().map(str::to_string))
            .collect::<Vec<_>>();
        let bytes = axum::body::to_bytes(res.into_body(), 65536).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap(), cookies)
    }

    fn seed_customer(
        root: &std::path::Path,
        user: &str,
        email: &str,
        verified: i64,
        role: &str,
        locked: bool,
    ) {
        let conn = open_store(root);
        conn.execute(
            "INSERT INTO account(user_id, role, customer_ref, created_at, locked_at, step_up_at, fallback_until)
             VALUES (?1, ?2, ?1, 1, ?3, NULL, NULL)",
            rusqlite::params![user, role, locked.then_some(500)],
        )
        .unwrap();
        let hash = auth::hash_str(&email.to_lowercase());
        conn.execute(
            "INSERT INTO identity_link(user_id, method, provider_sub, email_hash, verified_email, is_relay, disabled_at)
             VALUES (?1, 'email', ?2, ?3, ?4, 0, NULL)",
            rusqlite::params![user, format!("sub-{user}"), hash, verified],
        )
        .unwrap();
    }

    fn mail_files(mail_dir: &std::path::Path) -> Vec<String> {
        let mut out = Vec::new();
        if let Ok(entries) = std::fs::read_dir(mail_dir) {
            for entry in entries.flatten() {
                out.push(std::fs::read_to_string(entry.path()).unwrap());
            }
        }
        out.sort();
        out
    }

    fn fragment_token(body: &str) -> Option<String> {
        let at = body.find("#t=")?;
        let rest = &body[at + 3..];
        let end = rest
            .find(|c: char| !c.is_ascii_hexdigit())
            .unwrap_or(rest.len());
        let token = &rest[..end];
        (token.len() == 64).then(|| token.to_string())
    }

    fn audit_reasons(root: &std::path::Path, action: &str) -> Vec<(String, String)> {
        let conn = open_store(root);
        let mut stmt = conn
            .prepare("SELECT actor, reason FROM audit WHERE action = ?1 ORDER BY at")
            .unwrap();
        stmt.query_map([action], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap()
    }

    #[tokio::test]
    async fn request_unknown_email_is_silent_200() {
        let (tmp, app, mail_dir) = file_app();
        let (status, body, _) = roundtrip(
            &app,
            post_json(
                "/apostille/api/v1/auth/magic/start",
                r#"{"email":"nobody@example.test"}"#,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, serde_json::json!({"ok": true}));
        assert!(mail_files(&mail_dir).is_empty());
        let conn = open_store(tmp.path());
        let rows: i64 = conn
            .query_row("SELECT COUNT(*) FROM magic_token", [], |r| r.get(0))
            .unwrap();
        assert_eq!(rows, 0);
        // No audit on request (start precedent — buckets are the control).
        let audits: i64 = conn
            .query_row("SELECT COUNT(*) FROM audit", [], |r| r.get(0))
            .unwrap();
        assert_eq!(audits, 0);
    }

    #[tokio::test]
    async fn request_known_email_sends_one_link() {
        let (tmp, app, mail_dir) = file_app();
        seed_customer(
            tmp.path(),
            "u1",
            "someone@example.test",
            0,
            "customer",
            false,
        );
        let (status, body, cookies) = roundtrip(
            &app,
            post_json(
                "/apostille/api/v1/auth/magic/start",
                r#"{"email":"Someone@Example.test"}"#,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, serde_json::json!({"ok": true}));
        assert!(
            cookies.iter().any(|c| c.contains("hb_csrf=")),
            "{cookies:?}"
        );
        let files = mail_files(&mail_dir);
        assert_eq!(files.len(), 1);
        assert!(
            files[0].contains("To: someone@example.test"),
            "{}",
            files[0]
        );
        let token = fragment_token(&files[0]).expect("64-hex #t= link");
        assert!(
            files[0].contains("https://example.test/apostille/login#t="),
            "{}",
            files[0]
        );
        // DB holds hashes only.
        let conn = open_store(tmp.path());
        let (stored_hash, stored_email): (String, String) = conn
            .query_row("SELECT token_hash, email_hash FROM magic_token", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap();
        assert_eq!(stored_hash, auth::login::token_hash(&token));
        assert_ne!(stored_hash, token);
        assert_eq!(stored_email, auth::hash_str("someone@example.test"));
    }

    #[tokio::test]
    async fn request_rejects_malformed_and_bad_origin() {
        let (_tmp, app, _mail) = file_app();
        let (status, body, _) = roundtrip(
            &app,
            post_json(
                "/apostille/api/v1/auth/magic/start",
                r#"{"email":"not-an-email"}"#,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body, serde_json::json!({"error": "email malformed"}));
        let evil = Request::builder()
            .method("POST")
            .uri("/apostille/api/v1/auth/magic/start")
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::ORIGIN, "https://evil.test")
            .extension(addr())
            .body(Body::from(r#"{"email":"a@example.test"}"#))
            .unwrap();
        let (status, _, _) = roundtrip(&app, evil).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn request_unconfigured_503s_identically() {
        let (tmp, app) = test_app_with_mailer(None);
        seed_customer(
            tmp.path(),
            "u1",
            "someone@example.test",
            1,
            "customer",
            false,
        );
        let mut bodies = Vec::new();
        for email in ["someone@example.test", "nobody@example.test"] {
            let (status, body, _) = roundtrip(
                &app,
                post_json(
                    "/apostille/api/v1/auth/magic/start",
                    &format!(r#"{{"email":"{email}"}}"#),
                ),
            )
            .await;
            assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
            bodies.push(body);
        }
        assert_eq!(bodies[0], bodies[1]);
        assert_eq!(
            bodies[0],
            serde_json::json!({"error": "magic login unavailable"})
        );
    }

    #[tokio::test]
    async fn request_per_email_cap_trips_per_target() {
        let (tmp, app, _mail) = file_app();
        seed_customer(
            tmp.path(),
            "u1",
            "capped@example.test",
            1,
            "customer",
            false,
        );
        for _ in 0..3 {
            let (status, _, _) = roundtrip(
                &app,
                post_json(
                    "/apostille/api/v1/auth/magic/start",
                    r#"{"email":"capped@example.test"}"#,
                ),
            )
            .await;
            assert_eq!(status, StatusCode::OK);
        }
        let (status, body, _) = roundtrip(
            &app,
            post_json(
                "/apostille/api/v1/auth/magic/start",
                r#"{"email":"capped@example.test"}"#,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(body, serde_json::json!({"error": "rate limited"}));
        // A different address is unaffected (per-target, not per-IP).
        let (status, _, _) = roundtrip(
            &app,
            post_json(
                "/apostille/api/v1/auth/magic/start",
                r#"{"email":"other@example.test"}"#,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
    }

    #[tokio::test]
    async fn consume_happy_path_mints_fallback_session() {
        let (tmp, app, mail_dir) = file_app();
        seed_customer(tmp.path(), "u1", "happy@example.test", 0, "customer", false);
        let (status, _, _) = roundtrip(
            &app,
            post_json(
                "/apostille/api/v1/auth/magic/start",
                r#"{"email":"happy@example.test"}"#,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let token = fragment_token(&mail_files(&mail_dir)[0]).expect("link");
        // No CSRF cookie/echo anywhere: cross-browser consume works.
        let t0 = unix_now();
        let (status, body, cookies) = roundtrip(
            &app,
            post_json(
                "/apostille/api/v1/auth/magic/consume",
                &format!(r#"{{"token":"{token}"}}"#),
            ),
        )
        .await;
        let t1 = unix_now();
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, serde_json::json!({"user_id": "u1", "fallback": true}));
        assert!(
            cookies.iter().any(|c| c.contains(auth::PUB_COOKIE)),
            "{cookies:?}"
        );
        // Fallback armed (~7d), step-up untouched, hint flipped.
        let conn = open_store(tmp.path());
        let account = auth::passkey::get_account(&conn, "u1").unwrap().unwrap();
        let until = account.fallback_until.expect("armed");
        assert!(until >= t0 + auth::FALLBACK_WINDOW_SECS);
        assert!(until <= t1 + auth::FALLBACK_WINDOW_SECS);
        assert_eq!(account.step_up_at, None);
        let verified: i64 = conn
            .query_row(
                "SELECT verified_email FROM identity_link WHERE user_id = 'u1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(verified, 1);
        assert_eq!(
            audit_reasons(tmp.path(), "magic_login"),
            vec![("u1".to_string(), "fallback".to_string())]
        );
    }

    #[tokio::test]
    async fn consume_replay_and_garbage_are_identical_410() {
        let (tmp, app, mail_dir) = file_app();
        seed_customer(
            tmp.path(),
            "u1",
            "replay@example.test",
            1,
            "customer",
            false,
        );
        roundtrip(
            &app,
            post_json(
                "/apostille/api/v1/auth/magic/start",
                r#"{"email":"replay@example.test"}"#,
            ),
        )
        .await;
        let token = fragment_token(&mail_files(&mail_dir)[0]).expect("link");
        let (status, _, _) = roundtrip(
            &app,
            post_json(
                "/apostille/api/v1/auth/magic/consume",
                &format!(r#"{{"token":"{token}"}}"#),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let (status, replay_body, _) = roundtrip(
            &app,
            post_json(
                "/apostille/api/v1/auth/magic/consume",
                &format!(r#"{{"token":"{token}"}}"#),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::GONE);
        let (status, garbage_body, _) = roundtrip(
            &app,
            post_json(
                "/apostille/api/v1/auth/magic/consume",
                &format!(r#"{{"token":"{}"}}"#, "ff".repeat(32)),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::GONE);
        assert_eq!(replay_body, garbage_body);
        assert_eq!(
            replay_body,
            serde_json::json!({"error": "link invalid or expired"})
        );
        // Honest audit underneath the uniform response.
        assert!(
            audit_reasons(tmp.path(), "magic_failed")
                .iter()
                .any(|(_, r)| r == "replay"),
            "replay audited"
        );
        assert!(
            audit_reasons(tmp.path(), "magic_failed")
                .iter()
                .any(|(_, r)| r == "invalid"),
            "garbage audited"
        );
    }

    #[tokio::test]
    async fn consume_expired_row_is_410_with_audit() {
        let (tmp, app, _mail) = file_app();
        seed_customer(tmp.path(), "u1", "old@example.test", 1, "customer", false);
        // Plant an already-expired row directly (no 15-minute wait).
        let token = "ee".repeat(32);
        let conn = open_store(tmp.path());
        conn.execute(
            "INSERT INTO magic_token(token_hash, email_hash, created_at, expires_at, consumed_at)
             VALUES (?1, ?2, 1000, 1900, NULL)",
            rusqlite::params![
                auth::login::token_hash(&token),
                auth::hash_str("old@example.test")
            ],
        )
        .unwrap();
        drop(conn);
        let (status, body, _) = roundtrip(
            &app,
            post_json(
                "/apostille/api/v1/auth/magic/consume",
                &format!(r#"{{"token":"{token}"}}"#),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::GONE);
        assert_eq!(
            body,
            serde_json::json!({"error": "link invalid or expired"})
        );
        assert_eq!(
            audit_reasons(tmp.path(), "magic_expired"),
            vec![("anonymous".to_string(), "expired".to_string())]
        );
    }

    #[tokio::test]
    async fn consume_malformed_shape_is_400() {
        let (tmp, app, _mail) = file_app();
        let (status, body, _) = roundtrip(
            &app,
            post_json("/apostille/api/v1/auth/magic/consume", r#"{"token":"xyz"}"#),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body, serde_json::json!({"error": "link invalid"}));
        assert_eq!(
            audit_reasons(tmp.path(), "magic_failed"),
            vec![("anonymous".to_string(), "shape".to_string())]
        );
    }

    #[tokio::test]
    async fn consume_locked_midflight_is_410_shaped() {
        let (tmp, app, mail_dir) = file_app();
        seed_customer(
            tmp.path(),
            "u1",
            "lockme@example.test",
            1,
            "customer",
            false,
        );
        roundtrip(
            &app,
            post_json(
                "/apostille/api/v1/auth/magic/start",
                r#"{"email":"lockme@example.test"}"#,
            ),
        )
        .await;
        let token = fragment_token(&mail_files(&mail_dir)[0]).expect("link");
        // Lock lands between request and consume: fail closed.
        let conn = open_store(tmp.path());
        conn.execute(
            "UPDATE account SET locked_at = 500 WHERE user_id = 'u1'",
            [],
        )
        .unwrap();
        drop(conn);
        let (status, body, _) = roundtrip(
            &app,
            post_json(
                "/apostille/api/v1/auth/magic/consume",
                &format!(r#"{{"token":"{token}"}}"#),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::GONE);
        assert_eq!(
            body,
            serde_json::json!({"error": "link invalid or expired"})
        );
        assert_eq!(
            audit_reasons(tmp.path(), "account_locked"),
            vec![("u1".to_string(), "login refused".to_string())]
        );
    }

    #[tokio::test]
    async fn consume_ambiguous_dup_fails_closed() {
        let (tmp, app, mail_dir) = file_app();
        seed_customer(tmp.path(), "u1", "dup@example.test", 1, "customer", false);
        roundtrip(
            &app,
            post_json(
                "/apostille/api/v1/auth/magic/start",
                r#"{"email":"dup@example.test"}"#,
            ),
        )
        .await;
        let token = fragment_token(&mail_files(&mail_dir)[0]).expect("link");
        // A second owner appears mid-flight: fail closed, stay separate.
        seed_customer(tmp.path(), "u2", "dup@example.test", 1, "customer", false);
        let (status, body, _) = roundtrip(
            &app,
            post_json(
                "/apostille/api/v1/auth/magic/consume",
                &format!(r#"{{"token":"{token}"}}"#),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::GONE);
        assert_eq!(
            body,
            serde_json::json!({"error": "link invalid or expired"})
        );
        assert_eq!(
            audit_reasons(tmp.path(), "magic_failed"),
            vec![("anonymous".to_string(), "ambiguous".to_string())]
        );
    }

    #[tokio::test]
    async fn consume_staff_account_refused() {
        let (tmp, app, mail_dir) = file_app();
        seed_customer(tmp.path(), "u1", "staff@example.test", 1, "customer", false);
        roundtrip(
            &app,
            post_json(
                "/apostille/api/v1/auth/magic/start",
                r#"{"email":"staff@example.test"}"#,
            ),
        )
        .await;
        let token = fragment_token(&mail_files(&mail_dir)[0]).expect("link");
        let conn = open_store(tmp.path());
        conn.execute("UPDATE account SET role = 'staff' WHERE user_id = 'u1'", [])
            .unwrap();
        drop(conn);
        let (status, _, _) = roundtrip(
            &app,
            post_json(
                "/apostille/api/v1/auth/magic/consume",
                &format!(r#"{{"token":"{token}"}}"#),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::GONE);
        assert_eq!(
            audit_reasons(tmp.path(), "magic_failed"),
            vec![("anonymous".to_string(), "role".to_string())]
        );
    }

    #[tokio::test]
    async fn consume_orphaned_owner_is_410() {
        let (tmp, app, mail_dir) = file_app();
        seed_customer(tmp.path(), "u1", "gone@example.test", 1, "customer", false);
        roundtrip(
            &app,
            post_json(
                "/apostille/api/v1/auth/magic/start",
                r#"{"email":"gone@example.test"}"#,
            ),
        )
        .await;
        let token = fragment_token(&mail_files(&mail_dir)[0]).expect("link");
        let conn = open_store(tmp.path());
        conn.execute(
            "UPDATE identity_link SET disabled_at = 500 WHERE user_id = 'u1'",
            [],
        )
        .unwrap();
        drop(conn);
        let (status, _, _) = roundtrip(
            &app,
            post_json(
                "/apostille/api/v1/auth/magic/consume",
                &format!(r#"{{"token":"{token}"}}"#),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::GONE);
        assert_eq!(
            audit_reasons(tmp.path(), "magic_failed"),
            vec![("anonymous".to_string(), "orphaned".to_string())]
        );
    }

    #[tokio::test]
    async fn consume_enforces_origin() {
        let (_tmp, app, _mail) = file_app();
        let evil = Request::builder()
            .method("POST")
            .uri("/apostille/api/v1/auth/magic/consume")
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::ORIGIN, "https://evil.test")
            .extension(addr())
            .body(Body::from(format!(r#"{{"token":"{}"}}"#, "aa".repeat(32))))
            .unwrap();
        let (status, _, _) = roundtrip(&app, evil).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn fallback_idle_enforced_through_me() {
        let (tmp, app, mail_dir) = file_app();
        seed_customer(tmp.path(), "u1", "idle@example.test", 1, "customer", false);
        roundtrip(
            &app,
            post_json(
                "/apostille/api/v1/auth/magic/start",
                r#"{"email":"idle@example.test"}"#,
            ),
        )
        .await;
        let token = fragment_token(&mail_files(&mail_dir)[0]).expect("link");
        let (status, _, cookies) = roundtrip(
            &app,
            post_json(
                "/apostille/api/v1/auth/magic/consume",
                &format!(r#"{{"token":"{token}"}}"#),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let session = cookies
            .iter()
            .find(|c| c.contains(auth::PUB_COOKIE))
            .and_then(|c| c.split(';').next())
            .expect("session cookie")
            .to_string();
        let me = || {
            Request::builder()
                .method("GET")
                .uri("/apostille/api/v1/auth/me")
                .header(header::COOKIE, session.clone())
                .extension(addr())
                .body(Body::empty())
                .unwrap()
        };
        // Fresh: green.
        let (status, _, _) = roundtrip(&app, me()).await;
        assert_eq!(status, StatusCode::OK);
        // Simulate 61 minutes idle: fallback window (active) binds at 1h.
        let conn = open_store(tmp.path());
        conn.execute(
            "UPDATE session SET last_seen_at = ?1",
            [unix_now() - auth::FALLBACK_IDLE_SECS - 60],
        )
        .unwrap();
        drop(conn);
        let (status, _, _) = roundtrip(&app, me()).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        // Graduation restores the role default: same idle, green again.
        let conn = open_store(tmp.path());
        auth::passkey::clear_fallback(&conn, "u1").unwrap();
        drop(conn);
        let (status, _, _) = roundtrip(&app, me()).await;
        assert_eq!(status, StatusCode::OK);
    }

    #[test]
    fn file_mailer_writes_private_shaped_files() {
        let tmp = tempfile::tempdir().unwrap();
        let mailer = FileMailer {
            dir: tmp.path().to_path_buf(),
        };
        flow::send_magic_link(
            &mailer,
            "someone@example.test",
            "https://example.test/apostille/login",
            &"ab".repeat(32),
        )
        .expect("send");
        let entries = std::fs::read_dir(tmp.path()).unwrap();
        let paths = entries.map(|e| e.unwrap().path()).collect::<Vec<_>>();
        assert_eq!(paths.len(), 1);
        let name = paths[0].file_name().unwrap().to_str().unwrap();
        assert!(name.ends_with(".txt"), "{name}");
        assert!(
            name.contains(&auth::hash_str("someone@example.test")[..16]),
            "{name}"
        );
        let content = std::fs::read_to_string(&paths[0]).unwrap();
        assert!(content.contains("To: someone@example.test"), "{content}");
        assert!(content.contains(flow::MAGIC_SUBJECT), "{content}");
        assert!(fragment_token(&content).is_some(), "{content}");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(&paths[0]).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600, "{mode:o}");
        }
    }

    #[test]
    fn mailer_from_config_shapes() {
        let tmp = tempfile::tempdir().unwrap();
        // Default (no rows): unset → None (fail closed, loud 503).
        let cfg = config::RuntimeConfig::default();
        assert!(mailer_from_config(&cfg, tmp.path()).is_none());
        // Explicit file: works, dir created.
        let cfg = config::RuntimeConfig {
            mailer_provider: "file".to_string(),
            ..Default::default()
        };
        let mailer = mailer_from_config(&cfg, tmp.path()).expect("explicit file");
        assert!(matches!(mailer, Mailer::File(_)));
        assert!(tmp.path().join("mail").is_dir());
        // Unknown provider: None (fail closed).
        let cfg = config::RuntimeConfig {
            mailer_provider: "pigeon".to_string(),
            ..Default::default()
        };
        assert!(mailer_from_config(&cfg, tmp.path()).is_none());
        // SMTP partial: None (fail closed).
        let cfg = config::RuntimeConfig {
            mailer_provider: "smtp".to_string(),
            mailer_host: Some("smtp.example.test".to_string()),
            ..Default::default()
        };
        assert!(mailer_from_config(&cfg, tmp.path()).is_none());
        // SMTP bad port: None.
        let cfg = config::RuntimeConfig {
            mailer_port: Some("not-a-port".to_string()),
            mailer_username: Some("user@example.test".to_string()),
            ..cfg
        };
        assert!(mailer_from_config(&cfg, tmp.path()).is_none());
    }

    #[test]
    fn smtp_mailer_new_validates_without_dialing() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        config::migrate_config_tables(&conn).unwrap();
        let kek = config::Kek::generate();
        config::set_value(
            &conn,
            &kek,
            &config::SetValue {
                version: 1,
                key: config::keys::MAILER_PASSWORD,
                key_type: config::ConfigType::Secret,
                plaintext: b"app-password-stub",
                now: 1000,
                actor: "test",
            },
        )
        .unwrap();
        let (cfg, _) = config::RuntimeConfig::load(&conn, &kek).unwrap();
        let pass = cfg.mailer_password.unwrap();
        // No network happens here: build only.
        assert!(SmtpMailer::new("", 587, "u", pass.clone(), "a@example.test").is_err());
        assert!(SmtpMailer::new("h", 0, "u", pass.clone(), "a@example.test").is_err());
        assert!(SmtpMailer::new("h", 587, "", pass.clone(), "a@example.test").is_err());
        assert!(SmtpMailer::new("h", 587, "u", pass.clone(), "not-an-address").is_err());
        assert!(
            SmtpMailer::new(
                "smtp.example.test",
                587,
                "user@example.test",
                pass,
                "noreply@example.test"
            )
            .is_ok()
        );
    }
}
