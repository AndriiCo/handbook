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

use crate::analyze::{
    ValidationErrorKind::{ShortQuote, ValueNotInQuote},
    build_prompt, validate_response,
};
use contracts::error::HandbookError;
use contracts::model::{DraftRequest, ModelClient, ModelError, Usage};
use contracts::topics::Topic;
use handbook_store::db::{
    NewProposedFact, SuppressionHit, check_suppression, in_transaction, insert_analysis_audit,
    insert_audit, insert_proposed_fact, insert_suppression, normalize_statement, upsert_watermark,
};

/// Version of `prompts/aa-prompt-v1.md`, recorded in watermarks and `created_by`.
pub const PROMPT_VERSION: &str = "aa-prompt-v1";

/// Cap on records requested per draft call (the runner takes no caller budget yet).
/// Records per draft call. Spark reasons per record, so 32 blows past
/// even a 32k completion budget (`finish_reason = length` on staging);
/// 8 fits and still clears the 8-facts-per-slug bar across captures.
pub const DEFAULT_MAX_RECORDS: u32 = 8;

/// `fact.capture_id` for draft evidence with no ingest capture row behind it
/// yet. Accepted because `capture_id` has no FK enforcement (see
/// `migrations/schema_baseline.sql`); the CLI should pass a real capture id once
/// ingest exists.
pub const UNATTRIBUTED_CAPTURE_ID: i64 = 0;

/// Outcome of one [`run_analysis`] call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunReport {
    pub authority_slug: String,
    pub proposed: usize,
    pub rejected: usize,
    /// Valid records skipped by suppression (§11.3): logged, never
    /// inserted. Disjoint from `rejected` (validation failures).
    pub suppressed: usize,
    /// True when the run skipped analysis (unsupported language): no
    /// model call, no facts — only watermark, task, and audit rows.
    pub skipped: bool,
    pub errors: Vec<String>,
    /// Billed tokens for the draft call(s), retry included; zero when
    /// no model call ran (skipped paths).
    pub usage: Usage,
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

    let mut doc_ids: Vec<String> = req.sources.keys().cloned().collect();
    doc_ids.sort();
    let draft = DraftRequest {
        authority_slug: req.slug.to_string(),
        prompt_version: PROMPT_VERSION,
        input_text: req.input_text.to_string(),
        max_records: DEFAULT_MAX_RECORDS,
        capture_id: req.capture_id,
        doc_ids,
    };
    // Rendered for template validation; the current ModelClient shape carries
    // the evidence via DraftRequest, so no consumer takes this string yet.
    let _ = build_prompt(include_str!("../prompts/aa-prompt-v1.md"), &draft);

    // Whole-response failure: no writes, no watermark advance.
    let mut resp = client.draft_records(&draft)?;

    let allowed: Vec<&str> = Topic::ALL.iter().map(|t| t.as_str()).collect();
    let mut errors = Vec::new();
    let mut failed = HashSet::new();
    let validation_errors = validate_response(&resp, req.sources, &allowed)
        .err()
        .unwrap_or_default();
    // Refused-quote retry (operator fix `live-draft-first`): every record
    // failed on its quote and none is usable — ask once more with fewer
    // records and the refusal reasons attached. The note is appended
    // after the evidence, so existing citation offsets stay valid.
    if !validation_errors.is_empty()
        && validation_errors.len() == resp.records.len()
        && validation_errors
            .iter()
            .all(|e| matches!(e.kind, ShortQuote | ValueNotInQuote))
        && draft.max_records > 1
    {
        let reasons = validation_errors
            .iter()
            .map(|e| format!("record {}: {:?}", e.record_index, e.kind))
            .collect::<Vec<_>>()
            .join("; ");
        let narrowed = DraftRequest {
            input_text: format!(
                "{}\n\nValidator refused every record ({reasons}). Draft half as many records and keep every quote on the cited page.",
                draft.input_text
            ),
            max_records: (draft.max_records / 2).max(1),
            ..draft.clone()
        };
        match client.draft_records(&narrowed) {
            Ok(retry) => {
                let base = resp.records.len();
                resp.usage.input_tokens += retry.usage.input_tokens;
                resp.usage.output_tokens += retry.usage.output_tokens;
                if let Err(more) = validate_response(&retry, req.sources, &allowed) {
                    for e in more {
                        errors.push(format!("retry record {}: {:?}", e.record_index, e.kind));
                        failed.insert(base + e.record_index);
                    }
                }
                resp.records.extend(retry.records);
            }
            Err(e) => errors.push(format!("retry: {e}")),
        }
    }
    for e in &validation_errors {
        errors.push(format!("record {}: {:?}", e.record_index, e.kind));
        failed.insert(e.record_index);
    }

    // The fact table has no field/confidence columns: field folds
    // into the statement (see insert_proposed_fact docs). The model id
    // leads so the public `drafted_by` carries it (A1: muse-spark-1.3).
    let created_by = format!("{} via analysis-agent/{PROMPT_VERSION}", client.model_id());
    let capture_sha = capture_sha256(conn, req.capture_id);
    // B1: facts + watermark + audit commit atomically — a mid-run
    // failure never leaves content rows without their audit row.
    let (proposed, rejected, suppressed, usage) = in_transaction(conn, || {
        let mut proposed = 0;
        let mut suppressed = 0;
        for (index, record) in resp.records.iter().enumerate() {
            if failed.contains(&index) {
                continue;
            }
            let statement = format!("{}: {}", record.field, record.value);
            // Consultation (§11.3): after validation, before insert.
            let norm = normalize_statement(&statement);
            if let Some(hit) = check_suppression(conn, authority_id, &record.topic, &norm)? {
                let suppression_id = insert_suppression(
                    conn,
                    req.capture_id,
                    authority_id,
                    &record.topic,
                    &statement,
                    &norm,
                    hit,
                    req.at,
                )?;
                let (path, rule_id) = match hit {
                    SuppressionHit::RejectedMemory => ("A", serde_json::json!(null)),
                    SuppressionHit::Rule(id) => ("B", serde_json::json!(id)),
                };
                let reason = serde_json::json!({
                    "path": path,
                    "rule_id": rule_id,
                    "topic": record.topic,
                    "statement_norm": norm,
                    "usage": {
                        "input_tokens": resp.usage.input_tokens,
                        "output_tokens": resp.usage.output_tokens,
                    },
                })
                .to_string();
                insert_audit(
                    conn,
                    req.at,
                    &created_by,
                    "analyze.suppressed",
                    "capture",
                    req.capture_id,
                    &capture_sha,
                    &format!("suppression:{suppression_id}"),
                    &reason,
                )?;
                suppressed += 1;
                continue;
            }
            // Citations index the capture sidecar — but only when the
            // run is capture-bound (scan mode). Hand-fed runs carry
            // UNATTRIBUTED_CAPTURE_ID: their offsets have no sidecar
            // home and store as uncited (-1/-1).
            let citation = (req.capture_id != UNATTRIBUTED_CAPTURE_ID)
                .then_some((record.citation.byte_start, record.citation.byte_end));
            insert_proposed_fact(
                conn,
                &NewProposedFact {
                    authority_id,
                    capture_id: req.capture_id,
                    topic: &record.topic,
                    statement: &statement,
                    created_by: &created_by,
                    created_at: req.at,
                    citation,
                    reasoning: &resp.summary,
                },
            )?;
            proposed += 1;
        }
        let rejected = failed.len();
        upsert_watermark(conn, req.slug, req.at, PROMPT_VERSION)?;
        advance_capture_watermark(conn, req.slug, req.capture_id)?;
        let reason = format!(
            "proposed={proposed} rejected={rejected} suppressed={suppressed} input_tokens={} output_tokens={}",
            resp.usage.input_tokens, resp.usage.output_tokens
        );
        insert_analysis_audit(conn, req.at, req.actor, authority_id, &reason)?;
        Ok((proposed, rejected, suppressed, resp.usage.clone()))
    })
    .map_err(db_err)?;

    Ok(RunReport {
        authority_slug: req.slug.to_string(),
        proposed,
        rejected,
        suppressed,
        skipped: false,
        errors,
        usage,
    })
}

