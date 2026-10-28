//! Analysis-runner core (S2 lane): draft, validate, store as `proposed`.
//!
//! Pure orchestration around [`ModelClient`](contracts::model::ModelClient):
//! build a draft request, draft records, validate each record against the
//! frozen [`Topic`](contracts::topics::Topic) enum and source texts, and store
//! valid records as `proposed` facts. Never auto-accepts.
//!
//! Whole-response failures (client errors) return `Err` with no writes and
//! no watermark advance. Per-record validation failures are counted as
//! rejected and recorded in [`RunReport::errors`] as
//! `"record {index}: {kind:?}"`.

use std::collections::{HashMap, HashSet};

use rusqlite::{Connection, OptionalExtension};

use crate::analyze::{build_prompt, validate_response};
use contracts::error::HandbookError;
use contracts::model::{DraftRequest, ModelClient, ModelError};
use contracts::topics::Topic;
use handbook_store::db::{
    NewProposedFact, in_transaction, insert_analysis_audit, insert_proposed_fact, upsert_watermark,
};

/// Version of `prompts/aa-prompt-v1.md`, recorded in watermarks and `created_by`.
pub const PROMPT_VERSION: &str = "aa-prompt-v1";

/// Cap on records requested per draft call (the runner takes no caller budget yet).
pub const DEFAULT_MAX_RECORDS: u32 = 32;

/// `fact.capture_id` for draft evidence with no ingest capture row behind it
/// yet. Accepted because `capture_id` has no FK enforcement (see
/// `migrations/001_init.sql`); the CLI should pass a real capture id once
/// ingest exists.
pub const UNATTRIBUTED_CAPTURE_ID: i64 = 0;

/// Outcome of one [`run_analysis`] call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunReport {
    pub authority_slug: String,
    pub proposed: usize,
    pub rejected: usize,
    /// True when the run skipped analysis (unsupported language): no
    /// model call, no facts — only watermark, task, and audit rows.
    pub skipped: bool,
    pub errors: Vec<String>,
}

/// One [`run_analysis`] call's caller-owned inputs.
#[derive(Debug)]
pub struct RunRequest<'a> {
    pub slug: &'a str,
    /// Evidence language tag (`"en"` analyzed; anything else skipped).
    pub lang: &'a str,
    pub input_text: &'a str,
    /// Citation `doc_id` → source text for byte-exact validation.
    pub sources: &'a HashMap<String, String>,
    pub actor: &'a str,
    pub at: i64,
    /// Real capture id, or [`UNATTRIBUTED_CAPTURE_ID`] when hand-fed.
    pub capture_id: i64,
}

/// Draft records for one authority and store the valid ones as `proposed` facts.
///
/// EN-only for the Day-2 slice: any `req.lang` but `"en"` skips analysis
/// (no model call) and opens a `correction` task instead. Always upserts
/// the authority watermark and appends one `action='analysis_run'` audit
/// row on the success path (including fully-rejected, empty, and skipped
/// runs); returns `Err` untouched-storage on client or database failure.
pub fn run_analysis(
    conn: &Connection,
    client: &dyn ModelClient,
    req: &RunRequest<'_>,
) -> Result<RunReport, ModelError> {
    let authority_id = authority_id(conn, req.slug)?;

    // EN-only + unknown-skip (roadmap item 5): never guess-translate a
    // fact out of an unsupported language. The model is not called.
    if req.lang != "en" {
        return skip_unsupported_lang(conn, req, authority_id);
    }

    let draft = DraftRequest {
        authority_slug: req.slug.to_string(),
        prompt_version: PROMPT_VERSION,
        input_text: req.input_text.to_string(),
        max_records: DEFAULT_MAX_RECORDS,
        capture_id: req.capture_id,
    };
    // Rendered for template validation; the current ModelClient shape carries
    // the evidence via DraftRequest, so no consumer takes this string yet.
    let _ = build_prompt(include_str!("../prompts/aa-prompt-v1.md"), &draft);

    // Whole-response failure: no writes, no watermark advance.
    let resp = client.draft_records(&draft)?;

    let allowed: Vec<&str> = Topic::ALL.iter().map(|t| t.as_str()).collect();
    let mut errors = Vec::new();
    let mut failed = HashSet::new();
    if let Err(validation_errors) = validate_response(&resp, req.sources, &allowed) {
        for e in validation_errors {
            errors.push(format!("record {}: {:?}", e.record_index, e.kind));
            failed.insert(e.record_index);
        }
    }

    // The fact table has no field/citation/confidence columns: field folds
    // into the statement, citation/confidence are not stored (see
    // insert_proposed_fact docs).
    let created_by = format!("analysis-agent/{PROMPT_VERSION}");
    // B1: facts + watermark + audit commit atomically — a mid-run
    // failure never leaves content rows without their audit row.
    let (proposed, rejected) = in_transaction(conn, || {
        let mut proposed = 0;
        for (index, record) in resp.records.iter().enumerate() {
            if failed.contains(&index) {
                continue;
            }
            let statement = format!("{}: {}", record.field, record.value);
            insert_proposed_fact(
                conn,
                &NewProposedFact {
                    authority_id,
                    capture_id: req.capture_id,
                    topic: &record.topic,
                    statement: &statement,
                    created_by: &created_by,
                    created_at: req.at,
                },
            )?;
            proposed += 1;
        }
        let rejected = failed.len();
        upsert_watermark(conn, req.slug, req.at, PROMPT_VERSION)?;
        advance_capture_watermark(conn, req.slug, req.capture_id)?;
        let reason = format!(
            "proposed={proposed} rejected={rejected} input_tokens={} output_tokens={}",
            resp.usage.input_tokens, resp.usage.output_tokens
        );
        insert_analysis_audit(conn, req.at, req.actor, authority_id, &reason)?;
        Ok((proposed, rejected))
    })
    .map_err(db_err)?;

    Ok(RunReport {
        authority_slug: req.slug.to_string(),
        proposed,
        rejected,
        skipped: false,
        errors,
    })
}

