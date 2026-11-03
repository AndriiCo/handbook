//! Approval API (roadmap item 6): staff list/accept/reject over
//! proposed facts, every decision audited, plus the conflict banner.
//!
//! Conflict policy (decided: **banner**): facts sharing
//! `(authority, topic)` with differing statements publish with a
//! visible banner citing both sources. Accept proceeds despite
//! conflicts; the banner is the policy. Anonymous readers see
//! accepted facts only ([`public_facts`]).

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Json},
};
use handbook_store::{db, store::Store};
use rusqlite::{Connection, OptionalExtension as _};
use serde_json::json;

use crate::{AppState, admin::AdminAuth, store_unavailable, unix_now};

/// One conflicting sibling: same authority + topic, other statement.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ConflictView {
    pub id: i64,
    pub statement: String,
    pub status: String,
}

/// One quote-verified citation on a fact (A1): the exact source quote,
/// the captured page it came from, and when it was captured.
#[derive(Debug, Clone, serde::Serialize)]
pub struct PublicCitation {
    pub quote: String,
    pub source_url: String,
    pub captured_at: String,
}

/// One fact with its conflict banner.
#[derive(Debug, Clone, serde::Serialize)]
pub struct FactView {
    pub id: i64,
    pub authority_slug: String,
    pub topic: String,
    pub statement: String,
    pub status: String,
    pub created_by: String,
    /// Model that drafted the fact (`created_by` carries it as
    /// `"<model> via <pipeline>"`; surfaced under its own key for A1).
    pub drafted_by: String,
    pub created_at: i64,
    pub citations: Vec<PublicCitation>,
    pub conflict: bool,
    pub conflicts: Vec<ConflictView>,
}

