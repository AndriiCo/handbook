//! Voice-owned SQLite tables (`voice_call`, `voice_transcript`,
//! `voice_webhook`, `voice_annotation`) plus row helpers.
//!
//! The migration lives here — not in the shared `db::migrate` — so
//! this lane never edits the shared schema file. `main` runs
//! [`migrate`] next to the auth/config migrations.

use rusqlite::{Connection, OptionalExtension, Result, params};

use crate::CallStatus;

/// One prepared question with its asked flag (stored as JSON).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Question {
    pub text: String,
    pub asked: bool,
}

/// Call row for a dial request.
pub struct NewCall<'a> {
    pub id: &'a str,
    pub call_control_id: &'a str,
    pub call_leg_id: &'a str,
    pub call_session_id: &'a str,
    pub to_hash: &'a str,
    pub authority: Option<&'a str>,
    pub questions_json: &'a str,
    /// Resolved pack id (`"inline"` when dial carried its own list).
    pub script_id: &'a str,
    pub created_by: &'a str,
    pub now: i64,
}

/// Stored call row (joins nothing; transcripts load separately).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoiceCall {
    pub id: String,
    pub call_control_id: String,
    pub call_leg_id: String,
    pub call_session_id: String,
    /// SHA-256 hex of the dialed number: enough to correlate repeat
    /// calls without keeping callee PII in the row.
    pub to_hash: String,
    pub authority: Option<String>,
    pub status: String,
    pub status_detail: Option<String>,
    pub questions_json: String,
    pub recording_sha: Option<String>,
    pub script_id: String,
    pub recording_status: String,
    pub created_by: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub ended_at: Option<i64>,
}

/// One live transcript segment.
#[derive(Debug, Clone, PartialEq)]
pub struct TranscriptSegment {
    pub seq: i64,
    pub speaker: Option<String>,
    pub text: String,
    pub at: i64,
}

/// One staff annotation pinned to a transcript segment (`correction`
/// rows are written by segment fixes and quote before → after; `flag`
/// and `note` rows come from the annotate route).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoiceAnnotation {
    pub seq: i64,
    pub kind: String,
    pub note: String,
    pub author: String,
    pub created_at: i64,
}

/// Create the voice tables (idempotent; safe to run every boot).
///
/// # Errors
///
/// Returns rusqlite errors when the schema cannot be applied.
pub fn migrate(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS voice_call(
           id TEXT PRIMARY KEY,
           call_control_id TEXT NOT NULL,
           call_leg_id TEXT NOT NULL DEFAULT '',
           call_session_id TEXT NOT NULL DEFAULT '',
           to_hash TEXT NOT NULL,
           authority TEXT,
           status TEXT NOT NULL DEFAULT 'queued',
           status_detail TEXT,
           questions_json TEXT NOT NULL DEFAULT '[]',
           recording_sha TEXT,
           recording_status TEXT NOT NULL DEFAULT 'none',
           created_by TEXT NOT NULL,
           created_at INTEGER NOT NULL,
           updated_at INTEGER NOT NULL,
           ended_at INTEGER,
           script_id TEXT NOT NULL DEFAULT 'inline'
         );
         CREATE INDEX IF NOT EXISTS idx_voice_call_control
           ON voice_call(call_control_id);
         CREATE TABLE IF NOT EXISTS voice_transcript(
           call_id TEXT NOT NULL REFERENCES voice_call(id),
           seq INTEGER NOT NULL,
           speaker TEXT,
           text TEXT NOT NULL,
           at INTEGER NOT NULL,
           PRIMARY KEY (call_id, seq)
         );
         CREATE TABLE IF NOT EXISTS voice_webhook(
           id TEXT PRIMARY KEY,
           call_id TEXT,
           event_type TEXT NOT NULL,
           at INTEGER NOT NULL
         );
         CREATE TABLE IF NOT EXISTS voice_annotation(
           call_id TEXT NOT NULL REFERENCES voice_call(id),
           seq INTEGER NOT NULL,
           kind TEXT NOT NULL,
           note TEXT NOT NULL,
           author TEXT NOT NULL,
           created_at INTEGER NOT NULL,
           PRIMARY KEY (call_id, seq, created_at)
         );",
    )?;
    // `script_id` arrived after the first voice slice: backfill
    // pre-existing DBs (the default marks their inline lists).
    let has_script_id: i64 = conn.query_row(
        "SELECT COUNT(*) FROM pragma_table_info('voice_call')
         WHERE name = 'script_id'",
        [],
        |row| row.get(0),
    )?;
    if has_script_id == 0 {
        conn.execute(
            "ALTER TABLE voice_call
             ADD COLUMN script_id TEXT NOT NULL DEFAULT 'inline'",
            [],
        )?;
    }
    Ok(())
}

