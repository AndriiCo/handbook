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
        use auth::magic::Mailer as _;
        if mailer.send(&email, &subject, &text).is_err() {
            return unavailable();
        }
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

fn unsubscribe_token(state: &AppState, raw: &str) -> Response {
    let Some(token) = valid_token(raw) else {
        // Unknown shapes still 200: the token space is unguessable,
        // and a 404 would oracle link validity to scanners.
        return (StatusCode::OK, Json(json!({"ok": true}))).into_response();
    };
    // Unsubscribe stays live even when the master switch is off
    // (consent withdrawal must never depend on a feature flag).
    let Some(conn) = passkey::open_conn(state) else {
        return store_unavailable();
    };
    match notify::unsubscribe(&conn, &token) {
        Ok(_) => (StatusCode::OK, Json(json!({"ok": true}))).into_response(),
        Err(_) => store_unavailable(),
    }
}

/// `POST /api/v1/notify/unsubscribe` (+ RFC 8058 `List-Unsubscribe-Post`).
pub async fn unsubscribe_post(
    State(state): State<AppState>,
    Json(body): Json<TokenBody>,
) -> Response {
    unsubscribe_token(&state, &body.token)
}

/// `GET /api/v1/notify/unsubscribe?token=` (one-click footer URL).
pub async fn unsubscribe_get(
    State(state): State<AppState>,
    Query(query): Query<TokenQuery>,
) -> Response {
    let Some(token) = query.token else {
        return (StatusCode::OK, Json(json!({"ok": true}))).into_response();
    };
    unsubscribe_token(&state, &token)
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
    /// Bare `{origin}` (`build_digest` adds `/apostille/authority/…`
    /// itself — passing `base` here doubled the segment on staging).
    pub origin: String,
}

/// One worker pass: tail the audit, then lease and send due
/// digests. Never panics, never logs an address.
pub fn run_pass(cfg: &WorkerCfg) {
    let now = unix_now();
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
        let unsub_url = match notify::rotate_unsub(&conn, group.sub_id) {
            Ok(token) => format!("{}/notify/unsubscribe?t={token}", cfg.base),
            Err(_) => {
                let _ = notify::mark_failed(&conn, now, &group.row_ids);
                continue;
            }
        };
        let (subject, text) = notify::build_digest(&facts, &cfg.origin, &unsub_url, &cfg.postal);
        let to = match notify::open_email(&kek, &group.email_blob) {
            Ok(to) => to,
            Err(_) => {
                let _ = notify::mark_failed(&conn, now, &group.row_ids);
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
                let _ = notify::mark_sent(&conn, now, &group.row_ids);
            }
            Err(_) => {
                let _ = notify::mark_failed(&conn, now, &group.row_ids);
            }
        }
    }
}

/// Spawn the follow worker (the ACME-renewal precedent): poke-woken
/// plus a 60 s tick for backoff rows, each pass on a blocking
/// thread (rusqlite is `!Send`).
pub fn spawn_worker(cfg: WorkerCfg, poke: Arc<tokio::sync::Notify>) {
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = poke.notified() => {},
                _ = tokio::time::sleep(std::time::Duration::from_secs(60)) => {},
            }
            let step = cfg.clone();
            let _ = tokio::task::spawn_blocking(move || run_pass(&step)).await;
        }
    });
}
