//! Compile run: one staff button walks an authority through the
//! pipeline — capture its watched official pages, draft every new or
//! never-drafted capture, cross-check those drafts, structure offices,
//! compose the page plan — streamed over `run:{id}` (ids from
//! `DraftRuns::reserve_id`). Capped per run ([`MAX_CAPTURES`],
//! [`MAX_DRAFTS`]) and per UTC day (`compile.daily_runs`, default
//! [`DEFAULT_DAILY_RUNS`]), plus the shared `meta.daily_call_cap`
//! between model steps. Never on a timer.
//!
//! Every model step is the same code the staff buttons run (draft,
//! factcheck, offices, plan), so each one keeps its own guards, and a
//! step that cannot run (no text, too few accepted facts) is reported as
//! skipped instead of failing the run. Nothing here accepts anything:
//! drafts, offices and plans all land as proposals for `/review`.

use axum::{
    Router,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Json, Response},
    routing::get,
};
use handbook_store::db;
use rusqlite::{Connection, OptionalExtension};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::admin::AdminAuth;
use crate::admin_workers::{FetchRequest, fetch_one, http_url};
use crate::{AppState, store_unavailable, unix_now};

/// Default for `compile.daily_runs` (runs per UTC day, all authorities).
pub const DEFAULT_DAILY_RUNS: i64 = 6;
/// Watched pages fetched per run (seed URLs count toward it).
const MAX_CAPTURES: usize = 8;
/// Captures drafted per run (each is one or more model calls).
const MAX_DRAFTS: usize = 6;
/// Extra URLs staff may add to one run.
const MAX_SEEDS: usize = 5;
/// A `running` row older than this is treated as dead (a restart
/// killed its task), so it neither blocks the slug nor stays running.
const STALE_SECS: i64 = 3_600;

/// Boot DDL on the metadata DB (`handbook.sqlite`): `CREATE TABLE IF
/// NOT EXISTS` only — no ALTER, DROP or RENAME (AGENTS.md §7).
pub fn migrate_tables(conn: &Connection) -> rusqlite::Result<()> {
    // A run lives in a tokio task: after a restart a `running` row is
    // dead, and the one-running-per-slug index below must not trip on it.
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS compile_run (
            id INTEGER PRIMARY KEY,
            slug TEXT NOT NULL,
            actor TEXT NOT NULL,
            status TEXT NOT NULL CHECK (status IN ('running', 'done', 'failed')),
            started_at INTEGER NOT NULL,
            finished_at INTEGER,
            summary_json TEXT NOT NULL DEFAULT '{}'
         );
         CREATE INDEX IF NOT EXISTS idx_compile_run_started ON compile_run(started_at);
         CREATE INDEX IF NOT EXISTS idx_compile_run_slug ON compile_run(slug, status);
         UPDATE compile_run SET status = 'failed', finished_at = started_at,
                summary_json = json_object('error', 'interrupted')
          WHERE status = 'running';
         CREATE UNIQUE INDEX IF NOT EXISTS idx_compile_run_one_running
             ON compile_run(slug) WHERE status = 'running';
         -- Captures a Compile run already drafted: one that yielded no
         -- proposal is not re-drafted (and re-billed) by every later run.
         CREATE TABLE IF NOT EXISTS compile_drafted (
            capture_id INTEGER PRIMARY KEY,
            run_id INTEGER NOT NULL,
            at INTEGER NOT NULL
         );",
    )
}

/// `compile.daily_runs`, read through the 30 s config cache. A bad or
/// negative value keeps the default; 0 stops every run.
fn daily_runs(conn: &Connection, state: &AppState) -> i64 {
    state
        .kek
        .as_deref()
        .and_then(|kek| {
            state
                .config_cache
                .text(conn, kek, config::keys::COMPILE_DAILY_RUNS)
                .ok()
                .flatten()
        })
        .and_then(|v| v.trim().parse::<i64>().ok())
        .filter(|n| *n >= 0)
        .unwrap_or(DEFAULT_DAILY_RUNS)
}

