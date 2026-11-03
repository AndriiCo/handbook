//! Admin "Draft from this capture" (operator finding 12): one live
//! model draft per staff click. Steps go to `run:{id}` on the bus
//! (staff-only topic) and the run state stays readable under
//! `draft-runs/{id}`, which is also what the trace console polls.
//!
//! Cost guard: at most [`MAX_RUNNING`] drafts at once; anything more
//! gets 429. Proposed facts land in `/review` like any analyzer run —
//! nothing publishes without staff approval.

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Serialize;
use serde_json::json;
use std::collections::HashMap;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU64, Ordering},
};
use std::time::Instant;

use contracts::model::ModelClient;
use handbook_store::db;
use rusqlite::OptionalExtension;

use crate::admin::AdminAuth;
use crate::{AppState, store_unavailable, unix_now};

/// Cap on concurrent live drafts: each one is a billed model call.
const MAX_RUNNING: usize = 3;

/// One draft run, as the trace console reads it.
#[derive(Debug, Clone, Serialize)]
pub struct RunView {
    pub id: u64,
    pub capture_id: i64,
    pub slug: String,
    pub status: String,
    pub proposed: usize,
    pub rejected: usize,
    pub model: String,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub cached_tokens: u32,
    pub reasoning_tokens: u32,
    pub error: String,
    pub started_at: i64,
    pub finished_at: i64,
    /// Every attempt in order (the refused-quote retry is `n: 2`), so
    /// the trace console renders the full retry chain.
    pub attempts: Vec<AttemptView>,
    /// Every rejected record (also audited as `analyze.rejected` rows).
    pub rejections: Vec<RejectionView>,
}

/// One recorded attempt, as the run view and `run_finished` serve it.
#[derive(Debug, Clone, Serialize)]
pub struct AttemptView {
    pub n: u32,
    pub drafted: usize,
    pub proposed: usize,
    pub rejected: usize,
    pub kinds: Vec<String>,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub cached_tokens: u32,
    pub reasoning_tokens: u32,
}

impl AttemptView {
    fn from_report(attempt: &handbook_agent::runner::AttemptRecord) -> Self {
        AttemptView {
            n: attempt.n,
            drafted: attempt.drafted,
            proposed: attempt.proposed,
            rejected: attempt.rejected,
            kinds: attempt.kinds.clone(),
            input_tokens: attempt.usage.input_tokens,
            output_tokens: attempt.usage.output_tokens,
            cached_tokens: attempt.usage.cached_tokens,
            reasoning_tokens: attempt.usage.reasoning_tokens,
        }
    }
}

/// One rejected record, as the run view and the staff rejections
/// endpoints serve it.
#[derive(Debug, Clone, Serialize)]
pub struct RejectionView {
    pub attempt: u32,
    pub kind: String,
    pub topic: String,
    pub field: String,
    pub value: String,
    pub span_start: u32,
    pub span_end: u32,
}

impl RejectionView {
    fn from_report(rejection: &handbook_agent::runner::RejectedRecord) -> Self {
        RejectionView {
            attempt: rejection.attempt,
            kind: rejection.kind.clone(),
            topic: rejection.topic.clone(),
            field: rejection.field.clone(),
            value: rejection.value.clone(),
            span_start: rejection.span.0,
            span_end: rejection.span.1,
        }
    }
}

/// Process-wide draft-run registry (lives on [`AppState`]).
#[derive(Debug, Default)]
pub struct DraftRuns {
    next: AtomicU64,
    runs: Mutex<HashMap<u64, RunView>>,
}

impl DraftRuns {
    /// Reserve a fresh id from the one run counter without registering a
    /// draft run, so other run kinds (Compile, plan compose) stream on
    /// `run:{id}` without colliding with draft ids.
    pub fn reserve_id(&self) -> u64 {
        self.next.fetch_add(1, Ordering::SeqCst) + 1
    }

    /// Move the counter past `stored`, so ids persisted by an earlier
    /// boot (the `compile_run` table) are never handed out again.
    pub fn bump_past(&self, stored: u64) {
        self.next.fetch_max(stored, Ordering::SeqCst);
    }