/// Sibling facts that disagree: same authority + topic, different
/// statement, still live (`accepted`/`proposed`). Capped at 10.
pub fn sibling_conflicts(
    conn: &Connection,
    authority_id: i64,
    topic: &str,
    statement: &str,
    exclude_id: i64,
) -> Vec<ConflictView> {
    conn.prepare(
        "SELECT id, statement, status FROM fact
         WHERE authority_id = ?1 AND topic = ?2 AND id != ?3
           AND status IN ('accepted', 'proposed')
         ORDER BY id LIMIT 10",
    )
    .and_then(|mut stmt| {
        stmt.query_map(rusqlite::params![authority_id, topic, exclude_id], |row| {
            Ok(ConflictView {
                id: row.get(0)?,
                statement: row.get(1)?,
                status: row.get(2)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()
    })
    .unwrap_or_default()
    .into_iter()
    .filter(|c| c.statement != statement)
    .collect()
}

/// One fact row as selected (list/decide/public share the shape).
struct StoredFact {
    id: i64,
    authority_id: i64,
    slug: String,
    topic: String,
    statement: String,
    status: String,
    created_by: String,
    created_at: i64,
    capture_id: i64,
    citation_start: i64,
    citation_end: i64,
}

fn fact_view(
    conn: &Connection,
    store_root: &std::path::Path,
    fact: &StoredFact,
    public_only: bool,
) -> FactView {
    let mut conflicts = sibling_conflicts(
        conn,
        fact.authority_id,
        &fact.topic,
        &fact.statement,
        fact.id,
    );
    if public_only {
        conflicts.retain(|c| c.status == "accepted");
    }
    let conflict = !conflicts.is_empty();
    FactView {
        id: fact.id,
        authority_slug: fact.slug.clone(),
        topic: fact.topic.clone(),
        statement: fact.statement.clone(),
        status: fact.status.clone(),
        created_by: fact.created_by.clone(),
        drafted_by: fact.created_by.clone(),
        created_at: fact.created_at,
        citations: public_citations(conn, store_root, fact),
        conflict,
        conflicts,
    }
}

/// Quote-verified citations for one fact (A1): at most one entry, only
/// when the cited sidecar slice exists and is long enough to verify.
/// The URL is the capture's own source, falling back to the
/// authority's official website source; unattributed or uncited facts
/// honestly carry zero citations. Quotes over 400 chars and non-https
/// sources never ship: a `javascript:` or `mailto:` URI from a pack
/// cannot reach the public dossier.
fn public_citations(
    conn: &Connection,
    store_root: &std::path::Path,
    fact: &StoredFact,
) -> Vec<PublicCitation> {
    if fact.capture_id <= 0 || fact.citation_start < 0 || fact.citation_end <= fact.citation_start {
        return vec![];
    }
    let quote = sidecar_slice(
        store_root,
        fact.capture_id,
        fact.citation_start,
        fact.citation_end,
    );
    let quote_len = quote.chars().count();
    if !(8..=400).contains(&quote_len) {
        return vec![];
    }
    let (source_id, captured_at): (Option<i64>, Option<i64>) = conn
        .query_row(
            "SELECT source_id, captured_at FROM capture WHERE id = ?1",
            [fact.capture_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap_or((None, None));
    let captured_at = captured_at.unwrap_or(0);
    let mut source_url: Option<String> = source_id.and_then(|id| {
        conn.query_row("SELECT uri FROM source WHERE id = ?1", [id], |row| {
            row.get(0)
        })
        .ok()
    });
    if source_url.is_none() {
        source_url = conn
            .query_row(
                "SELECT uri FROM source WHERE authority_id = ?1 AND kind = 'website_official'
                 ORDER BY id LIMIT 1",
                [fact.authority_id],
                |row| row.get(0),
            )
            .ok();
    }
    let Some(source_url) = source_url else {
        return vec![];
    };
    if !source_url.starts_with("https://") {
        return vec![];
    }
    vec![PublicCitation {
        quote,
        source_url,
        captured_at: iso8601(captured_at),
    }]
}

/// Server-sliced sidecar quote: `""` on missing file or bad bounds.
fn sidecar_slice(store_root: &std::path::Path, capture_id: i64, start: i64, end: i64) -> String {
    let bytes = std::fs::read(store_root.join("text").join(format!("{capture_id}.txt")))
        .unwrap_or_default();
    if start < 0 || end > bytes.len() as i64 || start >= end {
        return String::new();
    }
    let (start, end) = (start as usize, end as usize);
    String::from_utf8_lossy(&bytes[start..end]).to_string()
}

/// Unix seconds → `YYYY-MM-DDTHH:MM:SSZ` (civil-from-days, no date crate).
fn iso8601(unix: i64) -> String {
    let days = unix.div_euclid(86_400);
    let secs = unix.rem_euclid(86_400);
    // Howard Hinnant's civil-from-days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!(
        "{y:04}-{:02}-{d:02}T{:02}:{:02}:{:02}Z",
        m,
        secs / 3_600,
        (secs % 3_600) / 60,
        secs % 60
    )
}

fn open(state: &AppState) -> Option<Connection> {
    db::open(&Store::new(&state.store_root)).ok()
}

#[derive(Debug, serde::Deserialize)]
pub struct ListParams {
    /// Fact status filter; default `proposed`.
    pub status: Option<String>,
    /// Authority slug filter.
    pub authority: Option<String>,
    /// Page size, default 50, max 200.
    pub limit: Option<i64>,
}

/// Staff list of facts (default: proposed), each with its banner.
pub async fn list_facts(
    _auth: AdminAuth,
    State(state): State<AppState>,
    Query(params): Query<ListParams>,
) -> impl IntoResponse {
    let status = params.status.as_deref().unwrap_or("proposed");
    if !["proposed", "accepted", "rejected", "superseded"].contains(&status) {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"error": "unknown status"})),
        )
            .into_response();
    }
    let limit = params.limit.unwrap_or(50).clamp(1, 200);
    let Some(conn) = open(&state) else {
        return store_unavailable();
    };
    let mut sql = "SELECT f.id, f.authority_id, a.slug, f.topic, f.statement,
                          f.status, f.created_by, f.created_at,
                          f.capture_id, f.citation_start, f.citation_end
                   FROM fact f JOIN authority a ON a.id = f.authority_id
                   WHERE f.status = ?1"
        .to_string();
    if params.authority.is_some() {
        sql.push_str(" AND a.slug = ?2");
    }
    sql.push_str(" ORDER BY f.id LIMIT ?");
    fn map_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<StoredFact> {
        Ok(StoredFact {
            id: row.get(0)?,
            authority_id: row.get(1)?,
            slug: row.get(2)?,
            topic: row.get(3)?,
            statement: row.get(4)?,
            status: row.get(5)?,
            created_by: row.get(6)?,
            created_at: row.get(7)?,
            capture_id: row.get(8)?,
            citation_start: row.get(9)?,
            citation_end: row.get(10)?,
        })
    }
    let rows: Vec<StoredFact> = (|| {
        let mut stmt = conn.prepare(&sql)?;
        match params.authority.as_deref() {
            Some(slug) => stmt
                .query_map(rusqlite::params![status, slug, limit], map_row)?
                .collect::<rusqlite::Result<Vec<_>>>(),
            None => stmt
                .query_map(rusqlite::params![status, limit], map_row)?
                .collect::<rusqlite::Result<Vec<_>>>(),
        }
    })()
    .unwrap_or_default();
    let facts: Vec<FactView> = rows
        .iter()
        .map(|row| fact_view(&conn, &state.store_root, row, false))
        .collect();
    let count = facts.len();
    (
        StatusCode::OK,
        Json(json!({"facts": facts, "count": count})),
    )
        .into_response()
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SuppressBody {
    /// `authority` (fact's authority) or `global` (all authorities).
    pub scope: String,
    /// Lock the rule to the fact's topic (default true when omitted).
    pub lock_topic: Option<bool>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionBody {
    pub reason: Option<String>,
    /// Accept-only: replacement statement (non-empty, ≤2000 chars).
    pub statement: Option<String>,
    /// Reject-only: also create a `suppress_rule` row.
    pub suppress: Option<SuppressBody>,
}

/// Split an accept-with-edit statement of the analyser's
/// `{field}: {value}` shape into its edited value: `Some(value)` only
/// when the key matches the guide's field-key grammar
/// (`[a-z][a-z0-9_]*`, colon, whitespace). Anything else is an
/// unkeyed statement and skips the value-in-quote gate.
fn edited_value(statement: &str) -> Option<&str> {
    let (key, rest) = statement.split_once(':')?;
    if !key.chars().next().is_some_and(|c| c.is_ascii_lowercase())
        || !key
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
    {
        return None;
    }
    let value = rest.trim_start_matches(|c: char| c.is_whitespace());
    if rest.len() == value.len() || value.is_empty() {
        return None;
    }
    Some(value)
}

/// Accept one proposed fact (banner policy: conflicts do not block).
/// Audited as `fact_accept`, then fanned out as `fact_accepted` on
/// the authority channel.
pub async fn accept_fact(
    auth: AdminAuth,
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(body): Json<DecisionBody>,
) -> impl IntoResponse {
    if body.suppress.is_some() {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"error": "suppress is reject-only"})),
        )
            .into_response();
    }
    let statement = match body.statement.as_deref() {
        None => None,
        Some(s) if s.trim().is_empty() || s.len() > 2000 => {
            return (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(json!({"error": "statement must be 1-2000 chars"})),
            )
                .into_response();
        }
        Some(s) => Some(s),
    };
    // Accept-with-edit value-in-quote gate: a `field: value`
    // replacement must keep its value inside the fact's cited quote
    // (same normalization as the analysis validator), else the edit
    // publishes unquoted text. A missing fact or a down store falls
    // through to `decide`, which answers 404/503 for those.
    if let Some(s) = statement
        && let Some(value) = edited_value(s)
    {
        let inside = open(&state).and_then(|conn| {
            let bounds: Option<(i64, i64, i64)> = conn
                .query_row(
                    "SELECT capture_id, citation_start, citation_end FROM fact WHERE id = ?1",
                    rusqlite::params![id],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .optional()
                .unwrap_or(None);
            bounds.map(|(capture_id, start, end)| {
                let quote = sidecar_slice(&state.store_root, capture_id, start, end);
                handbook_agent::analyze::normalize(&quote)
                    .contains(&handbook_agent::analyze::normalize(value))
            })
        });
        if inside == Some(false) {
            return (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(json!({"error": "edited value not in citation quote"})),
            )
                .into_response();
        }
    }
    decide(
        &state,
        &auth,
        id,
        body.reason.as_deref(),
        statement,
        None,
        "accepted",
    )
    .await
}

