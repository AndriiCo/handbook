//! SQLite metadata database: migrations and foundation queries.
//!
//! Fresh databases start from the `migrations/schema_baseline.sql`
//! snapshot (= v11; the 001–011 incrementals were squashed away in
//! prompt 055 after prod reached v11 — see git history). Later
//! migrations add `N+1` files and extend [`migrate`]. The P2
//! customer-PII database (`p2.sqlite`) has its own baseline
//! (`migrations/p2_schema_baseline.sql`, = P2 v2) and its own
//! [`migrate_p2`].

use rusqlite::Connection;

use contracts::{HandbookError, Result};

use crate::store::Store;

/// Current schema version (`pragma user_version`).
pub const SCHEMA_VERSION: i32 = 11;

/// Current P2 customer-PII schema version (`pragma user_version` on
/// `p2.sqlite`; independent of [`SCHEMA_VERSION`]).
pub const P2_SCHEMA_VERSION: i32 = 2;

/// Voice-audio retention in days (decided 2026-10-26).
pub const VOICE_RETENTION_DAYS: i64 = 365;

/// Open (creating if missing) the store's metadata database.
///
/// Sets `journal_mode=WAL` (one writer, many readers),
/// `busy_timeout=5000` (wait out lock contention instead of failing),
/// `synchronous=NORMAL` (WAL-safe durability), and `foreign_keys=ON`.
///
/// # SAFE DELETE rule
///
/// Every connection from this function enforces foreign keys, so a
/// `DELETE` that would orphan a child row fails instead of silently
/// dangling. Consequences for all writers:
/// - New tables MUST declare real `REFERENCES` clauses for references.
/// - Deletes MUST remove (or null, where nullable) child rows first.
/// - Never work around a violation with `PRAGMA foreign_keys=OFF`.
pub fn open(store: &Store) -> Result<Connection> {
    let conn = Connection::open(store.db_path())?;
    apply_pragmas(&conn)?;
    Ok(conn)
}

/// Open (creating if missing) the P2 customer-PII database.
///
/// Same PRAGMAs as [`open`]. Cross-DB references (authority /
/// form_template / capture ids) are opaque integers SQLite cannot
/// enforce: readers must treat a missing parent as "open a task +
/// audit", never as a join assumption. PII retention is OQ-7 (open).
pub fn open_p2(store: &Store) -> Result<Connection> {
    let conn = Connection::open(store.p2_db_path())?;
    apply_pragmas(&conn)?;
    Ok(conn)
}

/// The PRAGMA set every store database gets: `journal_mode=WAL` (one
/// writer, many readers), `busy_timeout=5000` (wait out lock contention
/// instead of failing), `synchronous=NORMAL` (WAL-safe durability),
/// and `foreign_keys=ON` (deletes that would orphan a child row fail).
fn apply_pragmas(conn: &Connection) -> Result<()> {
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "busy_timeout", 5000)?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    Ok(())
}

/// Apply pending migrations. Idempotent.
///
/// Fresh (`user_version = 0`) databases get the v11 baseline
/// snapshot in one transaction, so a crash or error can never
/// leave a half-applied schema behind. Databases already at
/// [`SCHEMA_VERSION`] are untouched. Anything in between (v1–v10)
/// is refused loudly: the incremental path was squashed away, so
/// those databases must first open under the last incremental
/// release (≤ dc400a9) or be restored from a v11 backup — silently
/// stamping them current would corrupt the version contract.
pub fn migrate(conn: &Connection) -> Result<()> {
    in_transaction(conn, || {
        let version: i32 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        if version == 0 {
            conn.execute_batch(include_str!("../migrations/schema_baseline.sql"))?;
            conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;
        } else if version != SCHEMA_VERSION {
            return Err(version_gap("handbook.sqlite", version, SCHEMA_VERSION));
        }
        Ok(())
    })?;
    debug_assert_eq!(
        conn.query_row("PRAGMA user_version", [], |row| row.get::<_, i32>(0))?,
        SCHEMA_VERSION
    );
    Ok(())
}

/// Apply pending P2 migrations to a [`open_p2`] connection. Idempotent.
///
/// Same baseline discipline as [`migrate`]: fresh databases get the
/// P2 v2 snapshot; v1 databases are refused loudly (upgrade through
/// the last incremental release first); versioned by the p2.sqlite
/// `user_version`, independently of [`SCHEMA_VERSION`].
pub fn migrate_p2(conn: &Connection) -> Result<()> {
    in_transaction(conn, || {
        let version: i32 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        if version == 0 {
            conn.execute_batch(include_str!("../migrations/p2_schema_baseline.sql"))?;
            conn.pragma_update(None, "user_version", P2_SCHEMA_VERSION)?;
        } else if version != P2_SCHEMA_VERSION {
            return Err(version_gap("p2.sqlite", version, P2_SCHEMA_VERSION));
        }
        Ok(())
    })?;
    debug_assert_eq!(
        conn.query_row("PRAGMA user_version", [], |row| row.get::<_, i32>(0))?,
        P2_SCHEMA_VERSION
    );
    Ok(())
}

/// Loud refusal for databases stranded between the old incremental
/// versions and the squashed baseline (or newer than this binary).
/// Ad-hoc error via the same `ToSqlConversionFailure` carrier db.rs
/// already uses for non-SQLite failures.
fn version_gap(db: &str, version: i32, current: i32) -> HandbookError {
    let remedy = if version < current {
        format!(
            "upgrade it through the last incremental release first \
             (≤ dc400a9 applies v{version}→v{current} step by step), \
             or restore from a v{current} backup"
        )
    } else {
        format!("it was written by a newer binary (v{version} > v{current}); upgrade this binary")
    };
    HandbookError::Db(rusqlite::Error::ToSqlConversionFailure(Box::new(
        std::io::Error::other(format!(
            "{db} is at schema v{version}, this binary baselines v{current}: {remedy}"
        )),
    )))
}

/// Run `f` inside one `BEGIN IMMEDIATE`/`COMMIT` transaction, rolling
/// back when `f` fails. Takes a shared `&Connection` (rusqlite uses
/// interior mutability), so callers with a shared handle still get
/// atomic multi-statement updates.
pub fn in_transaction<T>(conn: &Connection, f: impl FnOnce() -> Result<T>) -> Result<T> {
    conn.execute_batch("BEGIN IMMEDIATE")?;
    match f() {
        Ok(value) => {
            conn.execute_batch("COMMIT")?;
            Ok(value)
        }
        Err(e) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(e)
        }
    }
}

/// New-authority fields plus audit attribution.
pub struct NewAuthority<'a> {
    pub kind: &'a str,
    pub country: &'a str,
    pub region: &'a str,
    pub name: &'a str,
    pub slug: &'a str,
    pub actor: &'a str,
    pub at: i64,
}

/// Insert an authority (office / jurisdiction). Returns the new row id.
/// The authority row and its audit row commit atomically (B1).
pub fn add_authority(conn: &Connection, a: &NewAuthority<'_>) -> Result<i64> {
    in_transaction(conn, || {
        conn.execute(
            "INSERT INTO authority(kind, country, region, name, slug)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            (a.kind, a.country, a.region, a.name, a.slug),
        )?;
        let id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO audit(at, actor, action, entity, entity_id, reason)
             VALUES (?1, ?2, 'authority_add', 'authority', ?3, ?4)",
            rusqlite::params![a.at, a.actor, id, a.slug],
        )?;
        Ok(id)
    })
}

