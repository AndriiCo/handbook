//! Byte ingest: sha256 dedup, plain-blob WARC capture, text sidecar,
//! FTS row.
//!
//! "WARC plain-blob first" (roadmap item 4): the capture row says
//! `kind='warc'` but `path` points at the raw bytes under their blob
//! address — no WARC record envelope, no `warcinfo` TLS evidence yet.
//! The `notes` column says so on every row this module writes.

use rusqlite::{Connection, OptionalExtension};

use contracts::Result;
use handbook_store::db::in_transaction;
use handbook_store::store::{Store, sha256_hex};

/// Marker written into `capture.notes` by [`ingest_bytes`].
pub const PLAIN_BLOB_NOTE: &str =
    "warc-plain-blob: raw bytes at blob address, no WARC envelope (thin watch)";

/// One ingest call's inputs.
pub struct IngestInput<'a> {
    pub authority_id: i64,
    pub source_id: Option<i64>,
    /// `capture.kind` value (`warc` for F1/F2 watch fetches).
    pub kind: &'a str,
    /// `capture.trust` value (`official`, `staff_verified`, ...).
    pub trust: &'a str,
    pub bytes: &'a [u8],
    /// Sidecar + FTS body (see [`extract_text`]).
    pub text: &'a str,
    pub captured_at: i64,
    pub actor: &'a str,
    /// Short provenance for the audit row (usually the fetched URL).
    pub origin: &'a str,
}

/// Outcome of one [`ingest_bytes`] call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IngestOutcome {
    /// A new capture row (plus sidecar, FTS, audit) was stored.
    Created { capture_id: i64 },
    /// Identical bytes for this authority already exist; nothing stored.
    Duplicate { capture_id: i64 },
}

impl IngestOutcome {
    pub fn capture_id(&self) -> i64 {
        match *self {
            IngestOutcome::Created { capture_id } | IngestOutcome::Duplicate { capture_id } => {
                capture_id
            }
        }
    }
}

/// Store fetched bytes as a capture, or dedup-hit the existing row.
///
/// Dedup key is `(authority_id, sha256)`: byte-identical re-fetches
/// create no new work. The capture + FTS + audit rows commit
/// atomically; the `text/<id>.txt` sidecar is written after commit
/// (temp file + rename). A missing sidecar is a specified agent
/// skip+task state, never silent corruption.
pub fn ingest_bytes(
    conn: &Connection,
    store: &Store,
    input: &IngestInput<'_>,
) -> Result<IngestOutcome> {
    let sha = sha256_hex(input.bytes);
    let existing: Option<i64> = conn
        .query_row(
            "SELECT id FROM capture WHERE authority_id = ?1 AND sha256 = ?2",
            rusqlite::params![input.authority_id, sha],
            |row| row.get(0),
        )
        .optional()?;
    if let Some(capture_id) = existing {
        return Ok(IngestOutcome::Duplicate { capture_id });
    }

    // Idempotent content write; safe to do before the transaction.
    let sha_again = store.write_blob(input.bytes)?;
    debug_assert_eq!(sha, sha_again);
    let relative = store
        .blob_path(&sha)?
        .strip_prefix(store.root())
        .expect("blob path is under the store root")
        .to_str()
        .expect("blob path is UTF-8")
        .to_string();

    let capture_id = in_transaction(conn, || {
        conn.execute(
            "INSERT INTO capture(authority_id, source_id, captured_at, kind,
                                  path, bytes_raw, bytes_stored, sha256,
                                  trust, notes)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            rusqlite::params![
                input.authority_id,
                input.source_id,
                input.captured_at,
                input.kind,
                relative,
                input.bytes.len() as i64,
                input.bytes.len() as i64,
                sha,
                input.trust,
                PLAIN_BLOB_NOTE,
            ],
        )?;
        let capture_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO capture_fts(capture_id, body) VALUES (?1, ?2)",
            rusqlite::params![capture_id, input.text],
        )?;
        conn.execute(
            "INSERT INTO audit(at, actor, action, entity, entity_id, reason)
             VALUES (?1, ?2, 'ingest_capture', 'capture', ?3, ?4)",
            rusqlite::params![
                input.captured_at,
                input.actor,
                capture_id,
                format!("fetch {} sha256={sha}", input.origin),
            ],
        )?;
        Ok(capture_id)
    })?;

    write_sidecar(store, capture_id, input.text)?;
    Ok(IngestOutcome::Created { capture_id })
}

