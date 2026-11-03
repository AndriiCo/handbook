//! One `model_call` row per Meta HTTP attempt (model, pipeline,
//! latency, TTFT, input/output/cached/reasoning tokens, outcome); daily
//! caps count these rows; p50/p95 feed the trace console and dashboard.
//!
//! Owner: lane SA-22 plan (`docs/lanes/sa-22.md`). M1 landed by the
//! builder (P319) because the lane session could not start yet; the
//! lane owns this file from here.

use std::path::PathBuf;
use std::sync::Arc;

use axum::{
    Router,
    extract::{Query, State},
    http::StatusCode,
    response::{IntoResponse, Json, Response},
    routing::get,
};
use handbook_agent::model::{CallObserver, CallRecord};
use handbook_store::db;
use rusqlite::Connection;
use serde_json::json;

use crate::admin::AdminAuth;
use crate::{AppState, store_unavailable, unix_now};

/// Default `meta.daily_call_cap` (every pipeline, per UTC day).
pub const DEFAULT_DAILY_CALL_CAP: i64 = 400;

/// Boot DDL on the metadata DB (`handbook.sqlite`): `CREATE TABLE IF
/// NOT EXISTS` only — no ALTER, DROP or RENAME (AGENTS.md §7).
pub fn migrate_tables(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS model_call (
            id INTEGER PRIMARY KEY,
            at INTEGER NOT NULL,
            model TEXT NOT NULL,
            pipeline TEXT NOT NULL,
            run_id INTEGER,
            attempt INTEGER NOT NULL,
            latency_ms INTEGER NOT NULL,
            ttft_ms INTEGER,
            input_tokens INTEGER NOT NULL,
            output_tokens INTEGER NOT NULL,
            cached_tokens INTEGER NOT NULL,
            reasoning_tokens INTEGER NOT NULL,
            outcome TEXT NOT NULL,
            http_status INTEGER
        );
        CREATE INDEX IF NOT EXISTS model_call_at ON model_call(at);
        CREATE INDEX IF NOT EXISTS model_call_run ON model_call(run_id);",
    )
}

/// Insert one attempt row.
pub fn record(conn: &Connection, call: &CallRecord, at: i64) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO model_call(at, model, pipeline, run_id, attempt, latency_ms, ttft_ms,
                                input_tokens, output_tokens, cached_tokens, reasoning_tokens,
                                outcome, http_status)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
        rusqlite::params![
            at,
            call.model,
            call.pipeline,
            call.run_id.and_then(|id| i64::try_from(id).ok()),
            call.attempt,
            i64::try_from(call.latency_ms).unwrap_or(i64::MAX),
            call.ttft_ms.and_then(|ms| i64::try_from(ms).ok()),
            call.input_tokens,
            call.output_tokens,
            call.cached_tokens,
            call.reasoning_tokens,
            call.outcome,
            call.http_status,
        ],
    )
    .map(|_| ())
}

/// Writes each attempt to `model_call` in the store (a fresh connection
/// per row: the client runs on a blocking thread). Storage errors are
/// swallowed — telemetry must never fail a model call.
pub struct DbObserver {
    store_root: PathBuf,
}

impl DbObserver {
    pub fn new(store_root: PathBuf) -> Arc<Self> {
        Arc::new(DbObserver { store_root })
    }
}

impl CallObserver for DbObserver {
    fn record(&self, call: &CallRecord) {
        let store = handbook_store::store::Store::new(&self.store_root);
        if let Ok(conn) = db::open(&store) {
            let _ = record(&conn, call, unix_now());
        }
    }
}

/// Start of the UTC day containing `now`.
fn utc_day_start(now: i64) -> i64 {
    now - now.rem_euclid(86_400)
}

/// Calls recorded since the start of today's UTC day.
pub fn calls_today(conn: &Connection, now: i64) -> rusqlite::Result<i64> {
    conn.query_row(
        "SELECT COUNT(*) FROM model_call WHERE at >= ?1",
        [utc_day_start(now)],
        |row| row.get(0),
    )
}

/// True when today's calls reached `meta.daily_call_cap` (default
/// [`DEFAULT_DAILY_CALL_CAP`]). Fails open on a storage error only for
/// the read of the cap itself — a missing count fails closed.
pub fn over_daily_cap(conn: &Connection, state: &AppState, now: i64) -> bool {
    let cap = state
        .kek
        .as_deref()
        .and_then(|kek| {
            state
                .config_cache
                .text(conn, kek, config::keys::META_DAILY_CALL_CAP)
                .ok()
                .flatten()
        })
        .and_then(|v| v.trim().parse::<i64>().ok())
        .filter(|cap| *cap > 0)
        .unwrap_or(DEFAULT_DAILY_CALL_CAP);
    calls_today(conn, now).map_or(true, |n| n >= cap)
}

