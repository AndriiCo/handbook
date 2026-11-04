//! Work queue (`work.sqlite`): pending jobs that plant workers lease.
//!
//! Its own database file, created at runtime with `CREATE … IF NOT
//! EXISTS` DDL ([`migrate`], the `telemetry::migrate_tables` pattern):
//! nothing under `migrations/` and no schema-version constant changes,
//! so the ship rails pass, and `handbook.sqlite` is never touched.
//! Every state change and its `audit` row commit in one transaction
//! (B1). Lease tokens are
//! minted by the caller and stored inside `lease_owner`
//! (`<worker>:<token>`): the token, not the socket, owns a lease, so a
//! worker that reconnects mid-job can still heartbeat and complete.
//! Spec: `docs/specs/work-queue.md`.

use rusqlite::{Connection, OptionalExtension};

use contracts::Result;

use crate::db::{apply_pragmas, in_transaction};
use crate::store::Store;

/// Attempts an item gets when the enqueuer names none.
pub const DEFAULT_MAX_ATTEMPTS: i64 = 3;

/// Audit actor for lease sweeps (no human or worker triggered them).
pub const SWEEP_ACTOR: &str = "work-sweep";

/// Longest error / audit reason kept (bytes, cut on a char boundary).
const MAX_REASON: usize = 500;

/// First wait before a retryable worker failure is leased again; it
/// doubles per attempt up to [`RETRY_MAX_SECS`], so a rate limit does
/// not burn every attempt in seconds.
pub const RETRY_BASE_SECS: i64 = 60;
/// Longest wait between attempts.
pub const RETRY_MAX_SECS: i64 = 30 * 60;

/// Open (creating if missing) the work-queue database. Same PRAGMAs
/// as [`crate::db::open`] (WAL, busy timeout, foreign keys).
pub fn open(store: &Store) -> Result<Connection> {
    let conn = Connection::open(store.work_db_path())?;
    apply_pragmas(&conn)?;
    Ok(conn)
}

/// Create the work-queue tables when missing. Idempotent, one
/// transaction. Additive changes only (a new table, a new index): the
/// ship rails refuse ALTER/DROP, so a reshaped table is a new table.
pub fn migrate(conn: &Connection) -> Result<()> {
    in_transaction(conn, || {
        conn.execute_batch(TABLES)?;
        Ok(())
    })
}

