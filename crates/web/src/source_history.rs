//! SA-14 "source changed" (P394): what changed between captures of
//! one watched source, and drafting from those changes only.
//!
//! Staff (admin listener, guard stack from `mission_admin_routers`):
//! - `GET /apostille/api/v1/admin/sources/{id}/captures`: the source's
//!   captures, newest first, each with its line-diff size against the
//!   previous one; same-sha256 pairs are labelled extractor re-reads.
//! - `GET /apostille/api/v1/admin/captures/{id}/diff`: the line diff of
//!   one capture against the source's previous capture.
//! - `POST /apostille/api/v1/admin/captures/{id}/draft-changes`: Muse
//!   Spark drafts from that capture's added lines only (daily cap).
//! - `POST /apostille/api/v1/admin/changes/draft-pending`: the same for
//!   every new web capture past the change cursor
//!   ([`draft_pending_changes_blocking`], also the sweep hook).
//!
//! Public: `GET /api/v1/authorities/{slug}/changes`: per official
//! https source, the latest real change (added and removed lines only,
//! capped) with both capture dates, for the dossier's evidence panel.

use axum::{
    Router,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Json, Response},
    routing::{get, post},
};
use contracts::model::ModelClient;
use handbook_agent::changes::{ChangeOutcome, PendingRun};
use handbook_store::{
    db,
    history::{self, CaptureDiff, DiffLine, LineOp, SourceCapture},
    store::Store,
};
use rusqlite::{Connection, OptionalExtension as _};
use serde_json::{Value, json};

use crate::{AppState, admin::AdminAuth, approval::iso8601, store_unavailable, unix_now};

/// Captures listed per source.
const HISTORY_LIMIT: i64 = 100;
/// Newest captures whose diff size the history computes.
const SIZED_LIMIT: usize = 30;
/// Unchanged lines kept around each change in the staff diff.
const DIFF_CONTEXT: usize = 3;
/// Public panel caps: lines per side and characters per line.
const PUBLIC_LINES: usize = 12;
const PUBLIC_LINE_CHARS: usize = 300;
/// Captures per source the public panel looks back through.
const PUBLIC_LOOKBACK: i64 = 10;
/// `model_call.pipeline` label for change drafts.
const PIPELINE: &str = "change-draft";

pub fn admin_router(state: AppState) -> Router {
    Router::<AppState>::new()
        .route(
            "/apostille/api/v1/admin/sources/{id}/captures",
            get(source_captures),
        )
        .route(
            "/apostille/api/v1/admin/captures/{id}/diff",
            get(capture_diff),
        )
        .route(
            "/apostille/api/v1/admin/captures/{id}/draft-changes",
            post(draft_changes),
        )
        .route(
            "/apostille/api/v1/admin/changes/draft-pending",
            post(draft_pending),
        )
        .with_state(state)
}

pub fn public_routes() -> Router<AppState> {
    Router::new().route("/api/v1/authorities/{slug}/changes", get(public_changes))
}

fn open(state: &AppState) -> Option<(Store, Connection)> {
    let store = Store::new(&state.store_root);
    let conn = db::open(&store).ok()?;
    Some((store, conn))
}

fn not_found(what: &str) -> Response {
    (
        StatusCode::NOT_FOUND,
        Json(json!({"error": format!("unknown {what}")})),
    )
        .into_response()
}

fn capture_json(c: &SourceCapture) -> Value {
    json!({
        "id": c.id,
        "source_id": c.source_id,
        "captured_at": c.captured_at,
        "kind": c.kind,
        "trust": c.trust,
        "sha256": c.sha256,
        "bytes": c.bytes_raw,
    })
}

fn diff_counts(diff: &CaptureDiff) -> (usize, usize) {
    (diff.added().count(), diff.removed().count())
}