/// Delete voice-audio captures older than `VOICE_RETENTION_DAYS`.
///
/// For each expired `call_session.audio_capture_id`: removes the blob file
/// and its capture row, nulls the session link, and writes an `audit` row.
/// Transcripts, annotations, and consent records are never touched.
/// All database deletes run in one transaction. A stored path that does
/// not resolve inside the store root is skipped entirely (neither file
/// nor row is touched). Returns the number of audio captures deleted.
/// `now` is unix seconds (a parameter so tests don't depend on the clock).
pub fn retention_sweep(conn: &Connection, store: &Store, now: i64, actor: &str) -> Result<usize> {
    let cutoff = now.saturating_sub(VOICE_RETENTION_DAYS * 86_400);
    let expired: Vec<(i64, String)> = conn
        .prepare(
            "SELECT c.id, c.path FROM capture c
             JOIN call_session s ON s.audio_capture_id = c.id
             WHERE c.kind = 'audio' AND c.captured_at < ?1",
        )?
        .query_map([cutoff], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    // Canonicalize once: symlinked parents (e.g. macOS `/var` ->
    // `/private/var`) must compare against the resolved root.
    let canonical_root = store
        .root()
        .canonicalize()
        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;

    in_transaction(conn, || {
        let mut deleted = 0;
        for (id, path) in &expired {
            let full = store.root().join(path);
            // Traversal guard: the stored path is untrusted input. When
            // it does not resolve inside the store root, skip BOTH the
            // file and the row (fail closed; counted as skipped, not
            // deleted), so a malicious row can never delete outside.
            if !parent_inside_root(&canonical_root, &full) {
                continue;
            }
            // The row is the record; a missing file is already swept enough.
            if full.exists() {
                std::fs::remove_file(&full)
                    .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
            }
            conn.execute("DELETE FROM capture WHERE id = ?1", [id])?;
            conn.execute(
                "UPDATE call_session SET audio_capture_id = NULL
                 WHERE audio_capture_id = ?1",
                [id],
            )?;
            conn.execute(
                "INSERT INTO audit(at, actor, action, entity, entity_id, reason)
                 VALUES (?1, ?2, 'retention_delete', 'capture', ?3,
                         'voice audio older than 365 days')",
                rusqlite::params![now, actor, id],
            )?;
            deleted += 1;
        }
        Ok(deleted)
    })
}

/// True when `candidate`'s parent directory resolves (via the
/// filesystem) to a location inside `canonical_root`. Fail-closed: any
/// canonicalization failure reports "not inside".
fn parent_inside_root(canonical_root: &std::path::Path, candidate: &std::path::Path) -> bool {
    candidate
        .parent()
        .unwrap_or(candidate)
        .canonicalize()
        .is_ok_and(|parent| parent.starts_with(canonical_root))
}

/// Insert or replace the per-authority analysis watermark row (S2 lane).
pub fn upsert_watermark(
    conn: &Connection,
    slug: &str,
    at: i64,
    prompt_version: &str,
) -> Result<()> {
    conn.execute(
        "INSERT INTO watermark(authority_slug, last_run_at, last_prompt_version)
         VALUES (?1, ?2, ?3)
         ON CONFLICT(authority_slug) DO UPDATE SET
           last_run_at = excluded.last_run_at,
           last_prompt_version = excluded.last_prompt_version",
        rusqlite::params![slug, at, prompt_version],
    )?;
    Ok(())
}

/// Read back `(last_run_at, last_prompt_version)` for one authority, if any.
pub fn get_watermark(conn: &Connection, slug: &str) -> Result<Option<(i64, String)>> {
    match conn.query_row(
        "SELECT last_run_at, last_prompt_version FROM watermark
         WHERE authority_slug = ?1",
        [slug],
        |row| Ok((row.get(0)?, row.get(1)?)),
    ) {
        Ok(found) => Ok(Some(found)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// Highest analyzed capture id for one authority (0 = none yet).
/// The Option-B scan floor: captures at or below this id are done.
pub fn get_capture_watermark(conn: &Connection, slug: &str) -> Result<i64> {
    match conn.query_row(
        "SELECT last_capture_id FROM watermark WHERE authority_slug = ?1",
        [slug],
        |row| row.get(0),
    ) {
        Ok(id) => Ok(id),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(0),
        Err(e) => Err(e.into()),
    }
}

/// One row for [`insert_proposed_fact`].
pub struct NewProposedFact<'a> {
    pub authority_id: i64,
    pub capture_id: i64,
    pub topic: &'a str,
    pub statement: &'a str,
    pub created_by: &'a str,
    pub created_at: i64,
    /// Validated citation offsets into the capture sidecar, or `None`
    /// when uncited (stored as -1/-1).
    pub citation: Option<(u32, u32)>,
    /// Quarantined model summary (`fact.reasoning`; '' when absent).
    pub reasoning: &'a str,
}

/// Insert ONE `fact` row with `status='proposed'`. Returns the new row id.
///
/// Column mapping: `authority_id`, `capture_id`, `topic`, `statement`,
/// `created_by`, `created_at` pass through; `status` is always
/// `'proposed'`. The caller folds `field` into `statement`; citation
/// offsets index the fact's own capture sidecar (`None` → -1/-1).
pub fn insert_proposed_fact(conn: &Connection, f: &NewProposedFact<'_>) -> Result<i64> {
    let (start, end) = match f.citation {
        Some((s, e)) => (i64::from(s), i64::from(e)),
        None => (-1, -1),
    };
    conn.execute(
        "INSERT INTO fact(authority_id, capture_id, topic, statement,
                           status, created_by, created_at,
                           citation_start, citation_end, reasoning)
         VALUES (?1, ?2, ?3, ?4, 'proposed', ?5, ?6, ?7, ?8, ?9)",
        rusqlite::params![
            f.authority_id,
            f.capture_id,
            f.topic,
            f.statement,
            f.created_by,
            f.created_at,
            start,
            end,
            f.reasoning,
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Normalize a statement for suppression matching (§11.3): trim,
/// collapse internal whitespace, lowercase. One shared helper, used
/// identically by the analyzer, reject-with-suppress, and standalone
/// rule create.
pub fn normalize_statement(s: &str) -> String {
    s.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// A suppression-check hit: path A (rejected-memory) or B (rule id).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuppressionHit {
    RejectedMemory,
    Rule(i64),
}

/// Check one candidate against suppression (§11.3): path A
/// (normalized match on a `rejected` fact, same authority+topic),
/// then path B (live `suppress_rule`). Pure reads.
pub fn check_suppression(
    conn: &Connection,
    authority_id: i64,
    topic: &str,
    statement_norm: &str,
) -> Result<Option<SuppressionHit>> {
    let mut stmt = conn.prepare(
        "SELECT statement FROM fact
         WHERE authority_id = ?1 AND topic = ?2 AND status = 'rejected'",
    )?;
    let rows = stmt.query_map(rusqlite::params![authority_id, topic], |row| {
        row.get::<_, String>(0)
    })?;
    for statement in rows {
        if normalize_statement(&statement?) == statement_norm {
            return Ok(Some(SuppressionHit::RejectedMemory));
        }
    }
    let rule: Option<i64> = rusqlite::OptionalExtension::optional(conn.query_row(
        "SELECT id FROM suppress_rule
         WHERE revoked_at IS NULL AND statement_norm = ?1
           AND (authority_id IS NULL OR authority_id = ?2)
           AND (topic IS NULL OR topic = ?3)
         ORDER BY id LIMIT 1",
        rusqlite::params![statement_norm, authority_id, topic],
        |row| row.get(0),
    ))?;
    Ok(rule.map(SuppressionHit::Rule))
}

/// Log one analyzer skip (§11.3). Returns the new `suppression` row id.
#[allow(clippy::too_many_arguments)]
pub fn insert_suppression(
    conn: &Connection,
    capture_id: i64,
    authority_id: i64,
    topic: &str,
    statement: &str,
    statement_norm: &str,
    hit: SuppressionHit,
    created_at: i64,
) -> Result<i64> {
    let (path, rule_id) = match hit {
        SuppressionHit::RejectedMemory => ("A", None),
        SuppressionHit::Rule(id) => ("B", Some(id)),
    };
    conn.execute(
        "INSERT INTO suppression(capture_id, authority_id, topic, statement,
                                  statement_norm, rule_id, path, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        rusqlite::params![
            capture_id,
            authority_id,
            topic,
            statement,
            statement_norm,
            rule_id,
            path,
            created_at,
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Append one fully-specified `audit` row. Returns the new row id.
#[allow(clippy::too_many_arguments)]
pub fn insert_audit(
    conn: &Connection,
    at: i64,
    actor: &str,
    action: &str,
    entity: &str,
    entity_id: i64,
    before_ref: &str,
    after_ref: &str,
    reason: &str,
) -> Result<i64> {
    conn.execute(
        "INSERT INTO audit(at, actor, action, entity, entity_id,
                           before_ref, after_ref, reason)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        rusqlite::params![
            at, actor, action, entity, entity_id, before_ref, after_ref, reason
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Append one `action='analysis_run'` audit row for a finished analysis run.
/// `authority_id` is the run's `entity_id` (`entity='authority'`); `reason`
/// carries the proposed/rejected counts plus token usage.
pub fn insert_analysis_audit(
    conn: &Connection,
    at: i64,
    actor: &str,
    authority_id: i64,
    reason: &str,
) -> Result<i64> {
    conn.execute(
        "INSERT INTO audit(at, actor, action, entity, entity_id, reason)
         VALUES (?1, ?2, 'analysis_run', 'authority', ?3, ?4)",
        rusqlite::params![at, actor, authority_id, reason],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Insert a DEMO session row (migration 003). Demo-only: no password is
/// checked anywhere; real auth (passkey/OAuth) replaces this table.
pub fn create_demo_session(
    conn: &Connection,
    token: &str,
    customer_ref: &str,
    created_at: i64,
    expires_at: i64,
) -> Result<()> {
    conn.execute(
        "INSERT INTO demo_session(token, customer_ref, created_at, expires_at)
         VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params![token, customer_ref, created_at, expires_at],
    )?;
    Ok(())
}

/// Look up a DEMO session by token. Returns `(customer_ref, expires_at)`;
/// callers must reject expired rows (`now >= expires_at`).
pub fn get_demo_session(conn: &Connection, token: &str) -> Result<Option<(String, i64)>> {
    // Fully-qualified trait call: avoids touching the import block.
    // `?` lifts the rusqlite error into `contracts::Result`.
    Ok(rusqlite::OptionalExtension::optional(conn.query_row(
        "SELECT customer_ref, expires_at FROM demo_session WHERE token = ?1",
        [token],
        |row| Ok((row.get(0)?, row.get(1)?)),
    ))?)
}

/// Delete DEMO sessions with `expires_at <= now`. Returns rows deleted.
pub fn delete_expired_demo_sessions(conn: &Connection, now: i64) -> Result<usize> {
    let deleted = conn.execute("DELETE FROM demo_session WHERE expires_at <= ?1", [now])?;
    Ok(deleted)
}

/// Saved per-customer label choices (P2 `profile_label_defaults`).
/// Spellings match `labels::{PrintFormat,PaperSize}::as_str`; readers
/// must treat unknown values as "use the default", never as an error.
pub struct LabelDefaults {
    pub option_a_format: String,
    pub option_b_paper: String,
    pub updated_at: i64,
}

/// Read saved label choices, if any.
pub fn get_label_defaults(conn: &Connection, customer_ref: &str) -> Result<Option<LabelDefaults>> {
    Ok(rusqlite::OptionalExtension::optional(conn.query_row(
        "SELECT option_a_format, option_b_paper, updated_at
         FROM profile_label_defaults WHERE customer_ref = ?1",
        [customer_ref],
        |row| {
            Ok(LabelDefaults {
                option_a_format: row.get(0)?,
                option_b_paper: row.get(1)?,
                updated_at: row.get(2)?,
            })
        },
    ))?)
}

/// Insert or replace saved label choices.
pub fn upsert_label_defaults(
    conn: &Connection,
    customer_ref: &str,
    option_a_format: &str,
    option_b_paper: &str,
    now: i64,
) -> Result<()> {
    conn.execute(
        "INSERT INTO profile_label_defaults(customer_ref, option_a_format, option_b_paper, updated_at)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(customer_ref) DO UPDATE SET
           option_a_format = excluded.option_a_format,
           option_b_paper = excluded.option_b_paper,
           updated_at = excluded.updated_at",
        rusqlite::params![customer_ref, option_a_format, option_b_paper, now],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn migrated() -> (tempfile::TempDir, Store, Connection) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path());
        store.init().unwrap();
        let conn = open(&store).unwrap();
        migrate(&conn).unwrap();
        (dir, store, conn)
    }

    #[test]
    fn baseline_has_source_fetch_state() {
        use rusqlite::OptionalExtension as _;
        let (_dir, _store, conn) = migrated();
        conn.execute(
            "INSERT INTO source(authority_id, kind, uri)
             VALUES (0, 'website_official', 'https://example.com/')",
            [],
        )
        .unwrap();
        let (cadence, etag, last_modified, content_hash, simhash, checked): (
            i64,
            String,
            String,
            String,
            i64,
            i64,
        ) = conn
            .query_row(
                "SELECT poll_cadence_sec, etag, last_modified, content_hash,
                        simhash, last_checked_at FROM source",
                [],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!((cadence, simhash, checked), (86400, 0, 0));
        assert!(etag.is_empty() && last_modified.is_empty() && content_hash.is_empty());
        let index: Option<String> = conn
            .query_row(
                "SELECT name FROM sqlite_master WHERE type = 'index' AND name = 'idx_source_due'",
                [],
                |row| row.get(0),
            )
            .optional()
            .unwrap();
        assert_eq!(index.as_deref(), Some("idx_source_due"));
    }

    #[test]
    fn baseline_has_mail_poll_cursor() {
        let (_dir, _store, conn) = migrated();
        conn.execute("INSERT INTO mail_poll(id) VALUES (1)", [])
            .unwrap();
        let (email, history_id, polled, error): (String, String, i64, String) = conn
            .query_row(
                "SELECT email, history_id, last_poll_at, last_error FROM mail_poll WHERE id = 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert!(email.is_empty() && history_id.is_empty() && error.is_empty());
        assert_eq!(polled, 0);
        // Singleton: a second row is refused.
        assert!(
            conn.execute("INSERT INTO mail_poll(id) VALUES (2)", [])
                .is_err()
        );
    }

    #[test]
    fn baseline_has_suppression_and_citation_home() {
        let (_dir, _store, conn) = migrated();
        for table in ["suppress_rule", "suppression"] {
            let found: String = conn
                .query_row(
                    "SELECT name FROM sqlite_master WHERE type = 'table' AND name = ?1",
                    [table],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(found, table);
        }
        let cols: Vec<String> = conn
            .prepare("PRAGMA table_info(fact)")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        for col in ["citation_start", "citation_end", "reasoning"] {
            assert!(cols.contains(&col.to_string()), "fact.{col} missing");
        }
        // Insert stores offsets + reasoning; None citation → -1/-1.
        let id = insert_proposed_fact(
            &conn,
            &NewProposedFact {
                authority_id: 0,
                capture_id: 0,
                topic: "address",
                statement: "po_box: PO Box 1",
                created_by: "test",
                created_at: 0,
                citation: Some((3, 11)),
                reasoning: "why",
            },
        )
        .unwrap();
        let (start, end, reasoning): (i64, i64, String) = conn
            .query_row(
                "SELECT citation_start, citation_end, reasoning FROM fact WHERE id = ?1",
                [id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!((start, end), (3, 11));
        assert_eq!(reasoning, "why");
        let bare = insert_proposed_fact(
            &conn,
            &NewProposedFact {
                authority_id: 0,
                capture_id: 0,
                topic: "address",
                statement: "s",
                created_by: "test",
                created_at: 0,
                citation: None,
                reasoning: "",
            },
        )
        .unwrap();
        let (start, end): (i64, i64) = conn
            .query_row(
                "SELECT citation_start, citation_end FROM fact WHERE id = ?1",
                [bare],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!((start, end), (-1, -1));
    }

    #[test]
    fn suppression_check_paths_and_scopes() {
        let (_dir, _store, conn) = migrated();
        let auth = add_authority(
            &conn,
            &NewAuthority {
                kind: "office",
                country: "Test",
                region: "",
                name: "T",
                slug: "t-sos",
                actor: "test",
                at: 0,
            },
        )
        .unwrap();
        // Path A: exact-norm match on a rejected row, same topic.
        conn.execute(
            "INSERT INTO fact(authority_id, capture_id, topic, statement, status, created_at)
             VALUES (?1, 0, 'address', 'PO Box  1', 'rejected', 0)",
            [auth],
        )
        .unwrap();
        assert_eq!(
            check_suppression(&conn, auth, "address", "po box 1").unwrap(),
            Some(SuppressionHit::RejectedMemory)
        );
        // Other topic: no hit.
        assert_eq!(
            check_suppression(&conn, auth, "hours", "po box 1").unwrap(),
            None
        );
        // Path B: global rule, any topic.
        conn.execute(
            "INSERT INTO suppress_rule(statement_norm, statement, created_at)
             VALUES ('fee $9', 'Fee $9', 0)",
            [],
        )
        .unwrap();
        assert_eq!(
            check_suppression(&conn, auth, "fees", "fee $9").unwrap(),
            Some(SuppressionHit::Rule(1))
        );
        // Revoked rules never match.
        conn.execute("UPDATE suppress_rule SET revoked_at = 1 WHERE id = 1", [])
            .unwrap();
        assert_eq!(
            check_suppression(&conn, auth, "fees", "fee $9").unwrap(),
            None
        );
        // Authority-scoped rule does not leak across authorities.
        conn.execute(
            "INSERT INTO suppress_rule(authority_id, topic, statement_norm, statement, created_at)
             VALUES (?1, 'fees', 'fee $8', 'Fee $8', 0)",
            [auth],
        )
        .unwrap();
        assert_eq!(
            check_suppression(&conn, auth + 999, "fees", "fee $8").unwrap(),
            None
        );
        assert!(matches!(
            check_suppression(&conn, auth, "fees", "fee $8").unwrap(),
            Some(SuppressionHit::Rule(_))
        ));
    }

    #[test]
    fn normalize_statement_folds_whitespace_and_case() {
        assert_eq!(normalize_statement("  PO   Box 40220. "), "po box 40220.");
        assert_eq!(normalize_statement("Fee\t$8\nMonday"), "fee $8 monday");
    }

    #[test]
    fn migrate_is_idempotent() {
        let (_dir, _store, conn) = migrated();
        migrate(&conn).unwrap();
        let version: i32 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, SCHEMA_VERSION);
    }

    #[test]
    fn add_authority_roundtrip() {
        let (_dir, _store, conn) = migrated();
        let id = add_authority(
            &conn,
            &NewAuthority {
                kind: "state",
                country: "USA",
                region: "Washington",
                name: "WA Secretary of State",
                slug: "usa-wa-sos",
                actor: "test",
                at: 1_800_000_000,
            },
        )
        .unwrap();
        let slug: String = conn
            .query_row("SELECT slug FROM authority WHERE id = ?1", [id], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(slug, "usa-wa-sos");
    }

    #[test]
    fn task_dedup_key_rejects_live_duplicates() {
        let (_dir, _store, conn) = migrated();
        let authority = add_authority(
            &conn,
            &NewAuthority {
                kind: "state",
                country: "USA",
                region: "WA",
                name: "WA",
                slug: "usa-wa",
                actor: "test",
                at: 1_800_000_000,
            },
        )
        .unwrap();
        let insert = || {
            conn.execute(
                "INSERT INTO task(authority_id, kind, title, status, dedup_key, created_at)
                 VALUES (?1, 'validate_fact', 't', 'open', 'same-key', 0)",
                [authority],
            )
        };
        insert().unwrap();
        insert().unwrap_err();
        // Exactly one live row with this key exists.
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM task WHERE dedup_key = 'same-key'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn retention_sweep_deletes_only_expired_audio() {
        let (_dir, store, conn) = migrated();
        let authority = add_authority(
            &conn,
            &NewAuthority {
                kind: "state",
                country: "USA",
                region: "WA",
                name: "WA",
                slug: "usa-wa",
                actor: "test",
                at: 1_800_000_000,
            },
        )
        .unwrap();
        conn.execute(
            "INSERT INTO channel(authority_id, visibility, kind, value, purpose)
             VALUES (?1, 'private', 'phone', '+1', 'our_validation')",
            [authority],
        )
        .unwrap();
        let channel = conn.last_insert_rowid();

        let now = 1_800_000_000;
        let old = now - 366 * 86_400;
        // Old audio (expired), old transcript (kept), fresh audio (kept).
        let audio_old = store.write_blob(b"old audio").unwrap();
        let audio_new = store.write_blob(b"new audio").unwrap();
        let transcript = store.write_blob(b"old transcript").unwrap();
        for (kind, sha, at) in [
            ("audio", audio_old.as_str(), old),
            ("transcript", transcript.as_str(), old),
            ("audio", audio_new.as_str(), now),
        ] {
            conn.execute(
                "INSERT INTO capture(authority_id, captured_at, kind, path,
                                     bytes_raw, bytes_stored, sha256, trust)
                 VALUES (?1, ?2, ?3, ?4, 0, 0, ?5, 'staff_verified')",
                rusqlite::params![
                    authority,
                    at,
                    kind,
                    store
                        .blob_path(sha)
                        .unwrap()
                        .strip_prefix(store.root())
                        .unwrap()
                        .to_str()
                        .unwrap(),
                    sha,
                ],
            )
            .unwrap();
        }
        let (audio_id, transcript_id, fresh_id): (i64, i64, i64) = conn
            .query_row(
                "SELECT
                   MAX(CASE WHEN kind = 'audio' AND captured_at = ?1 THEN id END),
                   MAX(CASE WHEN kind = 'transcript' THEN id END),
                   MAX(CASE WHEN kind = 'audio' AND captured_at = ?2 THEN id END)
                 FROM capture",
                rusqlite::params![old, now],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        conn.execute(
            "INSERT INTO call_session(authority_id, channel_id, consent,
                                      audio_capture_id, transcript_capture_id)
             VALUES (?1, ?2, 'granted', ?3, ?4)",
            rusqlite::params![authority, channel, audio_id, transcript_id],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO call_session(authority_id, channel_id, consent,
                                      audio_capture_id)
             VALUES (?1, ?2, 'granted', ?3)",
            rusqlite::params![authority, channel, fresh_id],
        )
        .unwrap();

        let deleted = retention_sweep(&conn, &store, now, "test").unwrap();
        assert_eq!(deleted, 1);

        // Expired audio: row gone, file gone, session link nulled.
        let audio_rows: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM capture WHERE id = ?1",
                [audio_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(audio_rows, 0);
        assert!(!store.blob_path(&audio_old).unwrap().exists());
        let link: Option<i64> = conn
            .query_row(
                "SELECT audio_capture_id FROM call_session
                 WHERE transcript_capture_id = ?1",
                [transcript_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(link, None);

        // Transcript and fresh audio untouched.
        for id in [transcript_id, fresh_id] {
            let rows: i64 = conn
                .query_row("SELECT COUNT(*) FROM capture WHERE id = ?1", [id], |row| {
                    row.get(0)
                })
                .unwrap();
            assert_eq!(rows, 1);
        }
        assert!(store.blob_path(&audio_new).unwrap().exists());

        // One audit row explains the deletion.
        let audits: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM audit
                 WHERE action = 'retention_delete' AND entity_id = ?1",
                [audio_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(audits, 1);
    }

    fn audit_trigger_count(conn: &Connection) -> i64 {
        conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master
             WHERE type = 'trigger' AND tbl_name = 'audit'",
            [],
            |row| row.get(0),
        )
        .unwrap()
    }

    #[test]
    fn open_sets_pragmas() {
        let (_dir, _store, conn) = migrated();
        let journal: String = conn
            .query_row("PRAGMA journal_mode", [], |row| row.get(0))
            .unwrap();
        assert_eq!(journal.to_lowercase(), "wal");
        let busy: i32 = conn
            .query_row("PRAGMA busy_timeout", [], |row| row.get(0))
            .unwrap();
        assert_eq!(busy, 5000);
        let synchronous: i32 = conn
            .query_row("PRAGMA synchronous", [], |row| row.get(0))
            .unwrap();
        assert_eq!(synchronous, 1); // NORMAL
        let foreign_keys: i32 = conn
            .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
            .unwrap();
        assert_eq!(foreign_keys, 1); // ON
    }

    #[test]
    fn migrate_fresh_db_applies_baseline() {
        let (_dir, _store, conn) = migrated();
        let version: i32 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, SCHEMA_VERSION);
        assert_eq!(audit_trigger_count(&conn), 2);
    }

    #[test]
    fn migrate_refuses_stale_v1_db() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path());
        store.init().unwrap();
        let conn = open(&store).unwrap();
        // A stranded v1 database: the version stamp alone triggers the
        // refusal — the old incremental path is gone.
        conn.execute("CREATE TABLE probe(id INTEGER PRIMARY KEY)", [])
            .unwrap();
        conn.execute("INSERT INTO probe(id) VALUES (1)", [])
            .unwrap();
        conn.pragma_update(None, "user_version", 1).unwrap();

        let err = migrate(&conn).unwrap_err().to_string();
        assert!(err.contains("schema v1"), "unexpected error: {err}");
        assert!(err.contains("incremental"), "unexpected error: {err}");

        // Refusal touches nothing: version and rows intact.
        let version: i32 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, 1);
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM probe", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn audit_is_append_only() {
        let (_dir, _store, conn) = migrated();
        conn.execute(
            "INSERT INTO audit(at, actor, action, entity, entity_id)
             VALUES (1, 'test', 'accept', 'fact', 7)",
            [],
        )
        .unwrap();
        conn.execute("UPDATE audit SET actor = 'evil' WHERE id = 1", [])
            .unwrap_err();
        conn.execute("DELETE FROM audit WHERE id = 1", [])
            .unwrap_err();
        // INSERT still works and the row is intact.
        conn.execute(
            "INSERT INTO audit(at, actor, action, entity, entity_id)
             VALUES (2, 'test', 'reject', 'fact', 8)",
            [],
        )
        .unwrap();
        let (rows, actor): (i64, String) = conn
            .query_row(
                "SELECT COUNT(*), MAX(CASE WHEN id = 1 THEN actor END) FROM audit",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!((rows, actor.as_str()), (2, "test"));
    }

    #[test]
    fn retention_sweep_skips_paths_outside_root() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().join("store"));
        store.init().unwrap();
        let conn = open(&store).unwrap();
        migrate(&conn).unwrap();

        // A sentinel file OUTSIDE the store root that a malicious
        // capture row tries to aim the sweep at.
        let sentinel = dir.path().join("sentinel.txt");
        std::fs::write(&sentinel, b"do not touch").unwrap();

        let authority = add_authority(
            &conn,
            &NewAuthority {
                kind: "state",
                country: "USA",
                region: "WA",
                name: "WA",
                slug: "usa-wa",
                actor: "test",
                at: 1_800_000_000,
            },
        )
        .unwrap();
        conn.execute(
            "INSERT INTO channel(authority_id, visibility, kind, value, purpose)
             VALUES (?1, 'private', 'phone', '+1', 'our_validation')",
            [authority],
        )
        .unwrap();
        let channel = conn.last_insert_rowid();

        let now = 1_800_000_000;
        let old = now - 366 * 86_400;
        // One legitimate expired blob plus one malicious row.
        let legit = store.write_blob(b"legit audio").unwrap();
        let legit_rel = store
            .blob_path(&legit)
            .unwrap()
            .strip_prefix(store.root())
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        for path in [legit_rel.as_str(), "../sentinel.txt"] {
            conn.execute(
                "INSERT INTO capture(authority_id, captured_at, kind, path,
                                     bytes_raw, bytes_stored, sha256, trust)
                 VALUES (?1, ?2, 'audio', ?3, 0, 0, 'x', 'staff_verified')",
                rusqlite::params![authority, old, path],
            )
            .unwrap();
            let capture = conn.last_insert_rowid();
            conn.execute(
                "INSERT INTO call_session(authority_id, channel_id, consent,
                                          audio_capture_id)
                 VALUES (?1, ?2, 'granted', ?3)",
                rusqlite::params![authority, channel, capture],
            )
            .unwrap();
        }

        let deleted = retention_sweep(&conn, &store, now, "test").unwrap();
        assert_eq!(deleted, 1);

        // Nothing outside the root was touched: sentinel intact ...
        assert_eq!(std::fs::read(&sentinel).unwrap(), b"do not touch");
        // ... and the malicious row survives (file AND row skipped).
        let evil_rows: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM capture WHERE path = '../sentinel.txt'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(evil_rows, 1);
        let evil_audits: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM audit WHERE action = 'retention_delete'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(evil_audits, 1);
        // The legitimate capture is fully swept.
        assert!(!store.blob_path(&legit).unwrap().exists());
    }

    #[test]
    fn retention_sweep_deletes_missing_file_row_skips_absolute_path() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().join("store"));
        store.init().unwrap();
        let conn = open(&store).unwrap();
        migrate(&conn).unwrap();

        let sentinel = dir.path().join("sentinel.txt");
        std::fs::write(&sentinel, b"do not touch").unwrap();

        let authority = add_authority(
            &conn,
            &NewAuthority {
                kind: "state",
                country: "USA",
                region: "WA",
                name: "WA",
                slug: "usa-wa",
                actor: "test",
                at: 1_800_000_000,
            },
        )
        .unwrap();
        conn.execute(
            "INSERT INTO channel(authority_id, visibility, kind, value, purpose)
             VALUES (?1, 'private', 'phone', '+1', 'our_validation')",
            [authority],
        )
        .unwrap();
        let channel = conn.last_insert_rowid();

        let now = 1_800_000_000;
        let old = now - 366 * 86_400;
        // A blob whose file vanished but whose parent dirs remain (the
        // guard needs a resolvable parent), plus an absolute path
        // outside the root (`Path::join` discards the root for
        // absolute joins — the guard must still catch it).
        let ghost = store.write_blob(b"ghost audio").unwrap();
        let ghost_full = store.blob_path(&ghost).unwrap();
        std::fs::remove_file(&ghost_full).unwrap();
        let ghost_rel = ghost_full
            .strip_prefix(store.root())
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        let absolute = sentinel.to_str().unwrap().to_string();
        for path in [ghost_rel.as_str(), absolute.as_str()] {
            conn.execute(
                "INSERT INTO capture(authority_id, captured_at, kind, path,
                                     bytes_raw, bytes_stored, sha256, trust)
                 VALUES (?1, ?2, 'audio', ?3, 0, 0, 'x', 'staff_verified')",
                rusqlite::params![authority, old, path],
            )
            .unwrap();
            let capture = conn.last_insert_rowid();
            conn.execute(
                "INSERT INTO call_session(authority_id, channel_id, consent,
                                          audio_capture_id)
                 VALUES (?1, ?2, 'granted', ?3)",
                rusqlite::params![authority, channel, capture],
            )
            .unwrap();
        }

        let deleted = retention_sweep(&conn, &store, now, "test").unwrap();
        assert_eq!(deleted, 1);

        // Missing file: row swept anyway ("already swept enough").
        let ghost_rows: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM capture WHERE path = ?1",
                [&ghost_rel],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(ghost_rows, 0);
        // Absolute path: skipped, sentinel intact.
        assert_eq!(std::fs::read(&sentinel).unwrap(), b"do not touch");
        let abs_rows: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM capture WHERE path = ?1",
                [&absolute],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(abs_rows, 1);
    }

    #[test]
    fn migrate_baseline_failure_rolls_back_cleanly() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path());
        store.init().unwrap();
        let conn = open(&store).unwrap();
        // A fresh (v0) database with a conflicting table name: the
        // baseline must fail, and the failure must roll back cleanly —
        // no partial schema, version still 0.
        conn.execute("CREATE TABLE authority(id INTEGER PRIMARY KEY)", [])
            .unwrap();

        migrate(&conn).unwrap_err();

        let version: i32 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, 0);
        let tables: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master
                 WHERE type = 'table' AND name != 'authority'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(tables, 0);
    }

    #[test]
    fn watermark_upsert_get_roundtrip() {
        let (_dir, _store, conn) = migrated();
        assert_eq!(get_watermark(&conn, "usa-wa").unwrap(), None);
        upsert_watermark(&conn, "usa-wa", 100, "aa-prompt-v1").unwrap();
        assert_eq!(
            get_watermark(&conn, "usa-wa").unwrap(),
            Some((100, "aa-prompt-v1".to_string()))
        );
        // Upsert replaces the row for the same authority.
        upsert_watermark(&conn, "usa-wa", 200, "aa-prompt-v2").unwrap();
        assert_eq!(
            get_watermark(&conn, "usa-wa").unwrap(),
            Some((200, "aa-prompt-v2".to_string()))
        );
        // Other authorities are independent rows.
        upsert_watermark(&conn, "usa-ca", 300, "aa-prompt-v1").unwrap();
        assert_eq!(
            get_watermark(&conn, "usa-ca").unwrap(),
            Some((300, "aa-prompt-v1".to_string()))
        );
    }

    #[test]
    fn demo_session_roundtrip() {
        let (_dir, _store, conn) = migrated();
        create_demo_session(&conn, "tok-1", "demo-1", 1000, 2000).unwrap();
        assert_eq!(
            get_demo_session(&conn, "tok-1").unwrap(),
            Some(("demo-1".to_string(), 2000))
        );
        assert_eq!(get_demo_session(&conn, "bogus").unwrap(), None);
    }

    #[test]
    fn demo_session_expiry_sweep() {
        let (_dir, _store, conn) = migrated();
        create_demo_session(&conn, "old", "demo-1", 0, 999).unwrap();
        create_demo_session(&conn, "edge", "demo-1", 0, 1000).unwrap();
        create_demo_session(&conn, "live", "demo-2", 0, 1001).unwrap();
        assert_eq!(delete_expired_demo_sessions(&conn, 1000).unwrap(), 2);
        assert_eq!(get_demo_session(&conn, "old").unwrap(), None);
        assert_eq!(get_demo_session(&conn, "edge").unwrap(), None);
        assert_eq!(
            get_demo_session(&conn, "live").unwrap(),
            Some(("demo-2".to_string(), 1001))
        );
    }

    #[test]
    fn migrate_refuses_stale_mid_version_db() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path());
        store.init().unwrap();
        let conn = open(&store).unwrap();
        // v8: the last prod schema before the squash. Same refusal as
        // v1 — any version below the baseline must upgrade through
        // the last incremental release first.
        conn.pragma_update(None, "user_version", 8).unwrap();

        let err = migrate(&conn).unwrap_err().to_string();
        assert!(err.contains("schema v8"), "unexpected error: {err}");
        assert!(err.contains("v8→v11"), "unexpected error: {err}");

        let version: i32 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, 8);
    }

    #[test]
    fn constraints_reject_bad_values_and_dangling_refs() {
        let (_dir, _store, conn) = migrated();
        let authority = add_authority(
            &conn,
            &NewAuthority {
                kind: "state",
                country: "USA",
                region: "WA",
                name: "WA",
                slug: "usa-wa",
                actor: "test",
                at: 1_800_000_000,
            },
        )
        .unwrap();
        // Bad CHECK value on a controlled vocabulary.
        conn.execute(
            "INSERT INTO capture(authority_id, captured_at, kind, path,
                                 bytes_raw, bytes_stored, sha256, trust)
             VALUES (?1, 0, 'file', 'x', 0, 0, 's', 'bogus')",
            [authority],
        )
        .expect_err("bad trust value must be rejected");
        conn.execute(
            "INSERT INTO capture(authority_id, captured_at, kind, path,
                                 bytes_raw, bytes_stored, sha256, trust)
             VALUES (?1, 0, 'file', 'x', 0, 0, 's', 'official')",
            [authority],
        )
        .unwrap();
        let capture = conn.last_insert_rowid();
        // Bad fact.topic (not one of the 25 TOPICS.md strings).
        conn.execute(
            "INSERT INTO fact(authority_id, capture_id, topic, statement,
                              status, created_at)
             VALUES (?1, ?2, 'bitcoin', 's', 'proposed', 0)",
            rusqlite::params![authority, capture],
        )
        .expect_err("bad fact.topic must be rejected");
        // Dangling foreign key.
        conn.execute(
            "INSERT INTO capture(authority_id, captured_at, kind, path,
                                 bytes_raw, bytes_stored, sha256, trust)
             VALUES (99999, 0, 'file', 'x', 0, 0, 's', 'official')",
            [],
        )
        .expect_err("dangling capture.authority_id must be rejected");
    }

    #[test]
    fn audit_stays_append_only_after_constraint_rebuild() {
        let (_dir, _store, conn) = migrated();
        conn.execute(
            "INSERT INTO audit(at, actor, action, entity, entity_id)
             VALUES (1, 'test', 'accept', 'fact', 1)",
            [],
        )
        .unwrap();
        conn.execute("UPDATE audit SET actor = 'evil' WHERE id = 1", [])
            .expect_err("audit UPDATE must fail after the 005 rebuild");
        assert_eq!(audit_trigger_count(&conn), 2);
    }

    #[test]
    fn config_tables_roundtrip_and_constraints() {
        let (_dir, _store, conn) = migrated();
        conn.execute(
            "INSERT INTO config(key, type, value_enc, enc_version, updated_at, updated_by)
             VALUES ('app.mode', 'text', x'0102', 1, 100, 'test')",
            [],
        )
        .unwrap();
        let (type_, enc_version): (String, i64) = conn
            .query_row(
                "SELECT type, enc_version FROM config WHERE key = 'app.mode'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!((type_.as_str(), enc_version), ("text", 1));
        // Unknown key type violates the CHECK.
        conn.execute(
            "INSERT INTO config(key, type, value_enc, enc_version, updated_at, updated_by)
             VALUES ('bad', 'bogus', x'00', 1, 100, 'test')",
            [],
        )
        .expect_err("bad config.type must be rejected");
        // STRICT table: TEXT is not storable in an INTEGER column.
        conn.execute(
            "INSERT INTO config(key, type, value_enc, enc_version, updated_at, updated_by)
             VALUES ('strict', 'text', x'00', 'one', 100, 'test')",
            [],
        )
        .expect_err("STRICT must reject TEXT in enc_version");
        conn.execute(
            "INSERT INTO config_kek(version, cred_path, created_at, retired_at)
             VALUES (1, '/etc/credstore.encrypted/handbook-kek-v1.cred', 100, NULL)",
            [],
        )
        .unwrap();
        let retired: Option<i64> = conn
            .query_row(
                "SELECT retired_at FROM config_kek WHERE version = 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(retired, None);
    }

    #[test]
    fn form_template_active_unique_and_constraints() {
        let (_dir, _store, conn) = migrated();
        let authority = add_authority(
            &conn,
            &NewAuthority {
                kind: "state",
                country: "USA",
                region: "WA",
                name: "WA",
                slug: "usa-wa",
                actor: "test",
                at: 1_800_000_000,
            },
        )
        .unwrap();
        let insert = |authority: i64, code: &str, status: &str| {
            conn.execute(
                "INSERT INTO form_template(authority_id, code, title, version,
                                           status, schema_json, created_at)
                 VALUES (?1, ?2, 'WA request', 1, ?3, '{}', 0)",
                rusqlite::params![authority, code, status],
            )
        };
        insert(authority, "wa-apostille-request", "active").unwrap();
        // Exactly one active template per (authority, code).
        insert(authority, "wa-apostille-request", "active")
            .expect_err("second active template must be rejected");
        // Non-active rows with the same code are fine (partial index).
        insert(authority, "wa-apostille-request", "superseded").unwrap();
        // Unknown status violates the CHECK.
        insert(authority, "wa-other", "bogus").expect_err("bad status must be rejected");
        // Dangling authority violates the FK.
        insert(99999, "wa-orphan", "draft").expect_err("dangling authority must be rejected");
        // Spec defaults landed on the first row.
        let (replica, drawn, mode, cadence): (String, String, String, i64) = conn
            .query_row(
                "SELECT accepts_replica, accepts_drawn_signature, signature_mode,
                        verify_cadence_days
                 FROM form_template WHERE code = 'wa-apostille-request' AND status = 'active'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(
            (replica.as_str(), drawn.as_str(), mode.as_str(), cadence),
            ("unknown", "no", "wet_ink", 90)
        );
    }

    #[test]
    fn authority_p2_columns_default_and_checked() {
        let (_dir, _store, conn) = migrated();
        let authority = add_authority(
            &conn,
            &NewAuthority {
                kind: "state",
                country: "USA",
                region: "WA",
                name: "WA",
                slug: "usa-wa",
                actor: "test",
                at: 1_800_000_000,
            },
        )
        .unwrap();
        let (replica_default, ship_to): (String, Option<i64>) = conn
            .query_row(
                "SELECT replica_default, ship_to_fact_id FROM authority WHERE id = ?1",
                [authority],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(replica_default, "unknown");
        assert_eq!(ship_to, None);
        conn.execute(
            "UPDATE authority SET replica_default = 'bogus' WHERE id = ?1",
            [authority],
        )
        .expect_err("bad replica_default must be rejected");
    }

    #[test]
    fn fact_topic_check_matches_frozen_enum() {
        let (_dir, store, conn) = migrated();
        let authority = add_authority(
            &conn,
            &NewAuthority {
                kind: "state",
                country: "USA",
                region: "WA",
                name: "WA",
                slug: "usa-wa",
                actor: "test",
                at: 1_800_000_000,
            },
        )
        .unwrap();
        let sha = store.write_blob(b"freeze proof").unwrap();
        conn.execute(
            "INSERT INTO capture(authority_id, captured_at, kind, path,
                                 bytes_raw, bytes_stored, sha256, trust)
             VALUES (?1, 0, 'file', 'x', 0, 0, ?2, 'official')",
            rusqlite::params![authority, sha],
        )
        .unwrap();
        let capture = conn.last_insert_rowid();
        // Every frozen topic string is storable; the CHECK and the enum
        // agree exactly (rejections are covered by contracts' parse test
        // plus constraints_reject_bad_values_and_dangling_refs).
        for topic in contracts::topics::Topic::ALL {
            conn.execute(
                "INSERT INTO fact(authority_id, capture_id, topic, statement,
                                  status, created_at)
                 VALUES (?1, ?2, ?3, 's', 'proposed', 0)",
                rusqlite::params![authority, capture, topic.as_str()],
            )
            .unwrap();
        }
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM fact", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, contracts::topics::Topic::ALL.len() as i64);
    }

    #[test]
    fn baseline_seeds_no_facts_or_fixtures() {
        let (_dir, _store, conn) = migrated();
        // The baseline inserts no facts: fixtures can only enter via
        // test helpers (crate::fixtures, cfg(test)) or real ingest.
        let facts: i64 = conn
            .query_row("SELECT COUNT(*) FROM fact", [], |row| row.get(0))
            .unwrap();
        assert_eq!(facts, 0);
        // The only seeded rows are the id-0 sentinels from migration 005.
        let slugs: Vec<String> = conn
            .prepare("SELECT slug FROM authority ORDER BY id")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        assert_eq!(slugs, ["sentinel-unattributed-005"]);
        // And no seeded text anywhere carries the canonical fixture values.
        let mut leaks = 0;
        for (table, column) in [
            ("authority", "name"),
            ("authority", "slug"),
            ("capture", "path"),
            ("capture", "sha256"),
            ("capture", "notes"),
            ("source", "uri"),
            ("source", "notes"),
            ("domain", "name"),
            ("domain", "notes"),
        ] {
            let hits: i64 = conn
                .query_row(
                    &format!(
                        "SELECT COUNT(*) FROM {table} \
                         WHERE {column} LIKE '%40220%' OR {column} LIKE '%40234%'"
                    ),
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            leaks += hits;
        }
        assert_eq!(leaks, 0, "fixture values leaked into seeded rows");
    }

    fn migrated_p2() -> (tempfile::TempDir, Store, Connection) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path());
        store.init().unwrap();
        let conn = open_p2(&store).unwrap();
        migrate_p2(&conn).unwrap();
        (dir, store, conn)
    }

    #[test]
    fn migrate_p2_is_idempotent() {
        let (_dir, _store, conn) = migrated_p2();
        migrate_p2(&conn).unwrap();
        let version: i32 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, P2_SCHEMA_VERSION);
    }

    #[test]
    fn open_p2_sets_pragmas() {
        let (_dir, _store, conn) = migrated_p2();
        let journal: String = conn
            .query_row("PRAGMA journal_mode", [], |row| row.get(0))
            .unwrap();
        assert_eq!(journal.to_lowercase(), "wal");
        let busy: i32 = conn
            .query_row("PRAGMA busy_timeout", [], |row| row.get(0))
            .unwrap();
        assert_eq!(busy, 5000);
        let foreign_keys: i32 = conn
            .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
            .unwrap();
        assert_eq!(foreign_keys, 1); // ON
    }

    #[test]
    fn p2_tables_roundtrip_and_constraints() {
        let (_dir, _store, conn) = migrated_p2();
        conn.execute(
            "INSERT INTO form_submission(form_template_id, authority_id, customer_ref,
                                        values_json, status, created_at)
             VALUES (7, 1, 'cust-1', '{\"fee\": 30}', 'submitted', 100)",
            [],
        )
        .unwrap();
        let submission = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO shipment(submission_id, authority_id, status, created_at, updated_at)
             VALUES (?1, 1, 'labels_pending', 100, 100)",
            [submission],
        )
        .unwrap();
        let shipment = conn.last_insert_rowid();
        for direction in ["outbound", "return"] {
            conn.execute(
                "INSERT INTO shipping_label(shipment_id, direction, carrier, service,
                                           tracking_no, status, created_at)
                 VALUES (?1, ?2, 'usps', 'PRIORITY', 'track-1', 'issued', 100)",
                rusqlite::params![shipment, direction],
            )
            .unwrap();
        }
        // The tracking index is non-unique per the spec sketch: duplicate
        // tracking numbers insert fine (reissued labels share numbers).
        let labels: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM shipping_label WHERE tracking_no = 'track-1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(labels, 2);
        let index: String = conn
            .query_row(
                "SELECT name FROM sqlite_master
                 WHERE type = 'index' AND name = 'idx_label_tracking'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(index, "idx_label_tracking");
        // Unknown direction violates the CHECK.
        conn.execute(
            "INSERT INTO shipping_label(shipment_id, direction, carrier, service, created_at)
             VALUES (?1, 'sideways', 'usps', 'PRIORITY', 100)",
            [shipment],
        )
        .expect_err("bad direction must be rejected");
        // Dangling in-file parent violates the FK (opaque cross-DB ids
        // like form_template_id/authorities have no FK by design).
        conn.execute(
            "INSERT INTO shipment(submission_id, authority_id, status, created_at, updated_at)
             VALUES (99999, 1, 'draft', 100, 100)",
            [],
        )
        .expect_err("dangling submission_id must be rejected");
    }

    #[test]
    fn migrate_p2_refuses_stale_v1_db() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path());
        store.init().unwrap();
        let conn = open_p2(&store).unwrap();
        // A stranded P2 v1 database: same loud refusal as the main
        // schema — upgrade through the last incremental release.
        conn.pragma_update(None, "user_version", 1).unwrap();

        let err = migrate_p2(&conn).unwrap_err().to_string();
        assert!(err.contains("p2.sqlite"), "unexpected error: {err}");
        assert!(err.contains("schema v1"), "unexpected error: {err}");

        let version: i32 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, 1);
    }

    #[test]
    fn p2_baseline_carries_label_purchase_shape() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path());
        store.init().unwrap();
        let conn = open_p2(&store).unwrap();
        migrate_p2(&conn).unwrap();
        conn.execute(
            "INSERT INTO form_submission(form_template_id, authority_id, customer_ref, status, created_at)
             VALUES (7, 1, 'cust-1', 'submitted', 100)",
            [],
        )
        .unwrap();
        let submission = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO shipment(submission_id, authority_id, status, created_at, updated_at)
             VALUES (?1, 1, 'ready', 100, 100)",
            [submission],
        )
        .unwrap();
        let shipment = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO shipping_label(shipment_id, direction, carrier, service,
                                       tracking_no, status, created_at)
             VALUES (?1, 'outbound', 'usps', 'PRIORITY', 'track-1', 'issued', 100)",
            [shipment],
        )
        .unwrap();
        // Fresh rows default to prepaid 4x6 PDFs (squashed p2_002 shape).
        let (option, format, sha, content): (String, String, String, String) = conn
            .query_row(
                "SELECT purchase_option, label_format, label_sha256, content_type
                 FROM shipping_label WHERE shipment_id = ?1",
                [shipment],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(option, "prepaid");
        assert_eq!(format, "four_by_six");
        assert_eq!(sha, "");
        assert_eq!(content, "application/pdf");
        conn.execute(
            "INSERT INTO shipping_label(shipment_id, direction, carrier, service, created_at,
                                       purchase_option)
             VALUES (?1, 'return', 'usps', 'PRIORITY', 100, 'telegraph')",
            [shipment],
        )
        .expect_err("bad purchase_option must be rejected");
    }

    #[test]
    fn label_defaults_roundtrip_per_customer() {
        let (_dir, _store, conn) = migrated_p2();
        assert!(get_label_defaults(&conn, "cust-1").unwrap().is_none());
        upsert_label_defaults(&conn, "cust-1", "window_envelope", "letter", 100).unwrap();
        let got = get_label_defaults(&conn, "cust-1").unwrap().unwrap();
        assert_eq!(got.option_a_format, "window_envelope");
        assert_eq!(got.option_b_paper, "letter");
        assert_eq!(got.updated_at, 100);
        upsert_label_defaults(&conn, "cust-1", "a4", "a4", 200).unwrap();
        let got = get_label_defaults(&conn, "cust-1").unwrap().unwrap();
        assert_eq!(got.option_a_format, "a4");
        assert_eq!(got.updated_at, 200);
        assert!(get_label_defaults(&conn, "cust-2").unwrap().is_none());
    }
}
