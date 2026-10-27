//! SQLite metadata database: migrations and foundation queries.
//!
//! Schema lives in `migrations/001_init.sql` (see `docs/design.md` §1).
//! Later migrations add `N+1` files and extend [`migrate`].

use rusqlite::Connection;

use crate::store::Store;

/// Current schema version (`pragma user_version`).
pub const SCHEMA_VERSION: i32 = 4;

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
pub fn open(store: &Store) -> rusqlite::Result<Connection> {
    let conn = Connection::open(store.db_path())?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "busy_timeout", 5000)?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    Ok(conn)
}

/// Apply pending migrations. Idempotent.
///
/// The whole upgrade runs inside one transaction, so a crash or error
/// can never leave a half-applied migration behind.
pub fn migrate(conn: &Connection) -> rusqlite::Result<()> {
    in_transaction(conn, || {
        let version: i32 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        if version < 1 {
            conn.execute_batch(include_str!("../migrations/001_init.sql"))?;
            conn.pragma_update(None, "user_version", 1)?;
        }
        if version < 2 {
            conn.execute_batch(include_str!("../migrations/002_hardening.sql"))?;
            conn.pragma_update(None, "user_version", 2)?;
        }
        if version < 3 {
            conn.execute_batch(include_str!("../migrations/003_sessions.sql"))?;
            conn.pragma_update(None, "user_version", 3)?;
        }
        if version < 4 {
            conn.execute_batch(include_str!("../migrations/004_watermarks.sql"))?;
            conn.pragma_update(None, "user_version", 4)?;
        }
        Ok(())
    })?;
    debug_assert_eq!(
        conn.query_row("PRAGMA user_version", [], |row| row.get::<_, i32>(0))?,
        SCHEMA_VERSION
    );
    Ok(())
}

/// Run `f` inside one `BEGIN IMMEDIATE`/`COMMIT` transaction, rolling
/// back when `f` fails. Takes a shared `&Connection` (rusqlite uses
/// interior mutability), so callers with a shared handle still get
/// atomic multi-statement updates.
fn in_transaction<T>(
    conn: &Connection,
    f: impl FnOnce() -> rusqlite::Result<T>,
) -> rusqlite::Result<T> {
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

/// Insert an authority (office / jurisdiction). Returns the new row id.
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
pub fn add_authority(conn: &Connection, a: &NewAuthority<'_>) -> rusqlite::Result<i64> {
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
pub fn retention_sweep(
    conn: &Connection,
    store: &Store,
    now: i64,
    actor: &str,
) -> rusqlite::Result<usize> {
    let cutoff = now.saturating_sub(VOICE_RETENTION_DAYS * 86_400);
    let expired: Vec<(i64, String)> = conn
        .prepare(
            "SELECT c.id, c.path FROM capture c
             JOIN call_session s ON s.audio_capture_id = c.id
             WHERE c.kind = 'audio' AND c.captured_at < ?1",
        )?
        .query_map([cutoff], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
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
) -> rusqlite::Result<()> {
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
pub fn get_watermark(conn: &Connection, slug: &str) -> rusqlite::Result<Option<(i64, String)>> {
    match conn.query_row(
        "SELECT last_run_at, last_prompt_version FROM watermark
         WHERE authority_slug = ?1",
        [slug],
        |row| Ok((row.get(0)?, row.get(1)?)),
    ) {
        Ok(found) => Ok(Some(found)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e),
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
}

/// Insert ONE `fact` row with `status='proposed'`. Returns the new row id.
///
/// Column mapping (`migrations/001_init.sql`): `authority_id`, `capture_id`,
/// `topic`, `statement`, `created_by`, `created_at` pass through; `status` is
/// always `'proposed'` (the existing status column fits, so no mapping
/// fallback was needed). The `fact` table has no `field`/`citation`/
/// `confidence` columns: the caller folds `field` into `statement`, and
/// citation byte ranges plus confidence are not stored.
pub fn insert_proposed_fact(conn: &Connection, f: &NewProposedFact<'_>) -> rusqlite::Result<i64> {
    conn.execute(
        "INSERT INTO fact(authority_id, capture_id, topic, statement,
                           status, created_by, created_at)
         VALUES (?1, ?2, ?3, ?4, 'proposed', ?5, ?6)",
        rusqlite::params![
            f.authority_id,
            f.capture_id,
            f.topic,
            f.statement,
            f.created_by,
            f.created_at
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
) -> rusqlite::Result<i64> {
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
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO demo_session(token, customer_ref, created_at, expires_at)
         VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params![token, customer_ref, created_at, expires_at],
    )?;
    Ok(())
}

/// Look up a DEMO session by token. Returns `(customer_ref, expires_at)`;
/// callers must reject expired rows (`now >= expires_at`).
pub fn get_demo_session(conn: &Connection, token: &str) -> rusqlite::Result<Option<(String, i64)>> {
    // Fully-qualified trait call: avoids touching the import block.
    rusqlite::OptionalExtension::optional(conn.query_row(
        "SELECT customer_ref, expires_at FROM demo_session WHERE token = ?1",
        [token],
        |row| Ok((row.get(0)?, row.get(1)?)),
    ))
}

/// Delete DEMO sessions with `expires_at <= now`. Returns rows deleted.
pub fn delete_expired_demo_sessions(conn: &Connection, now: i64) -> rusqlite::Result<usize> {
    let deleted = conn.execute("DELETE FROM demo_session WHERE expires_at <= ?1", [now])?;
    Ok(deleted)
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
    fn migrate_fresh_db_applies_all_migrations() {
        let (_dir, _store, conn) = migrated();
        let version: i32 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, SCHEMA_VERSION);
        assert_eq!(audit_trigger_count(&conn), 2);
    }

    #[test]
    fn migrate_upgrades_001_db_to_002() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path());
        store.init().unwrap();
        let conn = open(&store).unwrap();
        // Build a 001-only database, as an older install would have.
        conn.execute_batch(include_str!("../migrations/001_init.sql"))
            .unwrap();
        conn.pragma_update(None, "user_version", 1).unwrap();
        assert_eq!(audit_trigger_count(&conn), 0);

        migrate(&conn).unwrap();

        let version: i32 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, SCHEMA_VERSION);
        assert_eq!(audit_trigger_count(&conn), 2);
        // Idempotent on the upgraded database too.
        migrate(&conn).unwrap();
        assert_eq!(audit_trigger_count(&conn), 2);
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
    fn migrate_upgrades_002_db_to_current() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path());
        store.init().unwrap();
        let conn = open(&store).unwrap();
        conn.execute_batch(include_str!("../migrations/001_init.sql"))
            .unwrap();
        conn.execute_batch(include_str!("../migrations/002_hardening.sql"))
            .unwrap();
        conn.pragma_update(None, "user_version", 2).unwrap();

        migrate(&conn).unwrap();

        let version: i32 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, SCHEMA_VERSION);
        create_demo_session(&conn, "t", "demo-1", 0, 1).unwrap();
        assert_eq!(
            get_demo_session(&conn, "t").unwrap(),
            Some(("demo-1".to_string(), 1))
        );
        upsert_watermark(&conn, "usa-wa", 5, "aa-prompt-v1").unwrap();
        assert!(get_watermark(&conn, "usa-wa").unwrap().is_some());
    }
}