/// Runs started since the start of today's UTC day (a storage error
/// counts as "at the cap": fail closed).
fn runs_today(conn: &Connection, now: i64) -> i64 {
    conn.query_row(
        "SELECT COUNT(*) FROM compile_run WHERE started_at >= ?1",
        [crate::telemetry::utc_day_start(now)],
        |row| row.get(0),
    )
    .unwrap_or(i64::MAX)
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CompileBody {
    slug: String,
    #[serde(default)]
    seed_urls: Vec<String>,
}

/// `POST /apostille/api/v1/admin/compile` — staff. Starts one Compile
/// run and returns its `run:{id}` topic at once (202). Preconditions
/// answer synchronously: 404 unknown authority, 409 a run already going
/// for it, 422 bad seed URLs, 429 the daily run cap or the model-call cap.
async fn start(
    auth: AdminAuth,
    State(state): State<AppState>,
    Json(body): Json<CompileBody>,
) -> Response {
    if body.seed_urls.len() > MAX_SEEDS
        || body
            .seed_urls
            .iter()
            .any(|u| !u.starts_with("https://") || u.len() > 2048 || !http_url(u))
    {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"error": format!("seed_urls: at most {MAX_SEEDS} https URLs")})),
        )
            .into_response();
    }
    let store = handbook_store::store::Store::new(&state.store_root);
    let Ok(conn) = db::open(&store) else {
        return store_unavailable();
    };
    let now = unix_now();
    let authority: Option<i64> = conn
        .query_row(
            "SELECT id FROM authority WHERE slug = ?1",
            [&body.slug],
            |row| row.get(0),
        )
        .optional()
        .unwrap_or(None);
    let Some(authority_id) = authority else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "unknown authority"})),
        )
            .into_response();
    };
    // Rows a restart orphaned stop counting as running.
    let _ = conn.execute(
        "UPDATE compile_run SET status = 'failed', finished_at = ?1,
                summary_json = json_object('error', 'interrupted')
         WHERE status = 'running' AND started_at < ?2",
        rusqlite::params![now, now - STALE_SECS],
    );
    let cap = daily_runs(&conn, &state);
    if runs_today(&conn, now) >= cap {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            Json(
                json!({"error": format!("daily Compile cap reached (compile.daily_runs = {cap})")}),
            ),
        )
            .into_response();
    }
    if crate::telemetry::over_daily_cap(&conn, &state, now) {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            Json(json!({"error": "daily model-call cap reached (meta.daily_call_cap)"})),
        )
            .into_response();
    }
    let running: Option<i64> = conn
        .query_row(
            "SELECT id FROM compile_run WHERE slug = ?1 AND status = 'running'",
            [&body.slug],
            |row| row.get(0),
        )
        .optional()
        .unwrap_or(None);
    if let Some(run) = running {
        return (
            StatusCode::CONFLICT,
            Json(
                json!({"error": "a Compile run is already going for this authority",
                        "run_id": run, "topic": format!("run:{run}")}),
            ),
        )
            .into_response();
    }
    // The run counter restarts at boot; stored ids must never repeat.
    let stored_max: i64 = conn
        .query_row("SELECT COALESCE(MAX(id), 0) FROM compile_run", [], |r| {
            r.get(0)
        })
        .unwrap_or(0);
    state.draft_runs.bump_past(stored_max.max(0) as u64);
    let run_id = state.draft_runs.reserve_id();
    let actor = auth.user.user_id.clone();
    if let Err(e) = conn.execute(
        "INSERT INTO compile_run(id, slug, actor, status, started_at)
         VALUES (?1, ?2, ?3, 'running', ?4)",
        rusqlite::params![run_id as i64, body.slug, actor, now],
    ) {
        // The one-running-per-slug index: a concurrent start won.
        return if e.to_string().contains("UNIQUE") {
            (
                StatusCode::CONFLICT,
                Json(json!({"error": "a Compile run is already going for this authority"})),
            )
                .into_response()
        } else {
            store_unavailable()
        };
    }
    let _ = conn.execute(
        "INSERT INTO audit(at, actor, action, entity, entity_id, before_ref, after_ref, reason)
         VALUES (?1, ?2, 'compile_start', 'authority', ?3, '', ?4, ?5)",
        rusqlite::params![
            now,
            actor,
            authority_id,
            format!("run:{run_id}"),
            body.seed_urls.join(" ")
        ],
    );
    drop(conn);
    let job = state.clone();
    let slug = body.slug.clone();
    let seeds = body.seed_urls.clone();
    tokio::spawn(async move {
        let bus = job.bus.clone();
        publish(&bus, run_id, "compile.started", json!({"slug": slug}));
        let blocking_slug = slug.clone();
        let outcome = tokio::task::spawn_blocking(move || {
            run_blocking(&job, &blocking_slug, authority_id, &seeds, &actor, run_id)
        })
        .await
        .unwrap_or_else(|e| Err(format!("compile task failed: {e}")));
        let (status, summary) = match outcome {
            Ok(summary) => ("done", summary),
            Err(error) => ("failed", json!({"error": error})),
        };
        if let Ok(conn) = db::open(&handbook_store::store::Store::new(&state.store_root)) {
            let _ = conn.execute(
                "UPDATE compile_run SET status = ?1, finished_at = ?2, summary_json = ?3
                 WHERE id = ?4",
                rusqlite::params![status, unix_now(), summary.to_string(), run_id as i64],
            );
        }
        let kind = if status == "done" {
            "compile.done"
        } else {
            "compile.failed"
        };
        publish(
            &bus,
            run_id,
            kind,
            json!({"slug": slug, "status": status, "summary": summary}),
        );
        crate::ws_bus::publish_ops(
            &bus,
            "compile.done",
            json!({"slug": slug, "run_id": run_id, "status": status}),
            unix_now(),
        );
    });
    (
        StatusCode::ACCEPTED,
        Json(json!({"run_id": run_id, "topic": format!("run:{run_id}")})),
    )
        .into_response()
}