    /// Allocate a run, or `None` when [`MAX_RUNNING`] are in flight.
    pub fn start(&self, capture_id: i64, slug: String, now: i64) -> Option<u64> {
        let mut runs = self.runs.lock().expect("draft-runs lock poisoned");
        if runs.values().filter(|r| r.status == "running").count() >= MAX_RUNNING {
            return None;
        }
        let id = self.next.fetch_add(1, Ordering::SeqCst) + 1;
        runs.insert(
            id,
            RunView {
                id,
                capture_id,
                slug,
                status: "running".to_string(),
                proposed: 0,
                rejected: 0,
                model: String::new(),
                input_tokens: 0,
                output_tokens: 0,
                cached_tokens: 0,
                reasoning_tokens: 0,
                error: String::new(),
                started_at: now,
                finished_at: 0,
                attempts: Vec::new(),
                rejections: Vec::new(),
            },
        );
        Some(id)
    }

    pub fn finish(&self, id: u64, outcome: &DraftOutcome, now: i64) {
        let mut runs = self.runs.lock().expect("draft-runs lock poisoned");
        if let Some(run) = runs.get_mut(&id) {
            run.status = "finished".to_string();
            run.proposed = outcome.proposed;
            run.rejected = outcome.rejected;
            run.model = outcome.model.clone();
            run.input_tokens = outcome.input_tokens;
            run.output_tokens = outcome.output_tokens;
            run.cached_tokens = outcome.cached_tokens;
            run.reasoning_tokens = outcome.reasoning_tokens;
            run.attempts = outcome.attempts.clone();
            run.rejections = outcome.rejections.clone();
            run.finished_at = now;
        }
    }

    pub fn fail(&self, id: u64, error: String, now: i64) {
        let mut runs = self.runs.lock().expect("draft-runs lock poisoned");
        if let Some(run) = runs.get_mut(&id) {
            run.status = "failed".to_string();
            run.error = error;
            run.finished_at = now;
        }
    }

    pub fn get(&self, id: u64) -> Option<RunView> {
        self.runs
            .lock()
            .expect("draft-runs lock poisoned")
            .get(&id)
            .cloned()
    }
}

/// `POST /apostille/api/v1/captures/{id}/draft` — staff-only (admin
/// listener). Starts one live draft from the capture's extracted text
/// and returns the run id immediately; steps stream to `run:{id}`.
pub async fn draft_capture(
    _auth: AdminAuth,
    State(state): State<AppState>,
    Path(capture_id): Path<i64>,
) -> Response {
    let store = handbook_store::store::Store::new(&state.store_root);
    let Ok(conn) = db::open(&store) else {
        return store_unavailable();
    };
    let row: Option<(String,)> = conn
        .query_row(
            "SELECT a.slug FROM capture c JOIN authority a ON a.id = c.authority_id
             WHERE c.id = ?1",
            [capture_id],
            |row| Ok((row.get(0)?,)),
        )
        .optional()
        .unwrap_or(None);
    let Some((slug,)) = row else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "unknown capture"})),
        )
            .into_response();
    };
    let sidecar = store.root().join("text").join(format!("{capture_id}.txt"));
    let text = std::fs::read_to_string(&sidecar).unwrap_or_default();
    if text.trim().is_empty() {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"error": "capture has no extracted text"})),
        )
            .into_response();
    }
    let now = unix_now();
    if crate::telemetry::over_daily_cap(&conn, &state, now) {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            Json(json!({"error": "daily model-call cap reached (meta.daily_call_cap)"})),
        )
            .into_response();
    }
    let Some(run_id) = state.draft_runs.start(capture_id, slug.clone(), now) else {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            Json(json!({"error": "too many drafts running"})),
        )
            .into_response();
    };
    let task = DraftTask {
        bus: state.bus.clone(),
        store_root: state.store_root.clone(),
        kek: state.kek.clone(),
        runs: state.draft_runs.clone(),
        run_id,
        capture_id,
        slug,
        text,
        replay: replay_corpus(&state),
    };
    tokio::spawn(async move { task.run().await });
    (
        StatusCode::ACCEPTED,
        Json(json!({"run_id": run_id, "topic": format!("run:{run_id}")})),
    )
        .into_response()
}