/// `work.sqlite` DDL (spec `docs/specs/work-queue.md` §1).
const TABLES: &str = "
CREATE TABLE IF NOT EXISTS work_item(
  id INTEGER PRIMARY KEY,
  kind TEXT NOT NULL,                 -- e.g. 'research.nonparty' (web lane vocabulary)
  payload_json TEXT NOT NULL DEFAULT '{}',
  status TEXT NOT NULL DEFAULT 'pending'
    CHECK(status IN ('pending', 'leased', 'done', 'failed')),
  lease_owner TEXT,                   -- '<worker>:<lease token>' while leased, else NULL
  lease_expires_at INTEGER,           -- unix seconds while leased, else NULL
  attempts INTEGER NOT NULL DEFAULT 0 CHECK(attempts >= 0),
  max_attempts INTEGER NOT NULL DEFAULT 3 CHECK(max_attempts >= 1),
  result_sha256 TEXT,                 -- store blob/xx/yy/<sha>, NULL until done
  result_bytes INTEGER,
  result_content_type TEXT,
  summary_json TEXT,                  -- small worker-reported summary
  error TEXT,                         -- last failure reason
  created_by TEXT NOT NULL DEFAULT '',
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  CHECK((status = 'leased') = (lease_owner IS NOT NULL AND lease_expires_at IS NOT NULL))
);
CREATE INDEX IF NOT EXISTS idx_work_item_status ON work_item(status, id);
-- A requeued item waits until not_before (retry backoff). Its own table:
-- the ship rails refuse ALTER on work_item.
CREATE TABLE IF NOT EXISTS work_retry_after(
  item_id INTEGER PRIMARY KEY,        -- work_item.id
  not_before INTEGER NOT NULL         -- unix seconds
);
CREATE TABLE IF NOT EXISTS audit(
  id INTEGER PRIMARY KEY,
  at INTEGER NOT NULL,
  actor TEXT NOT NULL DEFAULT '',
  action TEXT NOT NULL,
  entity TEXT NOT NULL,
  entity_id INTEGER NOT NULL,
  reason TEXT NOT NULL DEFAULT ''
);
CREATE INDEX IF NOT EXISTS idx_work_audit_entity ON audit(entity, entity_id);
-- Worker enrolment (docs/specs/worker-enrolment.md): a worker asks, staff
-- approve with the pairing code, the worker collects its token once.
CREATE TABLE IF NOT EXISTS worker_enrolment(
  id INTEGER PRIMARY KEY,
  public_id TEXT NOT NULL UNIQUE,     -- 32 hex, the worker polls by it
  username TEXT NOT NULL,
  hostname TEXT NOT NULL,
  version TEXT NOT NULL DEFAULT '',
  started_at INTEGER NOT NULL,
  capabilities_json TEXT NOT NULL DEFAULT '[]',
  peer_ip TEXT NOT NULL DEFAULT '',
  poll_hash TEXT NOT NULL,            -- hex SHA-256 of the worker's poll secret
  code_hash TEXT NOT NULL,            -- hex SHA-256 of the pairing code
  code_tries INTEGER NOT NULL DEFAULT 0,
  status TEXT NOT NULL DEFAULT 'pending'
    CHECK(status IN ('pending', 'approved', 'denied', 'delivered')),
  name TEXT,                          -- final worker name, set on approval
  expires_at INTEGER NOT NULL,
  created_at INTEGER NOT NULL,
  decided_by TEXT,
  decided_at INTEGER,
  credential_id INTEGER               -- worker_credential.id once delivered
);
CREATE INDEX IF NOT EXISTS idx_worker_enrolment_ip ON worker_enrolment(peer_ip, created_at);
CREATE TABLE IF NOT EXISTS worker_credential(
  id INTEGER PRIMARY KEY,
  public_id TEXT NOT NULL UNIQUE,     -- 32 hex, the middle of the w2. token
  name TEXT NOT NULL,
  token_hash TEXT NOT NULL,           -- hex SHA-256 of the whole token
  enrolment_id INTEGER,               -- NULL when minted by `xtask up`
  created_at INTEGER NOT NULL,
  created_by TEXT NOT NULL DEFAULT '',
  last_seen_at INTEGER,
  last_version TEXT,
  revoked_at INTEGER,
  revoked_by TEXT
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_worker_credential_active_name
  ON worker_credential(name) WHERE revoked_at IS NULL;
CREATE TRIGGER IF NOT EXISTS audit_no_update BEFORE UPDATE ON audit
BEGIN
  SELECT RAISE(ABORT, 'audit is append-only: UPDATE rejected');
END;
CREATE TRIGGER IF NOT EXISTS audit_no_delete BEFORE DELETE ON audit
BEGIN
  SELECT RAISE(ABORT, 'audit is append-only: DELETE rejected');
END;
";

/// Where a finished item's result bytes live (store blob address).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResultRef {
    pub sha256: String,
    pub bytes: i64,
    pub content_type: String,
}

/// One `work_item` row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkItem {
    pub id: i64,
    pub kind: String,
    pub payload_json: String,
    /// `pending | leased | done | failed`.
    pub status: String,
    /// `<worker>:<token>` while leased. Never send it to a browser:
    /// [`WorkItem::lease_worker`] is the displayable half.
    pub lease_owner: Option<String>,
    pub lease_expires_at: Option<i64>,
    pub attempts: i64,
    pub max_attempts: i64,
    pub result: Option<ResultRef>,
    pub summary_json: Option<String>,
    pub error: Option<String>,
    pub created_by: String,
    pub created_at: i64,
    pub updated_at: i64,
    /// A requeued item is not leased before this (retry backoff).
    pub retry_after: Option<i64>,
}

impl WorkItem {
    /// Worker name of the current lease (the token stays server-side).
    pub fn lease_worker(&self) -> Option<&str> {
        self.lease_owner
            .as_deref()
            .map(|owner| owner.split_once(':').map_or(owner, |(name, _)| name))
    }
}

/// One `audit` row of the work database.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditRow {
    pub at: i64,
    pub actor: String,
    pub action: String,
    pub reason: String,
}

/// New-item fields plus audit attribution.
pub struct NewItem<'a> {
    pub kind: &'a str,
    pub payload_json: &'a str,
    pub max_attempts: i64,
    pub actor: &'a str,
    pub at: i64,
}

/// One checkout request: who leases, for which kinds, for how long.
pub struct Checkout<'a> {
    /// `<worker>:<token>`; the caller mints the token.
    pub owner: &'a str,
    /// Worker name (audit actor).
    pub actor: &'a str,
    /// Kinds the worker announced; an empty list leases nothing.
    pub kinds: &'a [String],
    pub lease_secs: i64,
    pub now: i64,
}

/// Item status after [`fail`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailOutcome {
    /// Back to `pending` for another attempt.
    Requeued,
    /// Out of attempts (or not retryable): `failed`.
    Failed,
}

impl FailOutcome {
    pub fn status(self) -> &'static str {
        match self {
            FailOutcome::Requeued => "pending",
            FailOutcome::Failed => "failed",
        }
    }
}