async fn source_captures(
    _auth: AdminAuth,
    State(state): State<AppState>,
    Path(source_id): Path<i64>,
) -> Response {
    let Some((store, conn)) = open(&state) else {
        return store_unavailable();
    };
    let source: Option<Value> = conn
        .query_row(
            "SELECT s.id, a.slug, s.kind, s.uri, s.poll_cadence_sec, s.last_checked_at
             FROM source s JOIN authority a ON a.id = s.authority_id WHERE s.id = ?1",
            [source_id],
            |row| {
                Ok(json!({
                    "id": row.get::<_, i64>(0)?,
                    "authority": row.get::<_, String>(1)?,
                    "kind": row.get::<_, String>(2)?,
                    "uri": row.get::<_, String>(3)?,
                    "poll_cadence_sec": row.get::<_, i64>(4)?,
                    "last_checked_at": row.get::<_, i64>(5)?,
                }))
            },
        )
        .optional()
        .unwrap_or(None);
    let Some(source) = source else {
        return not_found("source");
    };
    let Ok(captures) = history::source_captures(&conn, source_id, HISTORY_LIMIT) else {
        return store_unavailable();
    };
    let rows: Vec<Value> = captures
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let mut row = capture_json(c);
            let diff = if i < SIZED_LIMIT {
                history::diff_with_previous(&conn, &store, c.id).unwrap_or(None)
            } else {
                None
            };
            let previous = match &diff {
                Some(d) => Some(d.from.id),
                None => history::previous_capture(&conn, c)
                    .unwrap_or(None)
                    .map(|p| p.id),
            };
            row["previous_id"] = json!(previous);
            row["reread"] = json!(diff.as_ref().is_some_and(|d| d.reread));
            let (added, removed) = diff.as_ref().map(diff_counts).unzip();
            row["added"] = json!(added);
            row["removed"] = json!(removed);
            row
        })
        .collect();
    let count = rows.len();
    Json(json!({"source": source, "captures": rows, "count": count})).into_response()
}

/// Staff diff lines: every change plus [`DIFF_CONTEXT`] unchanged
/// lines around it; longer unchanged runs fold into one `gap` entry.
fn folded_lines(lines: &[DiffLine]) -> Vec<Value> {
    let near_change = |i: usize| {
        let lo = i.saturating_sub(DIFF_CONTEXT);
        let hi = (i + DIFF_CONTEXT + 1).min(lines.len());
        lines[lo..hi].iter().any(|l| l.op != LineOp::Same)
    };
    let mut out = Vec::new();
    let mut gap = 0usize;
    for (i, line) in lines.iter().enumerate() {
        if line.op == LineOp::Same && !near_change(i) {
            gap += 1;
            continue;
        }
        if gap > 0 {
            out.push(json!({"op": "gap", "lines": gap}));
            gap = 0;
        }
        out.push(json!({
            "op": line.op.as_str(),
            "text": line.text,
            "old_line": line.old_line,
            "new_line": line.new_line,
        }));
    }
    if gap > 0 {
        out.push(json!({"op": "gap", "lines": gap}));
    }
    out
}

async fn capture_diff(
    _auth: AdminAuth,
    State(state): State<AppState>,
    Path(capture_id): Path<i64>,
) -> Response {
    let Some((store, conn)) = open(&state) else {
        return store_unavailable();
    };
    let Ok(Some(to)) = history::capture(&conn, capture_id) else {
        return not_found("capture");
    };
    let Ok(diff) = history::diff_with_previous(&conn, &store, capture_id) else {
        return store_unavailable();
    };
    let Some(diff) = diff else {
        return Json(json!({
            "capture": capture_json(&to),
            "previous": null,
            "reread": false,
            "missing_sidecar": false,
            "added": 0,
            "removed": 0,
            "lines": [],
        }))
        .into_response();
    };
    let (added, removed) = diff_counts(&diff);
    Json(json!({
        "capture": capture_json(&diff.to),
        "previous": capture_json(&diff.from),
        "reread": diff.reread,
        "missing_sidecar": diff.missing_sidecar,
        "added": added,
        "removed": removed,
        "lines": folded_lines(&diff.lines),
    }))
    .into_response()
}

/// The live drafting client for change drafts, every HTTP attempt
/// recorded in `model_call` (daily cap, trace console).
fn change_client(state: &AppState, conn: &Connection) -> Result<Box<dyn ModelClient>, String> {
    let live = crate::draft::live_client(conn, state.kek.as_deref())?;
    Ok(Box::new(live.with_observer(
        crate::telemetry::DbObserver::new(state.store_root.clone()),
        PIPELINE,
        None,
    )))
}

