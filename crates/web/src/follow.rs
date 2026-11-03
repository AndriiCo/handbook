//! Follower notifications, web tier (phase 1 email): public
//! subscribe / confirm / unsubscribe plus the in-process worker loop
//! (the ACME-renewal precedent: a spawned task, not a new unit).
//! Routes never touch the outbox; only the worker leases and sends.

use axum::{
    Json,
    extract::{ConnectInfo, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use serde_json::json;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use rusqlite::OptionalExtension as _;

use super::magic::Mailer;
use super::passkey;
use super::{AppState, store_unavailable, unix_now};

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

fn unavailable() -> Response {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(json!({"error": "follow updates unavailable"})),
    )
        .into_response()
}

fn valid_token(raw: &str) -> Option<String> {
    let token = raw.trim().to_string();
    (token.len() == 64 && token.bytes().all(|b| b.is_ascii_hexdigit())).then_some(token)
}

/// `GET /api/v1/notify/status`: the master switch as public
/// data (the dossier hides its follow card while off — production
/// runs off until the operator enables it). Static bool, no DB, no
/// limits, no oracle.
pub async fn status(State(state): State<AppState>) -> axum::Json<serde_json::Value> {
    axum::Json(serde_json::json!({ "enabled": state.notify_enabled }))
}

#[derive(serde::Deserialize)]
pub struct SubscribeBody {
    pub authority: String,
    pub email: String,
}

/// `POST /api/v1/notify/subscribe`: double-opt-in step one. Always
/// 200 for well-formed mail to a known authority (no oracle for
/// who follows what); the confirm mail goes out only when a mailer
/// is configured and the target is not suppressed.
pub async fn subscribe(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(body): Json<SubscribeBody>,
) -> Response {
    let now = unix_now();
    if !passkey::origin_ok(&headers, state.admin_mtls_off) {
        return passkey::csrf_fail();
    }
    let ip = passkey::client_ip(addr);
    if !state.notify_limits.check(&format!("nf:{ip}"), now, 10, 60) {
        return rate_limited();
    }
    if !state.notify_enabled {
        return unavailable();
    }
    let Some(email) = passkey::valid_email(&body.email) else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "email malformed"})),
        )
            .into_response();
    };
    let hash = auth::hash_str(&email);
    if !state
        .notify_limits
        .check(&format!("nf:{}", &hash[..16]), now, 3, 3600)
    {
        return rate_limited();
    }
    let Some(conn) = passkey::open_conn(&state) else {
        return store_unavailable();
    };
    let slug = body.authority.trim().to_lowercase();
    let row: Option<(i64, String)> = match conn
        .query_row(
            "SELECT id, name FROM authority WHERE slug = ?1",
            [&slug],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
    {
        Ok(row) => row,
        Err(_) => return store_unavailable(),
    };
    let Some((_authority_id, authority_name)) = row else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "unknown authority"})),
        )
            .into_response();
    };
    // Suppressed targets (bounces, complaints) stay silent: same
    // 200, no row, no mail — never an oracle.
    if notify::is_suppressed(&conn, &hash).unwrap_or(false) {
        return (StatusCode::OK, Json(json!({"ok": true}))).into_response();
    }
    let Some(kek) = state.kek.clone() else {
        return store_unavailable();
    };
    let outcome = match notify::subscribe(&conn, now, &slug, &email, &kek) {
        Ok(outcome) => outcome,
        Err(_) => return store_unavailable(),
    };
    if let Some(token) = outcome.confirm_token {
        let Some(mailer) = state.mailer.clone() else {
            return unavailable();
        };
        let url = format!("{}/notify/confirm#t={token}", state.notify_base);
        let (subject, text) = notify::build_confirm(&authority_name, &url, &state.notify_postal);
        // Fire-and-forget off the async thread: a new address (mail
        // queued) answers as fast as an already-active one (nothing
        // to send). Send failures never surface per-address (no
        // oracle) and never log the address.
        tokio::task::spawn_blocking(move || {
            use auth::magic::Mailer as _;
            if mailer.send(&email, &subject, &text).is_err() {
                eprintln!("notify: confirm mail failed");
            }
        });
    }
    (StatusCode::OK, Json(json!({"ok": true}))).into_response()
}

