//! Admin workers API (goal-025 stream 1): staff visibility and manual
//! drive over the URL download/analysis ingest paths.
//!
//! Mounted on the ADMIN listener only (see the ADMIN-WORKERS block in
//! `main.rs`): every handler takes [`AdminAuth`] (staff role), and the
//! router reuses the admin host guard + CSRF recipe from [`admin`].
//! No new worker runtime: each fetch runs inside the triggering
//! request via the existing `ingest::{fetch,capture}` paths, in
//! `spawn_blocking` (ureq is blocking and `Connection` is `!Send`).
//! No new WS topics (the topic enum is frozen).

use axum::{
    Router,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Json},
    routing::{get, post},
};
use handbook_store::{
    db,
    store::{Store, sha256_hex},
};
use rusqlite::{Connection, OptionalExtension as _};
use serde_json::json;
use std::collections::HashMap;
use std::path::Path as FsPath;
use std::time::Duration;

use crate::{
    AppState,
    admin::{AdminAuth, admin_host_guard, csrf_guard, require_admin_ip},
    store_unavailable, unix_now,
};

/// Per-fetch timeout for staff-triggered fetches. Shorter than
/// `xtask fetch-url`'s 30 s: interactive use, and a sweep runs up to
/// [`MAX_SWEEP_ITEMS`] of these sequentially.
const FETCH_TIMEOUT: Duration = Duration::from_secs(20);

/// Max sweep items per call (sequential in one request; worst case
/// roughly `MAX_SWEEP_ITEMS × FETCH_TIMEOUT` plus ingest).
const MAX_SWEEP_ITEMS: usize = 10;

/// Title prefix for fetch-failure tasks, shared with `xtask fetch-url`
/// (same rows, same dedup keys — one queue, two entry points).
const FETCH_FAILURE_TITLE_PREFIX: &str = "fetch failed: ";

/// Live task statuses (mirrors `idx_task_dedup`'s partial index).
const LIVE_TASK_STATUSES: &str = "('open', 'batched', 'in_progress')";

/// The staff workers router. Merged onto the ADMIN listener only —
/// never onto the public router. Carries the default 404 fallback
/// (not the SPA shell): axum refuses to merge two routers that both
/// set a fallback, and the admin SPA fallback must win.
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/apostille/api/v1/workers/status", get(status))
        .route("/apostille/api/v1/workers/captures", get(list_captures))
        .route(
            "/apostille/api/v1/workers/captures/{id}/retry",
            post(retry_capture),
        )
        .route("/apostille/api/v1/workers/fetch", post(fetch_one_url))
        .route(
            "/apostille/api/v1/workers/authorities",
            get(list_authorities),
        )
        .route("/apostille/api/v1/workers/sweep", post(sweep))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            csrf_guard,
        ))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            require_admin_ip,
        ))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            admin_host_guard,
        ))
        .layer(axum::middleware::from_fn_with_state(
            "admin",
            crate::request_id_and_log,
        ))
        .with_state(state)
}

fn open(state: &AppState) -> Option<Connection> {
    db::open(&Store::new(&state.store_root)).ok()
}

fn unprocessable(msg: &str) -> axum::response::Response {
    (
        StatusCode::UNPROCESSABLE_ENTITY,
        Json(json!({"error": msg})),
    )
        .into_response()
}

