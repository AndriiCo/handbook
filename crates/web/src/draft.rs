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
    extract::{Path, State},
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
    pub error: String,
    pub started_at: i64,
    pub finished_at: i64,
}

/// Process-wide draft-run registry (lives on [`AppState`]).
#[derive(Debug, Default)]
pub struct DraftRuns {
    next: AtomicU64,
    runs: Mutex<HashMap<u64, RunView>>,
}

impl DraftRuns {
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
                error: String::new(),
                started_at: now,
                finished_at: 0,
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
    };
    tokio::spawn(async move { task.run().await });
    (
        StatusCode::ACCEPTED,
        Json(json!({"run_id": run_id, "topic": format!("run:{run_id}")})),
    )
        .into_response()
}

/// `GET /apostille/api/v1/draft-runs/{id}` — the trace console polls
/// this while a run is `running`.
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

struct DraftTask {
    bus: crate::ws_bus::Bus,
    store_root: std::path::PathBuf,
    kek: Option<Arc<config::Kek>>,
    runs: Arc<DraftRuns>,
    run_id: u64,
    capture_id: i64,
    slug: String,
    text: String,
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
                        "elapsed_ms": started.elapsed().as_millis() as u64,
                    }),
                );
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
        let client = live_client(&conn, self.kek.as_deref())?;
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
        let report = handbook_agent::runner::run_analysis(
            &conn,
            &client as &dyn contracts::model::ModelClient,
            &req,
        )
        .map_err(|e| {
            // `ModelError` display strings are static client/DB
            // failure labels — no key material flows through here.
            format!("draft failed: {e}")
        })?;
        Ok(DraftOutcome {
            proposed: report.proposed,
            rejected: report.rejected,
            model: client.model_id().to_string(),
            input_tokens: report.usage.input_tokens,
            output_tokens: report.usage.output_tokens,
        })
    }
}

/// One finished analyzer pass, as the run registry and step events read it.
pub struct DraftOutcome {
    proposed: usize,
    rejected: usize,
    model: String,
    input_tokens: u32,
    output_tokens: u32,
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
fn live_client(
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
    use super::*;

    #[test]
    fn runs_start_finish_and_read_back() {
        let runs = DraftRuns::default();
        let id = runs.start(9, "vietnam".to_string(), 100).expect("start");
        assert_eq!(id, 1);
        let view = runs.get(id).expect("get");
        assert_eq!(view.status, "running");
        assert_eq!(view.capture_id, 9);
        runs.finish(
            id,
            &DraftOutcome {
                proposed: 2,
                rejected: 1,
                model: "muse-spark-1.3".to_string(),
                input_tokens: 100,
                output_tokens: 50,
            },
            200,
        );
        let done = runs.get(id).expect("get");
        assert_eq!(done.status, "finished");
        assert_eq!(done.proposed, 2);
        assert_eq!(done.model, "muse-spark-1.3");
        assert_eq!((done.input_tokens, done.output_tokens), (100, 50));
        assert!(runs.get(999).is_none());
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