fn publish(bus: &crate::ws_bus::Bus, run_id: u64, kind: &str, body: Value) {
    let mut event =
        json!({"type": kind, "topic": format!("run:{run_id}"), "run_id": run_id, "at": unix_now()});
    if let (Some(map), Some(extra)) = (event.as_object_mut(), body.as_object()) {
        for (k, v) in extra {
            map.insert(k.clone(), v.clone());
        }
    }
    bus.publish(&format!("run:{run_id}"), event.to_string());
}

/// One step's report on `run:{id}` (`compile.step`).
fn step(state: &AppState, run_id: u64, name: &str, status: &str, detail: Value) {
    let mut body = json!({"step": name, "status": status});
    if let (Some(map), Some(extra)) = (body.as_object_mut(), detail.as_object()) {
        for (k, v) in extra {
            map.insert(k.clone(), v.clone());
        }
    }
    publish(&state.bus, run_id, "compile.step", body);
}

/// True when the shared model-call cap stops the run before a model step.
fn capped(state: &AppState) -> bool {
    let Ok(conn) = db::open(&handbook_store::store::Store::new(&state.store_root)) else {
        return true;
    };
    crate::telemetry::over_daily_cap(&conn, state, unix_now())
}

/// The run itself (blocking: network, model calls and sqlite).
fn run_blocking(
    state: &AppState,
    slug: &str,
    authority_id: i64,
    seeds: &[String],
    actor: &str,
    run_id: u64,
) -> Result<Value, String> {
    let store = handbook_store::store::Store::new(&state.store_root);
    let conn = db::open(&store).map_err(|e| format!("store unavailable: {e}"))?;

    // 1. Capture: watched official pages, then staff seeds (deduped).
    step(state, run_id, "capture", "running", json!({}));
    let started = std::time::Instant::now();
    let mut urls: Vec<(String, String)> = conn
        .prepare(
            "SELECT uri, kind FROM source
             WHERE authority_id = ?1 AND kind IN ('website_official', 'website_thirdparty')
             ORDER BY kind = 'website_official' DESC, id",
        )
        .and_then(|mut stmt| {
            stmt.query_map([authority_id], |row| Ok((row.get(0)?, row.get(1)?)))?
                .collect::<rusqlite::Result<Vec<_>>>()
        })
        .map_err(|e| format!("store: {e}"))?;
    for seed in seeds {
        if !urls.iter().any(|(u, _)| u == seed) {
            urls.push((seed.clone(), "website_official".to_string()));
        }
    }
    let dropped = urls.len().saturating_sub(MAX_CAPTURES);
    urls.truncate(MAX_CAPTURES);
    let (mut created, mut duplicate, mut failed, mut skipped) = (0, 0, 0, 0);
    let mut fresh: Vec<i64> = Vec::new();
    for (url, kind) in &urls {
        let trust = if kind == "website_official" {
            "official"
        } else {
            "community"
        };
        let req = FetchRequest {
            slug: slug.to_string(),
            url: url.clone(),
            source_kind: kind.clone(),
            trust: trust.to_string(),
        };
        let outcome = fetch_one(&state.store_root, &req, actor, unix_now());
        match outcome.outcome {
            "created" => {
                created += 1;
                fresh.extend(outcome.capture_id);
            }
            "duplicate" => duplicate += 1,
            "skipped" => skipped += 1,
            _ => failed += 1,
        }
    }
    let capture = json!({"pages": urls.len(), "created": created, "duplicate": duplicate,
                         "failed": failed, "skipped": skipped, "dropped": dropped});
    step(
        state,
        run_id,
        "capture",
        "done",
        json!({"counts": capture, "latency_ms": started.elapsed().as_millis() as u64}),
    );

    // 2. Extract: the new captures, then never-drafted pages and files
    // (newest first).
    let undrafted: Vec<i64> = conn
        .prepare(
            "SELECT c.id FROM capture c
             WHERE c.authority_id = ?1 AND c.kind IN ('warc', 'file')
               AND c.trust != 'rejected'
               AND NOT EXISTS (SELECT 1 FROM fact f WHERE f.capture_id = c.id)
               AND NOT EXISTS (SELECT 1 FROM compile_drafted d WHERE d.capture_id = c.id)
             ORDER BY c.id DESC",
        )
        .and_then(|mut stmt| {
            stmt.query_map([authority_id], |row| row.get(0))?
                .collect::<rusqlite::Result<Vec<_>>>()
        })
        .map_err(|e| format!("store: {e}"))?;
    let mut queue = fresh.clone();
    for id in undrafted {
        if !queue.contains(&id) {
            queue.push(id);
        }
    }
    let waiting = queue.len().saturating_sub(MAX_DRAFTS);
    queue.truncate(MAX_DRAFTS);
    let started = std::time::Instant::now();
    let (mut proposed, mut rejected, mut no_text, mut draft_failed) = (0usize, 0usize, 0, 0);
    let mut drafted: Vec<i64> = Vec::new();
    let mut stopped = false;
    if queue.is_empty() {
        step(
            state,
            run_id,
            "extract",
            "skipped",
            json!({"reason": "no new or undrafted capture"}),
        );
    } else {
        step(
            state,
            run_id,
            "extract",
            "running",
            json!({"counts": {"captures": queue.len()}}),
        );
        for capture_id in &queue {
            if capped(state) {
                stopped = true;
                break;
            }
            let _ = conn.execute(
                "INSERT OR REPLACE INTO compile_drafted(capture_id, run_id, at) VALUES (?1, ?2, ?3)",
                rusqlite::params![capture_id, run_id as i64, unix_now()],
            );
            match crate::draft::draft_capture_blocking(state, *capture_id, slug, run_id) {
                Ok((p, r)) => {
                    proposed += p;
                    rejected += r;
                    if p > 0 {
                        drafted.push(*capture_id);
                    }
                }
                Err(e) if e.contains("no extracted text") => no_text += 1,
                Err(_) => draft_failed += 1,
            }
        }
        step(
            state,
            run_id,
            "extract",
            if stopped { "stopped" } else { "done" },
            json!({"counts": {"captures": queue.len(), "proposed": proposed, "rejected": rejected,
                              "no_text": no_text, "failed": draft_failed, "waiting": waiting},
                   "latency_ms": started.elapsed().as_millis() as u64}),
        );
    }

    // 3. Cross-check every capture that produced proposals.
    let mut checks = json!({"captures": drafted.len()});
    if stopped {
        step(
            state,
            run_id,
            "cross_check",
            "skipped",
            json!({"reason": "daily model-call cap reached"}),
        );
    } else if drafted.is_empty() {
        step(
            state,
            run_id,
            "cross_check",
            "skipped",
            json!({"reason": "no new proposals"}),
        );
    } else {
        step(state, run_id, "cross_check", "running", json!({}));
        let started = std::time::Instant::now();
        let (mut ok, mut bad) = (0, 0);
        for capture_id in &drafted {
            if capped(state) {
                stopped = true;
                break;
            }
            match crate::factcheck::run_blocking(
                &state.store_root,
                state.kek.as_deref(),
                *capture_id,
                run_id,
            ) {
                Ok(_) => ok += 1,
                Err(_) => bad += 1,
            }
        }
        checks = json!({"captures": drafted.len(), "checked": ok, "failed": bad});
        step(
            state,
            run_id,
            "cross_check",
            if stopped { "stopped" } else { "done" },
            json!({"counts": checks, "latency_ms": started.elapsed().as_millis() as u64}),
        );
    }

    // 4. Offices and 5. the page plan, from accepted facts only.
    let offices = if stopped || capped(state) {
        stopped = true;
        step(
            state,
            run_id,
            "offices",
            "skipped",
            json!({"reason": "daily model-call cap reached"}),
        );
        json!({"status": "skipped"})
    } else {
        model_step(state, run_id, "offices", |on_step| {
            crate::offices::compose_blocking(
                &state.store_root,
                state.kek.as_deref(),
                slug,
                run_id,
                on_step,
            )
        })
    };
    let plan = if stopped {
        step(
            state,
            run_id,
            "compose",
            "skipped",
            json!({"reason": "daily model-call cap reached"}),
        );
        json!({"status": "skipped"})
    } else {
        match crate::plan::compose_preconditions(&conn, state, slug) {
            Err((_, why)) => {
                step(
                    state,
                    run_id,
                    "compose",
                    "skipped",
                    json!({"reason": why["error"]}),
                );
                json!({"status": "skipped", "reason": why["error"]})
            }
            Ok(()) => model_step(state, run_id, "compose", |on_step| {
                crate::plan::compose_blocking(
                    &state.store_root,
                    state.kek.as_deref(),
                    slug,
                    run_id,
                    on_step,
                )
            }),
        }
    };
    Ok(json!({
        "capture": capture,
        "extract": {"captures": queue.len(), "proposed": proposed, "rejected": rejected,
                    "waiting": waiting},
        "cross_check": checks,
        "offices": offices,
        "plan": plan,
        "stopped_by_cap": stopped,
    }))
}

