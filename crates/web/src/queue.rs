//! Staff review queue (§11): enriched proposed facts, suppression
//! rules, and the skip log.
//!
//! Read-mostly views over `fact`/`capture`/`task`/`suppress_rule`/
//! `suppression`; the only mutations (accept-with-edit text,
//! reject-with-suppress rules) run through `approval::decide()` —
//! this module owns the standalone rule endpoints only. Error
//! vocabulary mirrors `approval` exactly (404 `fact not found` /
//! `unknown authority`, 409 `fact is not proposed` /
//! `duplicate rule` / `rule already revoked`, 422 validation).

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Json},
};
use handbook_store::{db, store::Store};
use rusqlite::{Connection, OptionalExtension as _};
use serde_json::json;

use crate::{AppState, admin::AdminAuth, approval::sibling_conflicts, store_unavailable, unix_now};

#[derive(Debug, serde::Deserialize)]
pub struct QueueParams {
    pub authority: Option<String>,
    pub topic: Option<String>,
    pub limit: Option<i64>,
    pub after_id: Option<i64>,
}

/// One enriched queue item (§11.2.1).
#[derive(Debug, serde::Serialize)]
pub struct QueueItem {
    pub id: i64,
    pub authority_slug: String,
    pub topic: String,
    pub statement: String,
    pub status: String,
    pub created_by: String,
    pub created_at: i64,
    pub diff: Option<QueueDiff>,
    pub citations: Vec<QueueCitation>,
    pub reasoning_untrusted: Option<String>,
    pub blast_radius: BlastRadius,
    pub provenance: Provenance,
}

#[derive(Debug, serde::Serialize)]
pub struct QueueDiff {
    pub base_statement: String,
    pub base_fact_id: i64,
    pub base_status: String,
}

#[derive(Debug, serde::Serialize)]
pub struct QueueCitation {
    pub capture_id: i64,
    pub start_byte: i64,
    pub end_byte: i64,
    pub quote: String,
}

#[derive(Debug, serde::Serialize)]
pub struct BlastRadius {
    pub conflict: bool,
    pub conflicts: Vec<crate::approval::ConflictView>,
    pub open_tasks: i64,
}

#[derive(Debug, serde::Serialize)]
pub struct Provenance {
    pub capture_id: i64,
    pub capture_kind: String,
    pub trust: String,
    pub authenticity: String,
    pub sidecar: String,
}

struct QueueRow {
    id: i64,
    authority_id: i64,
    authority_slug: String,
    topic: String,
    statement: String,
    status: String,
    created_by: String,
    created_at: i64,
    capture_id: i64,
    citation_start: i64,
    citation_end: i64,
    reasoning: String,
}

/// Staff list of proposed facts with decide-inline context.
pub async fn queue_items(
    _auth: AdminAuth,
    State(state): State<AppState>,
    Query(params): Query<QueueParams>,
) -> impl IntoResponse {
    if let Some(topic) = params.topic.as_deref()
        && contracts::topics::Topic::parse(topic).is_err()
    {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"error": "unknown topic"})),
        )
            .into_response();
    }
    let limit = params.limit.unwrap_or(50).clamp(1, 200);
    let Some(conn) = open(&state) else {
        return store_unavailable();
    };
    if let Some(slug) = params.authority.as_deref()
        && authority_id(&conn, slug).is_none()
    {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "unknown authority"})),
        )
            .into_response();
    }
    let rows = match queue_rows(
        &conn,
        params.authority.as_deref(),
        params.topic.as_deref(),
        limit,
        params.after_id,
    ) {
        Ok(rows) => rows,
        Err(_) => return store_unavailable(),
    };
    let items: Vec<QueueItem> = rows.iter().map(|row| enrich(&conn, &state, row)).collect();
    let count = items.len();
    (
        StatusCode::OK,
        Json(json!({"items": items, "count": count})),
    )
        .into_response()
}

/// One enriched queue item by id (any status).
pub async fn queue_item(
    _auth: AdminAuth,
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> impl IntoResponse {
    let Some(conn) = open(&state) else {
        return store_unavailable();
    };
    let row: Option<QueueRow> = conn
        .query_row(
            "SELECT f.id, f.authority_id, a.slug, f.topic, f.statement, f.status,
                    f.created_by, f.created_at, f.capture_id,
                    f.citation_start, f.citation_end, f.reasoning
             FROM fact f JOIN authority a ON a.id = f.authority_id
             WHERE f.id = ?1",
            [id],
            scan_queue_row,
        )
        .optional()
        .unwrap_or(None);
    let Some(row) = row else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "fact not found"})),
        )
            .into_response();
    };
    let item = enrich(&conn, &state, &row);
    (StatusCode::OK, Json(item)).into_response()
}