/// `GET /apostille/api/v1/draft-runs/{id}` — run state read-back
/// (the trace console subscribes to the `run:{id}` steps instead).
pub async fn draft_run(
    _auth: AdminAuth,
    State(state): State<AppState>,
    Path(run_id): Path<u64>,
) -> Response {
    match state.draft_runs.get(run_id) {
        Some(view) => (StatusCode::OK, Json(view)).into_response(),
        None => (StatusCode::NOT_FOUND, Json(json!({"error": "unknown run"}))).into_response(),
    }
}

/// `GET /apostille/api/v1/draft-runs/{id}/rejections` — staff-only.
/// Rejected records for one finished run, straight from the run
/// registry (404 when the run is unknown, e.g. after a restart — the
/// durable copy lives in the `analyze.rejected` audit rows).
///
pub async fn draft_run_rejections(
    _auth: AdminAuth,
    State(state): State<AppState>,
    Path(run_id): Path<u64>,
) -> Response {
    match state.draft_runs.get(run_id) {
        Some(view) => (
            StatusCode::OK,
            Json(json!({"run_id": run_id, "rejections": view.rejections})),
        )
            .into_response(),
        None => (StatusCode::NOT_FOUND, Json(json!({"error": "unknown run"}))).into_response(),
    }
}

/// One `analyze.rejected` audit row, as the recent-rejections endpoint
/// serves it (the /review "Blocked by the verifier" section reads this).
#[derive(Debug, Clone, Serialize)]
pub struct StoredRejection {
    pub id: i64,
    pub at: i64,
    pub authority_slug: String,
    pub attempt: u32,
    pub kind: String,
    pub topic: String,
    pub field: String,
    pub value: String,
    pub span_start: u32,
    pub span_end: u32,
}

/// Latest `analyze.rejected` audit rows, newest first, capped at
/// `limit`. Malformed reason JSON never fails the read — the row is
/// skipped (audit is append-only; a bad row stays for forensics).
pub fn recent_rejections(conn: &rusqlite::Connection, limit: u64) -> Vec<StoredRejection> {
    let mut stmt = match conn.prepare(
        "SELECT a.id, a.at, au.slug, a.reason FROM audit a
         JOIN authority au ON au.id = a.entity_id
         WHERE a.action = 'analyze.rejected' AND a.entity = 'authority'
         ORDER BY a.id DESC LIMIT ?1",
    ) {
        Ok(stmt) => stmt,
        Err(_) => return Vec::new(),
    };
    stmt.query_map([limit.min(100) as i64], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
        ))
    })
    .and_then(|rows| rows.collect::<rusqlite::Result<Vec<_>>>())
    .unwrap_or_default()
    .into_iter()
    .filter_map(|(id, at, authority_slug, reason)| {
        let body: serde_json::Value = serde_json::from_str(&reason).ok()?;
        let span = body.get("span")?.as_array()?;
        Some(StoredRejection {
            id,
            at,
            authority_slug,
            attempt: body.get("attempt")?.as_u64()? as u32,
            kind: body.get("kind")?.as_str()?.to_string(),
            topic: body.get("topic")?.as_str()?.to_string(),
            field: body.get("field")?.as_str()?.to_string(),
            value: body.get("value")?.as_str()?.to_string(),
            span_start: span.first()?.as_u64()? as u32,
            span_end: span.get(1)?.as_u64()? as u32,
        })
    })
    .collect()
}

/// `GET /apostille/api/v1/draft-runs/recent` — staff-only. Latest
/// verifier rejections across runs (`analyze.rejected` audit rows),
/// newest first, with the total blocked count.
///
pub async fn draft_rejections_recent(
    _auth: AdminAuth,
    State(state): State<AppState>,
    Query(params): Query<RecentParams>,
) -> Response {
    let store = handbook_store::store::Store::new(&state.store_root);
    let Ok(conn) = db::open(&store) else {
        return store_unavailable();
    };
    let limit = params.limit.unwrap_or(20).clamp(1, 100);
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM audit WHERE action = 'analyze.rejected'",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);
    (
        StatusCode::OK,
        Json(json!({"rejections": recent_rejections(&conn, limit), "count": count})),
    )
        .into_response()
}

