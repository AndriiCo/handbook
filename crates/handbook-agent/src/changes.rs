//! Drafting from changes only (P394): when the watch stores a changed
//! web capture of a source that has an earlier capture, Muse Spark
//! reads the lines the page ADDED since then (one line of context on
//! each side), never the whole page.
//!
//! The excerpt is built from the newer capture's sidecar, so every
//! cited span maps straight back into that sidecar:
//! [`run_analysis`] then validates and stores exactly as for a full
//! capture (quote-checking, suppression, audit, `proposed` only).
//! A citation that leaves its block, or rests only on context lines,
//! is mapped to an empty range and refused as `BadRange`: a change
//! draft can only cite what changed.
//!
//! [`draft_pending_changes`] walks new web captures past one cursor
//! (`watermark` key [`CURSOR_KEY`]) for whichever process runs it;
//! the caller passes the daily-cap check, so each pipeline keeps its
//! own call accounting.

use std::collections::HashMap;

use rusqlite::{Connection, OptionalExtension};

use contracts::error::HandbookError;
use contracts::model::{
    CitationRef, DraftRequest, DraftResponse, ModelClient, ModelError, ProposedRecord, Usage,
};
use handbook_store::history::{
    DiffLine, Hunk, LineOp, SourceCapture, added_hunks, capture, line_diff, previous_capture,
    sidecar_text,
};
use handbook_store::store::Store;

use crate::runner::{RunReport, RunRequest, run_analysis};

/// Context lines kept around each run of added lines.
pub const CONTEXT_LINES: usize = 1;

/// The excerpt's citation doc id (the newer sidecar, as in staff drafts).
pub const DOC_ID: &str = "fixture";

/// `watermark.authority_slug` key of the pending-changes cursor.
pub const CURSOR_KEY: &str = "changes:web";

/// Captures older than this are history, not news: the cursor passes
/// them without a model call (first run after deploy, long outages).
pub const MAX_AGE_SECS: i64 = 3 * 86_400;

/// Captures examined per [`draft_pending_changes`] call.
pub const BATCH: i64 = 20;

/// Why a capture drafted nothing (no model call was made).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeSkip {
    /// Unknown, unattributed, rejected, or not a web (`warc`) capture.
    NotWebCapture,
    /// The source's first capture: nothing to compare against.
    FirstCapture,
    /// Same bytes re-read by a newer extractor (P360).
    Reread,
    /// A sidecar is missing.
    MissingSidecar,
    /// Lines were only removed or reordered.
    NoAddedLines,
    /// Older than [`MAX_AGE_SECS`] when the cursor reached it.
    Stale,
}

impl ChangeSkip {
    pub fn as_str(self) -> &'static str {
        match self {
            ChangeSkip::NotWebCapture => "not a web capture",
            ChangeSkip::FirstCapture => "first capture of its source",
            ChangeSkip::Reread => "extractor re-read (same bytes)",
            ChangeSkip::MissingSidecar => "missing sidecar",
            ChangeSkip::NoAddedLines => "no added lines",
            ChangeSkip::Stale => "older than the change window",
        }
    }
}

/// One change draft's outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChangeOutcome {
    Skipped(ChangeSkip),
    Drafted {
        previous_id: i64,
        added_lines: usize,
        report: RunReport,
    },
}

/// The model's view of one change: preamble plus the added blocks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeExcerpt {
    pub text: String,
    /// `(excerpt_start, excerpt_end, sidecar_start)` per block.
    blocks: Vec<(usize, usize, usize)>,
    /// Sidecar byte ranges of the added lines themselves.
    added: Vec<(usize, usize)>,
    pub added_lines: usize,
}

impl ChangeExcerpt {
    /// Map an excerpt span to the newer sidecar, or `None` when it
    /// leaves its block or touches no added line.
    pub fn to_sidecar(&self, start: usize, end: usize) -> Option<(usize, usize)> {
        let (ex_start, _, side_start) = *self
            .blocks
            .iter()
            .find(|(s, e, _)| *s <= start && end <= *e && start < end)?;
        let mapped = (start - ex_start + side_start, end - ex_start + side_start);
        self.added
            .iter()
            .any(|(s, e)| mapped.0 < *e && *s < mapped.1)
            .then_some(mapped)
    }
}