#[derive(serde::Deserialize)]
pub struct TokenBody {
    pub token: String,
}

#[derive(serde::Deserialize)]
pub struct TokenQuery {
    pub token: Option<String>,
}

fn confirm_token(state: &AppState, raw: &str) -> Response {
    let now = unix_now();
    let Some(token) = valid_token(raw) else {
        return gone();
    };
    if !state.notify_enabled {
        return unavailable();
    }
    let Some(conn) = passkey::open_conn(state) else {
        return store_unavailable();
    };
    match notify::confirm(&conn, now, &token) {
        Ok(Some(_)) => (StatusCode::OK, Json(json!({"ok": true}))).into_response(),
        Ok(None) => gone(),
        Err(_) => store_unavailable(),
    }
}

/// `POST /api/v1/notify/confirm` (the confirm page POSTs the
/// fragment token as JSON, the magic-link shape).
pub async fn confirm_post(State(state): State<AppState>, Json(body): Json<TokenBody>) -> Response {
    confirm_token(&state, &body.token)
}

/// `GET /api/v1/notify/confirm?token=` (manual clients; the mailed
/// link itself carries the token in the fragment, never in a GET).
pub async fn confirm_get(
    State(state): State<AppState>,
    Query(query): Query<TokenQuery>,
) -> Response {
    let Some(token) = query.token else {
        return gone();
    };
    confirm_token(&state, &token)
}

fn html(status: StatusCode, body: String) -> Response {
    (
        status,
        [(axum::http::header::CONTENT_TYPE, "text/html; charset=utf-8")],
        body,
    )
        .into_response()
}

/// The RFC 8058 confirm form: GET never unsubscribes — the
/// mailbox provider (or the reader's click) POSTs this back.
/// `token` is 64-hex by construction, safe to embed unescaped.
fn unsub_form(token: &str) -> String {
    format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
         <title>Unsubscribe — Apostille Handbook</title></head>\
         <body><main><h1>Unsubscribe from updates</h1>\
         <p>Press the button below to stop update emails for this subscription.</p>\
         <form method=\"post\" action=\"?token={token}\">\
         <input type=\"hidden\" name=\"List-Unsubscribe\" value=\"One-Click\">\
         <button type=\"submit\">Unsubscribe</button></form>\
         </main></body></html>"
    )
}

fn unsub_gone_page() -> String {
    "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
     <title>Unsubscribe — Apostille Handbook</title></head>\
     <body><main><h1>Unsubscribe from updates</h1>\
     <p>This unsubscribe link is invalid or expired. Open the one-click \
     link from a recent update email, or follow the office again from \
     its dossier page for a fresh link.</p>\
     </main></body></html>"
        .to_string()
}

/// `POST /api/v1/notify/unsubscribe`: JSON `{token}` (the page) or
/// RFC 8058 one-click (`?token=` + `List-Unsubscribe=One-Click`
/// body). Idempotent 200 for a known token, 410 past it.
pub async fn unsubscribe_post(
    State(state): State<AppState>,
    Query(query): Query<TokenQuery>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let is_json = headers
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.starts_with("application/json"));
    let mut raw = query.token;
    if is_json
        && !body.is_empty()
        && let Ok(parsed) = serde_json::from_slice::<TokenBody>(&body)
    {
        raw = Some(parsed.token);
    }
    let Some(raw) = raw else {
        return gone();
    };
    let Some(token) = valid_token(&raw) else {
        return gone();
    };
    // Unsubscribe stays live even when the master switch is off
    // (consent withdrawal must never depend on a feature flag).
    let Some(conn) = passkey::open_conn(&state) else {
        return store_unavailable();
    };
    match notify::unsubscribe(&conn, &token) {
        Ok(true) => (StatusCode::OK, Json(json!({"ok": true}))).into_response(),
        Ok(false) => gone(),
        Err(_) => store_unavailable(),
    }
}