/// Nearest-rank percentile of an ascending slice (`p` in 0..=100).
fn percentile(sorted: &[i64], p: usize) -> Option<i64> {
    if sorted.is_empty() {
        return None;
    }
    let rank = (p * sorted.len()).div_ceil(100).max(1);
    sorted.get(rank - 1).copied()
}

/// Query for the trace console / dashboard read.
#[derive(Debug, serde::Deserialize)]
pub struct CallsQuery {
    pub pipeline: Option<String>,
    pub since: Option<i64>,
    pub run_id: Option<i64>,
    pub limit: Option<u32>,
}

/// Calls (newest first) + summary, shaped as
/// `docs/lanes/contracts/admin-model-calls.json`.
pub fn calls_view(
    conn: &Connection,
    q: &CallsQuery,
    now: i64,
) -> rusqlite::Result<serde_json::Value> {
    let since = q.since.unwrap_or(now - 86_400);
    let limit = i64::from(q.limit.unwrap_or(100).clamp(1, 500));
    let pipeline = q.pipeline.clone().unwrap_or_default();
    let run_id = q.run_id.unwrap_or(0);
    let filter = "at >= ?1 AND (?2 = '' OR pipeline = ?2) AND (?3 = 0 OR run_id = ?3)";
    let mut stmt = conn.prepare(&format!(
        "SELECT id, at, model, pipeline, run_id, latency_ms, ttft_ms, input_tokens,
                output_tokens, cached_tokens, reasoning_tokens, outcome, attempt
         FROM model_call WHERE {filter} ORDER BY id DESC LIMIT ?4"
    ))?;
    let calls: Vec<serde_json::Value> = stmt
        .query_map(rusqlite::params![since, pipeline, run_id, limit], |r| {
            Ok(json!({
                "id": r.get::<_, i64>(0)?,
                "at": r.get::<_, i64>(1)?,
                "model": r.get::<_, String>(2)?,
                "pipeline": r.get::<_, String>(3)?,
                "run_id": r.get::<_, Option<i64>>(4)?,
                "latency_ms": r.get::<_, i64>(5)?,
                "ttft_ms": r.get::<_, Option<i64>>(6)?,
                "input_tokens": r.get::<_, i64>(7)?,
                "output_tokens": r.get::<_, i64>(8)?,
                "cached_tokens": r.get::<_, i64>(9)?,
                "reasoning_tokens": r.get::<_, i64>(10)?,
                "outcome": r.get::<_, String>(11)?,
                "attempt": r.get::<_, i64>(12)?,
            }))
        })?
        .filter_map(Result::ok)
        .collect();
    let mut lat = conn.prepare(&format!(
        "SELECT latency_ms FROM model_call WHERE {filter} AND outcome = 'ok' ORDER BY latency_ms"
    ))?;
    let latencies: Vec<i64> = lat
        .query_map(rusqlite::params![since, pipeline, run_id], |r| r.get(0))?
        .filter_map(Result::ok)
        .collect();
    let (count, input, output, cached, reasoning): (i64, i64, i64, i64, i64) = conn.query_row(
        &format!(
            "SELECT COUNT(*), COALESCE(SUM(input_tokens), 0), COALESCE(SUM(output_tokens), 0),
                    COALESCE(SUM(cached_tokens), 0), COALESCE(SUM(reasoning_tokens), 0)
             FROM model_call WHERE {filter}"
        ),
        rusqlite::params![since, pipeline, run_id],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
    )?;
    Ok(json!({
        "calls": calls,
        "summary": {
            "since": since,
            "calls": count,
            "p50_ms": percentile(&latencies, 50),
            "p95_ms": percentile(&latencies, 95),
            "input_tokens": input,
            "output_tokens": output,
            "cached_tokens": cached,
            "reasoning_tokens": reasoning,
        },
    }))
}

/// Per-model rollup since `since` (dashboard Meta tiles, B3 stats).
#[allow(dead_code)] // B3 `/admin/stats` is the caller.
pub fn by_model(conn: &Connection, since: i64) -> rusqlite::Result<Vec<serde_json::Value>> {
    let mut models = conn.prepare(
        "SELECT model, COUNT(*), SUM(outcome != 'ok'), SUM(input_tokens), SUM(output_tokens),
                SUM(cached_tokens), SUM(reasoning_tokens)
         FROM model_call WHERE at >= ?1 GROUP BY model ORDER BY COUNT(*) DESC",
    )?;
    let rows: Vec<(String, i64, i64, i64, i64, i64, i64)> = models
        .query_map([since], |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
                r.get(6)?,
            ))
        })?
        .filter_map(Result::ok)
        .collect();
    let mut out = Vec::with_capacity(rows.len());
    for (model, calls, errors, input, output, cached, reasoning) in rows {
        let mut lat = conn.prepare(
            "SELECT latency_ms FROM model_call WHERE at >= ?1 AND model = ?2 AND outcome = 'ok'
             ORDER BY latency_ms",
        )?;
        let latencies: Vec<i64> = lat
            .query_map(rusqlite::params![since, model], |r| r.get(0))?
            .filter_map(Result::ok)
            .collect();
        out.push(json!({
            "model": model, "calls": calls, "errors": errors,
            "p50_ms": percentile(&latencies, 50), "p95_ms": percentile(&latencies, 95),
            "input_tokens": input, "output_tokens": output,
            "cached_tokens": cached, "reasoning_tokens": reasoning,
        }));
    }
    Ok(out)
}

