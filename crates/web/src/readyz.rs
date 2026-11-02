//! Readiness: `/readyz` tells the load balancer whether this
//! process may take traffic. Four checks, no secrets in the
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
//!    covered by check 2 instead).
//!
//! Process statics (no `AppState` field): the leaf is recorded
//! once per serving pair, heartbeats by the loops themselves.
//! Tests never set them, so handler tests observe the
//! fail-closed shape hermetically.

use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde_json::json;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use super::passkey;
use super::{AppState, store_unavailable, unix_now};

/// Refuse traffic this far out from expiry (21 days: inside the
/// ACME renewal window with margin for a failed attempt).
pub const CERT_MIN_DAYS_LEFT: i64 = 21;
/// The notify worker ticks every 60 s; silence past 5× is death.
pub const NOTIFY_MAX_BEAT_AGE_SECS: i64 = 300;

static SERVING_LEAF_NOT_AFTER: OnceLock<Option<i64>> = OnceLock::new();
static HEARTBEATS: std::sync::LazyLock<Mutex<HashMap<&'static str, i64>>> =
    std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));

/// Record the serving leaf's expiry (boot + every ACME renewal).
/// First write wins per process; a renewal is always later, and a
/// process never serves two pairs.
pub fn record_serving_leaf(cert_pem: &[u8]) {
    let not_after = super::mtls::cert_not_after_unix(cert_pem).ok();
    let _ = SERVING_LEAF_NOT_AFTER.set(not_after);
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
}

pub fn assess(
    db_ok: bool,
    cert_not_after: Option<i64>,
    kek_loaded: bool,
    beats: &[(&'static str, i64, i64)],
    now: i64,
) -> Assessment {
    let cert_ok = cert_not_after
        .map(|not_after| not_after - now >= CERT_MIN_DAYS_LEFT * 86_400)
        .unwrap_or(false);
    let timers: Vec<(&'static str, bool)> = beats
        .iter()
        .map(|(name, last, max_age)| (*name, now - *last <= *max_age))
        .collect();
    let ready = db_ok && cert_ok && kek_loaded && timers.iter().all(|(_, ok)| *ok);
    Assessment {
        ready,
        database: db_ok,
        tls_certificate: cert_ok,
        kek: kek_loaded,
        timers,
    }
}

/// `GET /apostille/readyz` — 200 with per-check bools when ready,
/// 503 naming the failed checks otherwise.
pub async fn readyz(State(state): State<AppState>) -> Response {
    let now = unix_now();
    let db_ok = passkey::open_conn(&state)
        .and_then(|conn| {
            conn.query_row("SELECT 1", [], |row| row.get::<_, i64>(0))
                .ok()
        })
        .is_some();
    let leaf = SERVING_LEAF_NOT_AFTER.get().copied().flatten();
    let beats = [("notify", last_beat("notify"), NOTIFY_MAX_BEAT_AGE_SECS)];
    let verdict = assess(db_ok, leaf, state.kek.is_some(), &beats, now);
    let timers: serde_json::Map<String, serde_json::Value> = verdict
        .timers
        .iter()
        .map(|(name, ok)| (name.to_string(), json!(ok)))
        .collect();
    let body = json!({
        "status": if verdict.ready { "ready" } else { "not-ready" },
        "checks": {
            "database": verdict.database,
            "tls_certificate": verdict.tls_certificate,
            "kek": verdict.kek,
            "timers": timers,
        },
    });
    if !db_ok {
        return store_unavailable();
    }
    if verdict.ready {
        (StatusCode::OK, axum::Json(body)).into_response()
    } else {
        (StatusCode::SERVICE_UNAVAILABLE, axum::Json(body)).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: i64 = 86_400;

    #[test]
    fn assess_passes_when_everything_holds() {
        let now = 1_800_000_000;
        let verdict = assess(
            true,
            Some(now + 30 * DAY),
            true,
            &[("notify", now, 300)],
            now,
        );
        assert!(verdict.ready);
        assert!(verdict.timers.iter().all(|(_, ok)| *ok));
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
            let verdict = assess(db, cert, kek, &[("notify", last, 300)], now);
            assert!(!verdict.ready, "{want} must fail readiness");
        }
    }
}