/// Reject one proposed fact; a non-empty reason is required.
/// Audited as `fact_reject`.
pub async fn reject_fact(
    auth: AdminAuth,
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(body): Json<DecisionBody>,
) -> impl IntoResponse {
    let reason = body.reason.as_deref().unwrap_or("").trim();
    if reason.is_empty() || reason.len() > 1024 {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"error": "reason required (1-1024 chars)"})),
        )
            .into_response();
    }
    if body.statement.is_some() {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"error": "statement is accept-only"})),
        )
            .into_response();
    }
    if let Some(sup) = body.suppress.as_ref()
        && sup.scope != "authority"
        && sup.scope != "global"
    {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"error": "suppress.scope must be authority|global"})),
        )
            .into_response();
    }
    decide(
        &state,
        &auth,
        id,
        Some(reason),
        None,
        body.suppress.as_ref(),
        "rejected",
    )
    .await
}

/// Reject-with-suppress: adopt (or reuse) the matching live rule.
/// Runs inside the caller's `BEGIN IMMEDIATE`, which serializes
/// concurrent deciders, so check-then-insert cannot duplicate.
fn adopt_suppress_rule(
    conn: &Connection,
    auth: &AdminAuth,
    fact: &StoredFact,
    sup: &SuppressBody,
    reason: &str,
    now: i64,
) -> rusqlite::Result<(i64, bool)> {
    let scope_authority = (sup.scope == "authority").then_some(fact.authority_id);
    let topic = sup.lock_topic.unwrap_or(true).then(|| fact.topic.clone());
    let norm = db::normalize_statement(&fact.statement);
    let existing: Option<i64> = conn
        .query_row(
            "SELECT id FROM suppress_rule
             WHERE revoked_at IS NULL AND statement_norm = ?1
               AND authority_id IS NOT DISTINCT FROM ?2
               AND topic IS NOT DISTINCT FROM ?3
             ORDER BY id LIMIT 1",
            rusqlite::params![norm, scope_authority, topic],
            |row| row.get(0),
        )
        .optional()?;
    if let Some(found) = existing {
        return Ok((found, true));
    }
    conn.execute(
        "INSERT INTO suppress_rule(authority_id, topic, statement_norm,
                                   statement, reason, created_by, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        rusqlite::params![
            scope_authority,
            topic,
            norm,
            fact.statement,
            reason,
            auth.user.user_id,
            now,
        ],
    )?;
    let rule_id = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO audit(at, actor, action, entity, entity_id, reason)
         VALUES (?1, ?2, 'suppress_rule.create', 'suppress_rule', ?3, ?4)",
        rusqlite::params![now, auth.user.user_id, rule_id, reason],
    )?;
    Ok((rule_id, false))
}