/// Query params for the recent-rejections endpoint.
#[derive(Debug, serde::Deserialize)]
pub struct RecentParams {
    pub limit: Option<u64>,
}

struct DraftTask {
    bus: crate::ws_bus::Bus,
    store_root: std::path::PathBuf,
    kek: Option<Arc<config::Kek>>,
    runs: Arc<DraftRuns>,
    run_id: u64,
    capture_id: i64,
    slug: String,
    text: String,
    /// Recorded-response corpus for keyless local boots (see
    /// [`replay_corpus`]); `None` everywhere else.
    replay: Option<std::path::PathBuf>,
}

/// `APOSTILLE_REPLAY` (set by `cargo xtask up`/`judge`), honored only
/// under the `admin.mtls_off` dev seam — which a release binary accepts
/// only with every bind on loopback — so production never replays.
fn replay_corpus(state: &AppState) -> Option<std::path::PathBuf> {
    replay_corpus_from(state.admin_mtls_off, std::env::var_os("APOSTILLE_REPLAY"))
}

/// Pure core of [`replay_corpus`] (env passed in, so tests need no
/// process-global `set_var`).
fn replay_corpus_from(
    dev_seam: bool,
    env: Option<std::ffi::OsString>,
) -> Option<std::path::PathBuf> {
    if !dev_seam {
        return None;
    }
    env.filter(|v| !v.is_empty()).map(std::path::PathBuf::from)
}

/// One step event onto `run:{id}`: envelope first, then the step body.
fn publish_step(bus: &crate::ws_bus::Bus, run_id: u64, kind: &str, body: serde_json::Value) {
    let mut event = json!({
        "type": kind,
        "topic": format!("run:{run_id}"),
        "run_id": run_id,
        "at": unix_now(),
    });
    if let Some(map) = event.as_object_mut()
        && let Some(extra) = body.as_object()
    {
        for (k, v) in extra {
            map.insert(k.clone(), v.clone());
        }
    }
    bus.publish(&format!("run:{run_id}"), event.to_string());
}

impl DraftTask {
    async fn run(self) {
        let started = Instant::now();
        // Moved into the blocking closure below; the join-error arm
        // still needs them, so bind them up front.
        let runs = self.runs.clone();
        let run_id = self.run_id;
        let capture_id = self.capture_id;
        let slug = self.slug.clone();
        let bus = self.bus.clone();
        let publish = |kind: &str, body: serde_json::Value| {
            publish_step(&bus, run_id, kind, body);
        };
        publish(
            "run_started",
            json!({"capture_id": capture_id, "slug": slug}),
        );
        // Blocking ureq client + sqlite handle: keep the whole draft
        // off the async runtime.
        let outcome = tokio::task::spawn_blocking(move || self.draft_blocking()).await;
        let now = unix_now();
        match outcome {
            Ok(Ok(report)) => {
                runs.finish(run_id, &report, now);
                publish(
                    "run_finished",
                    json!({
                        "capture_id": capture_id, "slug": slug,
                        "proposed": report.proposed, "rejected": report.rejected,
                        "model": report.model,
                        "input_tokens": report.input_tokens,
                        "output_tokens": report.output_tokens,
                        "cached_tokens": report.cached_tokens,
                        "reasoning_tokens": report.reasoning_tokens,
                        "attempts": report.attempts,
                        "elapsed_ms": started.elapsed().as_millis() as u64,
                    }),
                );
                // New proposals fan out to the staff `proposals` topic
                // so `/review` inserts the cards live (same envelope
                // as the voice-quote path: every event carries `topic`).
                for fact_id in &report.proposed_ids {
                    bus.publish(
                        "proposals",
                        json!({"type": "proposal.created", "topic": "proposals",
                               "fact_id": fact_id, "authority": slug, "at": now})
                        .to_string(),
                    );
                }
            }
            Ok(Err(error)) => {
                runs.fail(run_id, error.clone(), now);
                publish("run_failed", json!({"error": error}));
            }
            Err(join) => {
                let error = format!("draft task failed: {join}");
                runs.fail(run_id, error.clone(), now);
                publish("run_failed", json!({"error": error}));
            }
        }
    }