/// `GET /api/v1/notify/unsubscribe?token=` (the List-Unsubscribe
/// URL): never unsubscribes on GET — serves the RFC 8058 confirm
/// form for a known token, a 410 page past it.
pub async fn unsubscribe_get(
    State(state): State<AppState>,
    Query(query): Query<TokenQuery>,
) -> Response {
    let known = match query.token.as_deref().map(valid_token) {
        Some(Some(token)) => match passkey::open_conn(&state) {
            Some(conn) => match notify::unsub_known(&conn, &token) {
                Ok(known) => known,
                Err(_) => return store_unavailable(),
            },
            None => return store_unavailable(),
        },
        _ => false,
    };
    if known {
        // Safe: `valid_token` admitted 64 hex digits only.
        let token = query.token.unwrap_or_default();
        html(StatusCode::OK, unsub_form(&token))
    } else {
        html(StatusCode::GONE, unsub_gone_page())
    }
}

/// Owned worker inputs (cloned per pass; the `Connection` itself is
/// opened inside the blocking pass — rusqlite is `!Send`).
#[derive(Clone)]
pub struct WorkerCfg {
    pub store_root: PathBuf,
    pub enabled: bool,
    pub daily_cap: u64,
    pub postal: String,
    pub mailer: Option<Mailer>,
    pub kek: Option<Arc<config::Kek>>,
    /// `{origin}/apostille` (confirm/unsubscribe links).
    pub base: String,
    /// Bare public origin (`build_digest` adds `/apostille/authority/…`
    /// itself — passing `base` here doubled the segment on staging).
    pub origin: String,
}

/// One worker pass: tail the audit, then lease and send due
/// digests. Never panics, never logs an address.
pub fn run_pass(cfg: &WorkerCfg) {
    run_pass_at(cfg, unix_now());
}

/// [`run_pass`] at a fixed clock (tests pin an old `now` so the
/// liveness beat stays stale and never flips another test's
/// readyz sample; production always passes the wall clock).
fn run_pass_at(cfg: &WorkerCfg, now: i64) {
    let store = handbook_store::store::Store::new(&cfg.store_root);
    let Ok(conn) = handbook_store::db::open(&store) else {
        return;
    };
    // The tail always advances (even while disabled), so enabling
    // later never replays ancient accepts at full volume — the
    // digest window and daily cap bound the catch-up further.
    if notify::enqueue_from_audit(&conn, now).is_err() {
        return;
    }
    // Liveness for `/readyz`: every pass beats, sends or not.
    super::readyz::beat("notify", now);
    let Some(mailer) = cfg.mailer.clone() else {
        return;
    };
    if !cfg.enabled || cfg.postal.trim().is_empty() {
        return;
    }
    let Some(kek) = cfg.kek.clone() else {
        return;
    };
    let groups = match notify::lease_due_groups(&conn, now, cfg.daily_cap, 25) {
        Ok(groups) => groups,
        Err(_) => return,
    };
    for group in &groups {
        let mut facts = Vec::new();
        for fact_id in &group.fact_ids {
            match notify::load_fact(&conn, &cfg.store_root, *fact_id) {
                Ok(Some(fact)) => facts.push(fact),
                Ok(None) => {}
                Err(_) => {}
            }
        }
        // Fact vanished mid-lease: drop the rows, never retry them.
        if facts.is_empty() {
            let _ = notify::mark_sent(&conn, now, &group.row_ids);
            continue;
        }
        // One stable token per subscription (no per-send rotation):
        // the footer URL is the RFC 8058 endpoint itself, so the
        // List-Unsubscribe header and the GET form agree.
        let unsub_url = match notify::stable_unsub_token(&conn, &kek, group.sub_id) {
            Ok(token) => format!("{}/api/v1/notify/unsubscribe?token={token}", cfg.base),
            Err(_) => {
                let _ = notify::mark_failed(&conn, unix_now(), &group.row_ids);
                continue;
            }
        };
        let (subject, text) = notify::build_digest(&facts, &cfg.origin, &unsub_url, &cfg.postal);
        let to = match notify::open_email(&kek, &group.email_blob) {
            Ok(to) => to,
            Err(_) => {
                let _ = notify::mark_failed(&conn, unix_now(), &group.row_ids);
                continue;
            }
        };
        let list_unsub = format!("<{unsub_url}>");
        let headers = [
            ("List-Unsubscribe", list_unsub.as_str()),
            ("List-Unsubscribe-Post", "List-Unsubscribe=One-Click"),
        ];
        match mailer.send_with_headers(&to, &subject, &text, &headers) {
            Ok(()) => {
                // Fresh stamp per email (not the pass-start `now`):
                // the daily cap counts distinct sends, so two digests
                // in one second still count as two emails.
                let _ = notify::mark_sent(&conn, unix_now(), &group.row_ids);
            }
            Err(_) => {
                let _ = notify::mark_failed(&conn, unix_now(), &group.row_ids);
            }
        }
    }
}

