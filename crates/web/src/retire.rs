//! Retire an accepted fact (plan §5.5): the first supersede path, so
//! staff can undo a wrong approval. The fact turns `superseded` (the
//! `fact.status` CHECK has always allowed it). Every public read serves
//! `accepted` rows only, so it leaves `/facts`, the index count and
//! `verified_at`, the crawler head, translations, order destinations
//! and the MCP lookup at once; `/plan` drops it with every block, path
//! and accepted office that quotes it ([`crate::plan::offices_for_read`]),
//! and with every evidence note anchored to it or naming a number or
//! email only it published ([`crate::evidence::public_notes`]).
//! The decision is audited like accept/reject (`fact_retire`) and
//! announced as `fact_retired {fact_id}` on the authority channel (ids
//! only). A follower email already queued for it drops it at send time
//! (`notify::load_fact` loads accepted facts only). No client acts on
//! the event yet, so a dossier already open keeps showing the fact
//! until it reloads.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Json, Response},
};
use handbook_store::{db, store::Store};
use rusqlite::{Connection, OptionalExtension as _};
use serde_json::json;

use crate::{AppState, admin::AdminAuth, store_unavailable, unix_now};

/// A retire reason is staff-written: 8 to 500 characters.
const REASON_CHARS: std::ops::RangeInclusive<usize> = 8..=500;

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RetireBody {
    pub reason: String,
}

/// `POST /apostille/api/v1/admin/facts/{id}/retire` `{"reason"}` —
/// staff session + CSRF (admin listener), accepted facts only:
/// `200 {id, authority_slug, status}`, `404` unknown id, `409` not
/// accepted, `422` reason outside 8-500 chars.
pub async fn retire_fact(
    auth: AdminAuth,
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(body): Json<RetireBody>,
) -> Response {
    let reason = body.reason.trim();
    if !REASON_CHARS.contains(&reason.chars().count()) {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"error": "reason must be 8-500 chars"})),
        )
            .into_response();
    }
    let Ok(conn) = db::open(&Store::new(&state.store_root)) else {
        return store_unavailable();
    };
    let now = unix_now();
    let slug = match retire(&conn, &auth.user.user_id, id, reason, now) {
        Ok(Ok(slug)) => slug,
        Ok(Err(status)) => {
            let error = if status == StatusCode::NOT_FOUND {
                "fact not found"
            } else {
                "fact is not accepted"
            };
            return (status, Json(json!({"error": error}))).into_response();
        }
        Err(_) => {
            let _ = conn.execute_batch("ROLLBACK");
            return store_unavailable();
        }
    };
    crate::ws_bus::publish_authority_event(
        &state.bus,
        &slug,
        "fact_retired",
        json!({"fact_id": id}),
        now,
    );
    // Dashboard activity feed (staff-only `ops`).
    crate::ws_bus::publish_ops(
        &state.bus,
        "fact_retired",
        json!({"fact_id": id, "slug": slug}),
        now,
    );
    (
        StatusCode::OK,
        Json(json!({"id": id, "authority_slug": slug, "status": "superseded"})),
    )
        .into_response()
}

/// One retire inside `BEGIN IMMEDIATE` (serializes it against a
/// concurrent accept/reject/retire): `accepted` → `superseded` plus
/// one audit row in the accept/reject shape. Returns the fact's
/// authority slug, or the refusal status.
fn retire(
    conn: &Connection,
    actor: &str,
    id: i64,
    reason: &str,
    now: i64,
) -> rusqlite::Result<Result<String, StatusCode>> {
    conn.execute_batch("BEGIN IMMEDIATE")?;
    let row: Option<(String, String)> = conn
        .query_row(
            "SELECT f.status, a.slug FROM fact f JOIN authority a ON a.id = f.authority_id
             WHERE f.id = ?1",
            [id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let Some((status, slug)) = row else {
        conn.execute_batch("ROLLBACK")?;
        return Ok(Err(StatusCode::NOT_FOUND));
    };
    if status != "accepted" {
        conn.execute_batch("ROLLBACK")?;
        return Ok(Err(StatusCode::CONFLICT));
    }
    conn.execute("UPDATE fact SET status = 'superseded' WHERE id = ?1", [id])?;
    conn.execute(
        "INSERT INTO audit(at, actor, action, entity, entity_id,
                            before_ref, after_ref, reason)
         VALUES (?1, ?2, 'fact_retire', 'fact', ?3, 'accepted', 'superseded', ?4)",
        rusqlite::params![now, actor, id, reason],
    )?;
    conn.execute_batch("COMMIT")?;
    Ok(Ok(slug))
}