fn max_fact_id(conn: &Connection) -> i64 {
    conn.query_row("SELECT COALESCE(MAX(id), 0) FROM fact", [], |row| {
        row.get(0)
    })
    .unwrap_or(0)
}

/// Fan new proposals out to `/review`, as staff drafts do.
fn publish_proposals(state: &AppState, conn: &Connection, before: i64) -> Vec<i64> {
    let rows: Vec<(i64, String)> = conn
        .prepare(
            "SELECT f.id, a.slug FROM fact f JOIN authority a ON a.id = f.authority_id
             WHERE f.id > ?1 AND f.status = 'proposed' ORDER BY f.id",
        )
        .and_then(|mut stmt| {
            stmt.query_map([before], |row| Ok((row.get(0)?, row.get(1)?)))?
                .collect::<rusqlite::Result<_>>()
        })
        .unwrap_or_default();
    let now = unix_now();
    for (fact_id, slug) in &rows {
        state.bus.publish(
            "proposals",
            json!({"type": "proposal.created", "topic": "proposals",
                   "fact_id": fact_id, "authority": slug, "at": now})
            .to_string(),
        );
    }
    rows.into_iter().map(|(id, _)| id).collect()
}

fn outcome_json(outcome: &ChangeOutcome) -> Value {
    match outcome {
        ChangeOutcome::Skipped(skip) => json!({"outcome": "skipped", "reason": skip.as_str()}),
        ChangeOutcome::Drafted {
            previous_id,
            added_lines,
            report,
        } => json!({
            "outcome": "drafted",
            "previous_id": previous_id,
            "added_lines": added_lines,
            "proposed": report.proposed,
            "rejected": report.rejected,
            "suppressed": report.suppressed,
        }),
    }
}

/// One change draft for one capture, run inline (blocking: call inside
/// `spawn_blocking`). `client` is injected so tests drive the stub.
pub(crate) fn draft_change_blocking(
    state: &AppState,
    client: &dyn ModelClient,
    capture_id: i64,
    actor: &str,
) -> Result<(ChangeOutcome, Vec<i64>), String> {
    let (store, conn) = open(state).ok_or("store unavailable")?;
    let before = max_fact_id(&conn);
    let outcome = handbook_agent::changes::draft_from_change(
        &conn,
        &store,
        client,
        capture_id,
        actor,
        unix_now(),
    )
    .map_err(|e| format!("draft failed: {e}"))?;
    Ok((outcome, publish_proposals(state, &conn, before)))
}

async fn draft_changes(
    auth: AdminAuth,
    State(state): State<AppState>,
    Path(capture_id): Path<i64>,
) -> Response {
    let Some((_store, conn)) = open(&state) else {
        return store_unavailable();
    };
    if !matches!(history::capture(&conn, capture_id), Ok(Some(_))) {
        return not_found("capture");
    }
    if crate::telemetry::over_daily_cap(&conn, &state, unix_now()) {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            Json(json!({"error": "daily model-call cap reached (meta.daily_call_cap)"})),
        )
            .into_response();
    }
    drop(conn);
    let actor = format!("staff:{}", auth.user.user_id);
    let ran = tokio::task::spawn_blocking(move || {
        let (_store, conn) = open(&state).ok_or((
            StatusCode::INTERNAL_SERVER_ERROR,
            "store unavailable".to_string(),
        ))?;
        let client =
            change_client(&state, &conn).map_err(|e| (StatusCode::SERVICE_UNAVAILABLE, e))?;
        draft_change_blocking(&state, client.as_ref(), capture_id, &actor)
            .map_err(|e| (StatusCode::BAD_GATEWAY, e))
    })
    .await;
    match ran {
        Ok(Ok((outcome, proposed_ids))) => {
            let mut body = outcome_json(&outcome);
            body["capture_id"] = json!(capture_id);
            body["fact_ids"] = json!(proposed_ids);
            Json(body).into_response()
        }
        Ok(Err((status, error))) => (status, Json(json!({"error": error}))).into_response(),
        Err(join) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": format!("draft task failed: {join}")})),
        )
            .into_response(),
    }
}