/// `YYYY-MM-DD` (UTC) for unix seconds.
pub fn ymd(unix: i64) -> String {
    // Days-to-civil (H. Hinnant), valid across the whole i64 day range we use.
    let z = unix.div_euclid(86_400) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

/// Build the excerpt for `new` (the newer sidecar) from its diff.
pub fn build_excerpt(
    uri: &str,
    from: &SourceCapture,
    to: &SourceCapture,
    lines: &[DiffLine],
    new: &str,
) -> Option<ChangeExcerpt> {
    let hunks: Vec<Hunk> = added_hunks(lines, new, CONTEXT_LINES);
    if hunks.is_empty() {
        return None;
    }
    let rows: Vec<&str> = new.split('\n').collect();
    let mut offsets = Vec::with_capacity(rows.len());
    let mut at = 0;
    for row in &rows {
        offsets.push(at);
        at += row.len() + 1;
    }
    let added: Vec<(usize, usize)> = lines
        .iter()
        .filter(|l| l.op == LineOp::Added)
        .filter_map(|l| l.new_line)
        .filter(|n| *n < rows.len())
        .map(|n| (offsets[n], offsets[n] + rows[n].len()))
        .collect();
    let mut text = format!(
        "CHANGED LINES ONLY. These lines were added to {uri} between {} and {}, \
         with {CONTEXT_LINES} unchanged line of context around each block; the rest of \
         the page did not change and is omitted. Cite only inside the blocks. A closure, \
         outage, suspended or unavailable service, or changed schedule is topic \
         service_notice.\n",
        ymd(from.captured_at),
        ymd(to.captured_at),
    );
    let mut blocks = Vec::with_capacity(hunks.len());
    for hunk in &hunks {
        text.push_str("\n--- block ---\n");
        let ex_start = text.len();
        text.push_str(&new[hunk.byte_start..hunk.byte_end]);
        blocks.push((ex_start, text.len(), hunk.byte_start));
    }
    text.push('\n');
    Some(ChangeExcerpt {
        text,
        blocks,
        added_lines: added.len(),
        added,
    })
}

/// Wraps the drafting client: citations come back in excerpt space
/// and leave in sidecar space (unmappable ones as an empty range,
/// which the validator refuses as `BadRange`).
struct ExcerptClient<'a> {
    inner: &'a dyn ModelClient,
    excerpt: &'a ChangeExcerpt,
    note: String,
}

impl ModelClient for ExcerptClient<'_> {
    fn model_id(&self) -> &'static str {
        self.inner.model_id()
    }

    fn draft_records(&self, req: &DraftRequest) -> Result<DraftResponse, ModelError> {
        let mut resp = self.inner.draft_records(req)?;
        for record in &mut resp.records {
            let (start, end) = self
                .excerpt
                .to_sidecar(
                    record.citation.byte_start as usize,
                    record.citation.byte_end as usize,
                )
                .unwrap_or((0, 0));
            record.citation.byte_start = u32::try_from(start).unwrap_or(0);
            record.citation.byte_end = u32::try_from(end).unwrap_or(0);
        }
        resp.summary = format!("{} {}", self.note, resp.summary);
        Ok(resp)
    }
}

/// Draft facts from what one web capture added since its source's
/// previous capture. Stores `proposed` facts only (via
/// [`run_analysis`]); skips without a model call when there is
/// nothing new to read.
pub fn draft_from_change(
    conn: &Connection,
    store: &Store,
    client: &dyn ModelClient,
    capture_id: i64,
    actor: &str,
    at: i64,
) -> Result<ChangeOutcome, ModelError> {
    let to = capture(conn, capture_id).map_err(db_err)?;
    let Some(to) = to.filter(|c| c.kind == "warc" && c.trust != "rejected") else {
        return Ok(ChangeOutcome::Skipped(ChangeSkip::NotWebCapture));
    };
    let Some(from) = previous_capture(conn, &to).map_err(db_err)? else {
        return Ok(ChangeOutcome::Skipped(ChangeSkip::FirstCapture));
    };
    if from.sha256 == to.sha256 {
        return Ok(ChangeOutcome::Skipped(ChangeSkip::Reread));
    }
    let (Some(old), Some(new)) = (sidecar_text(store, from.id), sidecar_text(store, to.id)) else {
        return Ok(ChangeOutcome::Skipped(ChangeSkip::MissingSidecar));
    };
    let (slug, uri): (String, String) = conn
        .query_row(
            "SELECT a.slug, s.uri FROM source s JOIN authority a ON a.id = s.authority_id
             WHERE s.id = ?1",
            [to.source_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|e| db_err(e.into()))?;
    let lines = line_diff(&old, &new);
    let Some(excerpt) = build_excerpt(&uri, &from, &to, &lines, &new) else {
        return Ok(ChangeOutcome::Skipped(ChangeSkip::NoAddedLines));
    };
    let client = ExcerptClient {
        inner: client,
        excerpt: &excerpt,
        note: format!(
            "Change-only draft: {} added line(s) in capture #{} since #{}.",
            excerpt.added_lines, to.id, from.id
        ),
    };
    let mut sources = HashMap::new();
    sources.insert(DOC_ID.to_string(), new);
    let report = run_analysis(
        conn,
        &client,
        &RunRequest {
            slug: &slug,
            // Same default as staff drafts: captures carry no language tag.
            lang: "en",
            input_text: &excerpt.text,
            sources: &sources,
            actor,
            at,
            capture_id: to.id,
        },
    )?;
    Ok(ChangeOutcome::Drafted {
        previous_id: from.id,
        added_lines: excerpt.added_lines,
        report,
    })
}

/// One capture [`draft_pending_changes`] looked at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingRun {
    pub capture_id: i64,
    pub outcome: Result<ChangeOutcome, String>,
}

