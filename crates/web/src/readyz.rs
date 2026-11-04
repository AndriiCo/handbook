//! Readiness: `/readyz` tells the load balancer whether this
//! process may take traffic. Five checks, no secrets in the
//! output (only which check failed):
//!
//! 1. the store opens (a locked or missing DB is not ready);
//! 2. the serving leaf expires more than [`CERT_MIN_DAYS_LEFT`]
//!    days out (no recorded leaf, e.g. plain-HTTP dev boot, is
//!    not ready — traffic must never route to an identity-less
//!    process in production);
//! 3. the KEK is loaded (sealed rows are unreadable otherwise);
//! 4. every instrumented in-process timer ticked recently (today
//!    just the notify worker; the ACME loop's long sleeps are
//!    covered by check 2 instead);
//! 5. the Gmail poll (`apostille-mail.timer`, a separate process)
//!    finished a clean run within [`MAIL_MAX_POLL_AGE_SECS`], read
//!    from `mail_poll` (`last_poll_at` with `last_error` empty: a
//!    held or failed run records its reason there). No row means the
//!    poll never ran here, so mail is disabled, not unready. The body
//!    names the state (`mail`: `ok` / `failing` / `stale` /
//!    `disabled`); the error text itself is staff-only, on the admin
//!    mail page ([`admin_mail`]).
//!
//! Process statics (no `AppState` field): the leaf is
//! re-recorded on every renewal, heartbeats by the loops
//! themselves. Tests pin only long-expired leaf values, so
//! handler tests keep observing the fail-closed shape.

use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use rusqlite::{Connection, OptionalExtension};
use serde_json::json;
use std::collections::HashMap;
use std::sync::{
    Mutex,
    atomic::{AtomicI64, Ordering},
};

use super::passkey;
use super::{AppState, admin::AdminAuth, store_unavailable, unix_now};

/// Refuse traffic this far out from expiry (21 days: inside the
/// ACME renewal window with margin for a failed attempt).
pub const CERT_MIN_DAYS_LEFT: i64 = 21;
/// The notify worker ticks every 60 s; silence past 5× is death.
pub const NOTIFY_MAX_BEAT_AGE_SECS: i64 = 300;
/// The mail poll runs every 15 minutes: three ticks without a clean
/// run is a dead poll (an expired grant, a stopped timer).
pub const MAIL_MAX_POLL_AGE_SECS: i64 = 45 * 60;

/// The `mail_poll` row (the poller's only state).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MailPoll {
    pub mailbox: String,
    pub last_poll_at: i64,
    pub last_error: String,
}

/// Read `mail_poll` (None: the poll never ran on this store).
pub fn mail_poll(conn: &Connection) -> rusqlite::Result<Option<MailPoll>> {
    conn.query_row(
        "SELECT email, last_poll_at, last_error FROM mail_poll WHERE id = 1",
        [],
        |row| {
            Ok(MailPoll {
                mailbox: row.get(0)?,
                last_poll_at: row.get(1)?,
                last_error: row.get(2)?,
            })
        },
    )
    .optional()
}

/// `disabled` (no row), `failing` (the last run recorded an error),
/// `stale` (no clean run within the window), else `ok`.
pub fn mail_state(poll: Option<&MailPoll>, now: i64) -> &'static str {
    match poll {
        None => "disabled",
        Some(p) if !p.last_error.is_empty() => "failing",
        Some(p) if now - p.last_poll_at > MAIL_MAX_POLL_AGE_SECS => "stale",
        Some(_) => "ok",
    }
}

/// Serving leaf expiry as unix seconds (0 = none recorded).
/// Atomic, not first-write-wins: every ACME renewal overwrites it.
/// A stuck old value would 503 the process ~21 days before the OLD
/// leaf expires while a renewed leaf is already serving.
static SERVING_LEAF_NOT_AFTER: AtomicI64 = AtomicI64::new(0);
static HEARTBEATS: std::sync::LazyLock<Mutex<HashMap<&'static str, i64>>> =
    std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));

/// Store one leaf expiry (the overwrite half of
/// [`record_serving_leaf`], factored so tests can pin values
/// without minting certificates).
fn store_leaf_not_after(not_after: i64) {
    SERVING_LEAF_NOT_AFTER.store(not_after, Ordering::SeqCst);
}

/// Record the serving leaf's expiry (boot + every ACME renewal).
/// Every successful parse overwrites; unparsable input keeps the
/// old value — a bad read must never blank a good leaf.
pub fn record_serving_leaf(cert_pem: &[u8]) {
    if let Ok(not_after) = super::mtls::cert_not_after_unix(cert_pem) {
        store_leaf_not_after(not_after);
    }
}

/// Last recorded leaf expiry (None = no leaf recorded yet).
pub fn serving_leaf_not_after() -> Option<i64> {
    match SERVING_LEAF_NOT_AFTER.load(Ordering::SeqCst) {
        0 => None,
        not_after => Some(not_after),
    }
}