/// Most facts one bulk decision may cover.
const MAX_BULK: usize = 100;

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BulkBody {
    /// `accept` or `reject`.
    pub decision: String,
    /// Every id must belong to this authority slug.
    pub authority: String,
    pub ids: Vec<i64>,
    /// Required (1-1024) for reject, optional for accept.
    pub reason: Option<String>,
    /// Reject-only: adopt a suppress rule per fact.
    pub suppress: Option<SuppressBody>,
    /// Accept-only: also refuse facts that change a published fact or
    /// that the Wonder cross-check found contradicted.
    #[serde(default)]
    pub require_verified: bool,
}

fn unprocessable(msg: &str) -> axum::response::Response {
    (
        StatusCode::UNPROCESSABLE_ENTITY,
        Json(json!({"error": msg})),
    )
        .into_response()
}

/// Latest Wonder verdict on a fact, tolerating a store without the
/// `fact_check` table.
fn latest_verdict(conn: &Connection, fact_id: i64) -> Option<String> {
    conn.query_row(
        "SELECT verdict FROM fact_check WHERE fact_id = ?1 ORDER BY id DESC LIMIT 1",
        [fact_id],
        |row| row.get(0),
    )
    .ok()
}

/// Accept or reject many proposed facts of ONE authority as one
/// decision: a preflight over every id, then all-or-nothing inside a
/// single `BEGIN IMMEDIATE`. Each fact keeps its own `fact_accept` /
/// `fact_reject` audit row (notify and the dashboard read those), plus
/// one `fact_bulk_*` header row; an accept publishes ONE
/// `fact_accepted` event carrying every id, so open dossiers reload
/// once per batch instead of once per fact.
pub async fn bulk_decide(
    auth: AdminAuth,
    State(state): State<AppState>,
    Json(body): Json<BulkBody>,
) -> axum::response::Response {
    let (to, action, header_action) = match body.decision.as_str() {
        "accept" => ("accepted", "fact_accept", "fact_bulk_accept"),
        "reject" => ("rejected", "fact_reject", "fact_bulk_reject"),
        _ => return unprocessable("decision must be accept or reject"),
    };
    if body.ids.is_empty() || body.ids.len() > MAX_BULK {
        return unprocessable("ids must hold 1-100 fact ids");
    }
    let mut seen = std::collections::HashSet::new();
    if body.ids.iter().any(|id| *id <= 0 || !seen.insert(*id)) {
        return unprocessable("ids must be distinct positive fact ids");
    }
    let reason = body.reason.as_deref().unwrap_or("").trim();
    if reason.len() > 1024 {
        return unprocessable("reason too long (max 1024 chars)");
    }
    if to == "rejected" && reason.is_empty() {
        return unprocessable("reason required (1-1024 chars)");
    }
    if let Some(sup) = body.suppress.as_ref() {
        if to != "rejected" {
            return unprocessable("suppress is reject-only");
        }
        if sup.scope != "authority" && sup.scope != "global" {
            return unprocessable("suppress.scope must be authority|global");
        }
    }
    let Some(conn) = open(&state) else {
        return store_unavailable();
    };
    let authority_id: Option<i64> = match conn
        .query_row(
            "SELECT id FROM authority WHERE slug = ?1",
            [&body.authority],
            |row| row.get(0),
        )
        .optional()
    {
        Ok(found) => found,
        Err(_) => return store_unavailable(),
    };
    let Some(authority_id) = authority_id else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "unknown authority"})),
        )
            .into_response();
    };
    let now = unix_now();
    type Blocked = Vec<serde_json::Value>;
    type Decided = (Vec<(StoredFact, Option<(i64, bool)>)>, i64);
    let outcome: rusqlite::Result<Result<Decided, Blocked>> = (|| {
        conn.execute_batch("BEGIN IMMEDIATE")?;
        let mut facts = Vec::with_capacity(body.ids.len());
        let mut results = Vec::with_capacity(body.ids.len());
        let mut blocked = false;
        for &id in &body.ids {
            let row: Option<(String, StoredFact)> = conn
                .query_row(
                    "SELECT f.status, f.authority_id, a.slug, f.topic, f.statement,
                            f.created_by, f.created_at,
                            f.capture_id, f.citation_start, f.citation_end
                     FROM fact f JOIN authority a ON a.id = f.authority_id
                     WHERE f.id = ?1",
                    [id],
                    |row| {
                        Ok((
                            row.get(0)?,
                            StoredFact {
                                id,
                                authority_id: row.get(1)?,
                                slug: row.get(2)?,
                                topic: row.get(3)?,
                                statement: row.get(4)?,
                                status: to.to_string(),
                                created_by: row.get(5)?,
                                created_at: row.get(6)?,
                                capture_id: row.get(7)?,
                                citation_start: row.get(8)?,
                                citation_end: row.get(9)?,
                            },
                        ))
                    },
                )
                .optional()?;
            let refusal: Option<(u16, &str)> = match &row {
                None => Some((404, "fact not found")),
                Some((_, fact)) if fact.authority_id != authority_id => {
                    Some((422, "fact not in authority"))
                }
                Some((status, _)) if status != "proposed" => Some((409, "fact is not proposed")),
                Some((_, fact))
                    if to == "accepted"
                        && public_citations(&conn, &state.store_root, fact).is_empty() =>
                {
                    Some((422, "accept requires a verified citation"))
                }
                Some((_, fact))
                    if to == "accepted"
                        && body.require_verified
                        && sibling_conflicts(
                            &conn,
                            fact.authority_id,
                            &fact.topic,
                            &fact.statement,
                            id,
                        )
                        .iter()
                        .any(|c| c.status == "accepted") =>
                {
                    Some((422, "changes a published fact; review it on its own"))
                }
                Some(_)
                    if to == "accepted"
                        && body.require_verified
                        && latest_verdict(&conn, id).as_deref() == Some("contradicted") =>
                {
                    Some((422, "the cross-check found the source contradicts it"))
                }
                Some(_) => None,
            };
            match refusal {
                Some((code, error)) => {
                    blocked = true;
                    results.push(json!({"id": id, "ok": false, "code": code, "error": error}));
                }
                None => results.push(json!({"id": id, "ok": true})),
            }
            if let Some((_, fact)) = row {
                facts.push(fact);
            }
        }
        if blocked {
            conn.execute_batch("ROLLBACK")?;
            return Ok(Err(results));
        }
        let mut decided = Vec::with_capacity(facts.len());
        for fact in facts {
            conn.execute(
                "UPDATE fact SET status = ?1 WHERE id = ?2",
                rusqlite::params![to, fact.id],
            )?;
            conn.execute(
                "INSERT INTO audit(at, actor, action, entity, entity_id,
                                    before_ref, after_ref, reason)
                 VALUES (?1, ?2, ?3, 'fact', ?4, 'proposed', ?5, ?6)",
                rusqlite::params![now, auth.user.user_id, action, fact.id, to, reason],
            )?;
            let rule = match body.suppress.as_ref() {
                Some(sup) => Some(adopt_suppress_rule(&conn, &auth, &fact, sup, reason, now)?),
                None => None,
            };
            decided.push((fact, rule));
        }
        let ids_ref = body
            .ids
            .iter()
            .map(i64::to_string)
            .collect::<Vec<_>>()
            .join(",");
        conn.execute(
            "INSERT INTO audit(at, actor, action, entity, entity_id, after_ref, reason)
             VALUES (?1, ?2, ?3, 'authority', ?4, ?5, ?6)",
            rusqlite::params![
                now,
                auth.user.user_id,
                header_action,
                authority_id,
                format!("ids:{ids_ref}"),
                reason,
            ],
        )?;
        let header_id = conn.last_insert_rowid();
        conn.execute_batch("COMMIT")?;
        Ok(Ok((decided, header_id)))
    })();
    let (decided, header_id) = match outcome {
        Ok(Ok(done)) => done,
        Ok(Err(results)) => {
            let stale = results.iter().any(|r| r["code"].as_u64() == Some(409));
            let status = if stale {
                StatusCode::CONFLICT
            } else {
                StatusCode::UNPROCESSABLE_ENTITY
            };
            return (
                status,
                Json(json!({"error": "batch blocked; nothing changed", "results": results})),
            )
                .into_response();
        }
        Err(_) => {
            let _ = conn.execute_batch("ROLLBACK");
            return store_unavailable();
        }
    };
    if to == "accepted" {
        if let Err(e) = notify::enqueue_from_audit(&conn, now) {
            eprintln!("notify: enqueue_from_audit failed: {e}");
        }
        state.notify_poke.notify_one();
        for (fact, _) in &decided {
            let _ = crate::translate::request_after_accept(&conn, fact.id, now);
        }
        crate::ws_bus::publish_authority_event(
            &state.bus,
            &body.authority,
            "fact_accepted",
            json!({"fact_id": body.ids[0], "fact_ids": body.ids, "count": body.ids.len()}),
            now,
        );
        crate::ws_bus::publish_ops(
            &state.bus,
            "fact_accepted",
            json!({"fact_id": body.ids[0], "fact_ids": body.ids, "slug": body.authority}),
            now,
        );
    }
    let results: Vec<serde_json::Value> = decided
        .iter()
        .map(|(fact, rule)| {
            let mut row = json!({"id": fact.id, "ok": true, "status": to});
            if let Some((rule_id, deduped)) = rule {
                row["rule"] = json!({"id": rule_id, "deduped": deduped});
            }
            row
        })
        .collect();
    (
        StatusCode::OK,
        Json(json!({
            "authority": body.authority,
            "decision": body.decision,
            "count": results.len(),
            "bulk_audit_id": header_id,
            "results": results,
        })),
    )
        .into_response()
}