fn queue_rows(
    conn: &Connection,
    authority: Option<&str>,
    topic: Option<&str>,
    limit: i64,
    after_id: Option<i64>,
) -> rusqlite::Result<Vec<QueueRow>> {
    let mut sql = "SELECT f.id, f.authority_id, a.slug, f.topic, f.statement, f.status,
                          f.created_by, f.created_at, f.capture_id,
                          f.citation_start, f.citation_end, f.reasoning
                   FROM fact f JOIN authority a ON a.id = f.authority_id
                   WHERE f.status = 'proposed'"
        .to_string();
    enum Bind<'a> {
        Text(&'a str),
        Int(i64),
    }
    let mut binds: Vec<Bind<'_>> = Vec::new();
    let mut next = 1;
    if let Some(slug) = authority {
        sql.push_str(&format!(" AND a.slug = ?{next}"));
        binds.push(Bind::Text(slug));
        next += 1;
    }
    if let Some(topic) = topic {
        sql.push_str(&format!(" AND f.topic = ?{next}"));
        binds.push(Bind::Text(topic));
        next += 1;
    }
    if let Some(after) = after_id {
        sql.push_str(&format!(" AND f.id > ?{next}"));
        binds.push(Bind::Int(after));
        next += 1;
    }
    sql.push_str(&format!(" ORDER BY f.id LIMIT ?{next}"));
    binds.push(Bind::Int(limit));
    let values: Vec<rusqlite::types::Value> = binds
        .into_iter()
        .map(|b| match b {
            Bind::Text(s) => rusqlite::types::Value::Text(s.to_string()),
            Bind::Int(n) => rusqlite::types::Value::Integer(n),
        })
        .collect();
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(rusqlite::params_from_iter(values.iter()), scan_queue_row)?;
    rows.collect::<rusqlite::Result<_>>()
}

fn scan_queue_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<QueueRow> {
    Ok(QueueRow {
        id: row.get(0)?,
        authority_id: row.get(1)?,
        authority_slug: row.get(2)?,
        topic: row.get(3)?,
        statement: row.get(4)?,
        status: row.get(5)?,
        created_by: row.get(6)?,
        created_at: row.get(7)?,
        capture_id: row.get(8)?,
        citation_start: row.get(9)?,
        citation_end: row.get(10)?,
        reasoning: row.get(11)?,
    })
}

fn authority_id(conn: &Connection, slug: &str) -> Option<i64> {
    conn.query_row("SELECT id FROM authority WHERE slug = ?1", [slug], |row| {
        row.get(0)
    })
    .optional()
    .unwrap_or(None)
}