/// Insert a queued call row.
///
/// # Errors
///
/// Returns rusqlite errors (e.g. duplicate id).
pub fn insert_call(conn: &Connection, call: &NewCall<'_>) -> Result<()> {
    conn.execute(
        "INSERT INTO voice_call(id, call_control_id, call_leg_id,
                                 call_session_id, to_hash, authority,
                                 status, questions_json, script_id,
                                 created_by, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'queued', ?7, ?8, ?9, ?10, ?10)",
        params![
            call.id,
            call.call_control_id,
            call.call_leg_id,
            call.call_session_id,
            call.to_hash,
            call.authority,
            call.questions_json,
            call.script_id,
            call.created_by,
            call.now,
        ],
    )?;
    Ok(())
}

/// Load one call by local id (`None` when unknown).
///
/// # Errors
///
/// Returns rusqlite errors.
pub fn get_call(conn: &Connection, id: &str) -> Result<Option<VoiceCall>> {
    conn.query_row(
        "SELECT id, call_control_id, call_leg_id, call_session_id,
                to_hash, authority, status, status_detail,
                questions_json, recording_sha, recording_status,
                created_by, created_at, updated_at, ended_at,
                script_id
         FROM voice_call WHERE id = ?1",
        [id],
        row_to_call,
    )
    .optional()
}

/// Load one call by Telnyx leg id (`None` when unknown).
///
/// # Errors
///
/// Returns rusqlite errors.
pub fn get_call_by_leg(conn: &Connection, leg: &str) -> Result<Option<VoiceCall>> {
    conn.query_row(
        "SELECT id, call_control_id, call_leg_id, call_session_id,
                to_hash, authority, status, status_detail,
                questions_json, recording_sha, recording_status,
                created_by, created_at, updated_at, ended_at,
                script_id
         FROM voice_call WHERE call_control_id = ?1",
        [leg],
        row_to_call,
    )
    .optional()
}

/// Most recent calls first, capped at `limit`.
///
/// # Errors
///
/// Returns rusqlite errors.
pub fn list_calls(conn: &Connection, limit: i64) -> Result<Vec<VoiceCall>> {
    let mut stmt = conn.prepare(
        "SELECT id, call_control_id, call_leg_id, call_session_id,
                to_hash, authority, status, status_detail,
                questions_json, recording_sha, recording_status,
                created_by, created_at, updated_at, ended_at,
                script_id
         FROM voice_call ORDER BY created_at DESC, id DESC LIMIT ?1",
    )?;
    stmt.query_map([limit], row_to_call)?.collect()
}

/// Move a call to `status` (caller applies the forward-only rule).
///
/// # Errors
///
/// Returns rusqlite errors.
pub fn set_status(
    conn: &Connection,
    id: &str,
    status: CallStatus,
    detail: Option<&str>,
    now: i64,
) -> Result<()> {
    let ended: Option<i64> = status.is_terminal().then_some(now);
    conn.execute(
        "UPDATE voice_call SET status = ?2, status_detail = ?3,
                                updated_at = ?4, ended_at = ?5
         WHERE id = ?1",
        params![id, status.as_str(), detail, now, ended],
    )?;
    Ok(())
}

/// Record a webhook id; `Ok(false)` means a duplicate delivery.
///
/// # Errors
///
/// Returns rusqlite errors.
pub fn record_webhook(
    conn: &Connection,
    webhook_id: &str,
    call_id: Option<&str>,
    event_type: &str,
    now: i64,
) -> Result<bool> {
    let inserted = conn.execute(
        "INSERT OR IGNORE INTO voice_webhook(id, call_id, event_type, at)
         VALUES (?1, ?2, ?3, ?4)",
        params![webhook_id, call_id, event_type, now],
    )?;
    Ok(inserted == 1)
}