/// How often the web process drafts new changed captures (the watch
/// sweep runs once a day; a quarter hour keeps a manual fetch's
/// change close behind it).
const CHANGE_TICK: std::time::Duration = std::time::Duration::from_secs(15 * 60);

/// Background hook (P403): every [`CHANGE_TICK`], draft the changed
/// web captures past the cursor ([`draft_pending_changes_blocking`]).
/// Each capture is drafted once, from its added lines only, under
/// `meta.daily_call_cap`; without a Meta key it does nothing.
pub fn spawn(state: &AppState) {
    let state = state.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(CHANGE_TICK).await;
            let st = state.clone();
            let ran =
                tokio::task::spawn_blocking(move || draft_pending_changes_blocking(&st, "watch"))
                    .await;
            if let Ok(Ok(runs)) = ran
                && !runs.is_empty()
            {
                eprintln!("change drafts: {} capture(s) processed", runs.len());
            }
        }
    });
}

/// Draft every new web capture past the change cursor, asking the
/// daily cap before each model call (blocking: call inside
/// `spawn_blocking`). This is the sweep hook: run it after a watch
/// pass stores captures. Without a Meta key it fails closed with no
/// cursor movement.
pub fn draft_pending_changes_blocking(
    state: &AppState,
    actor: &str,
) -> Result<Vec<PendingRun>, String> {
    let (_store, conn) = open(state).ok_or("store unavailable")?;
    let client = change_client(state, &conn)?;
    draft_pending_with(state, client.as_ref(), actor)
}

/// [`draft_pending_changes_blocking`] with the client injected.
pub(crate) fn draft_pending_with(
    state: &AppState,
    client: &dyn ModelClient,
    actor: &str,
) -> Result<Vec<PendingRun>, String> {
    let (store, conn) = open(state).ok_or("store unavailable")?;
    let before = max_fact_id(&conn);
    let mut under_cap = || !crate::telemetry::over_daily_cap(&conn, state, unix_now());
    let runs = handbook_agent::changes::draft_pending_changes(
        &conn,
        &store,
        client,
        &mut under_cap,
        actor,
        unix_now(),
    )
    .map_err(|e| format!("change drafts failed: {e}"))?;
    publish_proposals(state, &conn, before);
    Ok(runs)
}

async fn draft_pending(auth: AdminAuth, State(state): State<AppState>) -> Response {
    let actor = format!("staff:{}", auth.user.user_id);
    let ran =
        tokio::task::spawn_blocking(move || draft_pending_changes_blocking(&state, &actor)).await;
    match ran {
        Ok(Ok(runs)) => {
            let rows: Vec<Value> = runs
                .iter()
                .map(|run| match &run.outcome {
                    Ok(outcome) => {
                        let mut row = outcome_json(outcome);
                        row["capture_id"] = json!(run.capture_id);
                        row
                    }
                    Err(error) => json!({
                        "capture_id": run.capture_id,
                        "outcome": "failed",
                        "error": error,
                    }),
                })
                .collect();
            Json(json!({"runs": rows, "count": rows.len()})).into_response()
        }
        Ok(Err(error)) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"error": error})),
        )
            .into_response(),
        Err(join) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": format!("draft task failed: {join}")})),
        )
            .into_response(),
    }
}

fn capped(lines: Vec<&DiffLine>) -> (Vec<String>, usize) {
    let more = lines.len().saturating_sub(PUBLIC_LINES);
    let kept = lines
        .into_iter()
        .take(PUBLIC_LINES)
        .map(|l| l.text.chars().take(PUBLIC_LINE_CHARS).collect())
        .collect();
    (kept, more)
}