fn enrich(conn: &Connection, state: &AppState, row: &QueueRow) -> QueueItem {
    let conflicts = sibling_conflicts(conn, row.authority_id, &row.topic, &row.statement, row.id);
    // Newest live sibling, accepted preferred: conflict list is
    // id-ordered, so scan from the back for accepted first.
    let base = conflicts
        .iter()
        .rev()
        .find(|c| c.status == "accepted")
        .or_else(|| conflicts.iter().next_back());
    let citations = match (row.citation_start, row.citation_end) {
        (s, e) if s >= 0 && e > s => vec![QueueCitation {
            capture_id: row.capture_id,
            start_byte: s,
            end_byte: e,
            quote: sidecar_quote(&state.store_root, row.capture_id, s, e),
        }],
        _ => vec![],
    };
    let open_tasks: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM task
             WHERE authority_id = ?1 AND status IN ('open', 'batched', 'in_progress')",
            [row.authority_id],
            |r| r.get(0),
        )
        .unwrap_or(0);
    let (kind, trust, authenticity): (String, String, String) = conn
        .query_row(
            "SELECT kind, trust, authenticity FROM capture WHERE id = ?1",
            [row.capture_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap_or_default();
    QueueItem {
        id: row.id,
        authority_slug: row.authority_slug.clone(),
        topic: row.topic.clone(),
        statement: row.statement.clone(),
        status: row.status.clone(),
        created_by: row.created_by.clone(),
        created_at: row.created_at,
        diff: base.map(|b| QueueDiff {
            base_statement: b.statement.clone(),
            base_fact_id: b.id,
            base_status: b.status.clone(),
        }),
        citations,
        reasoning_untrusted: (!row.reasoning.is_empty()).then(|| row.reasoning.clone()),
        blast_radius: BlastRadius {
            conflict: !conflicts.is_empty(),
            conflicts,
            open_tasks,
        },
        provenance: Provenance {
            capture_id: row.capture_id,
            capture_kind: kind,
            trust,
            authenticity,
            sidecar: format!("text/{}.txt", row.capture_id),
        },
    }
}

/// Server-sliced sidecar quote: "" on missing file or bad bounds —
/// one bad sidecar never 503s the queue. Lossy decode (evidence
/// bytes are not guaranteed UTF-8).
fn sidecar_quote(store_root: &std::path::Path, capture_id: i64, start: i64, end: i64) -> String {
    let bytes = std::fs::read(store_root.join("text").join(format!("{capture_id}.txt")))
        .unwrap_or_default();
    if start < 0 || end > bytes.len() as i64 || start >= end {
        return String::new();
    }
    let (start, end) = (start as usize, end as usize);
    String::from_utf8_lossy(&bytes[start..end]).to_string()
}

fn open(state: &AppState) -> Option<Connection> {
    db::open(&Store::new(&state.store_root)).ok()
}

// ---------------------------------------------------------------------------
// Suppress rules
// ---------------------------------------------------------------------------

#[derive(Debug, serde::Serialize)]
pub struct RuleView {
    pub id: i64,
    pub authority_id: Option<i64>,
    pub authority_slug: Option<String>,
    pub topic: Option<String>,
    pub statement: String,
    pub reason: String,
    pub created_by: String,
    pub created_at: i64,
    pub revoked_at: Option<i64>,
}

#[derive(Debug, serde::Deserialize)]
pub struct RuleListParams {
    pub authority: Option<String>,
    pub include_revoked: Option<bool>,
}

pub async fn list_rules(
    _auth: AdminAuth,
    State(state): State<AppState>,
    Query(params): Query<RuleListParams>,
) -> impl IntoResponse {
    let Some(conn) = open(&state) else {
        return store_unavailable();
    };
    if let Some(slug) = params.authority.as_deref()
        && authority_id(&conn, slug).is_none()
    {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "unknown authority"})),
        )
            .into_response();
    }
    let mut sql = "SELECT r.id, r.authority_id, a.slug, r.topic, r.statement,
                          r.reason, r.created_by, r.created_at, r.revoked_at
                   FROM suppress_rule r LEFT JOIN authority a ON a.id = r.authority_id
                   WHERE 1 = 1"
        .to_string();
    if params.authority.is_some() {
        sql.push_str(" AND a.slug = ?1");
    }
    if !params.include_revoked.unwrap_or(false) {
        sql.push_str(" AND r.revoked_at IS NULL");
    }
    sql.push_str(" ORDER BY r.id");
    let mut stmt = match conn.prepare(&sql) {
        Ok(stmt) => stmt,
        Err(_) => return store_unavailable(),
    };
    let bindings: Vec<&str> = params.authority.as_deref().into_iter().collect();
    let rows = stmt
        .query_map(rusqlite::params_from_iter(bindings), |row| {
            Ok(RuleView {
                id: row.get(0)?,
                authority_id: row.get(1)?,
                authority_slug: row.get(2)?,
                topic: row.get(3)?,
                statement: row.get(4)?,
                reason: row.get(5)?,
                created_by: row.get(6)?,
                created_at: row.get(7)?,
                revoked_at: row.get(8)?,
            })
        })
        .and_then(|mapped| mapped.collect::<rusqlite::Result<Vec<_>>>());
    match rows {
        Ok(rules) => {
            let count = rules.len();
            (
                StatusCode::OK,
                Json(json!({"rules": rules, "count": count})),
            )
                .into_response()
        }
        Err(_) => store_unavailable(),
    }
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateRuleBody {
    pub statement: String,
    pub authority: Option<String>,
    pub topic: Option<String>,
    pub reason: Option<String>,
}