/// Run one composer (offices or plan) and report it as a step; its own
/// "needs more facts" refusal is a skip, not a failed run.
fn model_step(
    state: &AppState,
    run_id: u64,
    name: &str,
    run: impl FnOnce(&dyn Fn(&str)) -> Result<Value, String>,
) -> Value {
    step(state, run_id, name, "running", json!({}));
    let started = std::time::Instant::now();
    let on_step = |sub: &str| step(state, run_id, name, "running", json!({"detail": sub}));
    match run(&on_step) {
        Ok(summary) => {
            step(
                state,
                run_id,
                name,
                "done",
                json!({"summary": summary, "latency_ms": started.elapsed().as_millis() as u64}),
            );
            json!({"status": "done", "summary": summary})
        }
        Err(error) => {
            step(state, run_id, name, "skipped", json!({"reason": error}));
            json!({"status": "skipped", "reason": error})
        }
    }
}

/// `GET /apostille/api/v1/admin/compile` — staff. Recent runs plus
/// today's count against the cap.
async fn list(_auth: AdminAuth, State(state): State<AppState>) -> Response {
    let store = handbook_store::store::Store::new(&state.store_root);
    let Ok(conn) = db::open(&store) else {
        return store_unavailable();
    };
    let now = unix_now();
    let runs: Vec<Value> = conn
        .prepare(
            "SELECT id, slug, status, started_at, finished_at, summary_json
             FROM compile_run ORDER BY id DESC LIMIT 20",
        )
        .and_then(|mut stmt| {
            stmt.query_map([], |row| {
                let summary: String = row.get(5)?;
                Ok(json!({
                    "run_id": row.get::<_, i64>(0)?,
                    "slug": row.get::<_, String>(1)?,
                    "status": row.get::<_, String>(2)?,
                    "started_at": row.get::<_, i64>(3)?,
                    "finished_at": row.get::<_, Option<i64>>(4)?,
                    "summary": serde_json::from_str::<Value>(&summary).unwrap_or(json!({})),
                }))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()
        })
        .unwrap_or_default();
    let today = runs_today(&conn, now);
    (
        StatusCode::OK,
        Json(json!({"runs": runs, "today": today, "daily_runs": daily_runs(&conn, &state)})),
    )
        .into_response()
}

/// `GET /apostille/api/v1/admin/compile/{id}` — one run's row.
async fn get_one(_auth: AdminAuth, State(state): State<AppState>, Path(id): Path<i64>) -> Response {
    let store = handbook_store::store::Store::new(&state.store_root);
    let Ok(conn) = db::open(&store) else {
        return store_unavailable();
    };
    let row = conn
        .query_row(
            "SELECT slug, status, started_at, finished_at, summary_json
             FROM compile_run WHERE id = ?1",
            [id],
            |row| {
                let summary: String = row.get(4)?;
                Ok(json!({
                    "run_id": id,
                    "slug": row.get::<_, String>(0)?,
                    "status": row.get::<_, String>(1)?,
                    "started_at": row.get::<_, i64>(2)?,
                    "finished_at": row.get::<_, Option<i64>>(3)?,
                    "summary": serde_json::from_str::<Value>(&summary).unwrap_or(json!({})),
                }))
            },
        )
        .optional()
        .unwrap_or(None);
    match row {
        Some(run) => (StatusCode::OK, Json(run)).into_response(),
        None => (StatusCode::NOT_FOUND, Json(json!({"error": "unknown run"}))).into_response(),
    }
}

/// Staff routes merged onto the ADMIN listener only (`AdminAuth` per
/// handler; host guard + CSRF layers inside, like `admin_workers::router`).
pub fn admin_router(state: AppState) -> Router {
    Router::<AppState>::new()
        .route("/apostille/api/v1/admin/compile", get(list).post(start))
        .route("/apostille/api/v1/admin/compile/{id}", get(get_one))
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn memory() -> Connection {
        let conn = Connection::open_in_memory().expect("memory db");
        migrate_tables(&conn).expect("ddl");
        conn
    }

    #[test]
    fn ddl_is_idempotent_and_runs_count_by_utc_day() {
        let conn = memory();
        migrate_tables(&conn).expect("second run is a no-op");
        let now = 1_800_000_000; // 2027-01-15 08:00 UTC
        let day = crate::telemetry::utc_day_start(now);
        for (id, at) in [(1, day - 1), (2, day), (3, now)] {
            conn.execute(
                "INSERT INTO compile_run(id, slug, actor, status, started_at)
                 VALUES (?1, 'usa-wa-sos', 'staff-1', 'done', ?2)",
                rusqlite::params![id, at],
            )
            .expect("insert");
        }
        assert_eq!(runs_today(&conn, now), 2, "yesterday's run does not count");
        conn.execute(
            "INSERT INTO compile_run(id, slug, actor, status, started_at)
             VALUES (4, 'x', 'staff-1', 'paused', ?1)",
            [now],
        )
        .expect_err("status is checked");
    }

    #[test]
    fn run_ids_never_repeat_a_stored_one_after_a_restart() {
        // A fresh boot's counter starts at 0; stored ids must be skipped.
        let runs = crate::draft::DraftRuns::default();
        runs.bump_past(41);
        assert_eq!(runs.reserve_id(), 42);
        runs.bump_past(10); // never moves backwards
        assert_eq!(runs.reserve_id(), 43);
        // A row a restart orphaned is closed at boot, and only one run per
        // authority can be running.
        let conn = memory();
        conn.execute(
            "INSERT INTO compile_run(id, slug, actor, status, started_at)
             VALUES (1, 'x', 's', 'running', 5)",
            [],
        )
        .unwrap();
        migrate_tables(&conn).unwrap();
        let status: String = conn
            .query_row("SELECT status FROM compile_run WHERE id = 1", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(status, "failed");
        let insert = |id: i64| {
            conn.execute(
                "INSERT INTO compile_run(id, slug, actor, status, started_at)
                 VALUES (?1, 'x', 's', 'running', 6)",
                [id],
            )
        };
        insert(2).unwrap();
        assert!(insert(3).is_err(), "a second running run for one slug");
    }

    #[test]
    fn a_broken_count_fails_closed() {
        let conn = Connection::open_in_memory().expect("memory db");
        // No table: the count errors, which must read as "at the cap".
        assert_eq!(runs_today(&conn, 1_800_000_000), i64::MAX);
    }
}