/// The newest real change of one source as the public panel shows it:
/// official web captures only, re-reads and empty diffs passed over.
fn latest_public_change(conn: &Connection, store: &Store, source_id: i64) -> Option<Value> {
    let captures: Vec<SourceCapture> = history::source_captures(conn, source_id, PUBLIC_LOOKBACK)
        .ok()?
        .into_iter()
        .filter(|c| c.kind == "warc" && matches!(c.trust.as_str(), "official" | "staff_verified"))
        .collect();
    for pair in captures.windows(2) {
        let (to, from) = (&pair[0], &pair[1]);
        if to.sha256 == from.sha256 {
            continue;
        }
        let (Some(old), Some(new)) = (
            history::sidecar_text(store, from.id),
            history::sidecar_text(store, to.id),
        ) else {
            continue;
        };
        let lines = history::line_diff(&old, &new);
        let (added, more_added) = capped(lines.iter().filter(|l| l.op == LineOp::Added).collect());
        let (removed, more_removed) =
            capped(lines.iter().filter(|l| l.op == LineOp::Removed).collect());
        if added.is_empty() && removed.is_empty() {
            continue;
        }
        return Some(json!({
            "captured_at": iso8601(to.captured_at),
            "previous_captured_at": iso8601(from.captured_at),
            "added": added,
            "removed": removed,
            "more_added": more_added,
            "more_removed": more_removed,
        }));
    }
    None
}