    /// Open a fresh connection (the request-scoped one cannot cross
    /// into `spawn_blocking`) and run one analyzer pass over this
    /// capture's extracted text.
    fn draft_blocking(&self) -> Result<DraftOutcome, String> {
        use std::collections::HashMap;

        let store = handbook_store::store::Store::new(&self.store_root);
        let conn = db::open(&store).map_err(|e| format!("store unavailable: {e}"))?;
        // High-water mark for the `proposals` fan-out: everything the
        // analyzer inserts below lands above it (ids are monotonic).
        // A concurrent analyzer run could interleave its own inserts —
        // harmless, since each `proposal.created` names a real fact id
        // and `/review` dedupes by id.
        let before: i64 = conn
            .query_row("SELECT COALESCE(MAX(id), 0) FROM fact", [], |row| {
                row.get(0)
            })
            .unwrap_or(0);
        // M1: every HTTP attempt lands in `model_call` (trace latency,
        // daily cap, dashboard tiles). With no key on a local boot, the
        // recorded corpus answers instead (labelled as recorded).
        let (client, model): (Box<dyn ModelClient>, String) =
            match (live_client(&conn, self.kek.as_deref()), &self.replay) {
                (Ok(live), _) => {
                    let live = live.with_observer(
                        crate::telemetry::DbObserver::new(self.store_root.clone()),
                        "draft",
                        Some(self.run_id),
                    );
                    let id = live.model_id().to_string();
                    (Box::new(live), id)
                }
                (Err(_), Some(corpus)) => {
                    let replay = handbook_agent::replay::ReplayClient::load(corpus, true)?;
                    let id = format!("{} (recorded)", handbook_agent::replay::REPLAY_MODEL);
                    (Box::new(replay), id)
                }
                (Err(e), None) => return Err(e),
            };
        let mut sources = HashMap::new();
        sources.insert("fixture".to_string(), self.text.clone());
        let req = handbook_agent::runner::RunRequest {
            slug: &self.slug,
            // Captures carry no language tag; the analyzer default is
            // English-only (anything else opens a correction task).
            lang: "en",
            input_text: &self.text,
            sources: &sources,
            actor: "staff-draft",
            at: unix_now(),
            capture_id: self.capture_id,
        };
        let report =
            handbook_agent::runner::run_analysis(&conn, client.as_ref(), &req).map_err(|e| {
                // `ModelError` display strings are static client/DB
                // failure labels — no key material flows through here.
                format!("draft failed: {e}")
            })?;
        let mut inserted = conn
            .prepare("SELECT id FROM fact WHERE id > ?1 AND status = 'proposed' ORDER BY id")
            .map_err(|e| format!("store unavailable: {e}"))?;
        let proposed_ids: Vec<i64> = inserted
            .query_map([before], |row| row.get(0))
            .map_err(|e| format!("store unavailable: {e}"))?
            .filter_map(|id| id.ok())
            .collect();
        Ok(DraftOutcome {
            proposed: report.proposed,
            rejected: report.rejected,
            model,
            input_tokens: report.usage.input_tokens,
            output_tokens: report.usage.output_tokens,
            cached_tokens: report.usage.cached_tokens,
            reasoning_tokens: report.usage.reasoning_tokens,
            attempts: report
                .attempts
                .iter()
                .map(AttemptView::from_report)
                .collect(),
            rejections: report
                .rejections
                .iter()
                .map(RejectionView::from_report)
                .collect(),
            proposed_ids,
        })
    }
}