pub async fn create_rule(
    auth: AdminAuth,
    State(state): State<AppState>,
    Json(body): Json<CreateRuleBody>,
) -> impl IntoResponse {
    if body.statement.trim().is_empty() || body.statement.len() > 2000 {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"error": "statement must be 1-2000 chars"})),
        )
            .into_response();
    }
    if let Some(topic) = body.topic.as_deref()
        && contracts::topics::Topic::parse(topic).is_err()
    {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"error": "unknown topic"})),
        )
            .into_response();
    }
    if let Some(reason) = body.reason.as_deref()
        && reason.len() > 1024
    {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"error": "reason too long (max 1024 chars)"})),
        )
            .into_response();
    }
    let Some(conn) = open(&state) else {
        return store_unavailable();
    };
    let scope_authority = match body.authority.as_deref() {
        None => None,
        Some(slug) => match authority_id(&conn, slug) {
            Some(id) => Some(id),
            None => {
                return (
                    StatusCode::NOT_FOUND,
                    Json(json!({"error": "unknown authority"})),
                )
                    .into_response();
            }
        },
    };
    let norm = db::normalize_statement(&body.statement);
    let existing: Option<i64> = conn
        .query_row(
            "SELECT id FROM suppress_rule
             WHERE revoked_at IS NULL AND statement_norm = ?1
               AND authority_id IS NOT DISTINCT FROM ?2
               AND topic IS NOT DISTINCT FROM ?3
             ORDER BY id LIMIT 1",
            rusqlite::params![norm, scope_authority, body.topic],
            |row| row.get(0),
        )
        .optional()
        .unwrap_or(None);
    if let Some(rule_id) = existing {
        return (
            StatusCode::CONFLICT,
            Json(json!({"error": "duplicate rule", "rule_id": rule_id})),
        )
            .into_response();
    }
    let now = unix_now();
    let inserted: rusqlite::Result<i64> = (|| {
        conn.execute_batch("BEGIN IMMEDIATE")?;
        conn.execute(
            "INSERT INTO suppress_rule(authority_id, topic, statement_norm,
                                        statement, reason, created_by, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![
                scope_authority,
                body.topic,
                norm,
                body.statement,
                body.reason.as_deref().unwrap_or(""),
                auth.user.user_id,
                now,
            ],
        )?;
        let rule_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO audit(at, actor, action, entity, entity_id, reason)
             VALUES (?1, ?2, 'suppress_rule.create', 'suppress_rule', ?3, ?4)",
            rusqlite::params![
                now,
                auth.user.user_id,
                rule_id,
                body.reason.as_deref().unwrap_or("")
            ],
        )?;
        conn.execute_batch("COMMIT")?;
        rusqlite::Result::Ok(rule_id)
    })();
    let rule_id = match inserted {
        Ok(id) => id,
        Err(_) => return store_unavailable(),
    };
    let view = RuleView {
        id: rule_id,
        authority_id: scope_authority,
        authority_slug: body.authority.clone(),
        topic: body.topic.clone(),
        statement: body.statement.clone(),
        reason: body.reason.clone().unwrap_or_default(),
        created_by: auth.user.user_id.clone(),
        created_at: now,
        revoked_at: None,
    };
    (StatusCode::OK, Json(view)).into_response()
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RevokeBody {
    pub reason: String,
}

pub async fn revoke_rule(
    auth: AdminAuth,
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(body): Json<RevokeBody>,
) -> impl IntoResponse {
    let reason = body.reason.trim();
    if reason.is_empty() || reason.len() > 1024 {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"error": "reason required (1-1024 chars)"})),
        )
            .into_response();
    }
    let Some(conn) = open(&state) else {
        return store_unavailable();
    };
    let now = unix_now();
    enum Revoke {
        Missing,
        AlreadyRevoked,
        Revoked(RuleView),
    }
    let outcome: rusqlite::Result<Revoke> = (|| {
        conn.execute_batch("BEGIN IMMEDIATE")?;
        let row: Option<RuleView> = conn
            .query_row(
                "SELECT r.authority_id, a.slug, r.topic, r.statement, r.reason,
                        r.created_by, r.created_at, r.revoked_at
                 FROM suppress_rule r LEFT JOIN authority a ON a.id = r.authority_id
                 WHERE r.id = ?1",
                [id],
                |r| {
                    Ok(RuleView {
                        id,
                        authority_id: r.get(0)?,
                        authority_slug: r.get(1)?,
                        topic: r.get(2)?,
                        statement: r.get(3)?,
                        reason: r.get(4)?,
                        created_by: r.get(5)?,
                        created_at: r.get(6)?,
                        revoked_at: r.get(7)?,
                    })
                },
            )
            .optional()?;
        let Some(mut view) = row else {
            conn.execute_batch("ROLLBACK")?;
            return Ok(Revoke::Missing);
        };
        if view.revoked_at.is_some() {
            conn.execute_batch("ROLLBACK")?;
            return Ok(Revoke::AlreadyRevoked);
        }
        conn.execute(
            "UPDATE suppress_rule SET revoked_at = ?1 WHERE id = ?2",
            rusqlite::params![now, id],
        )?;
        conn.execute(
            "INSERT INTO audit(at, actor, action, entity, entity_id, reason)
             VALUES (?1, ?2, 'suppress_rule.revoke', 'suppress_rule', ?3, ?4)",
            rusqlite::params![now, auth.user.user_id, id, reason],
        )?;
        conn.execute_batch("COMMIT")?;
        view.revoked_at = Some(now);
        Ok(Revoke::Revoked(view))
    })();
    match outcome {
        Ok(Revoke::Revoked(view)) => (StatusCode::OK, Json(view)).into_response(),
        Ok(Revoke::AlreadyRevoked) => (
            StatusCode::CONFLICT,
            Json(json!({"error": "rule already revoked"})),
        )
            .into_response(),
        Ok(Revoke::Missing) => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "rule not found"})),
        )
            .into_response(),
        Err(_) => store_unavailable(),
    }
}

