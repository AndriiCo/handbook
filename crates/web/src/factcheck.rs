//! `FactChecksV1`: Wonder cross-checks each proposed fact against the
//! whole capture sidecar (supported / contradicted / not_found + quote;
//! the quote must be a sidecar substring or the verdict is not_found).
//!
//! The drafting pass reads a capture in slices; this pass gives
//! muse-wonder-1.0 the entire sidecar at once (1M context), so a fact
//! that a later paragraph contradicts is caught before staff accept it.

use std::collections::HashMap;
use std::sync::Arc;

use axum::{
    Router,
    extract::{Query, State},
    http::StatusCode,
    response::{IntoResponse, Json, Response},
    routing::{get, post},
};
use handbook_agent::model::ChatJsonRequest;
use handbook_store::{db, store::Store};
use rusqlite::{Connection, OptionalExtension};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::admin::AdminAuth;
use crate::{AppState, store_unavailable, unix_now};

pub const CHECK_MODEL: &str = "muse-wonder-1.0";
const PROMPT: &str = include_str!("../prompts/factcheck-v1.md");
const PER_RUN_MARKER: &str = "<!-- PER-RUN -->";
/// Sidecar budget (characters) sent to Wonder in one call.
const MAX_SIDECAR: usize = 400_000;

fn squash(s: &str) -> String {
    s.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// Guard one verdict: a `supported` / `contradicted` verdict stands only
/// when its quote (8+ chars) is a whitespace-normalised substring of the
/// sidecar; otherwise it becomes `not_found` with an empty quote.
pub fn guard_verdict(
    verdict: &str,
    quote: &str,
    sidecar_squashed: &str,
    statement: &str,
) -> (&'static str, String) {
    let q = squash(quote);
    let quoted = q.chars().count() >= 8 && sidecar_squashed.contains(&q);
    // A supporting quote must carry the statement's numbers, amounts
    // and PO boxes (review #49), not just any 8 characters of the page.
    let carries = crate::translate::guard_ok(statement, quote);
    match (verdict, quoted) {
        ("supported", true) if carries => ("supported", quote.trim().chars().take(400).collect()),
        ("contradicted", true) => ("contradicted", quote.trim().chars().take(400).collect()),
        _ => ("not_found", String::new()),
    }
}

// ------------------------------------------------------------------ DB

/// Boot DDL on the metadata DB (`handbook.sqlite`): `CREATE TABLE IF
/// NOT EXISTS` only — no ALTER, DROP or RENAME (docs/ops/deploy.md § Rails).
pub fn migrate_tables(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS fact_check (
            id INTEGER PRIMARY KEY,
            fact_id INTEGER NOT NULL,
            authority_slug TEXT NOT NULL,
            capture_id INTEGER NOT NULL,
            verdict TEXT NOT NULL,
            quote TEXT NOT NULL,
            reason TEXT NOT NULL,
            model TEXT NOT NULL,
            run_id INTEGER,
            at INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS fact_check_fact ON fact_check(fact_id, at);
        CREATE INDEX IF NOT EXISTS fact_check_slug ON fact_check(authority_slug, at);",
    )
}

fn has_table(conn: &Connection) -> bool {
    conn.query_row(
        "SELECT 1 FROM sqlite_master WHERE type='table' AND name='fact_check'",
        [],
        |_| Ok(true),
    )
    .optional()
    .ok()
    .flatten()
    .unwrap_or(false)
}

/// Latest verdict per fact for one authority (all statuses).
pub fn latest_for_slug(conn: &Connection, slug: &str) -> rusqlite::Result<Vec<Value>> {
    if !has_table(conn) {
        return Ok(Vec::new());
    }
    let mut stmt = conn.prepare(
        "SELECT c.fact_id, c.verdict, c.quote, c.reason, c.model, c.at FROM fact_check c
         WHERE c.authority_slug = ?1 AND c.id = (SELECT MAX(id) FROM fact_check x WHERE x.fact_id = c.fact_id)
         ORDER BY c.fact_id",
    )?;
    stmt.query_map([slug], |r| {
        Ok(json!({"fact_id": r.get::<_, i64>(0)?, "verdict": r.get::<_, String>(1)?, "quote": r.get::<_, String>(2)?,
                  "reason": r.get::<_, String>(3)?, "model": r.get::<_, String>(4)?, "at": r.get::<_, i64>(5)?}))
    })?
    .collect()
}

/// Cross-check verdicts for one authority's accepted facts, checked
/// before or at accept time (`plan.rs` provenance reads this).
pub fn checks_for_slug(conn: &Connection, slug: &str) -> rusqlite::Result<Vec<Value>> {
    let accepted: Vec<i64> = conn
        .prepare("SELECT f.id FROM fact f JOIN authority a ON a.id = f.authority_id WHERE a.slug = ?1 AND f.status = 'accepted'")?
        .query_map([slug], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    Ok(latest_for_slug(conn, slug)?
        .into_iter()
        .filter(|c| {
            c["fact_id"]
                .as_i64()
                .is_some_and(|id| accepted.contains(&id))
        })
        .collect())
}

// ----------------------------------------------------------- the run

/// Check every live fact drafted from one capture in a single Wonder
/// call over the whole sidecar; stores one verdict per fact.
pub fn run_blocking(
    store_root: &std::path::Path,
    kek: Option<&config::Kek>,
    capture_id: i64,
    run_id: u64,
) -> Result<Value, String> {
    let store = Store::new(store_root);
    let conn = db::open(&store).map_err(|e| format!("store unavailable: {e}"))?;
    migrate_tables(&conn).map_err(|e| format!("store: {e}"))?;
    let slug: String = conn
        .query_row(
            "SELECT a.slug FROM capture c JOIN authority a ON a.id = c.authority_id WHERE c.id = ?1",
            [capture_id],
            |r| r.get(0),
        )
        .map_err(|_| "unknown capture".to_string())?;
    let sidecar =
        std::fs::read_to_string(store.root().join("text").join(format!("{capture_id}.txt")))
            .map_err(|_| "capture has no extracted text".to_string())?;
    let facts: Vec<(i64, String, String)> = conn
        .prepare(
            "SELECT id, topic, statement FROM fact WHERE capture_id = ?1
               AND status IN ('proposed', 'accepted') AND topic != 'phone_script' ORDER BY id LIMIT 60",
        )
        .and_then(|mut s| s.query_map([capture_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect())
        .map_err(|e| format!("store: {e}"))?;
    if facts.is_empty() {
        return Err("no live facts drafted from this capture".to_string());
    }
    let text: String = sidecar.chars().take(MAX_SIDECAR).collect();
    let ids: Vec<i64> = facts.iter().map(|f| f.0).collect();
    let (developer, per_run) = PROMPT
        .split_once(PER_RUN_MARKER)
        .expect("factcheck prompt carries a PER-RUN part");
    let fact_lines: Vec<String> = facts
        .iter()
        .map(|(id, topic, st)| format!("- {id} · {topic} · {st}"))
        .collect();
    let user = per_run
        .replace("{{facts}}", &fact_lines.join("\n"))
        .replace("{{text}}", &text)
        .trim()
        .to_string();
    let client = crate::draft::live_client(&conn, kek)?.with_observer(
        crate::telemetry::DbObserver::new(store_root.to_path_buf()),
        "factcheck",
        Some(run_id),
    );
    let resp = client
        .chat_json(&ChatJsonRequest {
            model: CHECK_MODEL,
            developer,
            user,
            schema_name: "FactChecksV1",
            schema: json!({"type": "object", "additionalProperties": false, "required": ["checks"],
                "properties": {"checks": {"type": "array", "items": {"type": "object", "additionalProperties": false,
                    "required": ["fact_id", "verdict", "quote", "reason"],
                    "properties": {"fact_id": {"type": "integer", "enum": ids},
                                   "verdict": {"type": "string", "enum": ["supported", "contradicted", "not_found"]},
                                   "quote": {"type": "string"}, "reason": {"type": "string"}}}}}}),
            reasoning_effort: "medium",
            prompt_cache_key: "fact-crosscheck-v1".to_string(),
            max_completion_tokens: 16_384,
            pipeline: "factcheck",
            run_id: Some(run_id),
        })
        .map_err(|e| format!("cross-check failed: {e}"))?;
    let squashed = squash(&text);
    let now = unix_now();
    let mut counts: HashMap<&'static str, usize> = HashMap::new();
    let mut seen = Vec::new();
    for c in resp.value["checks"].as_array().into_iter().flatten() {
        let Some(fact_id) = c["fact_id"]
            .as_i64()
            .filter(|id| ids.contains(id) && !seen.contains(id))
        else {
            continue;
        };
        seen.push(fact_id);
        let statement = facts
            .iter()
            .find(|f| f.0 == fact_id)
            .map(|f| f.2.as_str())
            .unwrap_or("");
        let (verdict, quote) = guard_verdict(
            c["verdict"].as_str().unwrap_or(""),
            c["quote"].as_str().unwrap_or(""),
            &squashed,
            statement,
        );
        *counts.entry(verdict).or_default() += 1;
        let reason: String = c["reason"]
            .as_str()
            .unwrap_or("")
            .chars()
            .take(200)
            .collect();
        conn.execute(
            "INSERT INTO fact_check(fact_id, authority_slug, capture_id, verdict, quote, reason, model, run_id, at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            rusqlite::params![fact_id, slug, capture_id, verdict, quote, reason, CHECK_MODEL, run_id as i64, now],
        )
        .map_err(|e| format!("store: {e}"))?;
    }
    // A fact the model skipped is recorded as not_found, never as silence.
    for id in ids.iter().filter(|id| !seen.contains(id)) {
        *counts.entry("not_found").or_default() += 1;
        conn.execute(
            "INSERT INTO fact_check(fact_id, authority_slug, capture_id, verdict, quote, reason, model, run_id, at)
             VALUES (?1, ?2, ?3, 'not_found', '', 'no verdict returned', ?4, ?5, ?6)",
            rusqlite::params![id, slug, capture_id, CHECK_MODEL, run_id as i64, now],
        )
        .map_err(|e| format!("store: {e}"))?;
    }
    Ok(
        json!({"slug": slug, "capture_id": capture_id, "checked": ids.len(),
              "supported": counts.get("supported").copied().unwrap_or(0),
              "contradicted": counts.get("contradicted").copied().unwrap_or(0),
              "not_found": counts.get("not_found").copied().unwrap_or(0),
              "sidecar_chars": text.chars().count(), "model": CHECK_MODEL, "latency_ms": resp.latency_ms,
              "input_tokens": resp.usage.input_tokens, "output_tokens": resp.usage.output_tokens,
              "cached_tokens": resp.usage.cached_tokens, "reasoning_tokens": resp.usage.reasoning_tokens}),
    )
}

// ------------------------------------------------------------- staff

#[derive(Debug, Deserialize)]
pub struct RunBody {
    pub capture_id: i64,
}

async fn run(
    _auth: AdminAuth,
    State(state): State<AppState>,
    Json(body): Json<RunBody>,
) -> Response {
    let Ok(conn) = db::open(&Store::new(&state.store_root)) else {
        return store_unavailable();
    };
    if crate::telemetry::over_daily_cap(&conn, &state, unix_now()) {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            Json(json!({"error": "daily model-call cap reached (meta.daily_call_cap)"})),
        )
            .into_response();
    }
    drop(conn);
    let run_id = state.draft_runs.reserve_id();
    let (bus, root, kek): (_, _, Option<Arc<config::Kek>>) = (
        state.bus.clone(),
        state.store_root.clone(),
        state.kek.clone(),
    );
    let capture_id = body.capture_id;
    tokio::spawn(async move {
        let topic = format!("run:{run_id}");
        bus.publish(&topic, json!({"type": "crosscheck.started", "topic": topic, "run_id": run_id, "capture_id": capture_id, "model": CHECK_MODEL}).to_string());
        let out = tokio::task::spawn_blocking(move || {
            run_blocking(&root, kek.as_deref(), capture_id, run_id)
        })
        .await;
        let (kind, body) = match out {
            Ok(Ok(summary)) => ("crosscheck.done", summary),
            Ok(Err(e)) => ("crosscheck.failed", json!({"error": e})),
            Err(e) => (
                "crosscheck.failed",
                json!({"error": format!("cross-check task failed: {e}")}),
            ),
        };
        let mut event =
            json!({"type": kind, "topic": topic, "run_id": run_id, "capture_id": capture_id});
        if let (Some(m), Some(extra)) = (event.as_object_mut(), body.as_object()) {
            m.extend(extra.clone());
        }
        bus.publish(&topic, event.to_string());
    });
    (
        StatusCode::ACCEPTED,
        Json(json!({"run_id": run_id, "topic": format!("run:{run_id}")})),
    )
        .into_response()
}

#[derive(Debug, Deserialize)]
pub struct ListQuery {
    pub slug: String,
}

async fn list(
    _auth: AdminAuth,
    State(state): State<AppState>,
    Query(q): Query<ListQuery>,
) -> Response {
    let Ok(conn) = db::open(&Store::new(&state.store_root)) else {
        return store_unavailable();
    };
    match latest_for_slug(&conn, &q.slug) {
        Ok(checks) => Json(json!({"checks": checks})).into_response(),
        Err(_) => store_unavailable(),
    }
}

/// Staff routes merged onto the ADMIN listener only (the builder wraps
/// them in the admin guard stack; handlers take `AdminAuth`).
pub fn admin_router(state: AppState) -> Router {
    Router::<AppState>::new()
        .route("/apostille/api/v1/admin/fact-checks", get(list))
        .route("/apostille/api/v1/admin/fact-checks/run", post(run))
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verdicts_need_a_sidecar_quote() {
        let side_b = squash("Apostilles cost $15 per document. Offices open at 8:00.");
        assert_eq!(
            guard_verdict(
                "supported",
                "Offices open at 8:00.",
                &side_b,
                "Apostilles cost $15 per document."
            )
            .0,
            "not_found",
            "an unrelated quote never supports a fact"
        );
        let side = squash("Apostilles cost $15 per document.\n\nWalk-in   service is in Olympia.");
        assert_eq!(
            guard_verdict(
                "supported",
                "cost $15 per document",
                &side,
                "Apostilles cost $15 per document."
            )
            .0,
            "supported"
        );
        assert_eq!(
            guard_verdict(
                "supported",
                "Walk-in service is in Olympia",
                &side,
                "Walk-in service is in Olympia."
            )
            .0,
            "supported",
            "whitespace-normalised"
        );
        assert_eq!(
            guard_verdict(
                "contradicted",
                "cost $20 per document",
                &side,
                "Apostilles cost $15 per document."
            ),
            ("not_found", String::new()),
            "an unquoted contradiction is not believed"
        );
        assert_eq!(
            guard_verdict(
                "supported",
                "$15",
                &side,
                "Apostilles cost $15 per document."
            )
            .0,
            "not_found",
            "too short to ground"
        );
        assert_eq!(
            guard_verdict(
                "bogus",
                "cost $15 per document",
                &side,
                "Apostilles cost $15 per document."
            )
            .0,
            "not_found"
        );
    }

    #[test]
    fn latest_verdict_per_fact_and_accepted_filter() {
        let conn = Connection::open_in_memory().unwrap();
        migrate_tables(&conn).unwrap();
        conn.execute_batch(
            "CREATE TABLE authority(id INTEGER PRIMARY KEY, slug TEXT);
             CREATE TABLE fact(id INTEGER PRIMARY KEY, authority_id INTEGER, status TEXT);
             INSERT INTO authority VALUES (1, 'usa-wa-sos');
             INSERT INTO fact VALUES (201, 1, 'accepted'), (202, 1, 'proposed');",
        )
        .unwrap();
        for (fact, verdict, at) in [
            (201, "not_found", 1),
            (201, "supported", 2),
            (202, "contradicted", 3),
        ] {
            conn.execute(
                "INSERT INTO fact_check(fact_id, authority_slug, capture_id, verdict, quote, reason, model, at)
                 VALUES (?1, 'usa-wa-sos', 9, ?2, '', '', 'muse-wonder-1.0', ?3)",
                rusqlite::params![fact, verdict, at],
            )
            .unwrap();
        }
        let all = latest_for_slug(&conn, "usa-wa-sos").unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0]["verdict"], "supported", "the newest check wins");
        let accepted = checks_for_slug(&conn, "usa-wa-sos").unwrap();
        assert_eq!(accepted.len(), 1);
        assert_eq!(accepted[0]["fact_id"], 201);
    }
}
