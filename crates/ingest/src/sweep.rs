//! Cadence-driven watch sweep (B5) + HCCH status poll (B10).
//!
//! Pure orchestration over [`crate::fetch`], [`crate::capture`], and
//! [`crate::simhash`]: enumerate due website `source` rows, re-fetch
//! with conditional GET, and ingest only what actually changed
//! (304 → liveness only; identical extracted text → skip; SimHash
//! near-dup → skip). Every skip still advances `last_checked_at`, so
//! the sweep never re-pays for an unchanged page. Fetch failures are
//! recorded as `validate_fact` tasks, never silent, and advance the
//! check time too: a dead URL waits one cadence instead of holding a
//! slot at the head of the queue (oldest check first) forever.
//!
//! A SimHash near-dup still ingests when the page gained a new line of
//! prose (`gained_lines`): a one-sentence closure banner on a long page
//! moves the fingerprint by ≤3 bits, and that notice is exactly what
//! the watch exists to see.
//!
//! HCCH (`poll_hcch`): the same core scoped to the `hcch` authority's
//! website source(s), plus one high-priority task on content change.
//! Deliberately no member-table auto-parse: the real page shape is
//! uncaptured (research traffic runs from prod IPs only), so the task
//! carries before/after capture ids for staff resolution. Auto-parse
//! lands once staff captures the real page shape.

use std::time::Duration;

use rusqlite::{Connection, OptionalExtension};

use contracts::Result;
use handbook_store::store::{Store, sha256_hex};

use crate::capture::{IngestInput, IngestOutcome, extract_text, ingest_bytes};
use crate::fetch::{
    FetchOutcome, check_robots, fetch_url_conditional, fetch_url_conditional_cached,
};
use crate::simhash::{is_near_dup, simhash};

/// One website `source` row with its fetch state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceRow {
    pub id: i64,
    pub authority_id: i64,
    pub slug: String,
    pub kind: String,
    pub uri: String,
    pub etag: String,
    pub last_modified: String,
    pub content_hash: String,
    pub simhash: u64,
    pub poll_cadence_sec: i64,
    pub last_checked_at: i64,
}

/// Outcome of polling one source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceOutcome {
    /// 304: still alive, nothing new (check time advanced).
    NotModified,
    /// New capture ingested.
    Created { capture_id: i64 },
    /// Byte-identical to an existing capture (stale validators).
    Duplicate { capture_id: i64 },
    /// Same extracted text or SimHash near-dup: state refreshed, no
    /// capture minted.
    NearDuplicate,
    /// Fetch failed; a `validate_fact` task was recorded.
    Failed { reason: String },
    /// Non-HTTP URI or robots disallow (disallow also records a task).
    Skipped { reason: String },
}

/// Website sources whose cadence has elapsed (`last_checked_at +
/// poll_cadence_sec <= now`), oldest check first.
pub fn due_sources(conn: &Connection, now: i64, limit: i64) -> Result<Vec<SourceRow>> {
    source_rows(
        conn,
        "SELECT s.id, s.authority_id, a.slug, s.kind, s.uri, s.etag,
                s.last_modified, s.content_hash, s.simhash,
                s.poll_cadence_sec, s.last_checked_at
         FROM source s JOIN authority a ON a.id = s.authority_id
         WHERE s.kind IN ('website_official', 'website_thirdparty')
           AND s.last_checked_at + s.poll_cadence_sec <= ?1
         ORDER BY s.last_checked_at, s.id LIMIT ?2",
        rusqlite::params![now, limit],
    )
}

/// All website sources for one authority slug (fetch-slug path).
pub fn authority_sources(conn: &Connection, slug: &str, limit: i64) -> Result<Vec<SourceRow>> {
    source_rows(
        conn,
        "SELECT s.id, s.authority_id, a.slug, s.kind, s.uri, s.etag,
                s.last_modified, s.content_hash, s.simhash,
                s.poll_cadence_sec, s.last_checked_at
         FROM source s JOIN authority a ON a.id = s.authority_id
         WHERE a.slug = ?1
           AND s.kind IN ('website_official', 'website_thirdparty')
         ORDER BY s.id LIMIT ?2",
        rusqlite::params![slug, limit],
    )
}