const COLUMNS: &str = "id, kind, payload_json, status, lease_owner, lease_expires_at, attempts,
     max_attempts, result_sha256, result_bytes, result_content_type, summary_json, error,
     created_by, created_at, updated_at,
     (SELECT not_before FROM work_retry_after r
      WHERE r.item_id = work_item.id AND work_item.status = 'pending')";

fn row_to_item(row: &rusqlite::Row<'_>) -> rusqlite::Result<WorkItem> {
    let sha: Option<String> = row.get(8)?;
    let bytes: Option<i64> = row.get(9)?;
    let content_type: Option<String> = row.get(10)?;
    Ok(WorkItem {
        id: row.get(0)?,
        kind: row.get(1)?,
        payload_json: row.get(2)?,
        status: row.get(3)?,
        lease_owner: row.get(4)?,
        lease_expires_at: row.get(5)?,
        attempts: row.get(6)?,
        max_attempts: row.get(7)?,
        result: sha.map(|sha256| ResultRef {
            sha256,
            bytes: bytes.unwrap_or(0),
            content_type: content_type.unwrap_or_else(|| "application/octet-stream".to_string()),
        }),
        summary_json: row.get(11)?,
        error: row.get(12)?,
        created_by: row.get(13)?,
        created_at: row.get(14)?,
        updated_at: row.get(15)?,
        retry_after: row.get(16)?,
    })
}

/// Wait before attempt `attempts + 1` after a retryable failure.
pub fn retry_delay(attempts: i64) -> i64 {
    let doublings = (attempts - 1).clamp(0, 16) as u32;
    (RETRY_BASE_SECS << doublings).min(RETRY_MAX_SECS)
}

/// Drop any backoff for `id` (it finished, failed for good, or staff
/// acted on it).
fn clear_retry(conn: &Connection, id: i64) -> Result<()> {
    conn.execute("DELETE FROM work_retry_after WHERE item_id = ?1", [id])?;
    Ok(())
}

fn audit(
    conn: &Connection,
    at: i64,
    actor: &str,
    action: &str,
    id: i64,
    reason: &str,
) -> Result<()> {
    audit_entity(conn, at, actor, action, "work_item", id, reason)
}

/// One `audit` row for any entity in `work.sqlite`.
pub(crate) fn audit_entity(
    conn: &Connection,
    at: i64,
    actor: &str,
    action: &str,
    entity: &str,
    id: i64,
    reason: &str,
) -> Result<()> {
    conn.execute(
        "INSERT INTO audit(at, actor, action, entity, entity_id, reason)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        rusqlite::params![at, actor, action, entity, id, clip(reason)],
    )?;
    Ok(())
}