#[allow(clippy::too_many_arguments)]
async fn decide(
    state: &AppState,
    auth: &AdminAuth,
    id: i64,
    reason: Option<&str>,
    statement: Option<&str>,
    suppress: Option<&SuppressBody>,
    to: &str,
) -> axum::response::Response {
    if reason.is_some_and(|r| r.len() > 1024) {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"error": "reason too long (max 1024 chars)"})),
        )
            .into_response();
    }
    let Some(conn) = open(state) else {
        return store_unavailable();
    };
    let now = unix_now();
    let action = if to == "accepted" {
        "fact_accept"
    } else {
        "fact_reject"
    };
    // Plain rusqlite plumbing (not `db::in_transaction`, whose
    // `contracts::Result` cannot carry our `StatusCode` outcomes).
    // Sync: `&Connection` is `!Send`, so it must not cross an await.
    decide_inner(
        state,
        auth,
        &conn,
        id,
        &Decision {
            to,
            reason,
            statement,
            suppress,
            now,
            action,
        },
    )
}

/// One accept/reject decision: target status plus audit attribution.
struct Decision<'a> {
    to: &'a str,
    reason: Option<&'a str>,
    /// Accept-only replacement statement (pre-validated).
    statement: Option<&'a str>,
    /// Reject-only suppression request (pre-validated).
    suppress: Option<&'a SuppressBody>,
    now: i64,
    action: &'a str,
}