fn source_rows(
    conn: &Connection,
    sql: &str,
    params: impl rusqlite::Params,
) -> Result<Vec<SourceRow>> {
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map(params, |row| {
        let stored: i64 = row.get(8)?;
        Ok(SourceRow {
            id: row.get(0)?,
            authority_id: row.get(1)?,
            slug: row.get(2)?,
            kind: row.get(3)?,
            uri: row.get(4)?,
            etag: row.get(5)?,
            last_modified: row.get(6)?,
            content_hash: row.get(7)?,
            simhash: u64::from_be_bytes(stored.to_be_bytes()),
            poll_cadence_sec: row.get(9)?,
            last_checked_at: row.get(10)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Poll one source through the full change-detection stack.
///
/// Exactly one of: skip (no fetch), 304, ingest, near-dup skip, or
/// failure-with-task. `last_checked_at` advances on every path that
/// fetched (even 304/near-dup), so progress never stalls on quiet
/// pages. Never panics on bad rows: failures become tasks.
pub fn poll_source(
    conn: &Connection,
    store: &Store,
    row: &SourceRow,
    trust: &str,
    actor: &str,
    now: i64,
    timeout: Duration,
) -> SourceOutcome {
    poll_source_with(conn, store, row, trust, actor, now, timeout, None)
}

/// [`poll_source`], recording every answer in `cache` when given (the
/// scheduled sweep passes a `refresh`-mode [`callcache::FetchCache`]:
/// always live, every response kept on disk with its manifest line).
#[allow(clippy::too_many_arguments)]
pub fn poll_source_with(
    conn: &Connection,
    store: &Store,
    row: &SourceRow,
    trust: &str,
    actor: &str,
    now: i64,
    timeout: Duration,
    cache: Option<&callcache::FetchCache>,
) -> SourceOutcome {
    if !(row.uri.starts_with("http://") || row.uri.starts_with("https://")) {
        let _ = touch_checked(conn, row.id, now);
        return SourceOutcome::Skipped {
            reason: "not http".to_string(),
        };
    }
    if row.kind == "website_thirdparty" {
        let verdict = check_robots(&row.uri, timeout);
        if update_robots(conn, row.id, &verdict.text, now).is_err() {
            return SourceOutcome::Failed {
                reason: "record robots verdict: database error".to_string(),
            };
        }
        if !verdict.allowed {
            let reason = "robots disallow".to_string();
            record_fetch_failure(conn, row, &reason, actor, now);
            let _ = touch_checked(conn, row.id, now);
            return SourceOutcome::Skipped { reason };
        }
    }
    let etag = (!row.etag.is_empty()).then_some(row.etag.as_str());
    let stamp = (!row.last_modified.is_empty()).then_some(row.last_modified.as_str());
    let answer = match cache {
        Some(cache) => fetch_url_conditional_cached(cache, &row.uri, timeout, etag, stamp),
        None => fetch_url_conditional(&row.uri, timeout, etag, stamp),
    };
    let fetched = match answer {
        Ok(FetchOutcome::NotModified) => {
            let _ = touch_checked(conn, row.id, now);
            return SourceOutcome::NotModified;
        }
        Ok(FetchOutcome::Fetched(fetched)) => fetched,
        Err(e) => {
            let reason = e.to_string();
            record_fetch_failure(conn, row, &reason, actor, now);
            let _ = touch_checked(conn, row.id, now);
            return SourceOutcome::Failed { reason };
        }
    };
    let text = extract_text(&fetched.bytes);
    let content_hash = sha256_hex(text.as_bytes());
    let validators = (
        fetched.etag.as_deref().unwrap_or(""),
        fetched.last_modified.as_deref().unwrap_or(""),
    );
    // Identical extracted text: template/byte churn only, no capture.
    if !row.content_hash.is_empty() && row.content_hash == content_hash {
        let _ = update_state(
            conn,
            row.id,
            validators.0,
            validators.1,
            &content_hash,
            row.simhash,
            now,
        );
        return SourceOutcome::NearDuplicate;
    }
    // Near-dup: keep the ORIGINAL fingerprint as the baseline (updating
    // it would let slow drift walk forever without ever ingesting).
    let fingerprint = simhash(&text);
    if is_near_dup(row.simhash, fingerprint)
        && !previous_text(conn, store, row.id).is_some_and(|prev| gained_lines(&prev, &text))
    {
        let _ = touch_validators(conn, row.id, validators.0, validators.1, now);
        return SourceOutcome::NearDuplicate;
    }
    let outcome = match ingest_bytes(
        conn,
        store,
        &IngestInput {
            authority_id: row.authority_id,
            source_id: Some(row.id),
            kind: "warc",
            trust,
            bytes: &fetched.bytes,
            text: &text,
            captured_at: now,
            actor,
            origin: &row.uri,
        },
    ) {
        Ok(outcome) => outcome,
        Err(e) => {
            let reason = format!("ingest: {e}");
            record_fetch_failure(conn, row, &reason, actor, now);
            let _ = touch_checked(conn, row.id, now);
            return SourceOutcome::Failed { reason };
        }
    };
    let _ = update_state(
        conn,
        row.id,
        validators.0,
        validators.1,
        &content_hash,
        fingerprint,
        now,
    );
    match outcome {
        IngestOutcome::Created { capture_id } => SourceOutcome::Created { capture_id },
        IngestOutcome::Duplicate { capture_id } => SourceOutcome::Duplicate { capture_id },
    }
}

/// Sidecar text of the newest capture of `source_id`, if any.
fn previous_text(conn: &Connection, store: &Store, source_id: i64) -> Option<String> {
    let id: i64 = conn
        .query_row(
            "SELECT id FROM capture WHERE source_id = ?1
             ORDER BY captured_at DESC, id DESC LIMIT 1",
            [source_id],
            |row| row.get(0),
        )
        .optional()
        .ok()??;
    std::fs::read_to_string(store.root().join("text").join(format!("{id}.txt"))).ok()
}

/// True when `new` holds a line of prose that `old` lacks. Lines are
/// compared case- and space-folded with digits masked, so a bumped
/// date or counter is never "new"; only lines of at least 16
/// characters with 8 letters count, so nav crumbs and buttons don't.
pub fn gained_lines(old: &str, new: &str) -> bool {
    fn fold(line: &str) -> Option<String> {
        let folded = line
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase()
            .chars()
            .map(|c| if c.is_ascii_digit() { '#' } else { c })
            .collect::<String>();
        let letters = folded.chars().filter(|c| c.is_alphabetic()).count();
        (folded.chars().count() >= 16 && letters >= 8).then_some(folded)
    }
    let seen: std::collections::HashSet<String> = old.lines().filter_map(fold).collect();
    new.lines()
        .filter_map(fold)
        .any(|line| !seen.contains(&line))
}

fn update_robots(conn: &Connection, source_id: i64, verdict: &str, now: i64) -> Result<()> {
    conn.execute(
        "UPDATE source SET robots = ?1 WHERE id = ?2",
        rusqlite::params![format!("{verdict} at {now}"), source_id],
    )?;
    Ok(())
}

fn touch_checked(conn: &Connection, source_id: i64, now: i64) -> Result<()> {
    conn.execute(
        "UPDATE source SET last_checked_at = ?1 WHERE id = ?2",
        rusqlite::params![now, source_id],
    )?;
    Ok(())
}

fn touch_validators(
    conn: &Connection,
    source_id: i64,
    etag: &str,
    last_modified: &str,
    now: i64,
) -> Result<()> {
    conn.execute(
        "UPDATE source SET etag = ?1, last_modified = ?2, last_checked_at = ?3 WHERE id = ?4",
        rusqlite::params![etag, last_modified, now, source_id],
    )?;
    Ok(())
}

fn update_state(
    conn: &Connection,
    source_id: i64,
    etag: &str,
    last_modified: &str,
    content_hash: &str,
    fingerprint: u64,
    now: i64,
) -> Result<()> {
    conn.execute(
        "UPDATE source
         SET etag = ?1, last_modified = ?2, content_hash = ?3, simhash = ?4,
             last_checked_at = ?5
         WHERE id = ?6",
        rusqlite::params![
            etag,
            last_modified,
            content_hash,
            i64::from_be_bytes(fingerprint.to_be_bytes()),
            now,
            source_id,
        ],
    )?;
    Ok(())
}

/// One [`record_task`] call's inputs.
pub struct NewTask<'a> {
    pub authority_id: i64,
    pub kind: &'a str,
    pub title: &'a str,
    pub detail: &'a str,
    pub priority: i64,
    pub dedup_key: &'a str,
    pub created_by: &'a str,
    pub created_at: i64,
}

/// Insert a task, or refresh the live row on dedup hit. Returns the
/// task id (new or existing). A DB failure here is returned — callers
/// that already hold a more important error may ignore it.
pub fn record_task(conn: &Connection, task: &NewTask<'_>) -> Result<i64> {
    let inserted = conn.execute(
        "INSERT INTO task(authority_id, kind, title, detail, status,
                           priority, dedup_key, created_by, created_at)
         VALUES (?1, ?2, ?3, ?4, 'open', ?5, ?6, ?7, ?8)",
        rusqlite::params![
            task.authority_id,
            task.kind,
            task.title,
            task.detail,
            task.priority,
            task.dedup_key,
            task.created_by,
            task.created_at,
        ],
    );
    match inserted {
        Ok(_) => Ok(conn.last_insert_rowid()),
        Err(rusqlite::Error::SqliteFailure(e, _))
            if e.code == rusqlite::ErrorCode::ConstraintViolation =>
        {
            conn.execute(
                "UPDATE task SET detail = ?1, created_at = ?2
                 WHERE dedup_key = ?3
                   AND status IN ('open', 'batched', 'in_progress')",
                rusqlite::params![task.detail, task.created_at, task.dedup_key],
            )?;
            Ok(conn.query_row(
                "SELECT id FROM task WHERE dedup_key = ?1
                 ORDER BY id DESC LIMIT 1",
                [task.dedup_key],
                |row| row.get::<_, i64>(0),
            )?)
        }
        Err(e) => Err(e.into()),
    }
}

/// Fetch-failure task for one source (F1 acceptance 4). Best-effort:
/// the poll already has its `Failed` outcome; a task-write failure
/// must not overwrite it.
fn record_fetch_failure(conn: &Connection, row: &SourceRow, failure: &str, actor: &str, now: i64) {
    let dedup_key = sha256_hex(format!("{}|fetch|{}", row.slug, row.uri).as_bytes());
    let title = format!("fetch failed: {}", row.uri);
    let _ = record_task(
        conn,
        &NewTask {
            authority_id: row.authority_id,
            kind: "validate_fact",
            title: &title,
            detail: failure,
            priority: 0,
            dedup_key: &dedup_key,
            created_by: actor,
            created_at: now,
        },
    );
}

/// Outcome of [`poll_hcch`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HcchOutcome {
    /// No `hcch` authority or no website source under it: staff must
    /// seed the ONE status-table URI first (packs-load + fetch-url).
    NoSource,
    /// All HCCH sources 304 / near-dup / byte-dup: nothing new.
    NotModified,
    /// At least one source changed: new capture(s) plus exactly one
    /// high-priority task pointing at them.
    Changed { capture_ids: Vec<i64>, task_id: i64 },
    /// Every source failed or was skipped.
    Failed { reason: String },
}

/// HCCH status-table poll (F2/B10): poll the `hcch` authority's website
/// source(s) through [`poll_source`]; on any new capture, ensure one
/// high-priority `validate_fact` task so staff diffs the member table
/// (no auto-parse — see the module docs).
pub fn poll_hcch(
    conn: &Connection,
    store: &Store,
    actor: &str,
    now: i64,
    timeout: Duration,
) -> Result<HcchOutcome> {
    let rows = authority_sources(conn, "hcch", 16)?;
    if rows.is_empty() {
        return Ok(HcchOutcome::NoSource);
    }
    let mut created = Vec::new();
    let mut failures = Vec::new();
    for row in &rows {
        match poll_source(conn, store, row, "official", actor, now, timeout) {
            SourceOutcome::Created { capture_id } => created.push(capture_id),
            SourceOutcome::Failed { reason } | SourceOutcome::Skipped { reason } => {
                failures.push(format!("{}: {reason}", row.uri));
            }
            SourceOutcome::NotModified
            | SourceOutcome::Duplicate { .. }
            | SourceOutcome::NearDuplicate => {}
        }
    }
    if created.is_empty() {
        if failures.len() == rows.len() {
            return Ok(HcchOutcome::Failed {
                reason: failures.join("; "),
            });
        }
        return Ok(HcchOutcome::NotModified);
    }
    // Before/after pointers for the staffer who diffs the table: the
    // previous capture of the SAME source (another HCCH page's capture
    // would diff against the wrong table).
    let prev: Option<i64> = conn
        .query_row(
            "SELECT id FROM capture
             WHERE source_id = (SELECT source_id FROM capture WHERE id = ?1)
               AND id < ?1
             ORDER BY id DESC LIMIT 1",
            [created[0]],
            |row| row.get(0),
        )
        .optional()?;
    let detail = match prev {
        Some(id) => format!(
            "HCCH status table changed: capture #{} (prev #{id}). Diff the member/effective-date rows from prod IPs; routing facts update after staff review.",
            created
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", #")
        ),
        None => format!(
            "HCCH status table first capture (#{}) — no baseline yet. Seed the member/effective-date baseline from prod IPs.",
            created[0]
        ),
    };
    let dedup_key = sha256_hex("hcch|status-table".as_bytes());
    let task_id = record_task(
        conn,
        &NewTask {
            authority_id: rows[0].authority_id,
            kind: "validate_fact",
            title: "HCCH status table changed",
            detail: &detail,
            priority: 100,
            dedup_key: &dedup_key,
            created_by: actor,
            created_at: now,
        },
    )?;
    Ok(HcchOutcome::Changed {
        capture_ids: created,
        task_id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use handbook_store::db::{NewAuthority, add_authority, migrate, open};
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::time::Duration;

    const NOW: i64 = 1_800_000_000;

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
                kind: "office",
                country: "Test",
                region: "",
                name: "Test Office",
                slug,
                actor: "test",
                at: NOW,
            },
        )
        .unwrap()
    }

    fn add_source(conn: &Connection, authority_id: i64, kind: &str, uri: &str) -> i64 {
        conn.execute(
            "INSERT INTO source(authority_id, kind, uri) VALUES (?1, ?2, ?3)",
            rusqlite::params![authority_id, kind, uri],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    fn set_state(
        conn: &Connection,
        id: i64,
        etag: &str,
        content_hash: &str,
        simhash: i64,
        checked: i64,
    ) {
        conn.execute(
            "UPDATE source SET etag = ?1, content_hash = ?2, simhash = ?3,
                              last_checked_at = ?4 WHERE id = ?5",
            rusqlite::params![etag, content_hash, simhash, checked, id],
        )
        .unwrap();
    }

    fn state(conn: &Connection, id: i64) -> (String, String, String, i64, i64) {
        conn.query_row(
            "SELECT etag, last_modified, content_hash, simhash, last_checked_at
             FROM source WHERE id = ?1",
            [id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .unwrap()
    }

    // Per-authority: migration 005 keeps an unattributed sentinel
    // capture (id 0) that a bare COUNT(*) would include.
    fn capture_count(conn: &Connection, authority_id: i64) -> i64 {
        conn.query_row(
            "SELECT COUNT(*) FROM capture WHERE authority_id = ?1",
            [authority_id],
            |row| row.get(0),
        )
        .unwrap()
    }

    /// Serve one canned reply, then exit. Binds port 0.
    fn serve(
        status_line: &'static str,
        extra: &'static [(&'static str, &'static str)],
        body: &'static [u8],
    ) -> (String, std::thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        let url = format!(
            "http://127.0.0.1:{}/",
            listener.local_addr().unwrap().port()
        );
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            let mut buf = [0u8; 4096];
            let _ = stream.read(&mut buf);
            let mut head = format!(
                "{status_line}\r\nContent-Length: {}\r\nConnection: close\r\n",
                body.len()
            );
            for (name, value) in extra {
                head.push_str(&format!("{name}: {value}\r\n"));
            }
            head.push_str("\r\n");
            stream.write_all(head.as_bytes()).expect("write head");
            stream.write_all(body).expect("write body");
        });
        (url, handle)
    }

    fn row(conn: &Connection, slug: &str) -> SourceRow {
        authority_sources(conn, slug, 10)
            .unwrap()
            .pop()
            .expect("one source")
    }

    #[test]
    fn due_sources_skips_fresh_and_non_website() {
        let (_dir, _store, conn) = migrated();
        let auth = authority(&conn, "t-sos");
        let due = add_source(&conn, auth, "website_official", "https://example.com/due");
        let fresh = add_source(&conn, auth, "website_official", "https://example.com/fresh");
        let mail = add_source(&conn, auth, "email_official", "ops@example.com");
        set_state(&conn, fresh, "", "", 0, NOW - 10);
        let ids: Vec<i64> = due_sources(&conn, NOW, 10)
            .unwrap()
            .iter()
            .map(|r| r.id)
            .collect();
        assert_eq!(ids, vec![due], "only the stale website row is due");
        let _ = mail;
    }

    #[test]
    fn poll_304_advances_check_without_capture() {
        let (_dir, store, conn) = migrated();
        let auth = authority(&conn, "t-sos");
        let (url, server) = serve("HTTP/1.0 304 Not Modified", &[], b"");
        let id = add_source(&conn, auth, "website_official", &url);
        set_state(&conn, id, "\"v1\"", "hash", 12345, 0);
        let outcome = poll_source(
            &conn,
            &store,
            &row(&conn, "t-sos"),
            "official",
            "test",
            NOW,
            Duration::from_secs(5),
        );
        assert_eq!(outcome, SourceOutcome::NotModified);
        assert_eq!(capture_count(&conn, auth), 0);
        assert_eq!(state(&conn, id).4, NOW);
        server.join().unwrap();
    }

    #[test]
    fn poll_change_ingests_and_sets_state() {
        let (_dir, store, conn) = migrated();
        let auth = authority(&conn, "t-sos");
        let (url, server) = serve(
            "HTTP/1.0 200 OK",
            &[
                ("ETag", "\"v2\""),
                ("Last-Modified", "Wed, 28 Oct 2026 01:00:00 GMT"),
            ],
            b"<p>Counter fee $8</p>",
        );
        let id = add_source(&conn, auth, "website_official", &url);
        let outcome = poll_source(
            &conn,
            &store,
            &row(&conn, "t-sos"),
            "official",
            "test",
            NOW,
            Duration::from_secs(5),
        );
        let SourceOutcome::Created { capture_id } = outcome else {
            panic!("want created, got: {outcome:?}");
        };
        assert_eq!(capture_count(&conn, auth), 1);
        assert!(
            store
                .root()
                .join("text")
                .join(format!("{capture_id}.txt"))
                .exists()
        );
        let (etag, stamp, hash, fp, checked) = state(&conn, id);
        assert_eq!(etag, "\"v2\"");
        assert_eq!(stamp, "Wed, 28 Oct 2026 01:00:00 GMT");
        assert_eq!(hash, sha256_hex("Counter fee $8".as_bytes()));
        assert_ne!(fp, 0);
        assert_eq!(checked, NOW);
        server.join().unwrap();
    }

    #[test]
    fn poll_same_text_new_bytes_is_near_dup() {
        let (_dir, store, conn) = migrated();
        let auth = authority(&conn, "t-sos");
        let (url, server) = serve("HTTP/1.0 200 OK", &[], b"<p>hello</p>");
        let id = add_source(&conn, auth, "website_official", &url);
        let first = poll_source(
            &conn,
            &store,
            &row(&conn, "t-sos"),
            "official",
            "test",
            NOW,
            Duration::from_secs(5),
        );
        assert!(matches!(first, SourceOutcome::Created { .. }), "{first:?}");
        server.join().unwrap();
        // Same extracted text ("hello"), different bytes and no validators.
        let (url2, server2) = serve(
            "HTTP/1.0 200 OK",
            &[],
            b"<div><p>hello</p></div><!-- template v2 -->",
        );
        conn.execute(
            "UPDATE source SET uri = ?1 WHERE id = ?2",
            rusqlite::params![url2, id],
        )
        .unwrap();
        let _ = url;
        let second = poll_source(
            &conn,
            &store,
            &row(&conn, "t-sos"),
            "official",
            "test",
            NOW + 100,
            Duration::from_secs(5),
        );
        assert_eq!(second, SourceOutcome::NearDuplicate);
        assert_eq!(
            capture_count(&conn, auth),
            1,
            "no second capture for template churn"
        );
        server2.join().unwrap();
    }

    #[test]
    fn poll_failure_records_task() {
        let (_dir, store, conn) = migrated();
        let auth = authority(&conn, "t-sos");
        let probe = TcpListener::bind("127.0.0.1:0").expect("bind probe");
        let port = probe.local_addr().unwrap().port();
        drop(probe);
        add_source(
            &conn,
            auth,
            "website_official",
            &format!("http://127.0.0.1:{port}/"),
        );
        let outcome = poll_source(
            &conn,
            &store,
            &row(&conn, "t-sos"),
            "official",
            "test",
            NOW,
            Duration::from_secs(5),
        );
        let SourceOutcome::Failed { reason } = outcome else {
            panic!("want failed, got: {outcome:?}");
        };
        assert!(reason.contains("fetch"), "{reason}");
        let (kind, title): (String, String) = conn
            .query_row(
                "SELECT kind, title FROM task ORDER BY id DESC LIMIT 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(kind, "validate_fact");
        assert!(title.starts_with("fetch failed: "), "{title}");
        // A dead URL waits one cadence: it no longer heads the queue.
        let id = row(&conn, "t-sos").id;
        assert_eq!(state(&conn, id).4, NOW);
        assert!(due_sources(&conn, NOW, 10).unwrap().is_empty());
    }

    #[test]
    fn poll_near_dup_with_a_new_banner_line_still_ingests() {
        let (_dir, store, conn) = migrated();
        let auth = authority(&conn, "t-sos");
        let (url, server) = serve(
            "HTTP/1.0 200 OK",
            &[],
            b"<p>Apostille services by mail and in person.</p><p>Updated 11/01/2026</p>",
        );
        let id = add_source(&conn, auth, "website_official", &url);
        let first = poll_source(
            &conn,
            &store,
            &row(&conn, "t-sos"),
            "official",
            "test",
            NOW,
            Duration::from_secs(5),
        );
        assert!(matches!(first, SourceOutcome::Created { .. }), "{first:?}");
        server.join().unwrap();
        const BANNER: &[u8] = b"<p>Apostille services are currently unavailable.</p>\
            <p>Apostille services by mail and in person.</p><p>Updated 11/02/2026</p>";
        // Force the SimHash gate to call it a near-dup: only the
        // gained line can let it through.
        let fingerprint = simhash(&extract_text(BANNER));
        conn.execute(
            "UPDATE source SET simhash = ?1 WHERE id = ?2",
            rusqlite::params![i64::from_be_bytes(fingerprint.to_be_bytes()), id],
        )
        .unwrap();
        let (url2, server2) = serve("HTTP/1.0 200 OK", &[], BANNER);
        conn.execute(
            "UPDATE source SET uri = ?1 WHERE id = ?2",
            rusqlite::params![url2, id],
        )
        .unwrap();
        let second = poll_source(
            &conn,
            &store,
            &row(&conn, "t-sos"),
            "official",
            "test",
            NOW + 100,
            Duration::from_secs(5),
        );
        assert!(
            matches!(second, SourceOutcome::Created { .. }),
            "{second:?}"
        );
        assert_eq!(capture_count(&conn, auth), 2);
        server2.join().unwrap();
    }

    #[test]
    fn poll_with_a_cache_keeps_every_answer_on_disk() {
        let (dir, store, conn) = migrated();
        let auth = authority(&conn, "t-sos");
        let (url, server) = serve("HTTP/1.0 200 OK", &[], b"<p>Fee $8 per document.</p>");
        add_source(&conn, auth, "website_official", &url);
        let cache = callcache::FetchCache::new(
            dir.path().join("fetch-cache"),
            callcache::CacheMode::Refresh,
        );
        let outcome = poll_source_with(
            &conn,
            &store,
            &row(&conn, "t-sos"),
            "official",
            "test",
            NOW,
            Duration::from_secs(5),
            Some(&cache),
        );
        assert!(
            matches!(outcome, SourceOutcome::Created { .. }),
            "{outcome:?}"
        );
        let saved = cache.latest(&url).expect("answer recorded");
        assert_eq!(saved.status, 200);
        assert_eq!(saved.body, b"<p>Fee $8 per document.</p>");
        server.join().unwrap();
    }

    #[test]
    fn gained_lines_ignores_dates_case_and_short_crumbs() {
        let old = "Apostille services by mail.\nUpdated 11/01/2026 at 9:00\nHome";
        assert!(!gained_lines(
            old,
            "APOSTILLE  services by mail.\nUpdated 11/03/2026 at 9:30"
        ));
        assert!(!gained_lines(
            old,
            "Apostille services by mail.\nMenu\nSearch"
        ));
        assert!(gained_lines(
            old,
            "Apostille services are unavailable until 11/10.\nHome"
        ));
        assert!(!gained_lines(old, ""));
    }

    #[test]
    fn poll_non_http_skips_without_task() {
        let (_dir, store, conn) = migrated();
        let auth = authority(&conn, "t-sos");
        add_source(&conn, auth, "website_official", "ftp://example.com/x");
        let outcome = poll_source(
            &conn,
            &store,
            &row(&conn, "t-sos"),
            "official",
            "test",
            NOW,
            Duration::from_secs(5),
        );
        assert_eq!(
            outcome,
            SourceOutcome::Skipped {
                reason: "not http".to_string()
            }
        );
        assert!(
            due_sources(&conn, NOW, 10).unwrap().is_empty(),
            "skip advances"
        );
        let tasks: i64 = conn
            .query_row("SELECT COUNT(*) FROM task", [], |row| row.get(0))
            .unwrap();
        assert_eq!(tasks, 0);
    }

    #[test]
    fn record_task_dedups_by_key() {
        let (_dir, _store, conn) = migrated();
        let auth = authority(&conn, "t-sos");
        let first = record_task(
            &conn,
            &NewTask {
                authority_id: auth,
                kind: "validate_fact",
                title: "t",
                detail: "d1",
                priority: 0,
                dedup_key: "k",
                created_by: "test",
                created_at: NOW,
            },
        )
        .unwrap();
        let second = record_task(
            &conn,
            &NewTask {
                authority_id: auth,
                kind: "validate_fact",
                title: "t",
                detail: "d2",
                priority: 0,
                dedup_key: "k",
                created_by: "test",
                created_at: NOW + 1,
            },
        )
        .unwrap();
        assert_eq!(first, second);
        let (detail, count): (String, i64) = conn
            .query_row(
                "SELECT detail, (SELECT COUNT(*) FROM task) FROM task WHERE id = ?1",
                [first],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!((detail.as_str(), count), ("d2", 1));
    }

    #[test]
    fn hcch_without_source_is_no_source() {
        let (_dir, store, conn) = migrated();
        authority(&conn, "hcch");
        let outcome = poll_hcch(&conn, &store, "test", NOW, Duration::from_secs(5)).unwrap();
        assert_eq!(outcome, HcchOutcome::NoSource);
    }

    #[test]
    fn hcch_change_creates_one_high_priority_task() {
        let (_dir, store, conn) = migrated();
        let auth = authority(&conn, "hcch");
        let (url, server) = serve(
            "HTTP/1.0 200 OK",
            &[("ETag", "\"h1\"")],
            b"<p>status table</p>",
        );
        add_source(&conn, auth, "website_official", &url);
        let outcome = poll_hcch(&conn, &store, "test", NOW, Duration::from_secs(5)).unwrap();
        let HcchOutcome::Changed {
            capture_ids,
            task_id,
        } = outcome
        else {
            panic!("want changed, got: {outcome:?}");
        };
        assert_eq!(capture_ids.len(), 1);
        let (kind, priority, title): (String, i64, String) = conn
            .query_row(
                "SELECT kind, priority, title FROM task WHERE id = ?1",
                [task_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(kind, "validate_fact");
        assert_eq!(priority, 100);
        assert_eq!(title, "HCCH status table changed");
        server.join().unwrap();
        // Second poll, same bytes, validators now stored → 304 → quiet.
        let (url2, server2) = serve("HTTP/1.0 304 Not Modified", &[], b"");
        conn.execute(
            "UPDATE source SET uri = ?1 WHERE authority_id = ?2",
            rusqlite::params![url2, auth],
        )
        .unwrap();
        let quiet = poll_hcch(&conn, &store, "test", NOW + 100, Duration::from_secs(5)).unwrap();
        assert_eq!(quiet, HcchOutcome::NotModified);
        let tasks: i64 = conn
            .query_row("SELECT COUNT(*) FROM task", [], |row| row.get(0))
            .unwrap();
        assert_eq!(tasks, 1, "no second task when nothing changed");
        server2.join().unwrap();
    }
}