/// Spawn the follow worker (the ACME-renewal precedent): poke-woken
/// plus a 60 s tick for backoff rows, each pass on a blocking
/// thread (rusqlite is `!Send`).
pub fn spawn_worker(cfg: WorkerCfg, poke: Arc<tokio::sync::Notify>) {
    tokio::spawn(async move {
        // First pass runs at boot, not after the first 60 s tick:
        // the pass beats (on a healthy store), so `/readyz` clears
        // its timer check within seconds of every boot.
        run_pass_blocking(&cfg).await;
        loop {
            tokio::select! {
                _ = poke.notified() => {},
                _ = tokio::time::sleep(std::time::Duration::from_secs(60)) => {},
            }
            run_pass_blocking(&cfg).await;
        }
    });
}

/// One [`run_pass`] off the async runtime.
async fn run_pass_blocking(cfg: &WorkerCfg) {
    let step = cfg.clone();
    let _ = tokio::task::spawn_blocking(move || run_pass(&step)).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, header};
    use tower::ServiceExt as _;

    fn test_addr() -> ConnectInfo<SocketAddr> {
        ConnectInfo("127.0.0.1:9".parse().unwrap())
    }

    fn open_store(root: &std::path::Path) -> rusqlite::Connection {
        let store = handbook_store::store::Store::new(root);
        handbook_store::db::open(&store).unwrap()
    }

    struct Harness {
        _tmp: tempfile::TempDir,
        app: axum::Router,
        store_root: std::path::PathBuf,
        kek: Arc<config::Kek>,
    }

    fn setup_root() -> (tempfile::TempDir, std::path::PathBuf, Arc<config::Kek>) {
        let tmp = tempfile::tempdir().unwrap();
        let mail_dir = tmp.path().join("mail");
        std::fs::create_dir_all(&mail_dir).unwrap();
        let conn = open_store(tmp.path());
        handbook_store::db::migrate(&conn).unwrap();
        auth::migrate_auth_tables(&conn).unwrap();
        config::migrate_config_tables(&conn).unwrap();
        notify::migrate(&conn).unwrap();
        handbook_store::db::add_authority(
            &conn,
            &handbook_store::db::NewAuthority {
                kind: "state",
                country: "USA",
                region: "Washington",
                name: "Viet Nam",
                slug: "vietnam",
                actor: "test",
                at: 1_700_000_000,
            },
        )
        .unwrap();
        drop(conn);
        (tmp, mail_dir, Arc::new(config::Kek::generate()))
    }

    fn build_app(
        store_root: &std::path::Path,
        mailer: Option<Mailer>,
        kek: &Arc<config::Kek>,
    ) -> axum::Router {
        let dir = store_root.to_str().unwrap().to_string();
        let state = AppState {
            spa_index: std::path::Path::new(&dir).join("index.html"),
            static_dir: PathBuf::from(&dir),
            store_root: store_root.to_path_buf(),
            bus: crate::ws_bus::Bus::new(),
            kek: Some(kek.clone()),
            admin_mtls_off: true,
            admin_allow_ips: config::loopback_allow_ips(),
            admin_spa_dir: PathBuf::from("test-admin-spa"),
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
            mailer,
            magic_link_base: "https://example.test/apostille/login".to_string(),
            notify_poke: std::sync::Arc::new(tokio::sync::Notify::new()),
            notify_enabled: true,
            notify_daily_cap: 3,
            notify_postal: "Test Postal, 1 Test Way".to_string(),
            notify_limits: std::sync::Arc::new(passkey::RateBuckets::default()),
            notify_base: "https://example.test/apostille".to_string(),
            meta_delete_status_base: "https://example.test/auth/meta/delete-status".to_string(),
            fedex_live: crate::fedex_live::FedexLiveState::default(),
        };
        crate::public_router(state, &dir, &dir, &dir)
    }

    fn harness(mailer: Option<Mailer>) -> Harness {
        let (tmp, _mail_dir, kek) = setup_root();
        let store_root = tmp.path().to_path_buf();
        let app = build_app(&store_root, mailer, &kek);
        Harness {
            _tmp: tmp,
            app,
            store_root,
            kek,
        }
    }

    fn post_json(uri: &str, body: &str) -> Request<Body> {
        Request::builder()
            .method("POST")
            .uri(uri)
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::ORIGIN, "http://localhost")
            .extension(test_addr())
            .body(Body::from(body.to_string()))
            .unwrap()
    }

    async fn status_and_text(res: Response) -> (StatusCode, String) {
        let status = res.status();
        let bytes = axum::body::to_bytes(res.into_body(), 1024 * 1024)
            .await
            .unwrap();
        (status, String::from_utf8_lossy(&bytes).to_string())
    }

    fn mail_files(dir: &std::path::Path) -> Vec<String> {
        let mut out: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| std::fs::read_to_string(e.path()).unwrap())
            .collect();
        out.sort();
        out
    }

    /// The 64-hex token after a marker inside a mail file.
    fn token_after(content: &str, marker: &str) -> String {
        let at = content.find(marker).expect("marker in mail") + marker.len();
        content[at..at + 64].to_string()
    }

    #[tokio::test]
    async fn unsubscribe_post_is_idempotent_and_410s_unknown() {
        let h = harness(None);
        let conn = open_store(&h.store_root);
        let out = notify::subscribe(&conn, 1_700_000_100, "vietnam", "a@example.com", &h.kek)
            .expect("subscribe");
        let confirm_token = out.confirm_token.expect("confirm token");
        notify::confirm(&conn, 1_700_000_101, &confirm_token).expect("confirm");
        let token = notify::stable_unsub_token(&conn, &h.kek, out.sub_id).expect("stable");
        drop(conn);
        let uri = "/apostille/api/v1/notify/unsubscribe";
        let body = serde_json::json!({"token": token}).to_string();

        // GET first: the RFC 8058 form, and GET alone changes nothing.
        let get = Request::builder()
            .uri(format!("{uri}?token={token}"))
            .body(Body::empty())
            .unwrap();
        let (status, text) = status_and_text(h.app.clone().oneshot(get).await.unwrap()).await;
        assert_eq!(status, StatusCode::OK);
        assert!(text.contains("<form"), "GET serves the confirm form");
        assert!(text.contains("List-Unsubscribe"), "one-click marker");
        let conn = open_store(&h.store_root);
        let sub_state: String = conn
            .query_row(
                "SELECT status FROM notify_subscription WHERE id = ?1",
                [out.sub_id],
                |r| r.get(0),
            )
            .unwrap();
        drop(conn);
        assert_eq!(sub_state, "active", "GET never unsubscribes");

        // POST twice: idempotent 200.
        for _ in 0..2 {
            let (status, text) =
                status_and_text(h.app.clone().oneshot(post_json(uri, &body)).await.unwrap()).await;
            assert_eq!(status, StatusCode::OK);
            assert!(text.contains("\"ok\":true"));
        }
        // Unknown token, malformed token, missing token: 410.
        let unknown = notify::mint_token();
        for bad in [
            serde_json::json!({"token": unknown}).to_string(),
            serde_json::json!({"token": "not-hex"}).to_string(),
        ] {
            let (status, _) =
                status_and_text(h.app.clone().oneshot(post_json(uri, &bad)).await.unwrap()).await;
            assert_eq!(status, StatusCode::GONE, "unknown token 410s");
        }
        let empty = Request::builder()
            .method("POST")
            .uri(uri)
            .body(Body::empty())
            .unwrap();
        let (status, _) = status_and_text(h.app.clone().oneshot(empty).await.unwrap()).await;
        assert_eq!(status, StatusCode::GONE, "missing token 410s");
        // GET past an unknown token: the 410 page says so plainly.
        let get = Request::builder()
            .uri(format!("{uri}?token={unknown}"))
            .body(Body::empty())
            .unwrap();
        let (status, text) = status_and_text(h.app.clone().oneshot(get).await.unwrap()).await;
        assert_eq!(status, StatusCode::GONE);
        assert!(text.contains("invalid or expired"), "clear page message");
    }

    #[tokio::test]
    async fn subscribe_answers_200_without_waiting_for_mail() {
        // Mailer whose writes always fail: the response must not
        // reflect the send outcome (mail left the request path).
        let (tmp, _mail_dir, kek) = setup_root();
        let store_root = tmp.path().to_path_buf();
        let app = build_app(
            &store_root,
            Some(Mailer::File(crate::magic::FileMailer {
                dir: store_root.join("no-such-mail-dir"),
            })),
            &kek,
        );
        let uri = "/apostille/api/v1/notify/subscribe";
        // New address (mail attempted in background) …
        let (status, new_text) = status_and_text(
            app.clone()
                .oneshot(post_json(
                    uri,
                    r#"{"authority":"vietnam","email":"new@example.com"}"#,
                ))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        // … and an already-active address answer alike.
        let conn = open_store(&store_root);
        let out = notify::subscribe(&conn, 1_700_000_100, "vietnam", "old@example.com", &kek)
            .expect("seed");
        notify::confirm(&conn, 1_700_000_101, &out.confirm_token.unwrap()).expect("confirm");
        drop(conn);
        let (status, old_text) = status_and_text(
            app.clone()
                .oneshot(post_json(
                    uri,
                    r#"{"authority":"vietnam","email":"old@example.com"}"#,
                ))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(new_text, old_text, "parity shape");
        assert!(new_text.contains("\"ok\":true"));
    }

    async fn poll_mail(dir: &std::path::Path, at_least: usize) -> Vec<String> {
        for _ in 0..100 {
            let mails = mail_files(dir);
            if mails.len() >= at_least {
                return mails;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        mail_files(dir)
    }

    #[tokio::test]
    async fn file_mailer_end_to_end_subscribe_confirm_digest_unsubscribe() {
        let (tmp, mail_dir, kek) = setup_root();
        let store_root = tmp.path().to_path_buf();
        let mailer = Mailer::File(crate::magic::FileMailer {
            dir: mail_dir.clone(),
        });
        let app = build_app(&store_root, Some(mailer.clone()), &kek);

        // subscribe → 200; the confirm mail lands in the file mailer.
        let (status, _) = status_and_text(
            app.clone()
                .oneshot(post_json(
                    "/apostille/api/v1/notify/subscribe",
                    r#"{"authority":"vietnam","email":"reader@example.com"}"#,
                ))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let mails = poll_mail(&mail_dir, 1).await;
        assert_eq!(mails.len(), 1, "confirm mail sent");
        let confirm_token = token_after(&mails[0], "#t=");

        // confirm → 200.
        let (status, _) = status_and_text(
            app.clone()
                .oneshot(post_json(
                    "/apostille/api/v1/notify/confirm",
                    &serde_json::json!({"token": confirm_token}).to_string(),
                ))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        // Staff accepts a fact; the worker sends the digest.
        let conn = open_store(&store_root);
        conn.execute(
            "INSERT INTO fact(authority_id, capture_id, topic, statement, status,
                              created_by, created_at)
             VALUES (1, 0, 'fees', 'eight dollars', 'accepted', 'test', 1_700_000_200)",
            [],
        )
        .unwrap();
        let fact_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO audit(at, actor, action, entity, entity_id, reason)
             VALUES (1_700_000_200, 'test', 'fact_accept', 'fact', ?1, '')",
            [fact_id],
        )
        .unwrap();
        drop(conn);
        let cfg = WorkerCfg {
            store_root: store_root.clone(),
            enabled: true,
            daily_cap: 3,
            postal: "Test Postal, 1 Test Way".to_string(),
            mailer: Some(mailer),
            kek: Some(kek.clone()),
            base: "https://example.test/apostille".to_string(),
            origin: "https://example.test".to_string(),
        };
        // Pinned old clock: exercises the whole pass while the
        // liveness beat stays stale (see `run_pass_at`).
        run_pass_at(&cfg, 1_700_000_250);
        let mails = poll_mail(&mail_dir, 2).await;
        assert_eq!(mails.len(), 2, "confirm + digest");
        let digest = mails
            .iter()
            .find(|m| m.contains("List-Unsubscribe:"))
            .expect("digest carries the RFC 8058 pair");
        assert!(
            digest.contains("List-Unsubscribe-Post: List-Unsubscribe=One-Click"),
            "one-click marker"
        );
        assert!(
            digest.contains("/apostille/api/v1/notify/unsubscribe?token="),
            "footer points at the API endpoint"
        );
        let unsub_token = token_after(digest, "token=");
        // Stable: the footer token is the subscription's token.
        let conn = open_store(&store_root);
        let sub_id: i64 = conn
            .query_row(
                "SELECT id FROM notify_subscription WHERE authority_slug = 'vietnam'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            notify::stable_unsub_token(&conn, &kek, sub_id).expect("stable"),
            unsub_token
        );
        drop(conn);

        // GET the footer URL: form only, still subscribed.
        let footer = format!("/apostille/api/v1/notify/unsubscribe?token={unsub_token}");
        let get = Request::builder().uri(&footer).body(Body::empty()).unwrap();
        let (status, text) = status_and_text(app.clone().oneshot(get).await.unwrap()).await;
        assert_eq!(status, StatusCode::OK);
        assert!(text.contains("<form"));
        // RFC 8058 one-click POST unsubscribes …
        let click = Request::builder()
            .method("POST")
            .uri(&footer)
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(Body::from("List-Unsubscribe=One-Click"))
            .unwrap();
        let (status, text) = status_and_text(app.clone().oneshot(click).await.unwrap()).await;
        assert_eq!(status, StatusCode::OK);
        assert!(text.contains("\"ok\":true"));
        // … and repeats stay 200 (idempotent).
        let (status, _) = status_and_text(
            app.clone()
                .oneshot(post_json(
                    "/apostille/api/v1/notify/unsubscribe",
                    &serde_json::json!({"token": unsub_token}).to_string(),
                ))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let conn = open_store(&store_root);
        let sub_state: String = conn
            .query_row(
                "SELECT status FROM notify_subscription WHERE id = ?1",
                [sub_id],
                |r| r.get(0),
            )
            .unwrap();
        drop(conn);
        assert_eq!(sub_state, "unsubscribed");
        // A later accept sends nothing more.
        let conn = open_store(&store_root);
        conn.execute(
            "INSERT INTO fact(authority_id, capture_id, topic, statement, status,
                              created_by, created_at)
             VALUES (1, 0, 'fees', 'nine dollars', 'accepted', 'test', 1_700_000_300)",
            [],
        )
        .unwrap();
        let fact_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO audit(at, actor, action, entity, entity_id, reason)
             VALUES (1_700_000_300, 'test', 'fact_accept', 'fact', ?1, '')",
            [fact_id],
        )
        .unwrap();
        drop(conn);
        run_pass_at(&cfg, 1_700_000_350);
        assert_eq!(mail_files(&mail_dir).len(), 2, "no mail after unsubscribe");
    }
}