/// Worker status: static fetch config, queue depths, last sweep run.
async fn status(_auth: AdminAuth, State(state): State<AppState>) -> impl IntoResponse {
    let Some(conn) = open(&state) else {
        return store_unavailable();
    };
    let scalar = |sql: &str| -> i64 { conn.query_row(sql, [], |row| row.get(0)).unwrap_or(0) };
    // `id > 0` skips the migration-005 sentinel rows (unattributed
    // authority + capture), which are FK backstops, not ingest state.
    let captures = scalar("SELECT COUNT(*) FROM capture WHERE id > 0");
    let sources = scalar("SELECT COUNT(*) FROM source");
    let open_fetch_failures = scalar(&format!(
        "SELECT COUNT(*) FROM task WHERE kind = 'validate_fact'
         AND status IN {LIVE_TASK_STATUSES}
         AND title LIKE 'fetch failed: %'"
    ));
    let last_sweep: Option<(i64, String, String)> = conn
        .query_row(
            "SELECT at, actor, reason FROM audit WHERE action = 'watch_sweep'
             ORDER BY id DESC LIMIT 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()
        .unwrap_or(None);
    let last_sweep = last_sweep
        .map(|(at, actor, summary)| json!({"at": at, "actor": actor, "summary": summary}));
    (
        StatusCode::OK,
        Json(json!({
            "mode": "manual",
            "fetch": {
                "backend": "ureq",
                "timeout_secs": FETCH_TIMEOUT.as_secs(),
                "max_body_bytes": ingest::fetch::MAX_BODY_BYTES,
            },
            "queue": {
                "captures": captures,
                "sources": sources,
                "open_fetch_failures": open_fetch_failures,
            },
            "last_sweep": last_sweep,
        })),
    )
        .into_response()
}

/// One capture row for the staff list.
#[derive(Debug, Clone, serde::Serialize)]
struct CaptureView {
    id: i64,
    authority_slug: String,
    /// The watched source (links the staff source history, SA-14).
    source_id: Option<i64>,
    source_uri: Option<String>,
    captured_at: i64,
    kind: String,
    bytes_raw: i64,
    sha256: String,
    trust: String,
    notes: String,
    /// First 160 chars of the extracted sidecar text, whitespace
    /// collapsed; empty when the sidecar is missing. Powers the trace
    /// console without shipping whole captures.
    excerpt: String,
}

/// Short text excerpt for one capture's sidecar (trace console).
fn capture_excerpt(store_root: &std::path::Path, capture_id: i64) -> String {
    let text = std::fs::read_to_string(store_root.join("text").join(format!("{capture_id}.txt")))
        .unwrap_or_default();
    let collapsed: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    collapsed.chars().take(160).collect()
}

#[derive(Debug, serde::Deserialize)]
struct CaptureParams {
    authority: Option<String>,
    kind: Option<String>,
    trust: Option<String>,
    limit: Option<i64>,
    before_id: Option<i64>,
}

/// Staff capture list, newest first. Unknown authority slug → 404.
async fn list_captures(
    _auth: AdminAuth,
    State(state): State<AppState>,
    Query(params): Query<CaptureParams>,
) -> impl IntoResponse {
    let limit = params.limit.unwrap_or(50).clamp(1, 200);
    let Some(conn) = open(&state) else {
        return store_unavailable();
    };
    if let Some(slug) = params.authority.as_deref() {
        let known: Option<i64> = conn
            .query_row("SELECT id FROM authority WHERE slug = ?1", [slug], |row| {
                row.get(0)
            })
            .optional()
            .unwrap_or(None);
        if known.is_none() {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({"error": "unknown authority"})),
            )
                .into_response();
        }
    }
    let mut sql = "SELECT c.id, a.slug, s.uri, c.captured_at, c.kind,
                          c.bytes_raw, c.sha256, c.trust, c.notes, c.source_id
                   FROM capture c
                   JOIN authority a ON a.id = c.authority_id
                   LEFT JOIN source s ON s.id = c.source_id
                   WHERE c.id > 0"
        .to_string();
    if params.authority.is_some() {
        sql.push_str(" AND a.slug = ?");
    }
    if params.kind.is_some() {
        sql.push_str(" AND c.kind = ?");
    }
    if params.trust.is_some() {
        sql.push_str(" AND c.trust = ?");
    }
    if params.before_id.is_some() {
        sql.push_str(" AND c.id < ?");
    }
    sql.push_str(" ORDER BY c.id DESC LIMIT ?");
    let mut values: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
    if let Some(slug) = params.authority.as_deref() {
        values.push(Box::new(slug.to_string()));
    }
    if let Some(kind) = params.kind.as_deref() {
        values.push(Box::new(kind.to_string()));
    }
    if let Some(trust) = params.trust.as_deref() {
        values.push(Box::new(trust.to_string()));
    }
    if let Some(before) = params.before_id {
        values.push(Box::new(before));
    }
    values.push(Box::new(limit));
    let refs: Vec<&dyn rusqlite::ToSql> = values.iter().map(|v| &**v).collect();
    let root = state.store_root.clone();
    let rows: Vec<CaptureView> = conn
        .prepare(&sql)
        .and_then(|mut stmt| {
            stmt.query_map(refs.as_slice(), |row| {
                let id: i64 = row.get(0)?;
                Ok(CaptureView {
                    id,
                    authority_slug: row.get(1)?,
                    source_id: row.get(9)?,
                    source_uri: row.get(2)?,
                    captured_at: row.get(3)?,
                    kind: row.get(4)?,
                    bytes_raw: row.get(5)?,
                    sha256: row.get(6)?,
                    trust: row.get(7)?,
                    notes: row.get(8)?,
                    excerpt: capture_excerpt(&root, id),
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()
        })
        .unwrap_or_default();
    let count = rows.len();
    (
        StatusCode::OK,
        Json(json!({"captures": rows, "count": count})),
    )
        .into_response()
}

/// One authority's ingest state for the staff overview.
#[derive(Debug, Clone, serde::Serialize)]
struct AuthorityView {
    slug: String,
    name: String,
    sources: i64,
    captures: i64,
    last_capture_at: Option<i64>,
    open_fetch_failures: i64,
    last_analysis_at: Option<i64>,
    last_prompt_version: Option<String>,
}

/// Per-authority ingest state, ordered by slug.
async fn list_authorities(_auth: AdminAuth, State(state): State<AppState>) -> impl IntoResponse {
    let Some(conn) = open(&state) else {
        return store_unavailable();
    };
    let authorities: Vec<(i64, String, String)> = conn
        .prepare("SELECT id, slug, name FROM authority WHERE id > 0 ORDER BY slug")
        .and_then(|mut stmt| {
            stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
                .collect::<rusqlite::Result<Vec<_>>>()
        })
        .unwrap_or_default();
    let grouped = |sql: &str| -> HashMap<i64, (i64, Option<i64>)> {
        conn.prepare(sql)
            .and_then(|mut stmt| {
                stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
                    .collect::<rusqlite::Result<Vec<(i64, i64, Option<i64>)>>>()
            })
            .unwrap_or_default()
            .into_iter()
            .map(|(id, count, max)| (id, (count, max)))
            .collect()
    };
    let sources = grouped("SELECT authority_id, COUNT(*), NULL FROM source GROUP BY authority_id");
    let captures = grouped(
        "SELECT authority_id, COUNT(*), MAX(captured_at) FROM capture
         WHERE id > 0 GROUP BY authority_id",
    );
    let failures = grouped(&format!(
        "SELECT authority_id, COUNT(*), NULL FROM task
         WHERE kind = 'validate_fact' AND status IN {LIVE_TASK_STATUSES}
           AND title LIKE 'fetch failed: %'
         GROUP BY authority_id"
    ));
    let watermarks: HashMap<String, (i64, String)> = conn
        .prepare("SELECT authority_slug, last_run_at, last_prompt_version FROM watermark")
        .and_then(|mut stmt| {
            stmt.query_map([], |row| {
                let slug: String = row.get(0)?;
                Ok((slug, (row.get(1)?, row.get(2)?)))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()
        })
        .unwrap_or_default()
        .into_iter()
        .collect();
    let views: Vec<AuthorityView> = authorities
        .into_iter()
        .map(|(id, slug, name)| {
            let (source_count, _) = sources.get(&id).copied().unwrap_or((0, None));
            let (capture_count, last_capture_at) = captures.get(&id).copied().unwrap_or((0, None));
            let (failure_count, _) = failures.get(&id).copied().unwrap_or((0, None));
            let (last_analysis_at, last_prompt_version) = watermarks
                .get(&slug)
                .map(|(at, version)| (Some(*at), Some(version.clone())))
                .unwrap_or((None, None));
            AuthorityView {
                slug,
                name,
                sources: source_count,
                captures: capture_count,
                last_capture_at,
                open_fetch_failures: failure_count,
                last_analysis_at,
                last_prompt_version,
            }
        })
        .collect();
    let count = views.len();
    (
        StatusCode::OK,
        Json(json!({"authorities": views, "count": count})),
    )
        .into_response()
}

/// One validated fetch request: an authority slug plus a URL.
pub(crate) struct FetchRequest {
    pub(crate) slug: String,
    pub(crate) url: String,
    pub(crate) source_kind: String,
    pub(crate) trust: String,
}

/// Outcome of one [`fetch_one`] call.
pub(crate) struct FetchOutcome {
    pub(crate) outcome: &'static str,
    pub(crate) capture_id: Option<i64>,
    status: Option<u16>,
    pub(crate) detail: Option<String>,
}

impl FetchOutcome {
    fn created(capture_id: i64, status: u16) -> Self {
        Self {
            outcome: "created",
            capture_id: Some(capture_id),
            status: Some(status),
            detail: None,
        }
    }

    fn duplicate(capture_id: i64, status: u16) -> Self {
        Self {
            outcome: "duplicate",
            capture_id: Some(capture_id),
            status: Some(status),
            detail: None,
        }
    }

    fn failed(detail: String) -> Self {
        Self {
            outcome: "failed",
            capture_id: None,
            status: None,
            detail: Some(detail),
        }
    }

    fn skipped(detail: &str) -> Self {
        Self {
            outcome: "skipped",
            capture_id: None,
            status: None,
            detail: Some(detail.to_string()),
        }
    }
}

/// Only absolute http(s) URLs are fetchable (same gate as the seed
/// shape; `fetch_url` would fail them as transport errors anyway).
pub(crate) fn http_url(url: &str) -> bool {
    url.starts_with("http://") || url.starts_with("https://")
}

/// Find-or-create the `source` row for a watched URL. Mirrors
/// `xtask fetch-url`; returns `None` only on DB failure.
fn ensure_source(conn: &Connection, authority: i64, kind: &str, uri: &str) -> Option<i64> {
    if let Ok(Some(id)) = conn
        .query_row(
            "SELECT id FROM source WHERE authority_id = ?1 AND uri = ?2",
            rusqlite::params![authority, uri],
            |row| row.get::<_, i64>(0),
        )
        .optional()
    {
        return Some(id);
    }
    conn.execute(
        "INSERT INTO source(authority_id, kind, uri) VALUES (?1, ?2, ?3)",
        rusqlite::params![authority, kind, uri],
    )
    .ok()?;
    Some(conn.last_insert_rowid())
}

/// Record a fetch failure as a `validate_fact` task: insert, or
/// refresh the live row's detail on dedup hit. Same dedup key as
/// `xtask fetch-url` (`sha256(slug|fetch|url)`), so staff retries
/// and CLI fetches share one queue. Failures are never silent.
fn record_fetch_failure(
    conn: &Connection,
    authority: i64,
    slug: &str,
    url: &str,
    failure: &str,
    actor: &str,
    at: i64,
) {
    let dedup_key = sha256_hex(format!("{slug}|fetch|{url}").as_bytes());
    let inserted = conn.execute(
        "INSERT INTO task(authority_id, kind, title, detail, status,
                           priority, dedup_key, created_by, created_at)
         VALUES (?1, 'validate_fact', ?2, ?3, 'open', 0, ?4, ?5, ?6)",
        rusqlite::params![
            authority,
            format!("{FETCH_FAILURE_TITLE_PREFIX}{url}"),
            failure,
            dedup_key,
            actor,
            at,
        ],
    );
    if let Err(rusqlite::Error::SqliteFailure(e, _)) = inserted
        && e.code == rusqlite::ErrorCode::ConstraintViolation
    {
        let _ = conn.execute(
            "UPDATE task SET detail = ?1, created_at = ?2
             WHERE dedup_key = ?3
             AND status IN ('open', 'batched', 'in_progress')",
            rusqlite::params![failure, at, dedup_key],
        );
    }
}

/// Run one fetch + ingest synchronously (blocking: network + sqlite).
/// Opens its own connection; call only inside `spawn_blocking`.
/// Robots gate, failure tasks, and dedup mirror `xtask fetch-url`.
pub(crate) fn fetch_one(
    store_root: &FsPath,
    req: &FetchRequest,
    actor: &str,
    now: i64,
) -> FetchOutcome {
    let store = Store::new(store_root);
    let Ok(conn) = db::open(&store) else {
        return FetchOutcome::failed("store unavailable".to_string());
    };
    let authority: Option<i64> = conn
        .query_row(
            "SELECT id FROM authority WHERE slug = ?1",
            [&req.slug],
            |row| row.get(0),
        )
        .optional()
        .unwrap_or(None);
    let Some(authority) = authority else {
        return FetchOutcome::failed("unknown authority".to_string());
    };
    let Some(source_id) = ensure_source(&conn, authority, &req.source_kind, &req.url) else {
        return FetchOutcome::failed("store unavailable".to_string());
    };
    if req.source_kind == "website_thirdparty" {
        let verdict = ingest::fetch::check_robots(&req.url, FETCH_TIMEOUT);
        let _ = conn.execute(
            "UPDATE source SET robots = ?1 WHERE id = ?2",
            rusqlite::params![format!("{} at {now}", verdict.text), source_id],
        );
        if !verdict.allowed {
            record_fetch_failure(
                &conn,
                authority,
                &req.slug,
                &req.url,
                "robots disallow",
                actor,
                now,
            );
            return FetchOutcome::skipped("robots disallow");
        }
    }
    let fetched = match ingest::fetch::fetch_url(&req.url, FETCH_TIMEOUT) {
        Ok(fetched) => fetched,
        Err(e) => {
            let failure = e.to_string();
            record_fetch_failure(&conn, authority, &req.slug, &req.url, &failure, actor, now);
            return FetchOutcome::failed(failure);
        }
    };
    let text = ingest::capture::extract_text(&fetched.bytes);
    match ingest::capture::ingest_bytes(
        &conn,
        &store,
        &ingest::capture::IngestInput {
            authority_id: authority,
            source_id: Some(source_id),
            kind: "warc",
            trust: &req.trust,
            bytes: &fetched.bytes,
            text: &text,
            captured_at: now,
            actor,
            origin: &req.url,
        },
    ) {
        Ok(ingest::capture::IngestOutcome::Created { capture_id }) => {
            FetchOutcome::created(capture_id, fetched.status)
        }
        Ok(ingest::capture::IngestOutcome::Duplicate { capture_id }) => {
            FetchOutcome::duplicate(capture_id, fetched.status)
        }
        Err(_) => FetchOutcome::failed("store unavailable".to_string()),
    }
}

/// One staff-action audit row (retry/fetch/sweep name the staff move;
/// the ingest path audits the capture itself separately).
struct StaffAudit<'a> {
    action: &'a str,
    entity: &'a str,
    entity_id: i64,
    before_ref: &'a str,
    after_ref: &'a str,
    reason: &'a str,
}

fn audit_staff_action(conn: &Connection, now: i64, actor: &str, audit: &StaffAudit) {
    let _ = conn.execute(
        "INSERT INTO audit(at, actor, action, entity, entity_id,
                            before_ref, after_ref, reason)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        rusqlite::params![
            now,
            actor,
            audit.action,
            audit.entity,
            audit.entity_id,
            audit.before_ref,
            audit.after_ref,
            audit.reason
        ],
    );
}

fn valid_source_kind(kind: &str) -> bool {
    matches!(kind, "website_official" | "website_thirdparty")
}

fn valid_trust(trust: &str) -> bool {
    matches!(trust, "official" | "staff_verified" | "community")
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct FetchBody {
    slug: String,
    url: String,
    source_kind: Option<String>,
    trust: Option<String>,
}

/// One-off fetch of an explicit URL (staff `xtask fetch-url`
/// equivalent). Unknown slug → 404; bad URL/enums → 422; fetch
/// failure → 502 with the failure task recorded.
async fn fetch_one_url(
    auth: AdminAuth,
    State(state): State<AppState>,
    Json(body): Json<FetchBody>,
) -> impl IntoResponse {
    let source_kind = body.source_kind.as_deref().unwrap_or("website_official");
    let trust = body.trust.as_deref().unwrap_or("official");
    if body.slug.is_empty() || body.slug.len() > 128 {
        return unprocessable("invalid slug");
    }
    if !http_url(&body.url) || body.url.len() > 2048 {
        return unprocessable("url must be absolute http(s)");
    }
    if !valid_source_kind(source_kind) {
        return unprocessable("unknown source_kind (website_official|website_thirdparty)");
    }
    if !valid_trust(trust) {
        return unprocessable("unknown trust (official|staff_verified|community)");
    }
    {
        let Some(conn) = open(&state) else {
            return store_unavailable();
        };
        let known: Option<i64> = conn
            .query_row(
                "SELECT id FROM authority WHERE slug = ?1",
                [&body.slug],
                |row| row.get(0),
            )
            .optional()
            .unwrap_or(None);
        if known.is_none() {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({"error": "unknown authority"})),
            )
                .into_response();
        }
    }
    let req = FetchRequest {
        slug: body.slug.clone(),
        url: body.url.clone(),
        source_kind: source_kind.to_string(),
        trust: trust.to_string(),
    };
    let store_root = state.store_root.clone();
    let actor = auth.user.user_id.clone();
    let now = unix_now();
    let outcome = tokio::task::spawn_blocking(move || fetch_one(&store_root, &req, &actor, now))
        .await
        .unwrap_or_else(|_| FetchOutcome::failed("fetch task failed".to_string()));
    if let Some(conn) = open(&state) {
        audit_staff_action(
            &conn,
            now,
            &auth.user.user_id,
            &StaffAudit {
                action: "worker_fetch",
                entity: "capture",
                entity_id: outcome.capture_id.unwrap_or(0),
                before_ref: "",
                after_ref: outcome.outcome,
                reason: &body.url,
            },
        );
    }
    fetch_outcome_response(&outcome)
}

/// Map a fetch outcome to its HTTP response: created/duplicate →
/// 200, skipped → 200 with reason, failed → 502 (the failure task
/// is already recorded — the 502 body says so).
fn fetch_outcome_response(outcome: &FetchOutcome) -> axum::response::Response {
    match outcome.outcome {
        "created" | "duplicate" => (
            StatusCode::OK,
            Json(json!({
                "outcome": outcome.outcome,
                "capture_id": outcome.capture_id,
                "status": outcome.status,
            })),
        )
            .into_response(),
        "skipped" => (
            StatusCode::OK,
            Json(json!({
                "outcome": "skipped",
                "reason": outcome.detail,
            })),
        )
            .into_response(),
        _ => (
            StatusCode::BAD_GATEWAY,
            Json(json!({
                "error": outcome.detail,
                "task": "recorded",
            })),
        )
            .into_response(),
    }
}

/// Re-fetch one capture's source URL through the standard fetch
/// path. Unknown id → 404; unattributed capture → 409.
async fn retry_capture(
    auth: AdminAuth,
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> impl IntoResponse {
    let loaded: Option<FetchRequest> = {
        let Some(conn) = open(&state) else {
            return store_unavailable();
        };
        conn.query_row(
            "SELECT a.slug, s.uri, s.kind, c.trust
             FROM capture c
             JOIN authority a ON a.id = c.authority_id
             LEFT JOIN source s ON s.id = c.source_id
             WHERE c.id = ?1",
            [id],
            |row| {
                let uri: Option<String> = row.get(1)?;
                let kind: Option<String> = row.get(2)?;
                Ok(match (uri, kind) {
                    (Some(uri), Some(kind)) => Some(FetchRequest {
                        slug: row.get(0)?,
                        url: uri,
                        source_kind: kind,
                        trust: row.get(3)?,
                    }),
                    _ => None,
                })
            },
        )
        .optional()
        .unwrap_or(None)
        .flatten()
    };
    // Distinguish "no such capture" (404) from "no source" (409).
    let Some(req) = loaded else {
        let Some(conn) = open(&state) else {
            return store_unavailable();
        };
        let exists: bool = conn
            .query_row("SELECT 1 FROM capture WHERE id = ?1", [id], |_| Ok(()))
            .optional()
            .unwrap_or(None)
            .is_some();
        return if exists {
            (
                StatusCode::CONFLICT,
                Json(json!({"error": "capture has no source to re-fetch"})),
            )
                .into_response()
        } else {
            (
                StatusCode::NOT_FOUND,
                Json(json!({"error": "capture not found"})),
            )
                .into_response()
        };
    };
    let url = req.url.clone();
    let store_root = state.store_root.clone();
    let actor = auth.user.user_id.clone();
    let now = unix_now();
    let outcome = tokio::task::spawn_blocking(move || fetch_one(&store_root, &req, &actor, now))
        .await
        .unwrap_or_else(|_| FetchOutcome::failed("fetch task failed".to_string()));
    if let Some(conn) = open(&state) {
        let after = match outcome.capture_id {
            Some(new) => format!("{}:{new}", outcome.outcome),
            None => outcome.outcome.to_string(),
        };
        let before = id.to_string();
        audit_staff_action(
            &conn,
            now,
            &auth.user.user_id,
            &StaffAudit {
                action: "worker_retry",
                entity: "capture",
                entity_id: outcome.capture_id.unwrap_or(id),
                before_ref: &before,
                after_ref: &after,
                reason: &url,
            },
        );
    }
    fetch_outcome_response(&outcome)
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct SweepItem {
    slug: String,
    url: String,
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct SweepBody {
    items: Vec<SweepItem>,
}

/// Manual watch-sweep over explicit items (1–10). Items validate up
/// front (first bad item fails the call, nothing fetched), then run
/// sequentially in one blocking task. Audits one `watch_sweep` row.
async fn sweep(
    auth: AdminAuth,
    State(state): State<AppState>,
    Json(body): Json<SweepBody>,
) -> impl IntoResponse {
    if body.items.is_empty() || body.items.len() > MAX_SWEEP_ITEMS {
        return unprocessable("items must hold 1-10 entries");
    }
    for item in &body.items {
        if item.slug.is_empty() || item.slug.len() > 128 {
            return unprocessable("invalid slug");
        }
        if !http_url(&item.url) || item.url.len() > 2048 {
            return unprocessable("every url must be absolute http(s)");
        }
    }
    {
        let Some(conn) = open(&state) else {
            return store_unavailable();
        };
        for item in &body.items {
            let known: Option<i64> = conn
                .query_row(
                    "SELECT id FROM authority WHERE slug = ?1",
                    [&item.slug],
                    |row| row.get(0),
                )
                .optional()
                .unwrap_or(None);
            if known.is_none() {
                return (
                    StatusCode::NOT_FOUND,
                    Json(json!({"error": format!("unknown authority: {}", item.slug)})),
                )
                    .into_response();
            }
        }
    }
    let reqs: Vec<FetchRequest> = body
        .items
        .iter()
        .map(|item| FetchRequest {
            slug: item.slug.clone(),
            url: item.url.clone(),
            source_kind: "website_official".to_string(),
            trust: "official".to_string(),
        })
        .collect();
    let store_root = state.store_root.clone();
    let actor = auth.user.user_id.clone();
    let now = unix_now();
    let outcomes = tokio::task::spawn_blocking(move || {
        reqs.iter()
            .map(|req| fetch_one(&store_root, req, &actor, now))
            .collect::<Vec<_>>()
    })
    .await
    .unwrap_or_default();
    let mut created = 0;
    let mut duplicate = 0;
    let mut failed = 0;
    let mut skipped = 0;
    let results: Vec<serde_json::Value> = body
        .items
        .iter()
        .zip(outcomes.iter())
        .map(|(item, outcome)| {
            match outcome.outcome {
                "created" => created += 1,
                "duplicate" => duplicate += 1,
                "skipped" => skipped += 1,
                _ => failed += 1,
            }
            json!({
                "slug": item.slug,
                "url": item.url,
                "outcome": outcome.outcome,
                "capture_id": outcome.capture_id,
                "detail": outcome.detail,
            })
        })
        .collect();
    let total = results.len();
    if let Some(conn) = open(&state) {
        audit_staff_action(
            &conn,
            now,
            &auth.user.user_id,
            &StaffAudit {
                action: "watch_sweep",
                entity: "sweep",
                entity_id: 0,
                before_ref: "",
                after_ref: &format!("{total} items"),
                reason: &format!(
                    "created={created} duplicate={duplicate} failed={failed} skipped={skipped}"
                ),
            },
        );
    }
    (
        StatusCode::OK,
        Json(json!({
            "results": results,
            "summary": {
                "total": total,
                "created": created,
                "duplicate": duplicate,
                "failed": failed,
                "skipped": skipped,
            },
        })),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt as _;

    const STAFF_TOKEN: &str = "staff-token-1";
    const CUSTOMER_TOKEN: &str = "customer-token-1";

    struct Fixture {
        _dir: tempfile::TempDir,
        state: AppState,
    }

    impl Fixture {
        fn new() -> Self {
            let dir = tempfile::TempDir::new().expect("tempdir");
            let store = Store::new(dir.path());
            store.init().unwrap();
            let meta = db::open(&store).unwrap();
            db::migrate(&meta).unwrap();
            auth::migrate_auth_tables(&meta).unwrap();
            let now = unix_now();
            for (slug, name) in [
                ("usa-wa-sos", "WA Secretary of State"),
                ("usa-or-sos", "OR Secretary of State"),
            ] {
                db::add_authority(
                    &meta,
                    &db::NewAuthority {
                        kind: "state",
                        country: "USA",
                        region: "WA",
                        name,
                        slug,
                        actor: "test",
                        at: now,
                    },
                )
                .unwrap();
            }
            for (token, user, role) in [
                (STAFF_TOKEN, "staff-1", auth::Role::Staff),
                (CUSTOMER_TOKEN, "cust-1", auth::Role::Customer),
            ] {
                auth::create_session(
                    &meta,
                    token,
                    &auth::NewSession {
                        user_id: user,
                        role,
                        customer_ref: None,
                        now,
                        ip: "127.0.0.1",
                        user_agent: "test",
                    },
                )
                .unwrap();
            }
            let state = AppState {
                spa_index: dir.path().join("index.html"),
                static_dir: dir.path().to_path_buf(),
                store_root: dir.path().to_path_buf(),
                bus: crate::ws_bus::Bus::new(),
                kek: None,
                admin_mtls_off: true,
                admin_allow_ips: config::loopback_allow_ips(),
                admin_spa_dir: dir.path().to_path_buf(),
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
            };
            Self { _dir: dir, state }
        }

        /// The merged admin app, exactly as `main.rs` mounts it.
        fn app(&self) -> Router {
            crate::admin::router(self.state.clone()).merge(router(self.state.clone()))
        }

        fn meta(&self) -> Connection {
            db::open(&Store::new(&self.state.store_root)).unwrap()
        }

        fn seed_capture(&self, slug: &str, uri: Option<&str>, kind: &str) -> i64 {
            let meta = self.meta();
            let authority: i64 = meta
                .query_row("SELECT id FROM authority WHERE slug = ?1", [slug], |row| {
                    row.get(0)
                })
                .unwrap();
            let source_id = match uri {
                Some(uri) => {
                    meta.execute(
                        "INSERT INTO source(authority_id, kind, uri)
                         VALUES (?1, 'website_official', ?2)",
                        rusqlite::params![authority, uri],
                    )
                    .unwrap();
                    Some(meta.last_insert_rowid())
                }
                None => None,
            };
            meta.execute(
                "INSERT INTO capture(authority_id, source_id, captured_at, kind,
                                      path, bytes_raw, bytes_stored, sha256, trust)
                 VALUES (?1, ?2, ?3, ?4, 'blob/aa', 2, 2, 'aa', 'official')",
                rusqlite::params![authority, source_id, unix_now(), kind],
            )
            .unwrap();
            meta.last_insert_rowid()
        }
    }

    fn authed(
        method: &str,
        uri: &str,
        token: &str,
        body: Option<serde_json::Value>,
    ) -> Request<Body> {
        authed_host(method, uri, token, body, "localhost")
    }

    fn authed_host(
        method: &str,
        uri: &str,
        token: &str,
        body: Option<serde_json::Value>,
        host: &str,
    ) -> Request<Body> {
        let mut builder = Request::builder()
            .uri(uri)
            .method(method)
            .header("host", host)
            .header(
                "cookie",
                format!("{adm}={token}; hb_csrf=t", adm = auth::ADM_COOKIE),
            );
        if method != "GET" {
            builder = builder
                .header("content-type", "application/json")
                .header("origin", "http://localhost:3001")
                .header(auth::CSRF_HEADER, "t");
        }
        builder = builder.extension(axum::extract::ConnectInfo(
            "127.0.0.1:443".parse::<std::net::SocketAddr>().unwrap(),
        ));
        builder
            .body(body.map_or_else(Body::empty, |v| Body::from(v.to_string())))
            .unwrap()
    }

    async fn json_body(res: axum::response::Response) -> (StatusCode, serde_json::Value) {
        let status = res.status();
        let bytes = axum::body::to_bytes(res.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or_default();
        (status, value)
    }

    /// Serve canned HTTP/1.0 replies in sequence, then exit. Binds port 0.
    fn serve_seq(
        replies: Vec<(&'static str, &'static [u8])>,
    ) -> (String, std::thread::JoinHandle<()>) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        let url = format!(
            "http://127.0.0.1:{}/",
            listener.local_addr().unwrap().port()
        );
        let handle = std::thread::spawn(move || {
            for (status_line, body) in replies {
                let (mut stream, _) = listener.accept().expect("accept");
                let mut buf = [0u8; 4096];
                let _ = stream.read(&mut buf);
                let head = format!(
                    "{status_line}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                stream.write_all(head.as_bytes()).expect("write head");
                stream.write_all(body).expect("write body");
            }
        });
        (url, handle)
    }

    /// A loopback URL nothing answers on (bind-then-drop a probe port).
    fn dead_url() -> String {
        let probe = std::net::TcpListener::bind("127.0.0.1:0").expect("bind probe");
        let port = probe.local_addr().unwrap().port();
        drop(probe);
        format!("http://127.0.0.1:{port}/")
    }

    #[tokio::test]
    async fn status_shape_with_empty_store() {
        let fx = Fixture::new();
        let (status, body) = json_body(
            fx.app()
                .oneshot(authed(
                    "GET",
                    "/apostille/api/v1/workers/status",
                    STAFF_TOKEN,
                    None,
                ))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["mode"], "manual");
        assert_eq!(body["fetch"]["backend"], "ureq");
        assert_eq!(body["fetch"]["timeout_secs"], 20);
        assert_eq!(body["queue"]["captures"], 0);
        assert_eq!(body["queue"]["sources"], 0);
        assert_eq!(body["queue"]["open_fetch_failures"], 0);
        assert!(body["last_sweep"].is_null());
    }

    #[tokio::test]
    async fn require_ip_gate_applies_to_merged_workers_routes() {
        let mut fx = Fixture::new();
        fx.state.admin_mtls_off = false;
        fx.state.admin_allow_ips = vec![];
        let (status, body) = json_body(
            fx.app()
                .oneshot(authed_host(
                    "GET",
                    "/apostille/api/v1/workers/status",
                    STAFF_TOKEN,
                    None,
                    "admin.andrii.ai",
                ))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        let msg = body["error"].as_str().unwrap();
        assert!(msg.contains("admin.allow_ips"), "points at the key: {msg}");
    }

    #[tokio::test]
    async fn guards_reject_anonymous_nonstaff_wronghost_and_csrf() {
        let fx = Fixture::new();
        let status_url = "/apostille/api/v1/workers/status";
        // No session cookie → 401.
        let req = Request::builder()
            .uri(status_url)
            .header("host", "localhost")
            .extension(axum::extract::ConnectInfo(
                "127.0.0.1:443".parse::<std::net::SocketAddr>().unwrap(),
            ))
            .body(Body::empty())
            .unwrap();
        let (status, _) = json_body(fx.app().oneshot(req).await.unwrap()).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        // Customer role → 403.
        let (status, _) = json_body(
            fx.app()
                .oneshot(authed("GET", status_url, CUSTOMER_TOKEN, None))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        // Wrong host → 403.
        let (status, _) = json_body(
            fx.app()
                .oneshot(authed_host(
                    "GET",
                    status_url,
                    STAFF_TOKEN,
                    None,
                    "evil.test",
                ))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        // Mutation without Origin/CSRF pair → 403.
        let req = Request::builder()
            .uri("/apostille/api/v1/workers/fetch")
            .method("POST")
            .header("host", "localhost")
            .header("cookie", format!("{}={STAFF_TOKEN}", auth::ADM_COOKIE))
            .header("content-type", "application/json")
            .body(Body::from(
                json!({"slug": "usa-wa-sos", "url": dead_url()}).to_string(),
            ))
            .unwrap();
        let (status, _) = json_body(fx.app().oneshot(req).await.unwrap()).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn merge_preserves_existing_admin_routes() {
        let fx = Fixture::new();
        // Staff self-view still answers through the merged router.
        let (status, body) = json_body(
            fx.app()
                .oneshot(authed("GET", "/apostille/api/v1/me", STAFF_TOKEN, None))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["user_id"], "staff-1");
        // Unknown admin API paths stay JSON 404s (wildcard intact).
        let (status, body) = json_body(
            fx.app()
                .oneshot(authed("GET", "/apostille/api/v1/nope", STAFF_TOKEN, None))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body["error"], "not found");
    }

    #[tokio::test]
    async fn captures_list_filters_and_paginates() {
        let fx = Fixture::new();
        let first = fx.seed_capture(
            "usa-wa-sos",
            Some("https://example.com/synthetic/1"),
            "warc",
        );
        let second = fx.seed_capture(
            "usa-or-sos",
            Some("https://example.com/synthetic/2"),
            "warc",
        );
        let base = "/apostille/api/v1/workers/captures";
        let (status, body) = json_body(
            fx.app()
                .oneshot(authed("GET", base, STAFF_TOKEN, None))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["count"], 2);
        assert_eq!(body["captures"][0]["id"], second, "newest first");
        assert_eq!(body["captures"][1]["id"], first);
        // Authority filter.
        let (status, body) = json_body(
            fx.app()
                .oneshot(authed(
                    "GET",
                    &format!("{base}?authority=usa-or-sos"),
                    STAFF_TOKEN,
                    None,
                ))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["count"], 1);
        assert_eq!(body["captures"][0]["authority_slug"], "usa-or-sos");
        // Unknown authority → 404.
        let (status, _) = json_body(
            fx.app()
                .oneshot(authed(
                    "GET",
                    &format!("{base}?authority=nope"),
                    STAFF_TOKEN,
                    None,
                ))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        // Pagination.
        let (status, body) = json_body(
            fx.app()
                .oneshot(authed(
                    "GET",
                    &format!("{base}?before_id={second}"),
                    STAFF_TOKEN,
                    None,
                ))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["count"], 1);
        assert_eq!(body["captures"][0]["id"], first);
    }

    #[tokio::test]
    async fn captures_carry_text_excerpt() {
        let fx = Fixture::new();
        let id = fx.seed_capture("usa-or-sos", None, "file");
        let long = "word ".repeat(60);
        std::fs::create_dir_all(fx.state.store_root.join("text")).unwrap();
        std::fs::write(
            fx.state.store_root.join("text").join(format!("{id}.txt")),
            &long,
        )
        .unwrap();
        let (status, body) = json_body(
            fx.app()
                .oneshot(authed(
                    "GET",
                    "/apostille/api/v1/workers/captures",
                    STAFF_TOKEN,
                    None,
                ))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let excerpt = body["captures"][0]["excerpt"].as_str().unwrap();
        assert_eq!(excerpt.chars().count(), 160, "{excerpt}");
        assert!(long.starts_with(excerpt.trim_end()), "{excerpt}");
    }

    #[tokio::test]
    async fn authorities_carry_ingest_state_and_watermarks() {
        let fx = Fixture::new();
        fx.seed_capture(
            "usa-wa-sos",
            Some("https://example.com/synthetic/1"),
            "warc",
        );
        let meta = fx.meta();
        db::upsert_watermark(&meta, "usa-wa-sos", 1_800_000_200, "v3").unwrap();
        let (status, body) = json_body(
            fx.app()
                .oneshot(authed(
                    "GET",
                    "/apostille/api/v1/workers/authorities",
                    STAFF_TOKEN,
                    None,
                ))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["count"], 2);
        let or = &body["authorities"][0];
        assert_eq!(or["slug"], "usa-or-sos", "ordered by slug");
        assert!(or["last_capture_at"].is_null());
        assert!(or["last_analysis_at"].is_null());
        let wa = &body["authorities"][1];
        assert_eq!(wa["slug"], "usa-wa-sos");
        assert_eq!(wa["sources"], 1);
        assert_eq!(wa["captures"], 1);
        assert!(wa["last_capture_at"].is_number());
        assert_eq!(wa["open_fetch_failures"], 0);
        assert_eq!(wa["last_analysis_at"], 1_800_000_200);
        assert_eq!(wa["last_prompt_version"], "v3");
    }

    #[tokio::test]
    async fn fetch_creates_then_dedups_and_audits() {
        let fx = Fixture::new();
        let (url, server) = serve_seq(vec![
            ("HTTP/1.0 200 OK", b"<p>PO Box 40220</p>" as &[u8]),
            ("HTTP/1.0 200 OK", b"<p>PO Box 40220</p>" as &[u8]),
        ]);
        let payload = || Some(json!({"slug": "usa-wa-sos", "url": url}));
        let (status, first) = json_body(
            fx.app()
                .oneshot(authed(
                    "POST",
                    "/apostille/api/v1/workers/fetch",
                    STAFF_TOKEN,
                    payload(),
                ))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(first["outcome"], "created");
        let id = first["capture_id"].as_i64().unwrap();
        let (status, second) = json_body(
            fx.app()
                .oneshot(authed(
                    "POST",
                    "/apostille/api/v1/workers/fetch",
                    STAFF_TOKEN,
                    payload(),
                ))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(second["outcome"], "duplicate");
        assert_eq!(second["capture_id"], id);
        server.join().unwrap();
        let meta = fx.meta();
        let audits: i64 = meta
            .query_row(
                "SELECT COUNT(*) FROM audit WHERE action = 'worker_fetch' AND actor = 'staff-1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(audits, 2);
        let captures: i64 = meta
            .query_row("SELECT COUNT(*) FROM capture WHERE id > 0", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(captures, 1, "dedup stores one row");
    }

    #[tokio::test]
    async fn fetch_validates_and_records_failures_as_tasks() {
        let fx = Fixture::new();
        let post = |body: serde_json::Value| {
            authed(
                "POST",
                "/apostille/api/v1/workers/fetch",
                STAFF_TOKEN,
                Some(body),
            )
        };
        // Unknown slug → 404.
        let (status, _) = json_body(
            fx.app()
                .oneshot(post(json!({"slug": "nope", "url": dead_url()})))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        // Non-http URL → 422.
        let (status, _) = json_body(
            fx.app()
                .oneshot(post(
                    json!({"slug": "usa-wa-sos", "url": "ftp://example.com/x"}),
                ))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        // Bad enum → 422.
        let (status, _) = json_body(
            fx.app()
                .oneshot(post(
                    json!({"slug": "usa-wa-sos", "url": dead_url(), "trust": "bogus"}),
                ))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        // Dead URL → 502 + one recorded task, refreshed (not duplicated) on retry.
        let url = dead_url();
        for _ in 0..2 {
            let (status, body) = json_body(
                fx.app()
                    .oneshot(post(json!({"slug": "usa-wa-sos", "url": url})))
                    .await
                    .unwrap(),
            )
            .await;
            assert_eq!(status, StatusCode::BAD_GATEWAY);
            assert_eq!(body["task"], "recorded");
        }
        let meta = fx.meta();
        let tasks: i64 = meta
            .query_row(
                "SELECT COUNT(*) FROM task WHERE kind = 'validate_fact' AND status = 'open'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(tasks, 1);
        let (status, body) = json_body(
            fx.app()
                .oneshot(authed(
                    "GET",
                    "/apostille/api/v1/workers/status",
                    STAFF_TOKEN,
                    None,
                ))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["queue"]["open_fetch_failures"], 1);
    }

    #[tokio::test]
    async fn retry_recreates_or_dedups_and_handles_edge_cases() {
        let fx = Fixture::new();
        let (url, server) = serve_seq(vec![("HTTP/1.0 200 OK", b"retry bytes" as &[u8])]);
        let id = fx.seed_capture("usa-wa-sos", Some(&url), "warc");
        let (status, body) = json_body(
            fx.app()
                .oneshot(authed(
                    "POST",
                    &format!("/apostille/api/v1/workers/captures/{id}/retry"),
                    STAFF_TOKEN,
                    Some(json!({})),
                ))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["outcome"], "created", "new bytes create a new capture");
        assert_ne!(body["capture_id"].as_i64().unwrap(), id);
        server.join().unwrap();
        let meta = fx.meta();
        let retry_audits: i64 = meta
            .query_row(
                "SELECT COUNT(*) FROM audit WHERE action = 'worker_retry'
                 AND before_ref = ?1",
                [id.to_string()],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(retry_audits, 1);
        // Unknown id → 404.
        let (status, _) = json_body(
            fx.app()
                .oneshot(authed(
                    "POST",
                    "/apostille/api/v1/workers/captures/9999/retry",
                    STAFF_TOKEN,
                    Some(json!({})),
                ))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        // Unattributed capture → 409.
        let orphan = fx.seed_capture("usa-wa-sos", None, "file");
        let (status, _) = json_body(
            fx.app()
                .oneshot(authed(
                    "POST",
                    &format!("/apostille/api/v1/workers/captures/{orphan}/retry"),
                    STAFF_TOKEN,
                    Some(json!({})),
                ))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn robots_disallow_skips_and_persists_verdict() {
        let fx = Fixture::new();
        // The canned server answers every path (robots.txt included).
        let (url, server) = serve_seq(vec![(
            "HTTP/1.0 200 OK",
            b"User-agent: *\nDisallow: /" as &[u8],
        )]);
        let body = json!({
            "slug": "usa-wa-sos",
            "url": url,
            "source_kind": "website_thirdparty",
        });
        let (status, out) = json_body(
            fx.app()
                .oneshot(authed(
                    "POST",
                    "/apostille/api/v1/workers/fetch",
                    STAFF_TOKEN,
                    Some(body),
                ))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(out["outcome"], "skipped");
        assert_eq!(out["reason"], "robots disallow");
        server.join().unwrap();
        let meta = fx.meta();
        let robots: String = meta
            .query_row("SELECT robots FROM source", [], |row| row.get(0))
            .unwrap();
        assert!(robots.starts_with("disallow-all"), "got: {robots}");
        let captures: i64 = meta
            .query_row("SELECT COUNT(*) FROM capture WHERE id > 0", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(captures, 0, "blocked fetch stores nothing");
    }

    #[tokio::test]
    async fn sweep_runs_items_sequentially_and_audits_once() {
        let fx = Fixture::new();
        let (good, server) = serve_seq(vec![("HTTP/1.0 200 OK", b"sweep bytes" as &[u8])]);
        let bad = dead_url();
        let body = json!({"items": [
            {"slug": "usa-wa-sos", "url": good},
            {"slug": "usa-or-sos", "url": bad},
        ]});
        let (status, out) = json_body(
            fx.app()
                .oneshot(authed(
                    "POST",
                    "/apostille/api/v1/workers/sweep",
                    STAFF_TOKEN,
                    Some(body),
                ))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(out["summary"]["total"], 2);
        assert_eq!(out["summary"]["created"], 1);
        assert_eq!(out["summary"]["failed"], 1);
        assert_eq!(out["results"][0]["outcome"], "created");
        assert_eq!(out["results"][1]["outcome"], "failed");
        assert!(out["results"][1]["detail"].is_string());
        server.join().unwrap();
        let meta = fx.meta();
        let sweeps: i64 = meta
            .query_row(
                "SELECT COUNT(*) FROM audit WHERE action = 'watch_sweep' AND actor = 'staff-1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(sweeps, 1);
        // The sweep is now visible as the last run in status.
        let (status, body) = json_body(
            fx.app()
                .oneshot(authed(
                    "GET",
                    "/apostille/api/v1/workers/status",
                    STAFF_TOKEN,
                    None,
                ))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["last_sweep"]["actor"], "staff-1");
    }

    #[tokio::test]
    async fn sweep_validates_before_fetching_anything() {
        let fx = Fixture::new();
        let post = |body: serde_json::Value| {
            authed(
                "POST",
                "/apostille/api/v1/workers/sweep",
                STAFF_TOKEN,
                Some(body),
            )
        };
        // Empty → 422.
        let (status, _) =
            json_body(fx.app().oneshot(post(json!({"items": []}))).await.unwrap()).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        // Eleven items → 422.
        let many: Vec<serde_json::Value> = (0..11)
            .map(|_| json!({"slug": "usa-wa-sos", "url": dead_url()}))
            .collect();
        let (status, _) = json_body(
            fx.app()
                .oneshot(post(json!({"items": many})))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        // Unknown slug → 404, nothing fetched.
        let (url, server) = serve_seq(vec![("HTTP/1.0 200 OK", b"must not fetch" as &[u8])]);
        let (status, _) = json_body(
            fx.app()
                .oneshot(post(json!({"items": [
                    {"slug": "usa-wa-sos", "url": url},
                    {"slug": "nope", "url": url},
                ]})))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        // Drop the server without joining: validation must have failed
        // before any connection, so the serving thread is still parked
        // in `accept` (joining would hang).
        drop(server);
        let meta = fx.meta();
        let captures: i64 = meta
            .query_row("SELECT COUNT(*) FROM capture WHERE id > 0", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(captures, 0);
    }
}