/// One draft pass over a capture's sidecar, run inline: the Compile
/// run's extract step (blocking: call inside `spawn_blocking`). It skips
/// the draft-run registry (the Compile run is the run) but fans new
/// proposals out to `/review` like a staff draft. Returns (proposed,
/// rejected).
pub(crate) fn draft_capture_blocking(
    state: &AppState,
    capture_id: i64,
    slug: &str,
    run_id: u64,
) -> Result<(usize, usize), String> {
    let store = handbook_store::store::Store::new(&state.store_root);
    let sidecar = store.root().join("text").join(format!("{capture_id}.txt"));
    let text = std::fs::read_to_string(&sidecar).unwrap_or_default();
    if text.trim().is_empty() {
        return Err("capture has no extracted text".to_string());
    }
    let task = DraftTask {
        bus: state.bus.clone(),
        store_root: state.store_root.clone(),
        kek: state.kek.clone(),
        runs: state.draft_runs.clone(),
        run_id,
        capture_id,
        slug: slug.to_string(),
        text,
        replay: replay_corpus(state),
    };
    let outcome = task.draft_blocking()?;
    let now = unix_now();
    for fact_id in &outcome.proposed_ids {
        state.bus.publish(
            "proposals",
            json!({"type": "proposal.created", "topic": "proposals",
                   "fact_id": fact_id, "authority": slug, "at": now})
            .to_string(),
        );
    }
    Ok((outcome.proposed, outcome.rejected))
}

/// One finished analyzer pass, as the run registry and step events read it.
pub struct DraftOutcome {
    proposed: usize,
    rejected: usize,
    model: String,
    input_tokens: u32,
    output_tokens: u32,
    cached_tokens: u32,
    reasoning_tokens: u32,
    attempts: Vec<AttemptView>,
    rejections: Vec<RejectionView>,
    /// Ids this run inserted (for the `proposals` fan-out below).
    proposed_ids: Vec<i64>,
}

/// Sealed `meta.base_url` when sealed config loads and names one,
/// else `None` (the caller falls back to the compiled default).
/// Key handling is untouched: this only reads the URL.
fn sealed_base_url(conn: &rusqlite::Connection, kek: Option<&config::Kek>) -> Option<String> {
    let kek = kek?;
    let (cfg, _warnings) = config::RuntimeConfig::load(conn, kek).ok()?;
    (!cfg.meta_base_url.is_empty()).then(|| cfg.meta_base_url.clone())
}

/// Live model client: `$META_API_KEY` first (staging, one-shot runs),
/// else the admin-sealed `meta.api_key`. Fails closed — never the stub.
pub(crate) fn live_client(
    conn: &rusqlite::Connection,
    kek: Option<&config::Kek>,
) -> Result<handbook_agent::model::MetaRestClient, String> {
    if let Ok(key) = std::env::var("META_API_KEY")
        && !key.is_empty()
    {
        let base = sealed_base_url(conn, kek)
            .unwrap_or_else(|| handbook_agent::model::META_BASE_URL.to_string());
        return Ok(handbook_agent::model::MetaRestClient::new(base, key));
    }
    let Some(kek) = kek else {
        return Err("meta api key not configured".to_string());
    };
    let (cfg, _warnings) =
        config::RuntimeConfig::load(conn, kek).map_err(|e| format!("config: {e}"))?;
    let Some(key) = cfg.meta_api_key else {
        return Err("meta api key not configured".to_string());
    };
    // Borrowed under the config value; never logged (the redacting
    // `SecretText` only exposes `as_str` to callers that must not log).
    if key.as_str().is_empty() {
        return Err("meta api key not configured".to_string());
    }
    let base = if cfg.meta_base_url.is_empty() {
        handbook_agent::model::META_BASE_URL.to_string()
    } else {
        cfg.meta_base_url
    };
    Ok(handbook_agent::model::MetaRestClient::new(
        base,
        key.as_str().to_string(),
    ))
}

#[cfg(test)]
mod tests {

    #[test]
    fn replay_only_under_the_dev_seam() {
        let corpus = || Some(std::ffi::OsString::from("/tmp/drafts-spark.jsonl"));
        assert_eq!(replay_corpus_from(false, corpus()), None);
        assert_eq!(
            replay_corpus_from(true, corpus()),
            Some(std::path::PathBuf::from("/tmp/drafts-spark.jsonl"))
        );
        assert_eq!(
            replay_corpus_from(true, Some(std::ffi::OsString::new())),
            None
        );
        assert_eq!(replay_corpus_from(true, None), None);
    }

    use super::*;

