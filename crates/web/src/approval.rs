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

/// One fact with its conflict banner.
#[derive(Debug, Clone, serde::Serialize)]
pub struct FactView {
    pub id: i64,
    pub authority_slug: String,
    pub topic: String,
    pub statement: String,
    pub status: String,
    pub created_by: String,
    pub created_at: i64,
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
}

fn fact_view(conn: &Connection, fact: &StoredFact, public_only: bool) -> FactView {
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
        created_at: fact.created_at,
        conflict,
        conflicts,
    }
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
                          f.status, f.created_by, f.created_at
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
        .map(|row| fact_view(&conn, row, false))
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
pub struct DecisionBody {
    pub reason: Option<String>,
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
    decide(&state, &auth, id, body.reason.as_deref(), "accepted").await
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
    decide(&state, &auth, id, Some(reason), "rejected").await
}

async fn decide(
    state: &AppState,
    auth: &AdminAuth,
    id: i64,
    reason: Option<&str>,
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
            now,
            action,
        },
    )
}

/// One accept/reject decision: target status plus audit attribution.
struct Decision<'a> {
    to: &'a str,
    reason: Option<&'a str>,
    now: i64,
    action: &'a str,
}

/// Transaction outcome: DB failure, HTTP outcome, or the decided fact.
type DecideOutcome = rusqlite::Result<Result<StoredFact, StatusCode>>;

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
                        f.created_by, f.created_at
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
        conn.execute(
            "UPDATE fact SET status = ?1 WHERE id = ?2",
            (decision.to, id),
        )?;
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
                decision.reason.unwrap_or("")
            ],
        )?;
        conn.execute_batch("COMMIT")?;
        Ok(Ok(fact))
    })();
    let fact = match outcome {
        Ok(Ok(found)) => found,
        Ok(Err(status)) => {
            let body = if status == StatusCode::NOT_FOUND {
                json!({"error": "fact not found"})
            } else {
                json!({"error": "fact is not proposed"})
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
    }
    let view = fact_view(conn, &fact, false);
    (StatusCode::OK, Json(view)).into_response()
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
    let authority_id: Option<i64> = conn
        .query_row("SELECT id FROM authority WHERE slug = ?1", [&slug], |row| {
            row.get(0)
        })
        .optional()
        .unwrap_or(None);
    let Some(authority_id) = authority_id else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "unknown authority"})),
        )
            .into_response();
    };
    let rows: Vec<StoredFact> = conn
        .prepare(
            "SELECT id, topic, statement, status, created_by, created_at
             FROM fact WHERE authority_id = ?1 AND status = 'accepted'
             ORDER BY id LIMIT 200",
        )
        .and_then(|mut stmt| {
            stmt.query_map([authority_id], |row| {
                Ok(StoredFact {
                    id: row.get(0)?,
                    authority_id,
                    slug: slug.clone(),
                    topic: row.get(1)?,
                    statement: row.get(2)?,
                    status: row.get(3)?,
                    created_by: row.get(4)?,
                    created_at: row.get(5)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()
        })
        .unwrap_or_default();
    let facts: Vec<FactView> = rows.iter().map(|row| fact_view(&conn, row, true)).collect();
    (
        StatusCode::OK,
        Json(json!({"authority": slug, "facts": facts})),
    )
        .into_response()
}