/// Skip path for unsupported evidence languages: no model call, no
/// facts. Opens (or dedup-hits) one `correction` task, advances the
/// watermark, and writes the audit row atomically.
fn skip_unsupported_lang(
    conn: &Connection,
    req: &RunRequest<'_>,
    authority_id: i64,
) -> Result<RunReport, ModelError> {
    let dedup_key = handbook_store::store::sha256_hex(
        format!("{}|lang-skip|{}", req.slug, req.lang).as_bytes(),
    );
    in_transaction(conn, || {
        match conn.execute(
            "INSERT INTO task(authority_id, kind, title, detail, status,
                               priority, dedup_key, created_by, created_at)
             VALUES (?1, 'correction', ?2, ?3, 'open', 0, ?4, ?5, ?6)",
            rusqlite::params![
                authority_id,
                format!("unsupported evidence language: {}", req.lang),
                "EN-only slice: evidence needs translation before analysis; no fact drafted",
                dedup_key,
                req.actor,
                req.at,
            ],
        ) {
            Ok(_) => Ok(()),
            // A live task with this key already exists: duplicate hit is
            // success, not failure (analysis-agent §2).
            Err(rusqlite::Error::SqliteFailure(e, _))
                if e.code == rusqlite::ErrorCode::ConstraintViolation =>
            {
                Ok(())
            }
            Err(e) => Err(HandbookError::from(e)),
        }?;
        upsert_watermark(conn, req.slug, req.at, PROMPT_VERSION)?;
        advance_capture_watermark(conn, req.slug, req.capture_id)?;
        insert_analysis_audit(
            conn,
            req.at,
            req.actor,
            authority_id,
            &format!("skipped: unsupported lang {} (EN-only slice)", req.lang),
        )?;
        Ok(())
    })
    .map_err(db_err)?;

    Ok(RunReport {
        authority_slug: req.slug.to_string(),
        proposed: 0,
        rejected: 0,
        skipped: true,
        errors: Vec::new(),
    })
}

/// The runner signature only carries [`ModelError`], so database failures
/// map there (a dedicated runner error is follow-up work).
fn db_err(e: HandbookError) -> ModelError {
    ModelError::Transport(format!("runner db error: {e}"))
}

fn authority_id(conn: &Connection, slug: &str) -> Result<i64, ModelError> {
    let found: Option<i64> = conn
        .query_row(
            "SELECT id FROM authority WHERE slug = ?1",
            [slug],
            |row| row.get(0),
        )
        .optional()
        .map_err(HandbookError::from)
        .map_err(db_err)?;
    found.ok_or_else(|| ModelError::Transport(format!("unknown authority slug: {slug}")))
}