/// Cut `text` to [`MAX_REASON`] bytes on a char boundary.
fn clip(text: &str) -> &str {
    if text.len() <= MAX_REASON {
        return text;
    }
    let mut end = MAX_REASON;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

/// Enqueue one item; the row and its `work_enqueue` audit commit
/// together. Returns the new id.
pub fn enqueue(conn: &Connection, item: &NewItem<'_>) -> Result<i64> {
    in_transaction(conn, || {
        conn.execute(
            "INSERT INTO work_item(kind, payload_json, max_attempts, created_by,
                                   created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
            rusqlite::params![
                item.kind,
                item.payload_json,
                item.max_attempts.max(1),
                item.actor,
                item.at
            ],
        )?;
        let id = conn.last_insert_rowid();
        audit(conn, item.at, item.actor, "work_enqueue", id, item.kind)?;
        Ok(id)
    })
}

/// Requeue (or fail, at `max_attempts`) every lease that expired
/// before `now`. Returns `(requeued, failed)`. Called before every
/// checkout and admin list, so no timer is needed.
pub fn requeue_expired(conn: &Connection, now: i64) -> Result<(usize, usize)> {
    in_transaction(conn, || sweep_expired(conn, now))
}

/// [`requeue_expired`] body, for callers already in a transaction.
fn sweep_expired(conn: &Connection, now: i64) -> Result<(usize, usize)> {
    let expired: Vec<(i64, i64, i64)> = conn
        .prepare(
            "SELECT id, attempts, max_attempts FROM work_item
             WHERE status = 'leased' AND lease_expires_at < ?1 ORDER BY id",
        )?
        .query_map([now], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
        .collect::<rusqlite::Result<_>>()?;
    let (mut requeued, mut failed) = (0, 0);
    for (id, attempts, max_attempts) in expired {
        if attempts < max_attempts {
            conn.execute(
                "UPDATE work_item SET status = 'pending', lease_owner = NULL,
                        lease_expires_at = NULL, error = 'lease expired', updated_at = ?2
                 WHERE id = ?1",
                rusqlite::params![id, now],
            )?;
            audit(
                conn,
                now,
                SWEEP_ACTOR,
                "work_requeue",
                id,
                &format!("lease expired on attempt {attempts}"),
            )?;
            requeued += 1;
        } else {
            let reason = format!("lease expired after {attempts} attempt(s)");
            conn.execute(
                "UPDATE work_item SET status = 'failed', lease_owner = NULL,
                        lease_expires_at = NULL, error = ?2, updated_at = ?3
                 WHERE id = ?1",
                rusqlite::params![id, reason, now],
            )?;
            audit(conn, now, SWEEP_ACTOR, "work_expire", id, &reason)?;
            failed += 1;
        }
    }
    Ok((requeued, failed))
}

/// Lease the oldest pending item whose kind the worker announced.
/// Sweeps expired leases first, bumps `attempts`, audits
/// `work_checkout`. `None` when nothing matches.
pub fn checkout(conn: &Connection, req: &Checkout<'_>) -> Result<Option<WorkItem>> {
    if req.kinds.is_empty() {
        return Ok(None);
    }
    in_transaction(conn, || {
        sweep_expired(conn, req.now)?;
        let marks: Vec<String> = (2..req.kinds.len() + 2).map(|i| format!("?{i}")).collect();
        let params = std::iter::once(&req.now as &dyn rusqlite::ToSql)
            .chain(req.kinds.iter().map(|k| k as &dyn rusqlite::ToSql));
        let id: Option<i64> = conn
            .query_row(
                &format!(
                    "SELECT id FROM work_item WHERE status = 'pending' AND kind IN ({})
                       AND NOT EXISTS (SELECT 1 FROM work_retry_after r
                                       WHERE r.item_id = work_item.id AND r.not_before > ?1)
                     ORDER BY id LIMIT 1",
                    marks.join(", ")
                ),
                rusqlite::params_from_iter(params),
                |row| row.get(0),
            )
            .optional()?;
        let Some(id) = id else {
            return Ok(None);
        };
        conn.execute(
            "UPDATE work_item SET status = 'leased', lease_owner = ?2,
                    lease_expires_at = ?3, attempts = attempts + 1, updated_at = ?4
             WHERE id = ?1",
            rusqlite::params![id, req.owner, req.now + req.lease_secs, req.now],
        )?;
        clear_retry(conn, id)?;
        let item = get_inner(conn, id)?.expect("row leased above");
        audit(
            conn,
            req.now,
            req.actor,
            "work_checkout",
            id,
            &format!("attempt {}", item.attempts),
        )?;
        Ok(Some(item))
    })
}

/// Extend a lease held by `owner`. Returns the new expiry, or `None`
/// when the lease is lost (swept, finished, or someone else's). A
/// lease stays its token's until a sweep requeues it, so a heartbeat
/// that arrives just after expiry still lands.
pub fn heartbeat(
    conn: &Connection,
    id: i64,
    owner: &str,
    lease_secs: i64,
    now: i64,
) -> Result<Option<i64>> {
    let expires = now + lease_secs;
    let changed = conn.execute(
        "UPDATE work_item SET lease_expires_at = ?3, updated_at = ?4
         WHERE id = ?1 AND status = 'leased' AND lease_owner = ?2",
        rusqlite::params![id, owner, expires, now],
    )?;
    Ok((changed == 1).then_some(expires))
}

/// Finish a leased item. `false` when the lease is lost (nothing
/// changes). The result blob must already be written by the caller.
pub fn complete(
    conn: &Connection,
    id: i64,
    owner: &str,
    actor: &str,
    result: Option<&ResultRef>,
    summary_json: Option<&str>,
    now: i64,
) -> Result<bool> {
    in_transaction(conn, || {
        let changed = conn.execute(
            "UPDATE work_item SET status = 'done', lease_owner = NULL, lease_expires_at = NULL,
                    result_sha256 = ?3, result_bytes = ?4, result_content_type = ?5,
                    summary_json = ?6, error = NULL, updated_at = ?7
             WHERE id = ?1 AND status = 'leased' AND lease_owner = ?2",
            rusqlite::params![
                id,
                owner,
                result.map(|r| r.sha256.as_str()),
                result.map(|r| r.bytes),
                result.map(|r| r.content_type.as_str()),
                summary_json,
                now
            ],
        )?;
        if changed != 1 {
            return Ok(false);
        }
        let reason =
            result.map_or_else(|| "no result".to_string(), |r| format!("{} bytes", r.bytes));
        audit(conn, now, actor, "work_complete", id, &reason)?;
        Ok(true)
    })
}

/// Record a worker-reported failure. Retryable failures with attempts
/// left go back to `pending`; the rest end `failed`. `None` when the
/// lease is lost.
pub fn fail(
    conn: &Connection,
    id: i64,
    owner: &str,
    actor: &str,
    error: &str,
    retryable: bool,
    now: i64,
) -> Result<Option<FailOutcome>> {
    in_transaction(conn, || {
        let row: Option<(i64, i64)> = conn
            .query_row(
                "SELECT attempts, max_attempts FROM work_item
                 WHERE id = ?1 AND status = 'leased' AND lease_owner = ?2",
                rusqlite::params![id, owner],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let Some((attempts, max_attempts)) = row else {
            return Ok(None);
        };
        let outcome = if retryable && attempts < max_attempts {
            FailOutcome::Requeued
        } else {
            FailOutcome::Failed
        };
        conn.execute(
            "UPDATE work_item SET status = ?2, lease_owner = NULL, lease_expires_at = NULL,
                    error = ?3, updated_at = ?4
             WHERE id = ?1",
            rusqlite::params![id, outcome.status(), clip(error), now],
        )?;
        let reason = if outcome == FailOutcome::Requeued {
            let wait = retry_delay(attempts);
            conn.execute(
                "INSERT INTO work_retry_after(item_id, not_before) VALUES (?1, ?2)
                 ON CONFLICT(item_id) DO UPDATE SET not_before = excluded.not_before",
                rusqlite::params![id, now + wait],
            )?;
            format!("pending (retry in {wait}s): {error}")
        } else {
            format!("failed: {error}")
        };
        audit(conn, now, actor, "work_fail", id, &reason)?;
        Ok(Some(outcome))
    })
}

/// Audit an action on an item that changes no queue state (e.g. staff
/// staging a result as packs). `false` when the item does not exist.
pub fn note(
    conn: &Connection,
    id: i64,
    actor: &str,
    action: &str,
    reason: &str,
    now: i64,
) -> Result<bool> {
    in_transaction(conn, || {
        if get_inner(conn, id)?.is_none() {
            return Ok(false);
        }
        audit(conn, now, actor, action, id, reason)?;
        Ok(true)
    })
}

/// Staff retry: a `failed` item goes back to `pending` with its
/// attempts reset. `false` for any other status.
pub fn retry(conn: &Connection, id: i64, actor: &str, now: i64) -> Result<bool> {
    in_transaction(conn, || {
        let changed = conn.execute(
            "UPDATE work_item SET status = 'pending', attempts = 0, updated_at = ?2
             WHERE id = ?1 AND status = 'failed'",
            rusqlite::params![id, now],
        )?;
        if changed == 1 {
            clear_retry(conn, id)?;
            audit(conn, now, actor, "work_retry", id, "staff retry")?;
        }
        Ok(changed == 1)
    })
}

/// Staff cancel: a `pending` or `leased` item ends `failed` ("cancelled
/// by …") and is never leased again; a worker still running it loses
/// the lease at its next heartbeat. `false` for any other status.
pub fn cancel(conn: &Connection, id: i64, actor: &str, now: i64) -> Result<bool> {
    in_transaction(conn, || {
        let reason = format!("cancelled by {actor}");
        let changed = conn.execute(
            "UPDATE work_item SET status = 'failed', lease_owner = NULL, lease_expires_at = NULL,
                    error = ?2, updated_at = ?3
             WHERE id = ?1 AND status IN ('pending', 'leased')",
            rusqlite::params![id, clip(&reason), now],
        )?;
        if changed == 1 {
            clear_retry(conn, id)?;
            audit(conn, now, actor, "work_cancel", id, &reason)?;
        }
        Ok(changed == 1)
    })
}

/// End a lease held by `owner` that has run longer than `max_secs`
/// since its checkout: the item fails for good ("ran over …") and
/// `true` comes back, so the caller answers the heartbeat `lease_lost`.
/// `false` while the job is inside its limit (or the lease is not
/// `owner`'s).
pub fn end_if_overrun(
    conn: &Connection,
    id: i64,
    owner: &str,
    max_secs: i64,
    now: i64,
) -> Result<bool> {
    in_transaction(conn, || {
        let leased_at: Option<i64> = conn
            .query_row(
                "SELECT (SELECT max(at) FROM audit WHERE entity = 'work_item'
                           AND entity_id = w.id AND action = 'work_checkout')
                 FROM work_item w WHERE w.id = ?1 AND w.status = 'leased' AND w.lease_owner = ?2",
                rusqlite::params![id, owner],
                |row| row.get(0),
            )
            .optional()?
            .flatten();
        let Some(leased_at) = leased_at else {
            return Ok(false);
        };
        if now - leased_at <= max_secs {
            return Ok(false);
        }
        let reason = format!("ran over its {} min limit", (max_secs + 59) / 60);
        conn.execute(
            "UPDATE work_item SET status = 'failed', lease_owner = NULL, lease_expires_at = NULL,
                    error = ?2, updated_at = ?3
             WHERE id = ?1",
            rusqlite::params![id, reason, now],
        )?;
        audit(conn, now, SWEEP_ACTOR, "work_timeout", id, &reason)?;
        Ok(true)
    })
}

fn get_inner(conn: &Connection, id: i64) -> Result<Option<WorkItem>> {
    Ok(conn
        .query_row(
            &format!("SELECT {COLUMNS} FROM work_item WHERE id = ?1"),
            [id],
            row_to_item,
        )
        .optional()?)
}

/// One item by id.
pub fn get(conn: &Connection, id: i64) -> Result<Option<WorkItem>> {
    get_inner(conn, id)
}

/// Newest first; `before_id` pages (`id < before_id`), `limit` is
/// clamped to 1..=200, `status` filters verbatim.
pub fn list(
    conn: &Connection,
    status: Option<&str>,
    limit: i64,
    before_id: Option<i64>,
) -> Result<Vec<WorkItem>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLUMNS} FROM work_item
         WHERE (?1 IS NULL OR status = ?1) AND (?2 IS NULL OR id < ?2)
         ORDER BY id DESC LIMIT ?3"
    ))?;
    let rows = stmt
        .query_map(
            rusqlite::params![status, before_id, limit.clamp(1, 200)],
            row_to_item,
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Item count per status (every status present, zero when empty).
pub fn counts(conn: &Connection) -> Result<Vec<(String, i64)>> {
    let mut out: Vec<(String, i64)> = ["pending", "leased", "done", "failed"]
        .iter()
        .map(|s| (s.to_string(), 0))
        .collect();
    let mut stmt = conn.prepare("SELECT status, COUNT(*) FROM work_item GROUP BY status")?;
    let rows = stmt.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get(1)?)))?;
    for row in rows {
        let (status, n) = row?;
        if let Some(slot) = out.iter_mut().find(|(s, _)| *s == status) {
            slot.1 = n;
        }
    }
    Ok(out)
}