/// One loop tick (`last` in unix seconds, 0 = never).
pub fn beat(name: &'static str, now: i64) {
    if let Ok(mut beats) = HEARTBEATS.lock() {
        beats.insert(name, now);
    }
}

fn last_beat(name: &str) -> i64 {
    HEARTBEATS
        .lock()
        .ok()
        .and_then(|beats| beats.get(name).copied())
        .unwrap_or(0)
}

/// Pure assessment (unit-tested): each beat is `(name, last_unix,
/// max_age_secs)`. Missing beats arrive as `last_unix = 0` and fail.
pub struct Assessment {
    pub ready: bool,
    pub database: bool,
    pub tls_certificate: bool,
    pub kek: bool,
    pub timers: Vec<(&'static str, bool)>,
    /// True unless the mail poll is `failing` or `stale`.
    pub mail_poll: bool,
}

/// `mail` is a [`mail_state`] (`disabled` passes).
pub fn assess(
    db_ok: bool,
    cert_not_after: Option<i64>,
    kek_loaded: bool,
    beats: &[(&'static str, i64, i64)],
    mail: &str,
    now: i64,
) -> Assessment {
    let cert_ok = cert_not_after
        .map(|not_after| not_after - now >= CERT_MIN_DAYS_LEFT * 86_400)
        .unwrap_or(false);
    let timers: Vec<(&'static str, bool)> = beats
        .iter()
        .map(|(name, last, max_age)| (*name, now - *last <= *max_age))
        .collect();
    let mail_ok = matches!(mail, "ok" | "disabled");
    let ready = db_ok && cert_ok && kek_loaded && mail_ok && timers.iter().all(|(_, ok)| *ok);
    Assessment {
        ready,
        database: db_ok,
        tls_certificate: cert_ok,
        kek: kek_loaded,
        timers,
        mail_poll: mail_ok,
    }
}

/// `GET /apostille/readyz` — 200 with per-check bools when ready,
/// 503 naming the failed checks otherwise.
pub async fn readyz(State(state): State<AppState>) -> Response {
    let now = unix_now();
    let conn = passkey::open_conn(&state);
    let db_ok = conn
        .as_ref()
        .and_then(|conn| {
            conn.query_row("SELECT 1", [], |row| row.get::<_, i64>(0))
                .ok()
        })
        .is_some();
    // An unreadable mail row on a live database reads as failing.
    let mail = match conn.as_ref().map(mail_poll) {
        Some(Ok(poll)) => mail_state(poll.as_ref(), now),
        _ => "failing",
    };
    drop(conn);
    let leaf = serving_leaf_not_after();
    let beats = [("notify", last_beat("notify"), NOTIFY_MAX_BEAT_AGE_SECS)];
    let verdict = assess(db_ok, leaf, state.kek.is_some(), &beats, mail, now);
    let timers: serde_json::Map<String, serde_json::Value> = verdict
        .timers
        .iter()
        .map(|(name, ok)| (name.to_string(), json!(ok)))
        .collect();
    // Every outcome carries the same per-check body (a dead
    // database reports `database: false` with 503, never a bare
    // 500), so the balancer sees WHICH check failed.
    let body = json!({
        "status": if verdict.ready { "ready" } else { "not-ready" },
        "checks": {
            "database": verdict.database,
            "tls_certificate": verdict.tls_certificate,
            "kek": verdict.kek,
            "timers": timers,
            "mail_poll": verdict.mail_poll,
        },
        "mail": mail,
    });
    if verdict.ready {
        (StatusCode::OK, axum::Json(body)).into_response()
    } else {
        (StatusCode::SERVICE_UNAVAILABLE, axum::Json(body)).into_response()
    }
}

/// `GET /apostille/api/v1/admin/mail` (admin listener, staff): the
/// poll's state, mailbox, last completed poll and last error, plus the
/// overall send budget and the suppression count — everything the
/// admin mail page shows.
pub async fn admin_mail(_auth: AdminAuth, State(state): State<AppState>) -> Response {
    let Some(conn) = passkey::open_conn(&state) else {
        return store_unavailable();
    };
    let now = unix_now();
    let Ok(poll) = mail_poll(&conn) else {
        return store_unavailable();
    };
    // Notify's tables are additive (boot creates them); a store
    // without them has sent and suppressed nothing.
    let sends_today = notify::sends_today(&conn, now).unwrap_or(0);
    let suppressed = notify::suppressed_count(&conn).unwrap_or(0);
    let state_name = mail_state(poll.as_ref(), now);
    let poll = poll.unwrap_or(MailPoll {
        mailbox: String::new(),
        last_poll_at: 0,
        last_error: String::new(),
    });
    axum::Json(json!({
        "state": state_name,
        "mailbox": poll.mailbox,
        "last_poll_at": poll.last_poll_at,
        "last_error": poll.last_error,
        "max_age_secs": MAIL_MAX_POLL_AGE_SECS,
        "now": now,
        "sends_today": sends_today,
        "send_cap": notify::OVERALL_DAILY_SEND_CAP,
        "suppressed": suppressed,
    }))
    .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: i64 = 86_400;

    #[test]
    fn mail_freshness_reads_the_poll_row() {
        let dir = tempfile::tempdir().unwrap();
        let store = handbook_store::store::Store::new(dir.path());
        store.init().unwrap();
        let conn = handbook_store::db::open(&store).unwrap();
        handbook_store::db::migrate(&conn).unwrap();
        let now = 1_800_000_000;
        let state = |conn: &Connection| mail_state(mail_poll(conn).unwrap().as_ref(), now);
        // No row: the poll never ran, mail is off, readiness holds.
        assert_eq!(state(&conn), "disabled");
        // A clean run 10 minutes ago.
        conn.execute(
            "INSERT INTO mail_poll(id, email, history_id, last_poll_at, last_error)
             VALUES (1, 'box@example.com', '100', ?1, '')",
            [now - 600],
        )
        .unwrap();
        assert_eq!(state(&conn), "ok");
        // A failed refresh (the poller records it without moving
        // `last_poll_at`): failing at once, whatever the age.
        conn.execute(
            "UPDATE mail_poll SET last_error = 'mail api: invalid_grant' WHERE id = 1",
            [],
        )
        .unwrap();
        assert_eq!(state(&conn), "failing");
        // Error cleared but no clean run inside the window: stale.
        conn.execute(
            "UPDATE mail_poll SET last_error = '', last_poll_at = ?1 WHERE id = 1",
            [now - MAIL_MAX_POLL_AGE_SECS - 1],
        )
        .unwrap();
        assert_eq!(state(&conn), "stale");
        conn.execute(
            "UPDATE mail_poll SET last_poll_at = ?1 WHERE id = 1",
            [now - MAIL_MAX_POLL_AGE_SECS],
        )
        .unwrap();
        assert_eq!(state(&conn), "ok", "the window's edge still counts");
    }

    #[test]
    fn readiness_fails_on_a_dead_mail_poll_but_not_a_disabled_one() {
        let now = 1_800_000_000;
        let beats = [("notify", now, 300)];
        let leaf = Some(now + 30 * DAY);
        for (mail, ready) in [
            ("ok", true),
            ("disabled", true),
            ("failing", false),
            ("stale", false),
        ] {
            let verdict = assess(true, leaf, true, &beats, mail, now);
            assert_eq!(verdict.ready, ready, "{mail}");
            assert_eq!(verdict.mail_poll, ready, "{mail}");
        }
    }

    #[test]
    fn assess_passes_when_everything_holds() {
        let now = 1_800_000_000;
        let verdict = assess(
            true,
            Some(now + 30 * DAY),
            true,
            &[("notify", now, 300)],
            "ok",
            now,
        );
        assert!(verdict.ready);
        assert!(verdict.timers.iter().all(|(_, ok)| *ok));
    }

    #[test]
    fn renewal_overwrites_the_recorded_leaf() {
        // Long-expired values only: the shared static must never
        // flip the process assessment green from a test (the
        // fail-closed handler test pins `tls_certificate: false`).
        store_leaf_not_after(100);
        assert_eq!(serving_leaf_not_after(), Some(100));
        store_leaf_not_after(200);
        assert_eq!(serving_leaf_not_after(), Some(200));
        // Unparsable input keeps the old value: a bad read never
        // blanks a good leaf.
        record_serving_leaf(b"not a pem file");
        assert_eq!(serving_leaf_not_after(), Some(200));
    }

    #[test]
    fn assess_fails_each_check_independently() {
        let now = 1_800_000_000;
        // Boundary: exactly 21 days still counts.
        let boundary = assess(
            true,
            Some(now + 21 * DAY),
            true,
            &[("notify", now, 300)],
            "disabled",
            now,
        );
        assert!(boundary.ready, "21 days is the floor, not past it");
        for (db, cert, kek, last, want) in [
            (false, Some(now + 30 * DAY), true, now, "database"),
            (true, None, true, now, "missing leaf"),
            (true, Some(now + 20 * DAY), true, now, "expiring leaf"),
            (true, Some(now + 30 * DAY), false, now, "kek"),
            (true, Some(now + 30 * DAY), true, now - 301, "stale beat"),
            (true, Some(now + 30 * DAY), true, 0, "never beat"),
        ] {
            let verdict = assess(db, cert, kek, &[("notify", last, 300)], "ok", now);
            assert!(!verdict.ready, "{want} must fail readiness");
        }
    }
}