/// Transaction outcome: DB failure, HTTP outcome, or the decided fact
/// plus an adopted rule id (`Some((id, deduped))` on reject-with-suppress).
type DecideOutcome = rusqlite::Result<Result<(StoredFact, Option<(i64, bool)>), StatusCode>>;

fn decide_inner(
    state: &AppState,
    auth: &AdminAuth,
    conn: &Connection,
    id: i64,
    decision: &Decision<'_>,
) -> axum::response::Response {
    let outcome: DecideOutcome = (|| {
        conn.execute_batch("BEGIN IMMEDIATE")?;
        let row: Option<(String, StoredFact)> = conn
            .query_row(
                "SELECT f.status, f.authority_id, a.slug, f.topic, f.statement,
                        f.created_by, f.created_at,
                        f.capture_id, f.citation_start, f.citation_end
                 FROM fact f JOIN authority a ON a.id = f.authority_id
                 WHERE f.id = ?1",
                [id],
                |row| {
                    Ok((
                        row.get(0)?,
                        StoredFact {
                            id,
                            authority_id: row.get(1)?,
                            slug: row.get(2)?,
                            topic: row.get(3)?,
                            statement: row.get(4)?,
                            status: decision.to.to_string(),
                            created_by: row.get(5)?,
                            created_at: row.get(6)?,
                            capture_id: row.get(7)?,
                            citation_start: row.get(8)?,
                            citation_end: row.get(9)?,
                        },
                    ))
                },
            )
            .optional()?;
        let Some((status, fact)) = row else {
            conn.execute_batch("ROLLBACK")?;
            return Ok(Err(StatusCode::NOT_FOUND));
        };
        if status != "proposed" {
            conn.execute_batch("ROLLBACK")?;
            return Ok(Err(StatusCode::CONFLICT));
        }
        // Accepting a fact requires a verified citation (operator fix
        // `a1-integrity`): quote 8-400 chars from the capture sidecar
        // over an https source. Uncited facts stay proposed.
        if decision.to == "accepted" && public_citations(conn, &state.store_root, &fact).is_empty()
        {
            conn.execute_batch("ROLLBACK")?;
            return Ok(Err(StatusCode::UNPROCESSABLE_ENTITY));
        }
        // Accept-with-edit: the replacement statement lands in the same
        // UPDATE; unchanged-or-absent keeps the current text. The audit
        // reason becomes the §11.5 envelope when the text changed.
        let edited = decision.statement.is_some_and(|s| s != fact.statement);
        if edited {
            conn.execute(
                "UPDATE fact SET status = ?1, statement = ?2 WHERE id = ?3",
                rusqlite::params![decision.to, decision.statement, id],
            )?;
        } else {
            conn.execute(
                "UPDATE fact SET status = ?1 WHERE id = ?2",
                (decision.to, id),
            )?;
        }
        let audit_reason = if edited {
            edit_envelope(decision.reason.unwrap_or(""), &fact.statement)
        } else {
            decision.reason.unwrap_or("").to_string()
        };
        conn.execute(
            "INSERT INTO audit(at, actor, action, entity, entity_id,
                                before_ref, after_ref, reason)
             VALUES (?1, ?2, ?3, 'fact', ?4, 'proposed', ?5, ?6)",
            rusqlite::params![
                decision.now,
                auth.user.user_id,
                decision.action,
                id,
                decision.to,
                audit_reason,
            ],
        )?;
        // Reject-with-suppress: idempotent rule adoption in the same
        // transaction (BEGIN IMMEDIATE serializes concurrent deciders,
        // so check-then-insert cannot duplicate).
        let mut rule_outcome: Option<(i64, bool)> = None;
        if let Some(sup) = decision.suppress {
            rule_outcome = Some(adopt_suppress_rule(
                conn,
                auth,
                &fact,
                sup,
                decision.reason.unwrap_or(""),
                decision.now,
            )?);
        }
        // Serve the decided text (post-edit) in the response view.
        let mut fact = fact;
        if edited {
            fact.statement = decision.statement.unwrap_or("").to_string();
        }
        conn.execute_batch("COMMIT")?;
        // Follower fan-out: tail the fresh `fact_accept` audit row
        // into the notify outbox, then poke the worker. Best-effort
        // past the COMMIT — the decision already stands, and the
        // worker's 60 s tick re-tails the cursor anyway.
        if decision.action == "fact_accept" {
            if let Err(e) = notify::enqueue_from_audit(conn, decision.now) {
                eprintln!("notify: enqueue_from_audit failed: {e}");
            }
            state.notify_poke.notify_one();
        }
        Ok(Ok((fact, rule_outcome)))
    })();
    let (fact, rule) = match outcome {
        Ok(Ok(found)) => found,
        Ok(Err(status)) => {
            let body = if status == StatusCode::NOT_FOUND {
                json!({"error": "fact not found"})
            } else if status == StatusCode::CONFLICT {
                json!({"error": "fact is not proposed"})
            } else {
                json!({"error": "accept requires a verified citation (quote 8-400 chars, https source)"})
            };
            return (status, Json(body)).into_response();
        }
        Err(_) => return store_unavailable(),
    };
    if decision.to == "accepted" {
        state.bus.publish(
            &format!("authority:{}", fact.slug),
            json!({"type": "fact_accepted", "topic": format!("authority:{}", fact.slug),
                   "fact_id": id, "at": decision.now})
            .to_string(),
        );
        // Dashboard activity feed (staff-only `ops`).
        crate::ws_bus::publish_ops(
            &state.bus,
            "fact_accepted",
            json!({"fact_id": id, "slug": fact.slug}),
            decision.now,
        );
        // SA-20: queue es/fr translation rows (best-effort;
        // staff-triggered render).
        let _ = crate::translate::request_after_accept(conn, id, decision.now);
    }
    let view = fact_view(conn, &state.store_root, &fact, false);
    if let Some((rule_id, deduped)) = rule {
        let mut body = serde_json::to_value(&view).unwrap_or(json!({}));
        body["rule"] = json!({"id": rule_id, "deduped": deduped});
        return (StatusCode::OK, Json(body)).into_response();
    }
    (StatusCode::OK, Json(view)).into_response()
}