/// Audit trail of one item, oldest first.
pub fn audit_for(conn: &Connection, id: i64) -> Result<Vec<AuditRow>> {
    let mut stmt = conn.prepare(
        "SELECT at, actor, action, reason FROM audit
         WHERE entity = 'work_item' AND entity_id = ?1 ORDER BY id",
    )?;
    let rows = stmt
        .query_map([id], |row| {
            Ok(AuditRow {
                at: row.get(0)?,
                actor: row.get(1)?,
                action: row.get(2)?,
                reason: row.get(3)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh() -> (tempfile::TempDir, Connection) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path());
        store.init().unwrap();
        let conn = open(&store).unwrap();
        migrate(&conn).unwrap();
        (dir, conn)
    }

    fn add(conn: &Connection, kind: &str, max_attempts: i64, at: i64) -> i64 {
        enqueue(
            conn,
            &NewItem {
                kind,
                payload_json: "{}",
                max_attempts,
                actor: "staff-1",
                at,
            },
        )
        .unwrap()
    }

    fn lease<'a>(owner: &'a str, kinds: &'a [String], now: i64) -> Checkout<'a> {
        Checkout {
            owner,
            actor: "plant",
            kinds,
            lease_secs: 300,
            now,
        }
    }

    fn actions(conn: &Connection, id: i64) -> Vec<String> {
        audit_for(conn, id)
            .unwrap()
            .into_iter()
            .map(|row| row.action)
            .collect()
    }

    #[test]
    fn migrate_is_idempotent_and_keeps_rows() {
        let (_dir, conn) = fresh();
        let id = add(&conn, "scrape.urls", 3, 100);
        migrate(&conn).unwrap();
        migrate(&conn).unwrap();
        assert_eq!(get(&conn, id).unwrap().unwrap().kind, "scrape.urls");
        // Not the handbook database: none of its tables live here.
        let handbook: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE name IN ('authority', 'fact', 'config')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(handbook, 0);
        assert!(note(&conn, id, "staff-1", "work_stage", "1 pack", 101).unwrap());
        assert!(!note(&conn, id + 1, "staff-1", "work_stage", "x", 101).unwrap());
        assert_eq!(actions(&conn, id), ["work_enqueue", "work_stage"]);
    }

    #[test]
    fn enqueue_checkout_complete_round_trip_with_audit() {
        let (_dir, conn) = fresh();
        let id = add(&conn, "scrape.urls", 3, 100);
        let kinds = vec!["scrape.urls".to_string()];
        let item = checkout(&conn, &lease("plant:tok", &kinds, 110))
            .unwrap()
            .unwrap();
        assert_eq!(
            (item.id, item.status.as_str(), item.attempts),
            (id, "leased", 1)
        );
        assert_eq!(item.lease_worker(), Some("plant"));
        assert_eq!(item.lease_expires_at, Some(410));
        assert!(
            checkout(&conn, &lease("plant:tok2", &kinds, 111))
                .unwrap()
                .is_none()
        );
        assert_eq!(
            heartbeat(&conn, id, "plant:tok", 300, 200).unwrap(),
            Some(500)
        );
        assert_eq!(heartbeat(&conn, id, "plant:other", 300, 200).unwrap(), None);
        let result = ResultRef {
            sha256: "ab".repeat(32),
            bytes: 2,
            content_type: "application/json".to_string(),
        };
        assert!(!complete(&conn, id, "plant:other", "plant", Some(&result), None, 210).unwrap());
        assert!(
            complete(
                &conn,
                id,
                "plant:tok",
                "plant",
                Some(&result),
                Some("{}"),
                210
            )
            .unwrap()
        );
        let done = get(&conn, id).unwrap().unwrap();
        assert_eq!(done.status, "done");
        assert_eq!(done.result, Some(result));
        assert_eq!(done.lease_owner, None);
        assert_eq!(
            actions(&conn, id),
            ["work_enqueue", "work_checkout", "work_complete"]
        );
    }

    #[test]
    fn checkout_honours_kinds_and_fifo_order() {
        let (_dir, conn) = fresh();
        let a = add(&conn, "eval.gold", 3, 100);
        let b = add(&conn, "scrape.urls", 3, 101);
        let c = add(&conn, "scrape.urls", 3, 102);
        let scrape = vec!["scrape.urls".to_string()];
        assert!(checkout(&conn, &lease("w:1", &[], 110)).unwrap().is_none());
        assert_eq!(
            checkout(&conn, &lease("w:1", &scrape, 110))
                .unwrap()
                .unwrap()
                .id,
            b
        );
        assert_eq!(
            checkout(&conn, &lease("w:2", &scrape, 110))
                .unwrap()
                .unwrap()
                .id,
            c
        );
        assert_eq!(get(&conn, a).unwrap().unwrap().status, "pending");
    }

    #[test]
    fn expired_leases_requeue_then_fail_at_the_retry_limit() {
        let (_dir, conn) = fresh();
        let id = add(&conn, "scrape.urls", 2, 100);
        let kinds = vec!["scrape.urls".to_string()];
        checkout(&conn, &lease("w:1", &kinds, 100))
            .unwrap()
            .unwrap();
        // Lease ran out at 400: the next checkout sweeps and re-leases.
        let again = checkout(&conn, &lease("w:2", &kinds, 401))
            .unwrap()
            .unwrap();
        assert_eq!((again.id, again.attempts), (id, 2));
        // The old token lost the lease.
        assert!(!complete(&conn, id, "w:1", "w", None, None, 402).unwrap());
        // Second expiry at max_attempts: failed, never re-leased.
        assert_eq!(requeue_expired(&conn, 702).unwrap(), (0, 1));
        let failed = get(&conn, id).unwrap().unwrap();
        assert_eq!(failed.status, "failed");
        assert_eq!(
            failed.error.as_deref(),
            Some("lease expired after 2 attempt(s)")
        );
        assert!(
            checkout(&conn, &lease("w:3", &kinds, 703))
                .unwrap()
                .is_none()
        );
        assert_eq!(
            actions(&conn, id),
            [
                "work_enqueue",
                "work_checkout",
                "work_requeue",
                "work_checkout",
                "work_expire"
            ]
        );
        // Staff retry resets attempts.
        assert!(retry(&conn, id, "staff-1", 800).unwrap());
        assert!(!retry(&conn, id, "staff-1", 801).unwrap());
        let back = checkout(&conn, &lease("w:4", &kinds, 802))
            .unwrap()
            .unwrap();
        assert_eq!(back.attempts, 1);
    }

    #[test]
    fn worker_failures_retry_until_the_limit() {
        let (_dir, conn) = fresh();
        let id = add(&conn, "scrape.urls", 2, 100);
        let kinds = vec!["scrape.urls".to_string()];
        checkout(&conn, &lease("w:1", &kinds, 100)).unwrap();
        assert_eq!(
            fail(&conn, id, "w:1", "w", "boom", true, 101).unwrap(),
            Some(FailOutcome::Requeued)
        );
        assert_eq!(
            fail(&conn, id, "w:1", "w", "boom", true, 101).unwrap(),
            None
        );
        // Backing off: not leased again until 101 + 60.
        assert!(
            checkout(&conn, &lease("w:2", &kinds, 102))
                .unwrap()
                .is_none()
        );
        assert_eq!(get(&conn, id).unwrap().unwrap().retry_after, Some(161));
        checkout(&conn, &lease("w:2", &kinds, 161))
            .unwrap()
            .unwrap();
        assert_eq!(get(&conn, id).unwrap().unwrap().retry_after, None);
        assert_eq!(
            fail(&conn, id, "w:2", "w", "boom again", true, 163).unwrap(),
            Some(FailOutcome::Failed)
        );
        let item = get(&conn, id).unwrap().unwrap();
        assert_eq!(
            (item.status.as_str(), item.error.as_deref()),
            ("failed", Some("boom again"))
        );
        let id2 = add(&conn, "scrape.urls", 3, 104);
        checkout(&conn, &lease("w:3", &kinds, 105)).unwrap();
        assert_eq!(
            fail(&conn, id2, "w:3", "w", "bad payload", false, 106).unwrap(),
            Some(FailOutcome::Failed)
        );
    }

    #[test]
    fn retry_delay_doubles_and_caps() {
        let waits: Vec<i64> = (1..=7).map(retry_delay).collect();
        assert_eq!(waits, [60, 120, 240, 480, 960, 1800, 1800]);
    }

    #[test]
    fn staff_cancel_ends_pending_and_leased_items() {
        let (_dir, conn) = fresh();
        let kinds = vec!["scrape.urls".to_string()];
        let leased = add(&conn, "scrape.urls", 3, 100);
        let pending = add(&conn, "scrape.urls", 3, 100);
        let first = checkout(&conn, &lease("w:1", &kinds, 101))
            .unwrap()
            .unwrap();
        assert_eq!(first.id, leased);
        for id in [leased, pending] {
            assert!(cancel(&conn, id, "staff-1", 102).unwrap());
            let item = get(&conn, id).unwrap().unwrap();
            assert_eq!(
                (item.status.as_str(), item.error.as_deref()),
                ("failed", Some("cancelled by staff-1"))
            );
        }
        // The worker loses the lease and nothing is leased again.
        assert_eq!(heartbeat(&conn, leased, "w:1", 300, 103).unwrap(), None);
        assert!(
            checkout(&conn, &lease("w:2", &kinds, 104))
                .unwrap()
                .is_none()
        );
        assert!(
            !cancel(&conn, pending, "staff-1", 105).unwrap(),
            "already failed"
        );
        assert!(actions(&conn, pending).contains(&"work_cancel".to_string()));
    }

    #[test]
    fn a_lease_that_runs_over_its_limit_fails() {
        let (_dir, conn) = fresh();
        let kinds = vec!["scrape.urls".to_string()];
        let id = add(&conn, "scrape.urls", 3, 100);
        checkout(&conn, &lease("w:1", &kinds, 100))
            .unwrap()
            .unwrap();
        assert!(!end_if_overrun(&conn, id, "w:1", 600, 700).unwrap());
        assert!(
            !end_if_overrun(&conn, id, "w:2", 600, 701).unwrap(),
            "not w:2's lease"
        );
        assert!(end_if_overrun(&conn, id, "w:1", 600, 701).unwrap());
        let item = get(&conn, id).unwrap().unwrap();
        assert_eq!(
            (item.status.as_str(), item.error.as_deref()),
            ("failed", Some("ran over its 10 min limit"))
        );
        assert_eq!(heartbeat(&conn, id, "w:1", 300, 702).unwrap(), None);
        assert!(!end_if_overrun(&conn, id, "w:1", 600, 703).unwrap());
    }

    #[test]
    fn list_pages_filters_and_counts() {
        let (_dir, conn) = fresh();
        let ids: Vec<i64> = (0..5)
            .map(|i| add(&conn, "scrape.urls", 3, 100 + i))
            .collect();
        let kinds = vec!["scrape.urls".to_string()];
        checkout(&conn, &lease("w:1", &kinds, 200)).unwrap();
        let page = list(&conn, None, 2, None).unwrap();
        assert_eq!(
            page.iter().map(|i| i.id).collect::<Vec<_>>(),
            [ids[4], ids[3]]
        );
        let next = list(&conn, None, 2, Some(ids[3])).unwrap();
        assert_eq!(
            next.iter().map(|i| i.id).collect::<Vec<_>>(),
            [ids[2], ids[1]]
        );
        assert_eq!(list(&conn, Some("leased"), 50, None).unwrap()[0].id, ids[0]);
        let counts = counts(&conn).unwrap();
        assert_eq!(counts[0], ("pending".to_string(), 4));
        assert_eq!(counts[1], ("leased".to_string(), 1));
        assert_eq!(counts[3], ("failed".to_string(), 0));
    }

    #[test]
    fn audit_is_append_only_and_reasons_are_clipped() {
        let (_dir, conn) = fresh();
        let id = add(&conn, "scrape.urls", 3, 100);
        assert!(conn.execute("UPDATE audit SET actor = 'x'", []).is_err());
        assert!(conn.execute("DELETE FROM audit", []).is_err());
        let kinds = vec!["scrape.urls".to_string()];
        checkout(&conn, &lease("w:1", &kinds, 101)).unwrap();
        let long = "é".repeat(400);
        fail(&conn, id, "w:1", "w", &long, false, 102).unwrap();
        let item = get(&conn, id).unwrap().unwrap();
        assert!(item.error.unwrap().len() <= MAX_REASON);
    }
}