    fn finished_outcome() -> DraftOutcome {
        DraftOutcome {
            proposed: 2,
            rejected: 1,
            model: "muse-spark-1.3".to_string(),
            input_tokens: 100,
            output_tokens: 50,
            cached_tokens: 80,
            reasoning_tokens: 30,
            attempts: vec![
                AttemptView {
                    n: 1,
                    drafted: 2,
                    proposed: 0,
                    rejected: 2,
                    kinds: vec!["value not in quote".to_string()],
                    input_tokens: 90,
                    output_tokens: 40,
                    cached_tokens: 70,
                    reasoning_tokens: 25,
                },
                AttemptView {
                    n: 2,
                    drafted: 3,
                    proposed: 2,
                    rejected: 0,
                    kinds: Vec::new(),
                    input_tokens: 10,
                    output_tokens: 10,
                    cached_tokens: 10,
                    reasoning_tokens: 5,
                },
            ],
            rejections: vec![RejectionView {
                attempt: 1,
                kind: "ValueNotInQuote".to_string(),
                topic: "fees".to_string(),
                field: "amount".to_string(),
                value: "300,000 VND".to_string(),
                span_start: 120,
                span_end: 220,
            }],
            proposed_ids: vec![11, 12],
        }
    }

    #[test]
    fn runs_start_finish_and_read_back() {
        let runs = DraftRuns::default();
        let id = runs.start(9, "vietnam".to_string(), 100).expect("start");
        assert_eq!(id, 1);
        let view = runs.get(id).expect("get");
        assert_eq!(view.status, "running");
        assert_eq!(view.capture_id, 9);
        runs.finish(id, &finished_outcome(), 200);
        let done = runs.get(id).expect("get");
        assert_eq!(done.status, "finished");
        assert_eq!(done.proposed, 2);
        assert_eq!(done.model, "muse-spark-1.3");
        assert_eq!((done.input_tokens, done.output_tokens), (100, 50));
        assert_eq!((done.cached_tokens, done.reasoning_tokens), (80, 30));
        // Attempts + rejections ride along for the trace console.
        assert_eq!(done.attempts.len(), 2);
        assert_eq!(done.attempts[0].n, 1);
        assert_eq!(
            done.attempts[0].kinds,
            vec!["value not in quote".to_string()]
        );
        assert_eq!(done.attempts[1].n, 2);
        assert!(done.attempts[1].kinds.is_empty());
        assert_eq!(done.rejections.len(), 1);
        assert_eq!(done.rejections[0].kind, "ValueNotInQuote");
        // The GET view serializes the per-call usage for the console.
        let body = serde_json::to_value(&done).expect("view serializes");
        assert_eq!(body["cached_tokens"], 80);
        assert_eq!(body["reasoning_tokens"], 30);
        assert_eq!(body["attempts"][0]["rejected"], 2);
        assert_eq!(body["attempts"][1]["proposed"], 2);
        assert_eq!(body["rejections"][0]["field"], "amount");
        assert!(runs.get(999).is_none());
    }

    #[test]
    fn attempt_and_rejection_views_convert_from_report() {
        let report_attempt = handbook_agent::runner::AttemptRecord {
            n: 1,
            drafted: 2,
            proposed: 0,
            rejected: 2,
            kinds: vec!["short quote".to_string()],
            usage: contracts::model::Usage {
                input_tokens: 1,
                output_tokens: 2,
                cached_tokens: 3,
                reasoning_tokens: 4,
            },
        };
        let view = AttemptView::from_report(&report_attempt);
        assert_eq!(
            (view.n, view.drafted, view.proposed, view.rejected),
            (1, 2, 0, 2)
        );
        assert_eq!(
            (
                view.input_tokens,
                view.output_tokens,
                view.cached_tokens,
                view.reasoning_tokens
            ),
            (1, 2, 3, 4)
        );
        let report_rejection = handbook_agent::runner::RejectedRecord {
            attempt: 2,
            kind: "BadRange".to_string(),
            topic: "fees".to_string(),
            field: "amount".to_string(),
            value: "x".to_string(),
            span: (7, 9),
        };
        let rview = RejectionView::from_report(&report_rejection);
        assert_eq!(rview.attempt, 2);
        assert_eq!((rview.span_start, rview.span_end), (7, 9));
    }