/// Append one transcript segment (next sequence number).
///
/// # Errors
///
/// Returns rusqlite errors.
pub fn append_transcript(
    conn: &Connection,
    call_id: &str,
    speaker: Option<&str>,
    text: &str,
    now: i64,
) -> Result<()> {
    let seq: i64 = conn.query_row(
        "SELECT COUNT(*) FROM voice_transcript WHERE call_id = ?1",
        [call_id],
        |row| row.get(0),
    )?;
    conn.execute(
        "INSERT INTO voice_transcript(call_id, seq, speaker, text, at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![call_id, seq, speaker, text, now],
    )?;
    Ok(())
}

/// Segments in order for one call.
///
/// # Errors
///
/// Returns rusqlite errors.
pub fn list_transcripts(conn: &Connection, call_id: &str) -> Result<Vec<TranscriptSegment>> {
    let mut stmt = conn.prepare(
        "SELECT seq, speaker, text, at FROM voice_transcript
         WHERE call_id = ?1 ORDER BY seq",
    )?;
    stmt.query_map([call_id], |row| {
        Ok(TranscriptSegment {
            seq: row.get(0)?,
            speaker: row.get(1)?,
            text: row.get(2)?,
            at: row.get(3)?,
        })
    })?
    .collect()
}

/// One segment by (call, seq) (`None` when unknown).
///
/// # Errors
///
/// Returns rusqlite errors.
pub fn get_segment(
    conn: &Connection,
    call_id: &str,
    seq: i64,
) -> Result<Option<TranscriptSegment>> {
    conn.query_row(
        "SELECT seq, speaker, text, at FROM voice_transcript
         WHERE call_id = ?1 AND seq = ?2",
        params![call_id, seq],
        |row| {
            Ok(TranscriptSegment {
                seq: row.get(0)?,
                speaker: row.get(1)?,
                text: row.get(2)?,
                at: row.get(3)?,
            })
        },
    )
    .optional()
}

/// Rewrite a segment's text in place (seq/speaker/timing untouched).
///
/// # Errors
///
/// Returns rusqlite errors.
pub fn set_segment_text(conn: &Connection, call_id: &str, seq: i64, text: &str) -> Result<()> {
    conn.execute(
        "UPDATE voice_transcript SET text = ?3 WHERE call_id = ?1 AND seq = ?2",
        params![call_id, seq, text],
    )?;
    Ok(())
}

/// Pin an annotation to a segment. `created_at` starts at `now` and
/// bumps forward while the `(call_id, seq, created_at)` key collides,
/// so two annotations in the same second never clobber each other.
///
/// # Errors
///
/// Returns rusqlite errors.
pub fn insert_annotation(
    conn: &Connection,
    call_id: &str,
    seq: i64,
    kind: &str,
    note: &str,
    author: &str,
    now: i64,
) -> Result<()> {
    let mut created_at = now;
    loop {
        let attempt = conn.execute(
            "INSERT INTO voice_annotation(call_id, seq, kind, note, author, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![call_id, seq, kind, note, author, created_at],
        );
        match attempt {
            Ok(_) => return Ok(()),
            Err(e) if is_key_collision(&e) => {
                if created_at == i64::MAX {
                    return Err(e);
                }
                created_at += 1;
            }
            Err(other) => return Err(other),
        }
    }
}

fn is_key_collision(e: &rusqlite::Error) -> bool {
    matches!(e, rusqlite::Error::SqliteFailure(err, _)
        if err.code == rusqlite::ErrorCode::ConstraintViolation)
}

/// Annotations for one call, oldest first.
///
/// # Errors
///
/// Returns rusqlite errors.
pub fn list_annotations(conn: &Connection, call_id: &str) -> Result<Vec<VoiceAnnotation>> {
    let mut stmt = conn.prepare(
        "SELECT seq, kind, note, author, created_at FROM voice_annotation
         WHERE call_id = ?1 ORDER BY created_at, seq",
    )?;
    stmt.query_map([call_id], |row| {
        Ok(VoiceAnnotation {
            seq: row.get(0)?,
            kind: row.get(1)?,
            note: row.get(2)?,
            author: row.get(3)?,
            created_at: row.get(4)?,
        })
    })?
    .collect()
}

