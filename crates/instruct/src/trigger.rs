//! Trigger intake: new evidence captures → findings.
//!
//! Per authority, captures with `id > cursor`, `kind IN (email,
//! transcript, comment)`, `trust != rejected`, oldest first. Text
//! comes from `text/<id>.txt` sidecars; long evidence is condensed
//! with the Atlas map-reduce summarizer (cache under
//! `<store>/instruct/cache`, keyed by content sha).
//!
//! The cursor lives in `watermark` under `instruct:<slug>` keys —
//! the analysis worker owns bare slugs, so the two never clobber
//! each other. No migration needed.

use rusqlite::Connection;

use atlas::chunk::{chunk_text, estimate_tokens};
use atlas::model::ModelPort;
use atlas::summarize::{Summarizer, SummaryCache, SummaryDraft, reduce};

use crate::error::Result;

/// Capture kinds that feed the worker.
pub const TRIGGER_KINDS: &[&str] = &["email", "transcript", "comment"];

/// One trigger: a capture + its evidence text.
#[derive(Debug, Clone)]
pub struct Trigger {
    pub capture_id: i64,
    pub kind: String,
    pub text: String,
    /// True when `text` is a condensed summary, not verbatim.
    pub condensed: bool,
}

/// Watermark key for one authority.
pub fn cursor_key(slug: &str) -> String {
    format!("instruct:{slug}")
}

/// Read the cursor (0 when never swept).
pub fn get_cursor(conn: &Connection, slug: &str) -> Result<i64> {
    let key = cursor_key(slug);
    match conn.query_row(
        "SELECT last_capture_id FROM watermark WHERE authority_slug = ?1",
        [&key],
        |row| row.get(0),
    ) {
        Ok(id) => Ok(id),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(0),
        Err(e) => Err(e.into()),
    }
}

/// Advance the cursor (same txn as the proposals — atomic).
pub fn set_cursor(conn: &Connection, slug: &str, capture_id: i64, now: i64) -> Result<()> {
    let key = cursor_key(slug);
    conn.execute(
        "INSERT INTO watermark(authority_slug, last_run_at, last_prompt_version, last_capture_id)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(authority_slug) DO UPDATE SET
           last_run_at = excluded.last_run_at,
           last_capture_id = excluded.last_capture_id",
        rusqlite::params![key, now, crate::PROMPT_VERSION, capture_id],
    )?;
    Ok(())
}

/// Collect triggers for one authority (oldest first, capped).
pub fn collect_triggers(
    conn: &Connection,
    authority_id: i64,
    slug: &str,
    sidecar_dir: &std::path::Path,
    max_triggers: usize,
) -> Result<(Vec<Trigger>, Vec<TriggerSkip>)> {
    let cursor = get_cursor(conn, slug)?;
    let mut stmt = conn.prepare(
        "SELECT id, kind FROM capture
         WHERE authority_id = ?1 AND id > ?2
           AND kind IN ('email', 'transcript', 'comment')
           AND trust != 'rejected'
         ORDER BY id LIMIT ?3",
    )?;
    let rows: Vec<(i64, String)> = stmt
        .query_map(
            rusqlite::params![authority_id, cursor, max_triggers as i64],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?
        .collect::<rusqlite::Result<_>>()?;
    let mut triggers = Vec::new();
    let mut skipped = Vec::new();
    for (id, kind) in rows {
        let path = sidecar_dir.join(format!("{id}.txt"));
        match std::fs::read_to_string(&path) {
            Ok(text) if !text.trim().is_empty() => triggers.push(Trigger {
                capture_id: id,
                kind,
                text,
                condensed: false,
            }),
            _ => skipped.push(TriggerSkip {
                capture_id: id,
                reason: "missing or empty sidecar",
            }),
        }
    }
    Ok((triggers, skipped))
}

/// A trigger skipped without model input.
#[derive(Debug, Clone)]
pub struct TriggerSkip {
    pub capture_id: i64,
    pub reason: &'static str,
}

/// Condense long trigger texts via map-reduce (short ones pass
/// through verbatim). Cache hits make reruns free.
pub fn condense_triggers<P: ModelPort>(
    port: &P,
    model: &str,
    cache: &SummaryCache,
    triggers: &mut [Trigger],
    summarize_at_tokens: u32,
    chunk_tokens: u32,
) -> Result<()> {
    let summarizer = PortSummarizer { port, model };
    for trigger in triggers.iter_mut() {
        if estimate_tokens(&trigger.text) <= summarize_at_tokens {
            continue;
        }
        let chunks = chunk_text(
            &format!("cap-{}", trigger.capture_id),
            &trigger.text,
            chunk_tokens,
            400,
        );
        let outcome = reduce(
            &summarizer,
            cache,
            &chunks,
            "Condense this evidence for instruction drafting. Keep every name, date, number, and requirement verbatim.",
            2,
            chunk_tokens * 2,
        )?;
        trigger.text = outcome.brief;
        trigger.condensed = true;
    }
    Ok(())
}

struct PortSummarizer<'a, P: ModelPort> {
    port: &'a P,
    model: &'a str,
}

impl<P: ModelPort> Summarizer for PortSummarizer<'_, P> {
    fn summarize(&self, instructions: &str, input: &str) -> atlas::error::Result<SummaryDraft> {
        let out = self.port.complete_chat(&atlas::model::ChatRequest {
            model: self.model.to_string(),
            instructions: instructions.to_string(),
            user_text: input.to_string(),
            schema: None,
            max_output_tokens: 2048,
            // `1.0` is the documented default (sampling untouched);
            // light reasoning fits condensation inside this cap.
            temperature: 1.0,
            reasoning_effort: Some("low".to_string()),
            cache_key: None,
        })?;
        Ok(SummaryDraft {
            text: out.text,
            input_tokens: out.input_tokens,
            output_tokens: out.output_tokens,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_round_trips_and_namespaces() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE watermark(authority_slug TEXT PRIMARY KEY,
             last_run_at INTEGER NOT NULL, last_prompt_version TEXT NOT NULL,
             last_capture_id INTEGER NOT NULL DEFAULT 0)",
        )
        .unwrap();
        assert_eq!(get_cursor(&conn, "wa").unwrap(), 0);
        set_cursor(&conn, "wa", 7, 1000).unwrap();
        assert_eq!(get_cursor(&conn, "wa").unwrap(), 7);
        // The analysis worker's bare key is untouched.
        use rusqlite::OptionalExtension as _;
        let bare: Option<i64> = conn
            .query_row(
                "SELECT last_capture_id FROM watermark WHERE authority_slug = 'wa'",
                [],
                |row| row.get(0),
            )
            .optional()
            .unwrap();
        assert_eq!(bare, None);
        assert_eq!(cursor_key("wa"), "instruct:wa");
    }

    #[test]
    fn condense_passes_short_text_through() {
        let cache = SummaryCache::new(std::env::temp_dir().join("instruct-test-unused"));
        let mut triggers = vec![Trigger {
            capture_id: 1,
            kind: "email".to_string(),
            text: "short".to_string(),
            condensed: false,
        }];
        let port = atlas::model::StubPort::new("SUMMARY", "{}");
        condense_triggers(&port, "test-model", &cache, &mut triggers, 10_000, 8_000).unwrap();
        assert_eq!(triggers[0].text, "short");
        assert!(!triggers[0].condensed);
    }
}