/// Draft every new web capture past [`CURSOR_KEY`], oldest first, at
/// most [`BATCH`] per call. `under_cap` is asked before each model
/// call (the caller's `meta.daily_call_cap` check); a `false` stops
/// the walk with the cursor before that capture, so it drafts on a
/// later call. Captures older than [`MAX_AGE_SECS`] and skips pass
/// with no call; a failed draft is reported and passed (staff can
/// re-run it from the diff view), so one bad capture never wedges
/// the cursor or burns the cap on retries.
pub fn draft_pending_changes(
    conn: &Connection,
    store: &Store,
    client: &dyn ModelClient,
    under_cap: &mut dyn FnMut() -> bool,
    actor: &str,
    now: i64,
) -> Result<Vec<PendingRun>, ModelError> {
    let cursor: i64 = conn
        .query_row(
            "SELECT last_capture_id FROM watermark WHERE authority_slug = ?1",
            [CURSOR_KEY],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| db_err(e.into()))?
        .unwrap_or(0);
    let pending: Vec<(i64, i64)> = conn
        .prepare(
            "SELECT id, captured_at FROM capture
             WHERE id > ?1 AND kind = 'warc' AND source_id IS NOT NULL
               AND trust != 'rejected'
             ORDER BY id LIMIT ?2",
        )
        .and_then(|mut stmt| {
            stmt.query_map([cursor, BATCH], |row| Ok((row.get(0)?, row.get(1)?)))?
                .collect::<rusqlite::Result<_>>()
        })
        .map_err(|e| db_err(e.into()))?;
    let mut runs = Vec::new();
    for (capture_id, captured_at) in pending {
        let outcome = if captured_at < now - MAX_AGE_SECS {
            Ok(ChangeOutcome::Skipped(ChangeSkip::Stale))
        } else if !needs_model(conn, store, capture_id)? {
            draft_from_change(conn, store, client, capture_id, actor, now)
                .map_err(|e| e.to_string())
        } else if !under_cap() {
            break;
        } else {
            draft_from_change(conn, store, client, capture_id, actor, now)
                .map_err(|e| e.to_string())
        };
        set_cursor(conn, capture_id, now)?;
        runs.push(PendingRun {
            capture_id,
            outcome,
        });
    }
    Ok(runs)
}

/// True when [`draft_from_change`] would call the model for this
/// capture (so the cap is only consulted for real calls).
fn needs_model(conn: &Connection, store: &Store, capture_id: i64) -> Result<bool, ModelError> {
    let Some(to) = capture(conn, capture_id).map_err(db_err)? else {
        return Ok(false);
    };
    let Some(from) = previous_capture(conn, &to).map_err(db_err)? else {
        return Ok(false);
    };
    if from.sha256 == to.sha256 {
        return Ok(false);
    }
    let (Some(old), Some(new)) = (sidecar_text(store, from.id), sidecar_text(store, to.id)) else {
        return Ok(false);
    };
    Ok(line_diff(&old, &new).iter().any(|l| l.op == LineOp::Added))
}

fn set_cursor(conn: &Connection, capture_id: i64, now: i64) -> Result<(), ModelError> {
    conn.execute(
        "INSERT INTO watermark(authority_slug, last_run_at, last_prompt_version, last_capture_id)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(authority_slug) DO UPDATE SET
           last_run_at = excluded.last_run_at,
           last_capture_id = MAX(last_capture_id, excluded.last_capture_id)",
        rusqlite::params![CURSOR_KEY, now, crate::runner::PROMPT_VERSION, capture_id],
    )
    .map_err(|e| db_err(e.into()))?;
    Ok(())
}