    fn audit_fixture() -> (tempfile::TempDir, handbook_store::store::Store) {
        use handbook_store::db::{NewAuthority, add_authority};
        let dir = tempfile::tempdir().unwrap();
        let store = handbook_store::store::Store::new(dir.path());
        store.init().unwrap();
        let conn = db::open(&store).unwrap();
        db::migrate(&conn).unwrap();
        let authority_id = add_authority(
            &conn,
            &NewAuthority {
                kind: "office",
                country: "Test",
                region: "",
                name: "Test Office",
                slug: "t-sos",
                actor: "test",
                at: 0,
            },
        )
        .unwrap();
        let rejected = |at: i64, kind: &str, value: &str| {
            db::insert_audit(
                &conn,
                at,
                "test",
                "analyze.rejected",
                "authority",
                authority_id,
                "sha",
                "attempt 1 record 0",
                &serde_json::json!({
                    "attempt": 1, "kind": kind, "topic": "fees",
                    "field": "amount", "value": value, "span": [0, 12],
                })
                .to_string(),
            )
        };
        // A bad row never fails the read — it is skipped (oldest, so
        // LIMIT 1 still lands on a good row).
        db::insert_audit(
            &conn,
            50,
            "test",
            "analyze.rejected",
            "authority",
            authority_id,
            "sha",
            "attempt 1 record 2",
            "not json",
        )
        .unwrap();
        rejected(100, "ValueNotInQuote", "300,000 VND").unwrap();
        rejected(200, "ShortQuote", "50 VND").unwrap();
        // Other actions are invisible to the endpoint.
        db::insert_analysis_audit(&conn, 400, "test", authority_id, "proposed=1 rejected=0")
            .unwrap();
        (dir, store)
    }

    #[test]
    fn recent_rejections_reads_newest_first_with_limit() {
        let (_dir, store) = audit_fixture();
        let conn = db::open(&store).unwrap();
        let rows = recent_rejections(&conn, 20);
        // Newest first; the malformed row is skipped.
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].kind, "ShortQuote");
        assert_eq!(rows[0].authority_slug, "t-sos");
        assert_eq!(rows[0].value, "50 VND");
        assert_eq!((rows[0].span_start, rows[0].span_end), (0, 12));
        assert_eq!(rows[1].kind, "ValueNotInQuote");
        let one = recent_rejections(&conn, 1);
        assert_eq!(one.len(), 1);
        assert_eq!(one[0].kind, "ShortQuote");
    }

    #[test]
    fn sealed_base_url_prefers_sealed_value() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        config::migrate_config_tables(&conn).unwrap();
        let kek = config::Kek::generate();
        // Fresh store: sealed load yields the compiled default.
        assert_eq!(
            sealed_base_url(&conn, Some(&kek)),
            Some(handbook_agent::model::META_BASE_URL.to_string())
        );
        // A sealed custom value wins.
        config::set_value(
            &conn,
            &kek,
            &config::SetValue {
                version: 1,
                key: config::keys::META_BASE_URL,
                key_type: config::ConfigType::Text,
                plaintext: b"https://meta-stub.test",
                now: 1,
                actor: "test",
            },
        )
        .unwrap();
        assert_eq!(
            sealed_base_url(&conn, Some(&kek)),
            Some("https://meta-stub.test".to_string())
        );
        // No sealed access: the caller falls back to the default.
        assert_eq!(sealed_base_url(&conn, None), None);
    }

    #[test]
    fn runs_cap_concurrent_drafts_and_fail_closed() {
        let runs = DraftRuns::default();
        let mut ids = Vec::new();
        for _ in 0..10 {
            if let Some(id) = runs.start(1, "vietnam".to_string(), 100) {
                ids.push(id);
            }
        }
        // At most MAX_RUNNING in flight; ids stay unique.
        assert_eq!(ids.len(), MAX_RUNNING);
        runs.fail(ids[0], "boom".to_string(), 200);
        let failed = runs.get(ids[0]).expect("get");
        assert_eq!(failed.status, "failed");
        assert_eq!(failed.error, "boom");
        // A freed slot admits the next draft.
        assert!(runs.start(2, "vietnam".to_string(), 300).is_some());
    }
}