/// `capture.sha256` for suppression `before_ref` ("" when unbound or
/// the row is gone — the audit must never fail the run).
fn capture_sha256(conn: &Connection, capture_id: i64) -> String {
    conn.query_row(
        "SELECT sha256 FROM capture WHERE id = ?1",
        [capture_id],
        |row| row.get::<_, String>(0),
    )
    .unwrap_or_default()
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
        suppressed: 0,
        skipped: true,
        errors: Vec::new(),
        usage: Usage {
            input_tokens: 0,
            output_tokens: 0,
        },
    })
}

/// The runner signature only carries [`ModelError`], so database failures
/// map there (a dedicated runner error is follow-up work).
fn db_err(e: HandbookError) -> ModelError {
    ModelError::Transport(format!("runner db error: {e}"))
}

fn authority_id(conn: &Connection, slug: &str) -> Result<i64, ModelError> {
    let found: Option<i64> = conn
        .query_row("SELECT id FROM authority WHERE slug = ?1", [slug], |row| {
            row.get(0)
        })
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
    let dedup_key = handbook_store::store::sha256_hex(
        format!("{slug}|sidecar-missing|{capture_id}").as_bytes(),
    );
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
        suppressed: 0,
        skipped: true,
        errors: Vec::new(),
        usage: Usage {
            input_tokens: 0,
            output_tokens: 0,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::StubClient;
    use contracts::model::{
        CitationRef, DraftRequest, DraftResponse, ModelError, ProposedRecord, Usage,
    };
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
                summary: String::new(),
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
        // Usage rides along even when every record is rejected.
        assert_eq!(
            report.usage,
            Usage {
                input_tokens: 1,
                output_tokens: 1
            }
        );
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

    /// Deterministic client failure (no network, no sleeps).
    struct FailingClient;
    impl ModelClient for FailingClient {
        fn draft_records(&self, _req: &DraftRequest) -> Result<DraftResponse, ModelError> {
            Err(ModelError::Transport("boom".to_string()))
        }
    }

    /// Proves the model is never called: any call panics.
    struct PanickingClient;
    impl ModelClient for PanickingClient {
        fn draft_records(&self, _req: &DraftRequest) -> Result<DraftResponse, ModelError> {
            panic!("model must not be called");
        }
    }

    #[test]
    fn client_failure_leaves_storage_untouched() {
        let (_dir, _store, conn) = migrated_with_authority("usa-wa-sos");
        let client = FailingClient;
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
        // A skipped run must succeed against a panicking client, which
        // proves the model was never called.
        let client = PanickingClient;
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

    fn bound_request<'a>(
        sources: &'a HashMap<String, String>,
        text: &'a str,
        capture_id: i64,
    ) -> RunRequest<'a> {
        RunRequest {
            slug: "usa-wa-sos",
            lang: "en",
            input_text: text,
            sources,
            actor: "test",
            at: 1_800_000_100,
            capture_id,
        }
    }

    #[test]
    fn suppression_path_a_skips_rejected_restatement() {
        let (_dir, _store, conn) = migrated_with_authority("usa-wa-sos");
        let id = seed_capture(&conn, "usa-wa-sos", "official");
        let auth: i64 = conn
            .query_row(
                "SELECT id FROM authority WHERE slug = 'usa-wa-sos'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        // The stub's address record restates this rejected claim.
        conn.execute(
            "INSERT INTO fact(authority_id, capture_id, topic, statement, status, created_at)
             VALUES (?1, ?2, 'address', 'po_box:  PO Box 40220', 'rejected', 0)",
            rusqlite::params![auth, id],
        )
        .unwrap();
        let text = "PO Box 40220, Mon-Fri 9-5!";
        let mut sources = HashMap::new();
        sources.insert("fixture".to_string(), text.to_string());
        let report = run_analysis(&conn, &StubClient, &bound_request(&sources, text, id)).unwrap();
        assert_eq!(report.proposed, 1);
        assert_eq!(report.suppressed, 1);
        assert_eq!(report.rejected, 0);
        let (path, rule_id): (String, Option<i64>) = conn
            .query_row("SELECT path, rule_id FROM suppression", [], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .unwrap();
        assert_eq!(path, "A");
        assert_eq!(rule_id, None);
        let actions: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM audit WHERE action = 'analyze.suppressed'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(actions, 1);
    }

    #[test]
    fn suppression_path_b_rule_skips_and_logs_rule_id() {
        let (_dir, _store, conn) = migrated_with_authority("usa-wa-sos");
        let id = seed_capture(&conn, "usa-wa-sos", "official");
        conn.execute(
            "INSERT INTO suppress_rule(statement_norm, statement, created_at)
             VALUES ('weekday: mon-fri 9-5', 'weekday: Mon-Fri 9-5', 0)",
            [],
        )
        .unwrap();
        let text = "PO Box 40220, Mon-Fri 9-5!";
        let mut sources = HashMap::new();
        sources.insert("fixture".to_string(), text.to_string());
        let report = run_analysis(&conn, &StubClient, &bound_request(&sources, text, id)).unwrap();
        assert_eq!((report.proposed, report.suppressed), (1, 1));
        let (path, rule_id): (String, Option<i64>) = conn
            .query_row("SELECT path, rule_id FROM suppression", [], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .unwrap();
        assert_eq!(path, "B");
        assert_eq!(rule_id, Some(1));
    }

    #[test]
    fn bound_run_stores_citations_and_reasoning() {
        let (_dir, _store, conn) = migrated_with_authority("usa-wa-sos");
        let id = seed_capture(&conn, "usa-wa-sos", "official");
        let text = "PO Box 40220, Mon-Fri 9-5!";
        let mut sources = HashMap::new();
        sources.insert("fixture".to_string(), text.to_string());
        run_analysis(&conn, &StubClient, &bound_request(&sources, text, id)).unwrap();
        let rows: Vec<(i64, i64, String)> = conn
            .prepare("SELECT citation_start, citation_end, reasoning FROM fact ORDER BY id")
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(
            rows,
            vec![
                (0, 12, "stub: two fixture records".to_string()),
                (14, 25, "stub: two fixture records".to_string()),
            ]
        );
    }

    #[test]
    fn created_by_names_model_and_prompt() {
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
        assert_eq!(report.proposed, 2);
        let by: String = conn
            .query_row(
                "SELECT created_by FROM fact ORDER BY id LIMIT 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(by, format!("stub via analysis-agent/{PROMPT_VERSION}"));
    }

    #[test]
    fn unbound_run_stores_uncited_but_keeps_reasoning() {
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
        run_analysis(&conn, &StubClient, &req).unwrap();
        let rows: Vec<(i64, i64, String)> = conn
            .prepare("SELECT citation_start, citation_end, reasoning FROM fact ORDER BY id")
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(rows.len(), 2);
        for (start, end, reasoning) in &rows {
            assert_eq!((*start, *end), (-1, -1));
            assert_eq!(reasoning, "stub: two fixture records");
        }
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
        let client = PanickingClient;
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
