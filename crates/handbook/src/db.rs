//! SQLite metadata database: migrations and foundation queries.
//!
//! Schema lives in `migrations/001_init.sql` (see `docs/design.md` §1).
//! Later migrations add `N+1` files and extend [`migrate`].

use rusqlite::Connection;

use crate::store::Store;

/// Current schema version (`pragma user_version`).
pub const SCHEMA_VERSION: i32 = 1;

/// Voice-audio retention in days (decided 2026-10-26).
pub const VOICE_RETENTION_DAYS: i64 = 365;

/// Open (creating if missing) the store's metadata database.
pub fn open(store: &Store) -> rusqlite::Result<Connection> {
    Connection::open(store.db_path())
}

/// Apply pending migrations. Idempotent.
pub fn migrate(conn: &Connection) -> rusqlite::Result<()> {
    let version: i32 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    if version < 1 {
        conn.execute_batch(include_str!("../migrations/001_init.sql"))?;
        conn.pragma_update(None, "user_version", 1)?;
    }
    debug_assert_eq!(
        conn.query_row("PRAGMA user_version", [], |row| row.get::<_, i32>(0))?,
        SCHEMA_VERSION
    );
    Ok(())
}

/// Insert an authority (office / jurisdiction). Returns the new row id.
pub fn add_authority(
    conn: &Connection,
    kind: &str,
    country: &str,
    region: &str,
    name: &str,
    slug: &str,
) -> rusqlite::Result<i64> {
    conn.execute(
        "INSERT INTO authority(kind, country, region, name, slug)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        (kind, country, region, name, slug),
    )?;
    Ok(conn.last_insert_rowid())
}

/// Delete voice-audio captures older than `VOICE_RETENTION_DAYS`.
///
/// For each expired `call_session.audio_capture_id`: removes the blob file
/// and its capture row, nulls the session link, and writes an `audit` row.
/// Transcripts, annotations, and consent records are never touched.
/// Returns the number of audio captures deleted. `now` is unix seconds
/// (a parameter so tests don't depend on the clock).
pub fn retention_sweep(
    conn: &Connection,
    store: &Store,
    now: i64,
    actor: &str,
) -> rusqlite::Result<usize> {
    let cutoff = now - VOICE_RETENTION_DAYS * 86_400;
    let expired: Vec<(i64, String)> = conn
        .prepare(
            "SELECT c.id, c.path FROM capture c
             JOIN call_session s ON s.audio_capture_id = c.id
             WHERE c.kind = 'audio' AND c.captured_at < ?1",
        )?
        .query_map([cutoff], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;

    for (id, path) in &expired {
        let full = store.root().join(path);
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
    }
    Ok(expired.len())
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
            "state",
            "USA",
            "Washington",
            "WA Secretary of State",
            "usa-wa-sos",
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
        let authority = add_authority(&conn, "state", "USA", "WA", "WA", "usa-wa").unwrap();
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
        let authority = add_authority(&conn, "state", "USA", "WA", "WA", "usa-wa").unwrap();
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
        assert!(!store.blob_path(&audio_old).exists());
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
        assert!(store.blob_path(&audio_new).exists());

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
}