async fn public_changes(State(state): State<AppState>, Path(slug): Path<String>) -> Response {
    let Some((store, conn)) = open(&state) else {
        return store_unavailable();
    };
    let authority: Option<i64> = conn
        .query_row("SELECT id FROM authority WHERE slug = ?1", [&slug], |row| {
            row.get(0)
        })
        .optional()
        .unwrap_or(None);
    let Some(authority) = authority else {
        return not_found("authority");
    };
    let sources: Vec<(i64, String)> = conn
        .prepare(
            "SELECT id, uri FROM source
             WHERE authority_id = ?1 AND kind = 'website_official' AND uri LIKE 'https://%'
             ORDER BY id LIMIT 50",
        )
        .and_then(|mut stmt| {
            stmt.query_map([authority], |row| Ok((row.get(0)?, row.get(1)?)))?
                .collect::<rusqlite::Result<_>>()
        })
        .unwrap_or_default();
    let changes: Vec<Value> = sources
        .into_iter()
        .filter_map(|(id, uri)| {
            let mut change = latest_public_change(&conn, &store, id)?;
            change["source_url"] = json!(uri);
            Some(change)
        })
        .collect();
    Json(json!({"authority": slug, "changes": changes})).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use handbook_agent::changes::NoticeStub;
    use tower::ServiceExt as _;

    const STAFF_TOKEN: &str = "staff-token-p394";
    const NOV_01: i64 = 1_793_491_200;
    const NOV_02: i64 = 1_793_577_600;
    const HUB: &str = "https://www.sos.wa.gov/corporations-charities/apostilles";
    const BEFORE: &str = include_str!("../../handbook-agent/fixtures/wa-hub-2026-11-01.html");
    const AFTER: &str = include_str!("../../handbook-agent/fixtures/wa-hub-2026-11-02.html");

    struct Fixture {
        _dir: tempfile::TempDir,
        state: AppState,
        source: i64,
        authority: i64,
    }

    fn state_for(root: &std::path::Path) -> AppState {
        AppState {
            spa_index: root.join("index.html"),
            static_dir: root.to_path_buf(),
            store_root: root.to_path_buf(),
            bus: crate::ws_bus::Bus::new(),
            kek: None,
            admin_mtls_off: true,
            admin_allow_ips: config::loopback_allow_ips(),
            admin_spa_dir: root.to_path_buf(),
            media_terms_read: false,
            label_cfg: crate::p2_labels::LabelCfg::default(),
            voice_cfg: voice::VoiceConfig::default(),
            voice_api_key: None,
            webauthn: std::sync::Arc::new(crate::passkey::test_webauthn()),
            auth_limits: std::sync::Arc::new(crate::passkey::RateBuckets::default()),
            demo_limits: std::sync::Arc::new(crate::passkey::RateBuckets::default()),
            draft_runs: std::sync::Arc::new(crate::draft::DraftRuns::default()),
            config_cache: std::sync::Arc::new(config::CachedReader::default()),
            google_cfg: None,
            apple_cfg: None,
            meta_cfg: None,
            mailer: None,
            magic_link_base: "https://example.test/apostille/login".to_string(),
            notify_poke: std::sync::Arc::new(tokio::sync::Notify::new()),
            notify_enabled: false,
            notify_daily_cap: 3,
            notify_postal: String::new(),
            notify_limits: std::sync::Arc::new(crate::passkey::RateBuckets::default()),
            notify_base: "https://example.test/apostille".to_string(),
            meta_delete_status_base: "https://example.test/auth/meta/delete-status".to_string(),
            fedex_live: crate::fedex_live::FedexLiveState::default(),
        }
    }

    impl Fixture {
        fn new() -> Self {
            let dir = tempfile::TempDir::new().unwrap();
            let store = Store::new(dir.path());
            store.init().unwrap();
            let conn = db::open(&store).unwrap();
            db::migrate(&conn).unwrap();
            auth::migrate_auth_tables(&conn).unwrap();
            crate::telemetry::migrate_tables(&conn).unwrap();
            let authority = db::add_authority(
                &conn,
                &db::NewAuthority {
                    kind: "state",
                    country: "USA",
                    region: "Washington",
                    name: "Washington Secretary of State",
                    slug: "usa-wa-sos",
                    actor: "test",
                    at: NOV_01,
                },
            )
            .unwrap();
            conn.execute(
                "INSERT INTO source(authority_id, kind, uri) VALUES (?1, 'website_official', ?2)",
                rusqlite::params![authority, HUB],
            )
            .unwrap();
            let source = conn.last_insert_rowid();
            auth::create_session(
                &conn,
                STAFF_TOKEN,
                &auth::NewSession {
                    user_id: "staff-1",
                    role: auth::Role::Staff,
                    customer_ref: None,
                    now: unix_now(),
                    ip: "127.0.0.1",
                    user_agent: "test",
                },
            )
            .unwrap();
            let state = state_for(dir.path());
            Self {
                _dir: dir,
                state,
                source,
                authority,
            }
        }

        fn conn(&self) -> Connection {
            db::open(&Store::new(&self.state.store_root)).unwrap()
        }

        /// Ingest one fetch exactly as the watch sweep stores it.
        fn ingest(&self, html: &str, at: i64) -> i64 {
            let store = Store::new(&self.state.store_root);
            let text = ingest::capture::extract_text(html.as_bytes());
            ingest::capture::ingest_bytes(
                &self.conn(),
                &store,
                &ingest::capture::IngestInput {
                    authority_id: self.authority,
                    source_id: Some(self.source),
                    kind: "warc",
                    trust: "official",
                    bytes: html.as_bytes(),
                    text: &text,
                    captured_at: at,
                    actor: "timer",
                    origin: HUB,
                },
            )
            .unwrap()
            .capture_id()
        }

        fn admin(&self) -> Router {
            crate::admin::router(self.state.clone()).merge(admin_router(self.state.clone()))
        }

        fn public(&self) -> Router {
            Router::new()
                .route(
                    "/api/v1/authorities/{slug}/facts",
                    get(crate::approval::public_facts),
                )
                .merge(public_routes())
                .with_state(self.state.clone())
        }
    }

    async fn call(app: Router, method: &str, uri: &str) -> (StatusCode, Value) {
        let mut req = Request::builder()
            .method(method)
            .uri(uri)
            .header("host", "localhost")
            .header(
                "cookie",
                format!("{adm}={STAFF_TOKEN}; hb_csrf=t", adm = auth::ADM_COOKIE),
            );
        if method != "GET" {
            req = req
                .header("content-type", "application/json")
                .header("origin", "http://localhost:3001")
                .header(auth::CSRF_HEADER, "t");
        }
        let req = req
            .extension(axum::extract::ConnectInfo(
                "127.0.0.1:443".parse::<std::net::SocketAddr>().unwrap(),
            ))
            .body(Body::from(if method == "GET" { "" } else { "{}" }))
            .unwrap();
        let res = app.oneshot(req).await.unwrap();
        let status = res.status();
        let bytes = axum::body::to_bytes(res.into_body(), 1 << 20)
            .await
            .unwrap();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    #[tokio::test]
    async fn wa_hub_outage_banner_diff_draft_accept_and_public_reads() {
        let f = Fixture::new();
        let before = f.ingest(BEFORE, NOV_01);
        let after = f.ingest(AFTER, NOV_02);
        assert_ne!(before, after);

        // Staff history: newest first, sized against the previous capture.
        let (status, history) = call(
            f.admin(),
            "GET",
            &format!("/apostille/api/v1/admin/sources/{}/captures", f.source),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{history}");
        assert_eq!(history["source"]["uri"], HUB);
        assert_eq!(history["captures"][0]["id"], after);
        assert_eq!(history["captures"][0]["previous_id"], before);
        assert_eq!(history["captures"][0]["added"], 1);
        assert_eq!(history["captures"][0]["removed"], 0);
        assert_eq!(history["captures"][0]["reread"], false);
        assert_eq!(history["captures"][1]["previous_id"], Value::Null);

        // Staff diff: the banner is the one added line.
        let (status, diff) = call(
            f.admin(),
            "GET",
            &format!("/apostille/api/v1/admin/captures/{after}/diff"),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{diff}");
        assert_eq!(diff["previous"]["id"], before);
        let added: Vec<&Value> = diff["lines"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|l| l["op"] == "added")
            .collect();
        assert_eq!(added.len(), 1);
        assert!(
            added[0]["text"]
                .as_str()
                .unwrap()
                .contains("Apostille services are unavailable")
        );

        // Change-only draft with the notice stub: one proposed notice.
        let (outcome, ids) =
            draft_change_blocking(&f.state, &NoticeStub, after, "staff:staff-1").unwrap();
        assert!(matches!(
            outcome,
            ChangeOutcome::Drafted { added_lines: 1, .. }
        ));
        assert_eq!(ids.len(), 1);
        let fact_id = ids[0];
        let (topic, status_col): (String, String) = f
            .conn()
            .query_row(
                "SELECT topic, status FROM fact WHERE id = ?1",
                [fact_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(
            (topic.as_str(), status_col.as_str()),
            ("service_notice", "proposed")
        );

        // Not public until a person accepts it.
        let (_, facts) = call(f.public(), "GET", "/api/v1/authorities/usa-wa-sos/facts").await;
        assert_eq!(facts["facts"].as_array().unwrap().len(), 0);
        let (status, accepted) = call(
            f.admin(),
            "POST",
            &format!("/apostille/api/v1/facts/{fact_id}/accept"),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{accepted}");

        // The banner's data: statement, quote, source, captured date.
        let (_, facts) = call(f.public(), "GET", "/api/v1/authorities/usa-wa-sos/facts").await;
        let notice = &facts["facts"][0];
        assert_eq!(notice["topic"], "service_notice");
        assert_eq!(notice["conflict"], false);
        let cite = &notice["citations"][0];
        assert!(
            cite["quote"]
                .as_str()
                .unwrap()
                .contains("Apostille services are unavailable"),
            "{cite}"
        );
        assert_eq!(cite["source_url"], HUB);
        assert_eq!(cite["captured_at"], "2026-11-02T00:00:00Z");

        // Public "What changed": added lines only, both dates.
        let (status, changes) =
            call(f.public(), "GET", "/api/v1/authorities/usa-wa-sos/changes").await;
        assert_eq!(status, StatusCode::OK, "{changes}");
        let change = &changes["changes"][0];
        assert_eq!(change["source_url"], HUB);
        assert_eq!(change["captured_at"], "2026-11-02T00:00:00Z");
        assert_eq!(change["previous_captured_at"], "2026-11-01T00:00:00Z");
        assert_eq!(change["added"].as_array().unwrap().len(), 1);
        assert_eq!(change["removed"].as_array().unwrap().len(), 0);
        let (status, _) = call(f.public(), "GET", "/api/v1/authorities/nope/changes").await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn rereads_are_labelled_and_hidden_from_the_public_panel() {
        let f = Fixture::new();
        let first = f.ingest(BEFORE, NOV_01);
        // Same bytes, a newer extractor's text (P360): a fresh capture.
        let store = Store::new(&f.state.store_root);
        let conn = f.conn();
        ingest::capture::ingest_bytes(
            &conn,
            &store,
            &ingest::capture::IngestInput {
                authority_id: f.authority,
                source_id: Some(f.source),
                kind: "warc",
                trust: "official",
                bytes: BEFORE.as_bytes(),
                text: "re-read text",
                captured_at: NOV_02,
                actor: "timer",
                origin: HUB,
            },
        )
        .unwrap();
        let (_, history) = call(
            f.admin(),
            "GET",
            &format!("/apostille/api/v1/admin/sources/{}/captures", f.source),
        )
        .await;
        assert_eq!(history["captures"][0]["reread"], true);
        assert_eq!(history["captures"][0]["previous_id"], first);
        let (_, changes) = call(f.public(), "GET", "/api/v1/authorities/usa-wa-sos/changes").await;
        assert_eq!(changes["changes"].as_array().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn admin_routes_refuse_requests_without_a_staff_session() {
        let f = Fixture::new();
        let after = f.ingest(AFTER, NOV_02);
        for (method, uri) in [
            (
                "GET",
                format!("/apostille/api/v1/admin/sources/{}/captures", f.source),
            ),
            (
                "GET",
                format!("/apostille/api/v1/admin/captures/{after}/diff"),
            ),
            (
                "POST",
                format!("/apostille/api/v1/admin/captures/{after}/draft-changes"),
            ),
            (
                "POST",
                "/apostille/api/v1/admin/changes/draft-pending".to_string(),
            ),
        ] {
            let req = Request::builder()
                .method(method)
                .uri(&uri)
                .header("host", "localhost")
                .extension(axum::extract::ConnectInfo(
                    "127.0.0.1:443".parse::<std::net::SocketAddr>().unwrap(),
                ))
                .body(Body::empty())
                .unwrap();
            let res = f.admin().oneshot(req).await.unwrap();
            assert!(
                matches!(
                    res.status(),
                    StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN
                ),
                "{uri}: {}",
                res.status()
            );
            // The public surface never serves the staff routes.
            let res = f
                .public()
                .oneshot(Request::builder().uri(&uri).body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(res.status(), StatusCode::NOT_FOUND, "{uri}: never public");
        }
    }

    #[tokio::test]
    async fn two_accepted_notices_stack_instead_of_conflicting() {
        let f = Fixture::new();
        f.ingest(BEFORE, NOV_01);
        let after = f.ingest(AFTER, NOV_02);
        let (_, ids) = draft_change_blocking(&f.state, &NoticeStub, after, "t").unwrap();
        let conn = f.conn();
        conn.execute(
            "UPDATE fact SET status = 'accepted' WHERE id = ?1",
            [ids[0]],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO fact(authority_id, capture_id, topic, statement, status, created_at,
                              citation_start, citation_end)
             SELECT authority_id, capture_id, topic, 'notice: other', 'accepted', 1,
                    citation_start, citation_end FROM fact WHERE id = ?1",
            [ids[0]],
        )
        .unwrap();
        let (_, facts) = call(f.public(), "GET", "/api/v1/authorities/usa-wa-sos/facts").await;
        let rows = facts["facts"].as_array().unwrap();
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|r| r["conflict"] == false), "{facts}");
    }

    #[tokio::test]
    async fn pending_walk_drafts_once_and_unknown_ids_404() {
        let f = Fixture::new();
        let now = unix_now();
        f.ingest(BEFORE, now - 3_600);
        f.ingest(AFTER, now - 60);
        let runs = draft_pending_with(&f.state, &NoticeStub, "timer").unwrap();
        let drafted = runs
            .iter()
            .filter(|r| matches!(r.outcome, Ok(ChangeOutcome::Drafted { .. })))
            .count();
        assert_eq!(drafted, 1);
        assert!(
            draft_pending_with(&f.state, &NoticeStub, "timer")
                .unwrap()
                .is_empty()
        );
        let (status, _) = call(
            f.admin(),
            "GET",
            "/apostille/api/v1/admin/sources/999/captures",
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        let (status, _) = call(
            f.admin(),
            "GET",
            "/apostille/api/v1/admin/captures/999/diff",
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }
}