/// Raise the Option-B scan floor past one capture (no-op for hand-fed
/// runs). Runs inside the caller's transaction via `MAX()`, so a crash
/// can never strand the watermark ahead of the facts.
fn advance_capture_watermark(
    conn: &Connection,
    slug: &str,
    capture_id: i64,
) -> Result<(), HandbookError> {
    if capture_id <= 0 {
        return Ok(());
    }
    conn.execute(
        "UPDATE watermark SET last_capture_id = MAX(last_capture_id, ?2)
         WHERE authority_slug = ?1",
        rusqlite::params![slug, capture_id],
    )?;
    Ok(())
}

/// Missing-sidecar skip (spec §1): correction task + audit + watermark,
/// atomically. The capture is unprocessable (ingest precondition failed),
/// so the scan floor still advances past it.
pub fn skip_missing_sidecar(
    conn: &Connection,
    slug: &str,
    capture_id: i64,
    actor: &str,
    at: i64,
) -> Result<RunReport, ModelError> {
    let authority_id = authority_id(conn, slug)?;
    let dedup_key =
        handbook_store::store::sha256_hex(format!("{slug}|sidecar-missing|{capture_id}").as_bytes());
    in_transaction(conn, || {
        match conn.execute(
            "INSERT INTO task(authority_id, kind, title, detail, status, priority, dedup_key, created_by, created_at)
             VALUES (?1, 'correction', ?2, ?3, 'open', 0, ?4, ?5, ?6)",
            rusqlite::params![
                authority_id,
                format!("missing sidecar for capture {capture_id}"),
                "sidecar text/<id>.txt absent: ingest precondition failed, nothing analyzed",
                dedup_key,
                actor,
                at
            ],
        ) {
            Ok(_) => Ok(()),
            Err(rusqlite::Error::SqliteFailure(e, _))
                if e.code == rusqlite::ErrorCode::ConstraintViolation =>
            {
                Ok(())
            }
            Err(e) => Err(HandbookError::from(e)),
        }?;
        upsert_watermark(conn, slug, at, PROMPT_VERSION)?;
        advance_capture_watermark(conn, slug, capture_id)?;
        insert_analysis_audit(
            conn,
            at,
            actor,
            authority_id,
            &format!("skipped: missing sidecar for capture {capture_id}"),
        )?;
        Ok(())
    })
    .map_err(db_err)?;
    Ok(RunReport {
        authority_slug: slug.to_string(),
        proposed: 0,
        rejected: 0,
        skipped: true,
        errors: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{MetaRestClient, StubClient};
    use contracts::model::{CitationRef, DraftResponse, ModelError, ProposedRecord, Usage};
    use handbook_store::db::{
        NewAuthority, add_authority, get_capture_watermark, get_watermark, migrate, open,
    };
    use handbook_store::store::Store;

    fn migrated_with_authority(slug: &str) -> (tempfile::TempDir, Store, Connection) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path());
        store.init().unwrap();
        let conn = open(&store).unwrap();
        migrate(&conn).unwrap();
        add_authority(
            &conn,
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
        .unwrap();
        (dir, store, conn)
    }

    #[test]
    fn stores_proposed_facts_watermark_and_audit() {
        let (_dir, _store, conn) = migrated_with_authority("usa-wa-sos");
        let text = "PO Box 40220, Mon-Fri 9-5!";
        let mut sources = HashMap::new();
        sources.insert("fixture".to_string(), text.to_string());
        let req = RunRequest {
            slug: "usa-wa-sos",
            lang: "en",
            input_text: text,
            sources: &sources,
            actor: "test",
            at: 1_800_000_100,
            capture_id: UNATTRIBUTED_CAPTURE_ID,
        };
        let report = run_analysis(&conn, &StubClient, &req).unwrap();
        assert_eq!(report.authority_slug, "usa-wa-sos");
        assert_eq!((report.proposed, report.rejected), (2, 0));
        assert!(!report.skipped);
        assert!(report.errors.is_empty());

        let facts: Vec<(String, String, String)> = conn
            .prepare("SELECT topic, statement, status FROM fact ORDER BY id")
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(
            facts,
            vec![
                (
                    "address".to_string(),
                    "po_box: PO Box 40220".to_string(),
                    "proposed".to_string()
                ),
                (
                    "hours".to_string(),
                    "weekday: Mon-Fri 9-5".to_string(),
                    "proposed".to_string()
                ),
            ]
        );

        assert_eq!(
            get_watermark(&conn, "usa-wa-sos").unwrap(),
            Some((1_800_000_100, PROMPT_VERSION.to_string()))
        );
        let audits: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM audit WHERE action = 'analysis_run'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(audits, 1);
    }

    struct BadTopicClient;

    impl ModelClient for BadTopicClient {
        fn draft_records(&self, _req: &DraftRequest) -> Result<DraftResponse, ModelError> {
            Ok(DraftResponse {
                records: vec![ProposedRecord {
                    topic: "not_a_topic".to_string(),
                    field: "po_box".to_string(),
                    value: "PO Box 99999".to_string(),
                    citation: CitationRef {
                        doc_id: "fixture".to_string(),
                        byte_start: 0,
                        byte_end: 12,
                    },
                    confidence: 0.9,
                }],
                usage: Usage {
                    input_tokens: 1,
                    output_tokens: 1,
                },
            })
        }
    }

    #[test]
    fn rejects_off_enum_topic_without_fact_row() {
        let (_dir, _store, conn) = migrated_with_authority("usa-wa-sos");
        let mut sources = HashMap::new();
        sources.insert("fixture".to_string(), "PO Box 99999!".to_string());
        let req = RunRequest {
            slug: "usa-wa-sos",
            lang: "en",
            input_text: "PO Box 99999!",
            sources: &sources,
            actor: "test",
            at: 1_800_000_200,
            capture_id: UNATTRIBUTED_CAPTURE_ID,
        };
        let report = run_analysis(&conn, &BadTopicClient, &req).unwrap();
        assert_eq!((report.proposed, report.rejected), (0, 1));
        assert!(!report.skipped);
        assert_eq!(report.errors.len(), 1);
        assert!(
            report.errors[0].contains("record 0"),
            "got: {}",
            report.errors[0]
        );
        assert!(
            report.errors[0].contains("UnknownTopic"),
            "got: {}",
            report.errors[0]
        );
        let facts: i64 = conn
            .query_row("SELECT COUNT(*) FROM fact", [], |row| row.get(0))
            .unwrap();
        assert_eq!(facts, 0);
        // The watermark still advances on a fully-rejected run.
        assert!(get_watermark(&conn, "usa-wa-sos").unwrap().is_some());
    }

    fn storage_counts(conn: &Connection) -> (i64, i64, bool) {
        let facts: i64 = conn
            .query_row("SELECT COUNT(*) FROM fact", [], |row| row.get(0))
            .unwrap();
        let audits: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM audit WHERE action = 'analysis_run'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let watermarked = get_watermark(conn, "usa-wa-sos").unwrap().is_some();
        (facts, audits, watermarked)
    }

    #[test]
    fn client_failure_leaves_storage_untouched() {
        let (_dir, _store, conn) = migrated_with_authority("usa-wa-sos");
        let client = MetaRestClient::new("https://api.meta.ai/v1".to_string(), "k".to_string());
        let mut sources = HashMap::new();
        sources.insert("d1".to_string(), "evidence".to_string());
        let req = RunRequest {
            slug: "usa-wa-sos",
            lang: "en",
            input_text: "evidence",
            sources: &sources,
            actor: "test",
            at: 1_800_000_300,
            capture_id: UNATTRIBUTED_CAPTURE_ID,
        };
        let err = run_analysis(&conn, &client, &req).expect_err("client error must propagate");
        assert!(matches!(err, ModelError::Transport(_)), "got: {err}");
        assert_eq!(storage_counts(&conn), (0, 0, false));
    }

    #[test]
    fn unsupported_lang_skips_without_model_call() {
        let (_dir, _store, conn) = migrated_with_authority("usa-wa-sos");
        // MetaRestClient always errors: a skipped run must succeed, which
        // proves the model was never called.
        let client = MetaRestClient::new("https://api.meta.ai/v1".to_string(), "k".to_string());
        let mut sources = HashMap::new();
        sources.insert("d1".to_string(), "evidence".to_string());
        let req = RunRequest {
            slug: "usa-wa-sos",
            lang: "unknown",
            input_text: "evidence",
            sources: &sources,
            actor: "test",
            at: 1_800_000_400,
            capture_id: UNATTRIBUTED_CAPTURE_ID,
        };
        let report = run_analysis(&conn, &client, &req).unwrap();
        assert!(report.skipped);
        assert_eq!((report.proposed, report.rejected), (0, 0));

        let facts: i64 = conn
            .query_row("SELECT COUNT(*) FROM fact", [], |row| row.get(0))
            .unwrap();
        assert_eq!(facts, 0);
        let tasks: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM task WHERE kind = 'correction' AND status = 'open'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(tasks, 1);
        assert!(get_watermark(&conn, "usa-wa-sos").unwrap().is_some());

        // A second skip dedup-hits the same task: still success, still
        // exactly one task row.
        let req = RunRequest {
            slug: "usa-wa-sos",
            lang: "unknown",
            input_text: "evidence",
            sources: &sources,
            actor: "test",
            at: 1_800_000_500,
            capture_id: UNATTRIBUTED_CAPTURE_ID,
        };
        let again = run_analysis(&conn, &client, &req).unwrap();
        assert!(again.skipped);
        let tasks: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM task WHERE kind = 'correction' AND status = 'open'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(tasks, 1);
    }

    #[test]
    fn unknown_slug_leaves_storage_untouched() {
        let (_dir, _store, conn) = migrated_with_authority("usa-wa-sos");
        let text = "PO Box 40220, Mon-Fri 9-5!";
        let mut sources = HashMap::new();
        sources.insert("fixture".to_string(), text.to_string());
        let req = RunRequest {
            slug: "nope",
            lang: "en",
            input_text: text,
            sources: &sources,
            actor: "test",
            at: 1_800_000_300,
            capture_id: UNATTRIBUTED_CAPTURE_ID,
        };
        let err = run_analysis(&conn, &StubClient, &req).expect_err("unknown slug must fail");
        assert!(matches!(err, ModelError::Transport(_)), "got: {err}");
        assert_eq!(storage_counts(&conn), (0, 0, false));
    }

    fn seed_capture(conn: &Connection, authority_slug: &str, trust: &str) -> i64 {
        let authority: i64 = conn
            .query_row(
                "SELECT id FROM authority WHERE slug = ?1",
                [authority_slug],
                |r| r.get(0),
            )
            .unwrap();
        conn.execute(
            "INSERT INTO capture(authority_id, captured_at, kind, path, bytes_raw, bytes_stored, sha256, trust)
             VALUES (?1, 1_800_000_050, 'warc', 'blobs/synthetic', 4, 4, 'synthetic', ?2)",
            rusqlite::params![authority, trust],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    #[test]
    fn stores_real_capture_id_and_advances_capture_watermark() {
        let (_dir, _store, conn) = migrated_with_authority("usa-wa-sos");
        let id = seed_capture(&conn, "usa-wa-sos", "official");
        let text = "PO Box 40220, Mon-Fri 9-5!";
        let mut sources = HashMap::new();
        sources.insert("fixture".to_string(), text.to_string());
        let req = RunRequest {
            slug: "usa-wa-sos",
            lang: "en",
            input_text: text,
            sources: &sources,
            actor: "test",
            at: 1_800_000_100,
            capture_id: id,
        };
        let report = run_analysis(&conn, &StubClient, &req).unwrap();
        assert_eq!((report.proposed, report.rejected), (2, 0));
        let attributed: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM fact WHERE capture_id = ?1",
                [id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(attributed, 2);
        assert_eq!(get_capture_watermark(&conn, "usa-wa-sos").unwrap(), id);
    }

    #[test]
    fn non_en_skip_advances_capture_watermark_without_facts() {
        let (_dir, _store, conn) = migrated_with_authority("usa-wa-sos");
        let id = seed_capture(&conn, "usa-wa-sos", "official");
        let mut sources = HashMap::new();
        sources.insert("fixture".to_string(), "x".to_string());
        let req = RunRequest {
            slug: "usa-wa-sos",
            lang: "es",
            input_text: "x",
            sources: &sources,
            actor: "test",
            at: 1_800_000_100,
            capture_id: id,
        };
        let client = MetaRestClient::new("https://api.meta.ai/v1".to_string(), "k".to_string());
        let report = run_analysis(&conn, &client, &req).unwrap(); // success proves no model call
        assert!(report.skipped);
        let facts: i64 = conn
            .query_row("SELECT COUNT(*) FROM fact", [], |r| r.get(0))
            .unwrap();
        assert_eq!(facts, 0);
        assert_eq!(get_capture_watermark(&conn, "usa-wa-sos").unwrap(), id);
    }

    #[test]
    fn missing_sidecar_skip_files_correction_and_advances() {
        let (_dir, _store, conn) = migrated_with_authority("usa-wa-sos");
        let id = seed_capture(&conn, "usa-wa-sos", "official");
        let report = skip_missing_sidecar(&conn, "usa-wa-sos", id, "test", 1_800_000_100).unwrap();
        assert!(report.skipped);
        let tasks: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM task WHERE kind = 'correction'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(tasks, 1);
        assert_eq!(get_capture_watermark(&conn, "usa-wa-sos").unwrap(), id);
    }
}