/// Link a downloaded recording blob to the call.
///
/// # Errors
///
/// Returns rusqlite errors.
pub fn set_recording(
    conn: &Connection,
    id: &str,
    sha: Option<&str>,
    status: &str,
    now: i64,
) -> Result<()> {
    conn.execute(
        "UPDATE voice_call SET recording_sha = ?2, recording_status = ?3,
                                updated_at = ?4 WHERE id = ?1",
        params![id, sha, status, now],
    )?;
    Ok(())
}

/// Mark question `idx` asked (`false` when the index is out of range
/// or the stored JSON is corrupt — the row is left untouched).
///
/// # Errors
///
/// Returns rusqlite errors.
pub fn mark_question_asked(conn: &Connection, id: &str, idx: usize, now: i64) -> Result<bool> {
    let Some(call) = get_call(conn, id)? else {
        return Ok(false);
    };
    let mut questions: Vec<Question> =
        serde_json::from_str(&call.questions_json).unwrap_or_default();
    let Some(question) = questions.get_mut(idx) else {
        return Ok(false);
    };
    question.asked = true;
    let json = serde_json::to_string(&questions).unwrap_or_else(|_| "[]".to_string());
    conn.execute(
        "UPDATE voice_call SET questions_json = ?2, updated_at = ?3 WHERE id = ?1",
        params![id, json, now],
    )?;
    Ok(true)
}