/// §11.5 edit envelope: `{"note","edit_before"}`, capped at 1024
/// chars total by truncating `edit_before` with a `…` marker.
fn edit_envelope(note: &str, edit_before: &str) -> String {
    let envelope = |before: &str| json!({"note": note, "edit_before": before}).to_string();
    let full = envelope(edit_before);
    if full.len() <= 1024 {
        return full;
    }
    // Budget the fixed framing by measuring with an empty before.
    let framing = envelope("").len();
    let keep = 1024usize.saturating_sub(framing + "…".len()).max(1);
    let truncated: String = edit_before.chars().take(keep).collect();
    envelope(&format!("{truncated}…"))
}

/// Public accepted-facts read for one authority, with conflict
/// banners. Anonymous callers reach this; proposed/rejected facts
/// never appear, and banners cite accepted siblings only.
pub async fn public_facts(
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> impl IntoResponse {
    let Some(conn) = open(&state) else {
        return store_unavailable();
    };
    let Some(facts) = accepted_fact_views(&conn, &state.store_root, &slug) else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "unknown authority"})),
        )
            .into_response();
    };
    (
        StatusCode::OK,
        Json(json!({"authority": slug, "facts": facts})),
    )
        .into_response()
}

/// Accepted facts of one authority exactly as the public read serves
/// them (no `phone_script`, conflict banners on accepted siblings), or
/// `None` for an unknown slug. Shared by `/facts` and the `/plan` read.
pub fn accepted_fact_views(
    conn: &Connection,
    store_root: &std::path::Path,
    slug: &str,
) -> Option<Vec<FactView>> {
    let authority_id: i64 = conn
        .query_row("SELECT id FROM authority WHERE slug = ?1", [slug], |row| {
            row.get(0)
        })
        .optional()
        .unwrap_or(None)?;
    let rows: Vec<StoredFact> = conn
        .prepare(
            "SELECT id, topic, statement, status, created_by, created_at,
                    capture_id, citation_start, citation_end
             FROM fact WHERE authority_id = ?1 AND status = 'accepted'
               AND topic != 'phone_script'
             ORDER BY id LIMIT 200",
        )
        .and_then(|mut stmt| {
            stmt.query_map([authority_id], |row| {
                Ok(StoredFact {
                    id: row.get(0)?,
                    authority_id,
                    slug: slug.to_string(),
                    topic: row.get(1)?,
                    statement: row.get(2)?,
                    status: row.get(3)?,
                    created_by: row.get(4)?,
                    created_at: row.get(5)?,
                    capture_id: row.get(6)?,
                    citation_start: row.get(7)?,
                    citation_end: row.get(8)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()
        })
        .unwrap_or_default();
    Some(
        rows.iter()
            .map(|row| fact_view(conn, store_root, row, true))
            .collect(),
    )
}

/// Public authority index. Anonymous callers reach this; rows are
/// real authorities only (`id != 0` — the id-0 sentinel backs
/// unattributed facts and is never listed) with their accepted-fact
/// counts, ordered by slug.
///
/// v1 has no pagination: the authority table holds dozens of rows,
/// so one unbounded query stays cheap.
pub async fn public_authorities(State(state): State<AppState>) -> impl IntoResponse {
    let Some(conn) = open(&state) else {
        return store_unavailable();
    };
    let rows: Vec<serde_json::Value> = conn
        .prepare(
            // `kind` (country/state/province/office) drives the home
            // lanes; `verified_at` is the newest staff accept of any of
            // this authority's facts (audit `fact_accept`), so the UI
            // never hardcodes either.
            "SELECT a.slug, a.name, a.country, a.region,
                    COUNT(CASE WHEN f.status = 'accepted' THEN 1 END),
                    a.kind,
                    va.v
             FROM authority a LEFT JOIN fact f ON f.authority_id = a.id
             LEFT JOIN (SELECT f2.authority_id AS aid, MAX(au.at) AS v
                        FROM audit au JOIN fact f2 ON f2.id = au.entity_id
                        WHERE au.entity = 'fact' AND au.action = 'fact_accept'
                        GROUP BY f2.authority_id) va ON va.aid = a.id
             WHERE a.id != 0
             GROUP BY a.id ORDER BY a.slug",
        )
        .and_then(|mut stmt| {
            stmt.query_map([], |row| {
                Ok(json!({
                    "slug": row.get::<_, String>(0)?,
                    "name": row.get::<_, String>(1)?,
                    "country": row.get::<_, String>(2)?,
                    "region": row.get::<_, String>(3)?,
                    "fact_count": row.get::<_, i64>(4)?,
                    "kind": row.get::<_, String>(5)?,
                    "verified_at": row.get::<_, Option<i64>>(6)?,
                }))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()
        })
        .unwrap_or_default();
    // Readable addresses ride along in a separate fail-soft read, so a
    // path problem can never empty the index GP1/GP2 read.
    let paths = crate::authority_path::all_paths(&conn);
    let rows: Vec<serde_json::Value> = rows
        .into_iter()
        .map(|mut row| {
            if let Some(p) = row["slug"].as_str().and_then(|slug| paths.get(slug)) {
                row["path"] = json!(p.primary);
                row["aliases"] = json!(p.aliases);
            }
            row
        })
        .collect();
    (StatusCode::OK, Json(json!({"authorities": rows}))).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edited_value_splits_keyed_statements() {
        assert_eq!(edited_value("per_document: $8"), Some("$8"));
        assert_eq!(edited_value("weekday: Mon-Fri 8-5"), Some("Mon-Fri 8-5"));
        // Trailing prose after the value is part of the value.
        assert_eq!(edited_value("a:  b with spaces"), Some("b with spaces"));
    }

    #[test]
    fn edited_value_skips_unkeyed_statements() {
        assert_eq!(edited_value("plain statement, no key"), None);
        assert_eq!(edited_value("UPPER: not a key"), None);
        assert_eq!(edited_value("no colon at all"), None);
        assert_eq!(edited_value("field:nospace"), None);
        assert_eq!(edited_value("field:   "), None);
        assert_eq!(edited_value(": value"), None);
    }

    #[test]
    fn edited_value_uses_validator_normalization() {
        // The gate compares with the analysis validator's
        // normalization, so case and whitespace drift still match.
        let quote = "Walk-in customers go to Town Center 3, Tumwater.";
        let value = edited_value("counter: town center 3, tumwater").unwrap();
        assert!(
            handbook_agent::analyze::normalize(quote)
                .contains(&handbook_agent::analyze::normalize(value))
        );
        let estranged = edited_value("counter: $15 per document").unwrap();
        assert!(
            !handbook_agent::analyze::normalize(quote)
                .contains(&handbook_agent::analyze::normalize(estranged))
        );
    }
}