/// Thin-watch text extraction: lossy UTF-8, naive `<tag>` stripping,
/// whitespace collapsed. HTML-aware extraction (scripts, entities,
/// PDFs) is F5 work; this only needs to feed sidecars + FTS today.
pub fn extract_text(bytes: &[u8]) -> String {
    let lossy = String::from_utf8_lossy(bytes);
    let mut out = String::with_capacity(lossy.len());
    let mut in_tag = false;
    for ch in lossy.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn write_sidecar(store: &Store, capture_id: i64, text: &str) -> Result<()> {
    let dir = store.root().join("text");
    std::fs::create_dir_all(&dir)?;
    let final_path = dir.join(format!("{capture_id}.txt"));
    let tmp_path = dir.join(format!(".{capture_id}.txt.tmp"));
    std::fs::write(&tmp_path, text.as_bytes())?;
    std::fs::rename(&tmp_path, &final_path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use handbook_store::db::{NewAuthority, add_authority, migrate, open};

    fn migrated() -> (tempfile::TempDir, Store, Connection) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path());
        store.init().unwrap();
        let conn = open(&store).unwrap();
        migrate(&conn).unwrap();
        (dir, store, conn)
    }

    fn authority(conn: &Connection, slug: &str) -> i64 {
        add_authority(
            conn,
            &NewAuthority {
                kind: "state",
                country: "USA",
                region: "Washington",
                name: "WA Secretary of State",
                slug,
                actor: "test",
                at: 1_800_000_000,
            },
        )
        .unwrap()
    }

    fn input<'a>(authority_id: i64, bytes: &'a [u8], text: &'a str) -> IngestInput<'a> {
        IngestInput {
            authority_id,
            source_id: None,
            kind: "warc",
            trust: "official",
            bytes,
            text,
            captured_at: 1_800_000_100,
            actor: "test",
            origin: "https://example.com/synthetic/1",
        }
    }

    #[test]
    fn creates_capture_sidecar_fts_and_audit() {
        let (_dir, store, conn) = migrated();
        let authority_id = authority(&conn, "usa-wa-sos");
        let outcome = ingest_bytes(
            &conn,
            &store,
            &input(authority_id, b"<p>PO Box 40220</p>", "PO Box 40220"),
        )
        .unwrap();
        let IngestOutcome::Created { capture_id } = outcome else {
            panic!("first ingest must create, got: {outcome:?}");
        };

        let (kind, trust, notes): (String, String, String) = conn
            .query_row(
                "SELECT kind, trust, notes FROM capture WHERE id = ?1",
                [capture_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(kind, "warc");
        assert_eq!(trust, "official");
        assert_eq!(notes, PLAIN_BLOB_NOTE);

        let sidecar = store.root().join("text").join(format!("{capture_id}.txt"));
        assert_eq!(std::fs::read_to_string(&sidecar).unwrap(), "PO Box 40220");

        let hits: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM capture_fts WHERE capture_id = ?1 AND body MATCH '40220'",
                [capture_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(hits, 1);

        let audits: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM audit WHERE action = 'ingest_capture' AND entity_id = ?1",
                [capture_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(audits, 1);
    }

    #[test]
    fn identical_bytes_dedup_to_one_capture() {
        let (_dir, store, conn) = migrated();
        let authority_id = authority(&conn, "usa-wa-sos");
        let first = ingest_bytes(
            &conn,
            &store,
            &input(authority_id, b"<p>PO Box 40220</p>", "PO Box 40220"),
        )
        .unwrap();
        let second = ingest_bytes(
            &conn,
            &store,
            &input(authority_id, b"<p>PO Box 40220</p>", "PO Box 40220"),
        )
        .unwrap();
        assert_eq!(first.capture_id(), second.capture_id());
        assert!(matches!(second, IngestOutcome::Duplicate { .. }));

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM capture WHERE id > 0", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn same_bytes_for_another_authority_are_distinct() {
        let (_dir, store, conn) = migrated();
        let a = authority(&conn, "usa-wa-sos");
        let b = authority(&conn, "usa-or-sos");
        let first = ingest_bytes(&conn, &store, &input(a, b"same", "same")).unwrap();
        let second = ingest_bytes(&conn, &store, &input(b, b"same", "same")).unwrap();
        assert_ne!(first.capture_id(), second.capture_id());
    }

    #[test]
    fn extract_text_strips_tags_and_collapses_space() {
        assert_eq!(
            extract_text(b"<html><body><p>PO  Box\n40220</p></body></html>"),
            "PO Box 40220"
        );
        assert_eq!(extract_text(b"plain"), "plain");
    }
}