fn row_to_call(row: &rusqlite::Row<'_>) -> Result<VoiceCall> {
    Ok(VoiceCall {
        id: row.get(0)?,
        call_control_id: row.get(1)?,
        call_leg_id: row.get(2)?,
        call_session_id: row.get(3)?,
        to_hash: row.get(4)?,
        authority: row.get(5)?,
        status: row.get(6)?,
        status_detail: row.get(7)?,
        questions_json: row.get(8)?,
        recording_sha: row.get(9)?,
        recording_status: row.get(10)?,
        created_by: row.get(11)?,
        created_at: row.get(12)?,
        updated_at: row.get(13)?,
        ended_at: row.get(14)?,
        script_id: row.get(15)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn memory() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        conn
    }

    fn new_call<'a>(id: &'a str, now: i64) -> NewCall<'a> {
        NewCall {
            id,
            call_control_id: "leg-1",
            call_leg_id: "leg-1",
            call_session_id: "sess-1",
            to_hash: "abc",
            authority: Some("usa-wa-sos"),
            questions_json: r#"[{"text":"fee?","asked":false}]"#,
            script_id: "inline",
            created_by: "staff-1",
            now,
        }
    }

    #[test]
    fn migrate_is_idempotent_and_round_trips() {
        let conn = memory();
        migrate(&conn).unwrap();
        insert_call(&conn, &new_call("c-1", 100)).unwrap();
        let call = get_call(&conn, "c-1").unwrap().unwrap();
        assert_eq!(call.status, "queued");
        assert_eq!(call.script_id, "inline");
        assert_eq!(call.authority.as_deref(), Some("usa-wa-sos"));
        assert_eq!(get_call_by_leg(&conn, "leg-1").unwrap().unwrap().id, "c-1");
        assert!(get_call(&conn, "missing").unwrap().is_none());
        set_status(&conn, "c-1", CallStatus::Completed, Some("normal"), 200).unwrap();
        let call = get_call(&conn, "c-1").unwrap().unwrap();
        assert_eq!(call.status, "completed");
        assert_eq!(call.ended_at, Some(200));
        let listed = list_calls(&conn, 10).unwrap();
        assert_eq!(listed.len(), 1);
    }

    #[test]
    fn migrate_backfills_script_id_on_old_rows() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE voice_call(
               id TEXT PRIMARY KEY,
               call_control_id TEXT NOT NULL,
               call_leg_id TEXT NOT NULL DEFAULT '',
               call_session_id TEXT NOT NULL DEFAULT '',
               to_hash TEXT NOT NULL,
               authority TEXT,
               status TEXT NOT NULL DEFAULT 'queued',
               status_detail TEXT,
               questions_json TEXT NOT NULL DEFAULT '[]',
               recording_sha TEXT,
               recording_status TEXT NOT NULL DEFAULT 'none',
               created_by TEXT NOT NULL,
               created_at INTEGER NOT NULL,
               updated_at INTEGER NOT NULL,
               ended_at INTEGER
             );",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO voice_call(id, call_control_id, to_hash, created_by,
                                    created_at, updated_at)
             VALUES ('c-0', 'leg-0', 'h', 's', 1, 1)",
            [],
        )
        .unwrap();
        migrate(&conn).unwrap();
        migrate(&conn).unwrap();
        let call = get_call(&conn, "c-0").unwrap().unwrap();
        assert_eq!(call.script_id, "inline");
    }

    #[test]
    fn webhook_ids_dedupe() {
        let conn = memory();
        assert!(record_webhook(&conn, "wh-1", Some("c-1"), "call.answered", 1).unwrap());
        assert!(!record_webhook(&conn, "wh-1", Some("c-1"), "call.answered", 1).unwrap());
    }

    #[test]
    fn transcripts_keep_order() {
        let conn = memory();
        insert_call(&conn, &new_call("c-1", 100)).unwrap();
        append_transcript(&conn, "c-1", None, "one", 1).unwrap();
        append_transcript(&conn, "c-1", Some("agent"), "two", 2).unwrap();
        let segments = list_transcripts(&conn, "c-1").unwrap();
        assert_eq!(segments.len(), 2);
        assert_eq!(segments[0].seq, 0);
        assert_eq!(segments[1].text, "two");
    }

    #[test]
    fn segment_text_rewrite_keeps_speaker_and_timing() {
        let conn = memory();
        insert_call(&conn, &new_call("c-1", 100)).unwrap();
        append_transcript(&conn, "c-1", Some("them"), "Eight dollers.", 42).unwrap();
        let segment = get_segment(&conn, "c-1", 0).unwrap().unwrap();
        assert_eq!(segment.text, "Eight dollers.");
        assert!(get_segment(&conn, "c-1", 7).unwrap().is_none());
        assert!(get_segment(&conn, "missing", 0).unwrap().is_none());
        set_segment_text(&conn, "c-1", 0, "Eight dollars.").unwrap();
        let fixed = get_segment(&conn, "c-1", 0).unwrap().unwrap();
        assert_eq!(fixed.text, "Eight dollars.");
        assert_eq!(fixed.speaker.as_deref(), Some("them"));
        assert_eq!(fixed.at, 42);
    }

    #[test]
    fn annotations_round_trip_and_same_second_inserts_both_land() {
        let conn = memory();
        insert_call(&conn, &new_call("c-1", 100)).unwrap();
        append_transcript(&conn, "c-1", None, "hello", 1).unwrap();
        insert_annotation(
            &conn,
            "c-1",
            0,
            "correction",
            "helo → hello",
            "staff-1",
            900,
        )
        .unwrap();
        // Same (call, seq, created_at) key: the second insert bumps
        // forward instead of clobbering the first.
        insert_annotation(&conn, "c-1", 0, "flag", "check fee", "staff-1", 900).unwrap();
        let annotations = list_annotations(&conn, "c-1").unwrap();
        assert_eq!(annotations.len(), 2);
        assert_eq!(annotations[0].kind, "correction");
        assert_eq!(annotations[0].created_at, 900);
        assert_eq!(annotations[1].kind, "flag");
        assert_eq!(annotations[1].created_at, 901);
        assert!(list_annotations(&conn, "missing").unwrap().is_empty());
    }

    #[test]
    fn questions_toggle() {
        let conn = memory();
        insert_call(&conn, &new_call("c-1", 100)).unwrap();
        assert!(mark_question_asked(&conn, "c-1", 0, 200).unwrap());
        assert!(!mark_question_asked(&conn, "c-1", 7, 200).unwrap());
        assert!(!mark_question_asked(&conn, "missing", 0, 200).unwrap());
        let call = get_call(&conn, "c-1").unwrap().unwrap();
        let questions: Vec<Question> = serde_json::from_str(&call.questions_json).unwrap();
        assert!(questions[0].asked);
    }
}