fn db_err(e: HandbookError) -> ModelError {
    ModelError::Transport(format!("change draft db error: {e}"))
}

/// Deterministic change-draft fixture for tests and offline dev, the
/// [`StubClient`](crate::model::StubClient) of this path: one
/// `service_notice` record per evidence line announcing a closure or
/// outage, citing that line in the request evidence.
#[derive(Debug, Clone, Default)]
pub struct NoticeStub;

/// Words that mark a line as a service notice for [`NoticeStub`].
const NOTICE_WORDS: [&str; 6] = [
    "unavailable",
    "closed",
    "suspended",
    "outage",
    "paused",
    "not available",
];

impl ModelClient for NoticeStub {
    fn model_id(&self) -> &'static str {
        "stub"
    }

    fn draft_records(&self, req: &DraftRequest) -> Result<DraftResponse, ModelError> {
        let mut records = Vec::new();
        let mut at = 0;
        for line in req.input_text.split('\n') {
            let start = at;
            at += line.len() + 1;
            let lower = line.to_lowercase();
            // The preamble names the topic words too; it is not evidence.
            if lower.starts_with("changed lines only") || line.trim().chars().count() < 8 {
                continue;
            }
            if NOTICE_WORDS.iter().any(|w| lower.contains(w)) {
                records.push(ProposedRecord {
                    topic: "service_notice".to_string(),
                    field: "notice".to_string(),
                    value: line.trim().to_string(),
                    citation: CitationRef {
                        doc_id: DOC_ID.to_string(),
                        byte_start: start as u32,
                        byte_end: (start + line.len()) as u32,
                    },
                    confidence: 0.9,
                });
            }
        }
        records.truncate(req.max_records as usize);
        Ok(DraftResponse {
            summary: format!("stub: {} notice record(s)", records.len()),
            records,
            usage: Usage {
                input_tokens: 0,
                output_tokens: 0,
                cached_tokens: 0,
                reasoning_tokens: 0,
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use handbook_store::db::{NewAuthority, add_authority, migrate, open};
    use std::cell::RefCell;

    /// The WA apostille hub, before and after the 11/02 outage banner
    /// (shape of the real page: nav, heading, mail-in block, fee).
    pub const WA_HUB_BEFORE: &str = include_str!("../fixtures/wa-hub-2026-11-01.html");
    pub const WA_HUB_AFTER: &str = include_str!("../fixtures/wa-hub-2026-11-02.html");
    const NOV_01: i64 = 1_793_491_200;
    const NOV_02: i64 = 1_793_577_600;

    struct Fixture {
        _dir: tempfile::TempDir,
        store: Store,
        conn: Connection,
        source: i64,
        authority: i64,
    }

    fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path());
        store.init().unwrap();
        let conn = open(&store).unwrap();
        migrate(&conn).unwrap();
        let authority = add_authority(
            &conn,
            &NewAuthority {
                kind: "state",
                country: "USA",
                region: "Washington",
                name: "Washington Secretary of State",
                slug: "usa-wa-sos",
                actor: "test",
                at: 1_793_000_000,
            },
        )
        .unwrap();
        conn.execute(
            "INSERT INTO source(authority_id, kind, uri) VALUES (?1, 'website_official',
             'https://www.sos.wa.gov/corporations-charities/apostilles')",
            [authority],
        )
        .unwrap();
        let source = conn.last_insert_rowid();
        Fixture {
            _dir: dir,
            store,
            conn,
            source,
            authority,
        }
    }

    /// Ingest a capture the way the sweep stores one.
    fn ingest(f: &Fixture, html: &str, at: i64) -> i64 {
        let text = ingest::capture::extract_text(html.as_bytes());
        ingest::capture::ingest_bytes(
            &f.conn,
            &f.store,
            &ingest::capture::IngestInput {
                authority_id: f.authority,
                source_id: Some(f.source),
                kind: "warc",
                trust: "official",
                bytes: html.as_bytes(),
                text: &text,
                captured_at: at,
                actor: "test",
                origin: "https://www.sos.wa.gov/corporations-charities/apostilles",
            },
        )
        .unwrap()
        .capture_id()
    }

    /// Records every request so tests see exactly what the model read.
    struct Spy<'a> {
        inner: &'a dyn ModelClient,
        seen: RefCell<Vec<String>>,
    }

    impl ModelClient for Spy<'_> {
        fn model_id(&self) -> &'static str {
            self.inner.model_id()
        }

        fn draft_records(&self, req: &DraftRequest) -> Result<DraftResponse, ModelError> {
            self.seen.borrow_mut().push(req.input_text.clone());
            self.inner.draft_records(req)
        }
    }

    #[test]
    fn ymd_formats_utc_dates() {
        assert_eq!(ymd(0), "1970-01-01");
        assert_eq!(ymd(NOV_02), "2026-11-02");
        assert_eq!(ymd(NOV_02 + 86_399), "2026-11-02");
        assert_eq!(ymd(951_782_400), "2000-02-29");
    }

    #[test]
    fn wa_outage_banner_drafts_a_proposed_service_notice_from_added_lines_only() {
        let f = fixture();
        let before = ingest(&f, WA_HUB_BEFORE, NOV_01);
        let after = ingest(&f, WA_HUB_AFTER, NOV_02);
        let spy = Spy {
            inner: &NoticeStub,
            seen: RefCell::new(Vec::new()),
        };
        let outcome =
            draft_from_change(&f.conn, &f.store, &spy, after, "test", NOV_02 + 100).unwrap();
        let ChangeOutcome::Drafted {
            previous_id,
            added_lines,
            report,
        } = outcome
        else {
            panic!("want a draft, got {outcome:?}");
        };
        assert_eq!(previous_id, before);
        assert_eq!(added_lines, 1);
        assert_eq!(report.proposed, 1, "{report:?}");

        // The model read the banner and its one line of context, not
        // the fee or the PO box further down the page.
        let seen = spy.seen.borrow();
        assert_eq!(seen.len(), 1);
        assert!(
            seen[0].contains("Apostille services are unavailable"),
            "{}",
            seen[0]
        );
        assert!(seen[0].contains("2026-11-01 and 2026-11-02"), "{}", seen[0]);
        assert!(!seen[0].contains("PO Box 40220"), "{}", seen[0]);
        assert!(!seen[0].contains("$15"), "{}", seen[0]);

        let (topic, statement, status, start, end, capture_id, reasoning): (
            String,
            String,
            String,
            i64,
            i64,
            i64,
            String,
        ) = f
            .conn
            .query_row(
                "SELECT topic, statement, status, citation_start, citation_end, capture_id,
                        reasoning
                 FROM fact",
                [],
                |r| {
                    Ok((
                        r.get(0)?,
                        r.get(1)?,
                        r.get(2)?,
                        r.get(3)?,
                        r.get(4)?,
                        r.get(5)?,
                        r.get(6)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(topic, "service_notice");
        assert_eq!(status, "proposed", "change drafts are never accepted");
        assert_eq!(capture_id, after);
        assert!(
            statement.starts_with("notice: Notice posted 11/02/2026"),
            "{statement}"
        );
        assert!(
            reasoning.starts_with("Change-only draft: 1 added line(s)"),
            "{reasoning}"
        );
        // The citation indexes the newer capture's own sidecar.
        let sidecar = sidecar_text(&f.store, after).unwrap();
        let quote = &sidecar[start as usize..end as usize];
        assert!(
            quote.contains("Apostille services are unavailable"),
            "{quote}"
        );
    }

    #[test]
    fn nothing_new_makes_no_model_call() {
        let f = fixture();
        let first = ingest(&f, WA_HUB_AFTER, NOV_01);
        let spy = Spy {
            inner: &NoticeStub,
            seen: RefCell::new(Vec::new()),
        };
        let skip = |id| draft_from_change(&f.conn, &f.store, &spy, id, "t", 1).unwrap();
        assert_eq!(
            skip(first),
            ChangeOutcome::Skipped(ChangeSkip::FirstCapture)
        );
        // The banner comes down: lines removed only.
        let cleared = ingest(&f, WA_HUB_BEFORE, NOV_02);
        assert_eq!(
            skip(cleared),
            ChangeOutcome::Skipped(ChangeSkip::NoAddedLines)
        );
        // Same bytes, new extractor text (P360 re-read).
        let sha: String = f
            .conn
            .query_row("SELECT sha256 FROM capture WHERE id = ?1", [cleared], |r| {
                r.get(0)
            })
            .unwrap();
        f.conn
            .execute(
                "INSERT INTO capture(authority_id, source_id, captured_at, kind, path,
                                     bytes_raw, bytes_stored, sha256, trust)
                 VALUES (?1, ?2, 3, 'warc', 'blob', 1, 1, ?3, 'official')",
                rusqlite::params![f.authority, f.source, sha],
            )
            .unwrap();
        let reread = f.conn.last_insert_rowid();
        assert_eq!(skip(reread), ChangeOutcome::Skipped(ChangeSkip::Reread));
        assert_eq!(skip(0), ChangeOutcome::Skipped(ChangeSkip::NotWebCapture));
        assert!(spy.seen.borrow().is_empty());
    }

    #[test]
    fn citations_on_context_or_preamble_are_refused() {
        let f = fixture();
        ingest(&f, WA_HUB_BEFORE, NOV_01);
        let after = ingest(&f, WA_HUB_AFTER, NOV_02);
        // StubClient drafts the PO box and hours, neither of which is
        // in the excerpt: both land as rejected, nothing is stored.
        let outcome = draft_from_change(
            &f.conn,
            &f.store,
            &crate::model::StubClient,
            after,
            "t",
            NOV_02 + 100,
        )
        .unwrap();
        let ChangeOutcome::Drafted { report, .. } = outcome else {
            panic!("{outcome:?}");
        };
        assert_eq!(report.proposed, 0);
        let facts: i64 = f
            .conn
            .query_row("SELECT COUNT(*) FROM fact", [], |r| r.get(0))
            .unwrap();
        assert_eq!(facts, 0);

        // Excerpt-space mapping: a span on the context line above the
        // banner, or across a block boundary, maps to nothing.
        let from = capture(&f.conn, after - 1).unwrap().unwrap();
        let to = capture(&f.conn, after).unwrap().unwrap();
        let old = sidecar_text(&f.store, from.id).unwrap();
        let new = sidecar_text(&f.store, to.id).unwrap();
        let excerpt = build_excerpt("u", &from, &to, &line_diff(&old, &new), &new).unwrap();
        let banner = excerpt.text.find("Notice posted").unwrap();
        let mapped = excerpt.to_sidecar(banner, banner + 13).unwrap();
        assert_eq!(&new[mapped.0..mapped.1], "Notice posted");
        let context = excerpt.text.find("Elections").unwrap();
        assert_eq!(
            excerpt.to_sidecar(context, context + 9),
            None,
            "context line"
        );
        assert_eq!(excerpt.to_sidecar(0, 20), None, "preamble");
    }

    #[test]
    fn pending_walk_respects_the_cap_and_never_rereads() {
        let f = fixture();
        let now = NOV_02 + 100;
        ingest(&f, WA_HUB_BEFORE, now - 86_400);
        let after = ingest(&f, WA_HUB_AFTER, now - 60);
        let spy = Spy {
            inner: &NoticeStub,
            seen: RefCell::new(Vec::new()),
        };
        // Cap reached: the first capture passes (no call needed), the
        // changed one waits for budget.
        let runs = draft_pending_changes(&f.conn, &f.store, &spy, &mut || false, "t", now).unwrap();
        assert_eq!(runs.len(), 1);
        assert!(spy.seen.borrow().is_empty());
        // Budget back: exactly one call, then the cursor is past it.
        let mut asked = 0;
        let runs = draft_pending_changes(
            &f.conn,
            &f.store,
            &spy,
            &mut || {
                asked += 1;
                true
            },
            "t",
            now,
        )
        .unwrap();
        assert_eq!(asked, 1);
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].capture_id, after);
        assert!(matches!(runs[0].outcome, Ok(ChangeOutcome::Drafted { .. })));
        let again = draft_pending_changes(&f.conn, &f.store, &spy, &mut || true, "t", now).unwrap();
        assert!(again.is_empty());
        assert_eq!(spy.seen.borrow().len(), 1);
    }

    #[test]
    fn stale_changes_pass_without_a_call() {
        let f = fixture();
        let now = NOV_02 + 100;
        ingest(&f, WA_HUB_BEFORE, now - 10 * 86_400);
        ingest(&f, WA_HUB_AFTER, now - 9 * 86_400);
        let spy = Spy {
            inner: &NoticeStub,
            seen: RefCell::new(Vec::new()),
        };
        let runs = draft_pending_changes(&f.conn, &f.store, &spy, &mut || true, "t", now).unwrap();
        assert_eq!(runs.len(), 2);
        assert!(spy.seen.borrow().is_empty());
    }
}