/// `GET /apostille/api/v1/admin/model-calls` — staff-only.
async fn list_calls(
    _auth: AdminAuth,
    State(state): State<AppState>,
    Query(q): Query<CallsQuery>,
) -> Response {
    let store = handbook_store::store::Store::new(&state.store_root);
    let Ok(conn) = db::open(&store) else {
        return store_unavailable();
    };
    match calls_view(&conn, &q, unix_now()) {
        Ok(body) => Json(body).into_response(),
        Err(_) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"error": "telemetry unavailable"})),
        )
            .into_response(),
    }
}

/// Staff routes merged onto the ADMIN listener only (`AdminAuth` per
/// handler; host guard + CSRF layers inside, like `admin_workers::router`).
pub fn admin_router(state: AppState) -> Router {
    Router::<AppState>::new()
        .route("/apostille/api/v1/admin/model-calls", get(list_calls))
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(
        model: &str,
        pipeline: &'static str,
        latency_ms: u64,
        outcome: &'static str,
    ) -> CallRecord {
        CallRecord {
            model: model.to_string(),
            pipeline,
            run_id: Some(9),
            attempt: 1,
            latency_ms,
            ttft_ms: Some(latency_ms / 4),
            input_tokens: 100,
            output_tokens: 20,
            cached_tokens: 60,
            reasoning_tokens: 5,
            outcome,
            http_status: Some(200),
        }
    }

    #[test]
    fn rows_roll_up_into_percentiles_tokens_and_the_daily_count() {
        let conn = Connection::open_in_memory().unwrap();
        migrate_tables(&conn).unwrap();
        migrate_tables(&conn).unwrap(); // idempotent
        let now = 1_793_750_400 + 3_600; // 01:00 UTC
        for ms in [100, 200, 300, 400, 1_000] {
            record(&conn, &call("muse-spark-1.3", "draft", ms, "ok"), now).unwrap();
        }
        record(
            &conn,
            &call("muse-wonder-1.0", "factcheck", 9_000, "timeout"),
            now,
        )
        .unwrap();
        // Yesterday's row counts for neither today nor the default window.
        record(
            &conn,
            &call("muse-spark-1.3", "draft", 50, "ok"),
            now - 90_000,
        )
        .unwrap();
        assert_eq!(calls_today(&conn, now).unwrap(), 6);
        let q = CallsQuery {
            pipeline: None,
            since: None,
            run_id: None,
            limit: None,
        };
        let v = calls_view(&conn, &q, now).unwrap();
        assert_eq!(v["summary"]["calls"], 6);
        assert_eq!(v["summary"]["p50_ms"], 300, "ok calls only");
        assert_eq!(v["summary"]["p95_ms"], 1_000);
        assert_eq!(v["summary"]["input_tokens"], 600);
        assert_eq!(v["calls"][0]["outcome"], "timeout", "newest first");
        let q = CallsQuery {
            pipeline: Some("factcheck".into()),
            since: None,
            run_id: None,
            limit: None,
        };
        assert_eq!(calls_view(&conn, &q, now).unwrap()["summary"]["calls"], 1);
        let models = by_model(&conn, now - 86_400).unwrap();
        assert_eq!(models[0]["model"], "muse-spark-1.3");
        assert_eq!(models[0]["calls"], 5);
        assert_eq!(models[1]["errors"], 1);
    }

    #[test]
    fn percentile_is_nearest_rank() {
        assert_eq!(percentile(&[], 50), None);
        assert_eq!(percentile(&[7], 95), Some(7));
        assert_eq!(percentile(&[1, 2, 3, 4], 50), Some(2));
        assert_eq!(percentile(&[1, 2, 3, 4], 95), Some(4));
    }

    #[test]
    fn contract_seed_round_trips_the_call_fields() {
        let seed: serde_json::Value = serde_json::from_str(include_str!(
            "../../../docs/lanes/contracts/admin-model-calls.json"
        ))
        .unwrap();
        for key in [
            "id",
            "at",
            "model",
            "pipeline",
            "run_id",
            "latency_ms",
            "ttft_ms",
            "input_tokens",
            "output_tokens",
            "cached_tokens",
            "reasoning_tokens",
            "outcome",
            "attempt",
        ] {
            assert!(seed["calls"][0].get(key).is_some(), "seed field {key}");
        }
        for key in ["since", "calls", "p50_ms", "p95_ms", "input_tokens"] {
            assert!(seed["summary"].get(key).is_some(), "seed summary {key}");
        }
    }
}