// ---------------------------------------------------------------------------
// Suppressions log
// ---------------------------------------------------------------------------

#[derive(Debug, serde::Deserialize)]
pub struct SuppressionParams {
    pub authority: Option<String>,
    pub limit: Option<i64>,
    pub after_id: Option<i64>,
}

pub async fn list_suppressions(
    _auth: AdminAuth,
    State(state): State<AppState>,
    Query(params): Query<SuppressionParams>,
) -> impl IntoResponse {
    let limit = params.limit.unwrap_or(50).clamp(1, 200);
    let Some(conn) = open(&state) else {
        return store_unavailable();
    };
    if let Some(slug) = params.authority.as_deref()
        && authority_id(&conn, slug).is_none()
    {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "unknown authority"})),
        )
            .into_response();
    }
    // Numbered placeholders must be dense from ?1: track the next
    // index as optional filters append (SQLite rejects gaps).
    let mut sql = "SELECT s.id, s.capture_id, a.slug, s.topic, s.statement,
                          s.path, s.rule_id, s.created_at
                   FROM suppression s JOIN authority a ON a.id = s.authority_id
                   WHERE 1 = 1"
        .to_string();
    enum Bind<'a> {
        Text(&'a str),
        Int(i64),
    }
    let mut binds: Vec<Bind<'_>> = Vec::new();
    let mut next = 1;
    if let Some(slug) = params.authority.as_deref() {
        sql.push_str(&format!(" AND a.slug = ?{next}"));
        binds.push(Bind::Text(slug));
        next += 1;
    }
    if let Some(after) = params.after_id {
        sql.push_str(&format!(" AND s.id > ?{next}"));
        binds.push(Bind::Int(after));
        next += 1;
    }
    sql.push_str(&format!(" ORDER BY s.id LIMIT ?{next}"));
    binds.push(Bind::Int(limit));
    let mut stmt = match conn.prepare(&sql) {
        Ok(stmt) => stmt,
        Err(_) => return store_unavailable(),
    };
    let values: Vec<rusqlite::types::Value> = binds
        .into_iter()
        .map(|b| match b {
            Bind::Text(s) => rusqlite::types::Value::Text(s.to_string()),
            Bind::Int(n) => rusqlite::types::Value::Integer(n),
        })
        .collect();
    let rows = stmt
        .query_map(rusqlite::params_from_iter(values.iter()), |row| {
            Ok(json!({
                "id": row.get::<_, i64>(0)?,
                "capture_id": row.get::<_, i64>(1)?,
                "authority_slug": row.get::<_, String>(2)?,
                "topic": row.get::<_, String>(3)?,
                "statement": row.get::<_, String>(4)?,
                "path": row.get::<_, String>(5)?,
                "rule_id": row.get::<_, Option<i64>>(6)?,
                "created_at": row.get::<_, i64>(7)?,
            }))
        })
        .and_then(|mapped| mapped.collect::<rusqlite::Result<Vec<_>>>());
    match rows {
        Ok(list) => {
            let count = list.len();
            (
                StatusCode::OK,
                Json(json!({"suppressions": list, "count": count})),
            )
                .into_response()
        }
        Err(_) => store_unavailable(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        Router,
        body::Body,
        http::{Request, StatusCode},
        routing::{get, post},
    };
    use handbook_store::db::{self, NewAuthority, NewProposedFact, add_authority};
    use std::path::PathBuf;
    use tower::ServiceExt as _;

    const CSRF: &str = "t";

    struct Fixture {
        _tmp: tempfile::TempDir,
        state: AppState,
    }

    fn fixture() -> Fixture {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::new(tmp.path());
        store.init().unwrap();
        let conn = db::open(&store).unwrap();
        db::migrate(&conn).unwrap();
        auth::migrate_auth_tables(&conn).unwrap();
        auth::create_session(
            &conn,
            "staff-1",
            &auth::NewSession {
                user_id: "staff-1",
                role: auth::Role::Staff,
                customer_ref: Some("cust-1"),
                now: crate::unix_now(),
                ip: "127.0.0.1",
                user_agent: "test",
            },
        )
        .unwrap();
        let auth_id = add_authority(
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
        // One capture + sidecar; quotes slice bytes [0..12).
        conn.execute(
            "INSERT INTO capture(authority_id, source_id, captured_at, kind, path,
                                  bytes_raw, bytes_stored, sha256, authenticity, trust)
             VALUES (?1, NULL, 0, 'warc', 'blob', 19, 19, 'hash', '{\"tls\": true}', 'official')",
            [auth_id],
        )
        .unwrap();
        let capture_id = conn.last_insert_rowid();
        std::fs::create_dir_all(tmp.path().join("text")).unwrap();
        std::fs::write(
            tmp.path().join("text").join(format!("{capture_id}.txt")),
            "PO Box 40234 is here",
        )
        .unwrap();
        // Proposed cited fact + accepted sibling (conflict) + open task.
        db::insert_proposed_fact(
            &conn,
            &NewProposedFact {
                authority_id: auth_id,
                capture_id,
                topic: "address",
                statement: "po_box: PO Box 40234",
                created_by: "analysis-agent/aa-prompt-v1",
                created_at: 100,
                citation: Some((0, 12)),
                reasoning: "sidecar differs from live fact",
            },
        )
        .unwrap();
        conn.execute(
            "INSERT INTO fact(authority_id, capture_id, topic, statement, status, created_at)
             VALUES (?1, ?2, 'address', 'po_box: PO Box 40220', 'accepted', 50)",
            rusqlite::params![auth_id, capture_id],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO task(authority_id, kind, title, status, dedup_key, created_at)
             VALUES (?1, 'validate_fact', 't', 'open', 'k', 0)",
            [auth_id],
        )
        .unwrap();
        drop(conn);
        let state = AppState {
            spa_index: PathBuf::from("test-spa/index.html"),
            static_dir: PathBuf::from("static"),
            store_root: tmp.path().to_path_buf(),
            bus: crate::ws_bus::Bus::new(),
            kek: None,
            admin_mtls_off: true,
            admin_spa_dir: tmp.path().join("admin-spa"),
            media_terms_read: false,
            label_cfg: crate::p2_labels::LabelCfg::default(),
            voice_cfg: voice::VoiceConfig::default(),
            voice_api_key: None,
            webauthn: std::sync::Arc::new(crate::passkey::test_webauthn()),
            auth_limits: std::sync::Arc::new(crate::passkey::RateBuckets::default()),
            google_cfg: None,
            apple_cfg: None,
            meta_cfg: None,
            mailer: None,
            magic_link_base: "https://example.test/apostille/login".to_string(),
            meta_delete_status_base: "https://example.test/auth/meta/delete-status".to_string(),
        };
        Fixture { _tmp: tmp, state }
    }

    fn router(state: AppState) -> Router {
        Router::new()
            .route("/apostille/api/v1/admin/queue", get(queue_items))
            .route("/apostille/api/v1/admin/queue/{id}", get(queue_item))
            .route(
                "/apostille/api/v1/admin/facts/{id}/accept",
                post(crate::approval::accept_fact),
            )
            .route(
                "/apostille/api/v1/admin/facts/{id}/reject",
                post(crate::approval::reject_fact),
            )
            .route(
                "/apostille/api/v1/admin/suppress-rules",
                get(list_rules).post(create_rule),
            )
            .route(
                "/apostille/api/v1/admin/suppress-rules/{id}/revoke",
                post(revoke_rule),
            )
            .route(
                "/apostille/api/v1/admin/suppressions",
                get(list_suppressions),
            )
            .layer(axum::middleware::from_fn_with_state(
                state.clone(),
                crate::admin::csrf_guard,
            ))
            .layer(axum::middleware::from_fn_with_state(
                state.clone(),
                crate::admin::admin_host_guard,
            ))
            .with_state(state)
    }

    fn request(method: &str, uri: &str, body: Option<serde_json::Value>) -> Request<Body> {
        let mut builder = Request::builder()
            .uri(uri)
            .method(method)
            .header("host", "admin.andrii.ai")
            .header(
                "cookie",
                format!("{}=staff-1; {}={CSRF}", auth::ADM_COOKIE, auth::CSRF_COOKIE),
            )
            .header(auth::CSRF_HEADER, CSRF)
            .header("origin", "https://admin.andrii.ai");
        if method == "POST" {
            builder = builder.header("content-type", "application/json");
        }
        builder
            .body(Body::from(body.map(|v| v.to_string()).unwrap_or_default()))
            .unwrap()
    }

    async fn json(res: axum::response::Response) -> (StatusCode, serde_json::Value) {
        let status = res.status();
        let bytes = axum::body::to_bytes(res.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        (status, value)
    }

    #[tokio::test]
    async fn queue_lists_enriched_item() {
        let fx = fixture();
        let res = router(fx.state.clone())
            .oneshot(request("GET", "/apostille/api/v1/admin/queue", None))
            .await
            .unwrap();
        let (status, v) = json(res).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(v["count"], 1);
        let item = &v["items"][0];
        assert_eq!(item["statement"], "po_box: PO Box 40234");
        assert_eq!(item["diff"]["base_statement"], "po_box: PO Box 40220");
        assert_eq!(item["diff"]["base_status"], "accepted");
        assert_eq!(item["citations"][0]["quote"], "PO Box 40234");
        assert_eq!(item["citations"][0]["start_byte"], 0);
        assert_eq!(
            item["reasoning_untrusted"],
            "sidecar differs from live fact"
        );
        assert_eq!(item["blast_radius"]["conflict"], true);
        assert_eq!(item["blast_radius"]["open_tasks"], 1);
        assert_eq!(item["provenance"]["trust"], "official");
        assert!(
            item["provenance"]["sidecar"]
                .as_str()
                .unwrap()
                .ends_with(".txt")
        );
    }

    #[tokio::test]
    async fn queue_filters_and_errors() {
        let fx = fixture();
        let app = router(fx.state.clone());
        let res = app
            .oneshot(request(
                "GET",
                "/apostille/api/v1/admin/queue?authority=nope",
                None,
            ))
            .await
            .unwrap();
        let (status, v) = json(res).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(v["error"], "unknown authority");

        let res = router(fx.state.clone())
            .oneshot(request(
                "GET",
                "/apostille/api/v1/admin/queue?topic=bogus",
                None,
            ))
            .await
            .unwrap();
        let (status, v) = json(res).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(v["error"], "unknown topic");

        // Cursor past the only item → empty page.
        let res = router(fx.state.clone())
            .oneshot(request(
                "GET",
                "/apostille/api/v1/admin/queue?after_id=9999",
                None,
            ))
            .await
            .unwrap();
        let (status, v) = json(res).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(v["count"], 0);
    }

    #[tokio::test]
    async fn queue_item_serves_decided_rows_and_404s() {
        let fx = fixture();
        // Accept the only proposal, then fetch it by id (decided rows visible).
        let res = router(fx.state.clone())
            .oneshot(request(
                "POST",
                "/apostille/api/v1/admin/facts/1/accept",
                Some(json!({"reason": "ok"})),
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let res = router(fx.state.clone())
            .oneshot(request("GET", "/apostille/api/v1/admin/queue/1", None))
            .await
            .unwrap();
        let (status, v) = json(res).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(v["status"], "accepted");
        let res = router(fx.state.clone())
            .oneshot(request("GET", "/apostille/api/v1/admin/queue/9999", None))
            .await
            .unwrap();
        let (status, v) = json(res).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(v["error"], "fact not found");
    }

    #[tokio::test]
    async fn accept_with_edit_updates_text_and_audits_envelope() {
        let fx = fixture();
        let res = router(fx.state.clone())
            .oneshot(request(
                "POST",
                "/apostille/api/v1/admin/facts/1/accept",
                Some(json!({"reason": "fix", "statement": "po_box: PO Box 40234, Olympia"})),
            ))
            .await
            .unwrap();
        let (status, v) = json(res).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(v["statement"], "po_box: PO Box 40234, Olympia");
        let conn = db::open(&Store::new(&fx.state.store_root)).unwrap();
        let (statement, reason): (String, String) = conn
            .query_row(
                "SELECT f.statement, a.reason FROM fact f
                 JOIN audit a ON a.entity = 'fact' AND a.entity_id = f.id
                 WHERE f.id = 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(statement, "po_box: PO Box 40234, Olympia");
        let envelope: serde_json::Value = serde_json::from_str(&reason).unwrap();
        assert_eq!(envelope["note"], "fix");
        assert_eq!(envelope["edit_before"], "po_box: PO Box 40234");
    }

    #[tokio::test]
    async fn double_decide_is_409() {
        let fx = fixture();
        let uri = "/apostille/api/v1/admin/facts/1/accept";
        let res = router(fx.state.clone())
            .oneshot(request("POST", uri, Some(json!({"reason": "a"}))))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let res = router(fx.state.clone())
            .oneshot(request("POST", uri, Some(json!({"reason": "b"}))))
            .await
            .unwrap();
        let (status, v) = json(res).await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(v["error"], "fact is not proposed");
    }

    #[tokio::test]
    async fn reject_with_suppress_creates_rule_idempotently() {
        let fx = fixture();
        let body = json!({"reason": "wrong office",
                          "suppress": {"scope": "authority", "lock_topic": true}});
        let res = router(fx.state.clone())
            .oneshot(request(
                "POST",
                "/apostille/api/v1/admin/facts/1/reject",
                Some(body.clone()),
            ))
            .await
            .unwrap();
        let (status, v) = json(res).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(v["rule"]["deduped"], false);
        let rule_id = v["rule"]["id"].as_i64().unwrap();
        // A second proposal with the same statement, rejected with the
        // same rule shape: decision wins (200), rule deduped.
        let conn = db::open(&Store::new(&fx.state.store_root)).unwrap();
        conn.execute(
            "INSERT INTO fact(authority_id, capture_id, topic, statement, status, created_at)
             VALUES (1, 1, 'address', 'po_box: PO Box 40234', 'proposed', 0)",
            [],
        )
        .unwrap();
        drop(conn);
        let res = router(fx.state.clone())
            .oneshot(request(
                "POST",
                "/apostille/api/v1/admin/facts/3/reject",
                Some(body),
            ))
            .await
            .unwrap();
        let (status, v) = json(res).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(v["rule"]["id"], rule_id);
        assert_eq!(v["rule"]["deduped"], true);
        // Rule row: authority-scoped, topic-locked, normalized.
        let conn = db::open(&Store::new(&fx.state.store_root)).unwrap();
        let (authority_id, topic, norm): (Option<i64>, Option<String>, String) = conn
            .query_row(
                "SELECT authority_id, topic, statement_norm FROM suppress_rule WHERE id = ?1",
                [rule_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(authority_id, Some(1));
        assert_eq!(topic.as_deref(), Some("address"));
        assert_eq!(norm, "po_box: po box 40234");
    }

    #[tokio::test]
    async fn decide_cross_field_bodies_are_422() {
        let fx = fixture();
        for (uri, body, error) in [
            (
                "/apostille/api/v1/admin/facts/1/accept",
                json!({"reason": "x", "suppress": {"scope": "global"}}),
                "suppress is reject-only",
            ),
            (
                "/apostille/api/v1/admin/facts/1/reject",
                json!({"reason": "x", "statement": "y"}),
                "statement is accept-only",
            ),
            (
                "/apostille/api/v1/admin/facts/1/reject",
                json!({"reason": "x", "suppress": {"scope": "bogus"}}),
                "suppress.scope must be authority|global",
            ),
        ] {
            let res = router(fx.state.clone())
                .oneshot(request("POST", uri, Some(body)))
                .await
                .unwrap();
            let (status, v) = json(res).await;
            assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{uri}");
            assert_eq!(v["error"], error, "{uri}");
        }
    }

    #[tokio::test]
    async fn rules_crud_with_409s() {
        let fx = fixture();
        let uri = "/apostille/api/v1/admin/suppress-rules";
        let body = json!({"statement": "Fee $9", "authority": "t-sos", "topic": "fees", "reason": "stale"});
        let res = router(fx.state.clone())
            .oneshot(request("POST", uri, Some(body.clone())))
            .await
            .unwrap();
        let (status, v) = json(res).await;
        assert_eq!(status, StatusCode::OK);
        let rule_id = v["id"].as_i64().unwrap();
        assert_eq!(v["authority_slug"], "t-sos");
        // Duplicate → 409 with the live id.
        let res = router(fx.state.clone())
            .oneshot(request("POST", uri, Some(body)))
            .await
            .unwrap();
        let (status, v) = json(res).await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(v["error"], "duplicate rule");
        assert_eq!(v["rule_id"], rule_id);
        // List shows it; revoked filter hides after revoke.
        let res = router(fx.state.clone())
            .oneshot(request("GET", uri, None))
            .await
            .unwrap();
        let (status, v) = json(res).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(v["count"], 1);
        let res = router(fx.state.clone())
            .oneshot(request(
                "POST",
                &format!("{uri}/{rule_id}/revoke"),
                Some(json!({"reason": "oops"})),
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let res = router(fx.state.clone())
            .oneshot(request(
                "POST",
                &format!("{uri}/{rule_id}/revoke"),
                Some(json!({"reason": "again"})),
            ))
            .await
            .unwrap();
        let (status, v) = json(res).await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(v["error"], "rule already revoked");
        let res = router(fx.state.clone())
            .oneshot(request("GET", uri, None))
            .await
            .unwrap();
        let (_, v) = json(res).await;
        assert_eq!(v["count"], 0);
        let res = router(fx.state.clone())
            .oneshot(request("GET", &format!("{uri}?include_revoked=true"), None))
            .await
            .unwrap();
        let (_, v) = json(res).await;
        assert_eq!(v["count"], 1);
    }

    #[tokio::test]
    async fn suppressions_log_lists_skips() {
        let fx = fixture();
        let conn = db::open(&Store::new(&fx.state.store_root)).unwrap();
        conn.execute(
            "INSERT INTO suppression(capture_id, authority_id, topic, statement,
                                     statement_norm, rule_id, path, created_at)
             VALUES (1, 1, 'address', 's', 's', NULL, 'A', 0)",
            [],
        )
        .unwrap();
        drop(conn);
        let res = router(fx.state.clone())
            .oneshot(request("GET", "/apostille/api/v1/admin/suppressions", None))
            .await
            .unwrap();
        let (status, v) = json(res).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(v["count"], 1);
        assert_eq!(v["suppressions"][0]["path"], "A");
        assert_eq!(v["suppressions"][0]["authority_slug"], "t-sos");
    }
}
