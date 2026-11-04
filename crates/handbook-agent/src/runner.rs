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

/// Evidence languages the runner analyzes (`es`/`fr` read natively off
/// the source page; anything else skips via [`skip_unsupported_lang`]).
/// Day-2 was EN-only; SA-16 adds Spanish (Mexico SRE) and French.
pub const SUPPORTED_LANGS: [&str; 3] = ["en", "es", "fr"];

/// Cap on records requested per draft call, unless the caller passes its
/// own ([`run_analysis_with_max_records`], `xtask analyze --max-records`).
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
    /// Every draft call in order (the refused-quote retry is attempt 2).
    /// Empty on skipped paths, which make no model call.
    pub attempts: Vec<AttemptRecord>,
    /// Every rejected record in chronological order (attempt 1 first).
    /// Each one also lands as an `analyze.rejected` audit row inside
    /// the run's transaction.
    pub rejections: Vec<RejectedRecord>,
}

/// One draft call's recorded outcome (visible-checks): every model
/// call lands here with its own usage split, so the trace console can
/// show `attempt 1: 2 rejected (…) → retry → attempt 2: 3 proposed`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttemptRecord {
    /// 1-based attempt number within the run.
    pub n: u32,
    /// Records the model drafted in this call.
    pub drafted: usize,
    pub proposed: usize,
    pub rejected: usize,
    /// Unique human labels for this attempt's failures, in first-seen
    /// order: validation kinds (`value not in quote`, …), or
    /// `call failed` when the draft call itself errored. A `truncated`
    /// label is reserved but never emitted: the current
    /// [`ModelClient`](contracts::model::ModelClient) contract carries
    /// no truncation signal.
    pub kinds: Vec<String>,
    pub usage: Usage,
}

/// One rejected record, as the `analyze.rejected` audit row and the
/// staff rejections endpoint serve it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RejectedRecord {
    /// 1-based attempt that drafted the record.
    pub attempt: u32,
    /// [`ValidationErrorKind`](crate::analyze::ValidationErrorKind)
    /// debug name (e.g. `ValueNotInQuote`).
    pub kind: String,
    pub topic: String,
    pub field: String,
    /// Record value, truncated to 200 chars.
    pub value: String,
    /// Claimed citation span (`byte_start`, `byte_end`).
    pub span: (u32, u32),
}

/// Cap on stored rejection values (chars, not bytes).
pub const MAX_REJECTION_VALUE_CHARS: usize = 200;

/// Human label for a validation failure kind (trace display).
fn kind_label(kind: &crate::analyze::ValidationErrorKind) -> &'static str {
    use crate::analyze::ValidationErrorKind::*;
    match kind {
        UnknownTopic => "unknown topic",
        MissingDoc => "missing doc",
        BadRange => "bad range",
        BadConfidence => "bad confidence",
        ShortQuote => "short quote",
        LongQuote => "long quote",
        ValueNotInQuote => "value not in quote",
        BadValue => "bad value",
    }
}

/// Truncate a rejection value to [`MAX_REJECTION_VALUE_CHARS`] chars.
fn truncate_value(value: &str) -> String {
    value.chars().take(MAX_REJECTION_VALUE_CHARS).collect()
}

/// Push a failure label once, in first-seen order.
fn push_kind(kinds: &mut Vec<String>, label: &str) {
    if !kinds.iter().any(|k| k == label) {
        kinds.push(label.to_string());
    }
}

/// Build the [`RejectedRecord`] for one validation failure.
fn rejected_record(
    attempt: u32,
    kind: &str,
    record: &contracts::model::ProposedRecord,
) -> RejectedRecord {
    RejectedRecord {
        attempt,
        kind: kind.to_string(),
        topic: record.topic.clone(),
        field: record.field.clone(),
        value: truncate_value(&record.value),
        span: (record.citation.byte_start, record.citation.byte_end),
    }
}

/// One [`run_analysis`] call's caller-owned inputs.
#[derive(Debug)]
pub struct RunRequest<'a> {
    pub slug: &'a str,
    /// Evidence language tag (`SUPPORTED_LANGS` analyzed; anything else skipped).
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
/// Any `req.lang` outside [`SUPPORTED_LANGS`] skips analysis (no model
/// call) and opens a `correction` task instead. Always upserts
/// the authority watermark and appends one `action='analysis_run'` audit
/// row on the success path (including fully-rejected, empty, and skipped
/// runs); returns `Err` untouched-storage on client or database failure.
pub fn run_analysis(
    conn: &Connection,
    client: &dyn ModelClient,
    req: &RunRequest<'_>,
) -> Result<RunReport, ModelError> {
    run_analysis_with_max_records(conn, client, req, None)
}

/// [`run_analysis`] asking for up to `max_records` records per draft
/// call instead of [`DEFAULT_MAX_RECORDS`] (`None` keeps the default;
/// `xtask analyze --max-records`). The refused-quote retry below, like
/// the live client's truncation retry, asks for half of it.
pub fn run_analysis_with_max_records(
    conn: &Connection,
    client: &dyn ModelClient,
    req: &RunRequest<'_>,
    max_records: Option<u32>,
) -> Result<RunReport, ModelError> {
    let authority_id = authority_id(conn, req.slug)?;

    // Supported-langs + unknown-skip (roadmap item 5): never
    // guess-translate a fact out of an unsupported language. The model
    // is not called. `es`/`fr` evidence drafts natively (SA-16).
    if !SUPPORTED_LANGS.contains(&req.lang) {
        return skip_unsupported_lang(conn, req, authority_id);
    }

    let mut doc_ids: Vec<String> = req.sources.keys().cloned().collect();
    doc_ids.sort();
    let draft = DraftRequest {
        authority_slug: req.slug.to_string(),
        prompt_version: PROMPT_VERSION,
        input_text: req.input_text.to_string(),
        max_records: max_records.unwrap_or(DEFAULT_MAX_RECORDS),
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
    // Attempt 1's kinds + rejections (built before the retry extends
    // `resp.records`, so indices below stay attempt-local).
    // `rejection_idx` parallels `rejections` with each record's global
    // index in the merged response (for the audit `after_ref`).
    let mut kinds1 = Vec::new();
    let mut rejections1 = Vec::new();
    let mut rejection_idx = Vec::new();
    for e in &validation_errors {
        push_kind(&mut kinds1, kind_label(&e.kind));
        if let Some(record) = resp.records.get(e.record_index) {
            rejection_idx.push(e.record_index);
            rejections1.push(rejected_record(1, &format!("{:?}", e.kind), record));
        }
    }
    // Per-attempt global index ranges plus kinds/usage, assembled into
    // `attempts` after validation (ranges let proposed/rejected split
    // across the refused-quote retry).
    let mut attempt_meta: Vec<(u32, usize, usize, Vec<String>, Usage)> =
        vec![(1, 0, resp.records.len(), kinds1, resp.usage.clone())];
    let mut rejections = rejections1;
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
                // Attempt 1's usage was snapshotted before the merge
                // below; the merged total still lands in `resp.usage`
                // (audit + report, unchanged).
                resp.usage.input_tokens += retry.usage.input_tokens;
                resp.usage.output_tokens += retry.usage.output_tokens;
                resp.usage.cached_tokens += retry.usage.cached_tokens;
                resp.usage.reasoning_tokens += retry.usage.reasoning_tokens;
                let mut kinds2 = Vec::new();
                if let Err(more) = validate_response(&retry, req.sources, &allowed) {
                    for e in more {
                        errors.push(format!("retry record {}: {:?}", e.record_index, e.kind));
                        failed.insert(base + e.record_index);
                        push_kind(&mut kinds2, kind_label(&e.kind));
                        if let Some(record) = retry.records.get(e.record_index) {
                            rejection_idx.push(base + e.record_index);
                            rejections.push(rejected_record(2, &format!("{:?}", e.kind), record));
                        }
                    }
                }
                let usage2 = retry.usage.clone();
                let drafted2 = retry.records.len();
                resp.records.extend(retry.records);
                attempt_meta.push((2, base, base + drafted2, kinds2, usage2));
            }
            Err(e) => {
                errors.push(format!("retry: {e}"));
                let end = resp.records.len();
                attempt_meta.push((
                    2,
                    end,
                    end,
                    vec!["call failed".to_string()],
                    Usage {
                        input_tokens: 0,
                        output_tokens: 0,
                        cached_tokens: 0,
                        reasoning_tokens: 0,
                    },
                ));
            }
        }
    }
    for e in &validation_errors {
        errors.push(format!("record {}: {:?}", e.record_index, e.kind));
        failed.insert(e.record_index);
    }
    // `rejections` is already chronological: attempt 1's rows were
    // collected above and the retry appended attempt 2's after them.

    // The fact table has no field/confidence columns: field folds
    // into the statement (see insert_proposed_fact docs). The model id
    // leads so the public `drafted_by` carries it (A1: muse-spark-1.3).
    let created_by = format!("{} via analysis-agent/{PROMPT_VERSION}", client.model_id());
    let capture_sha = capture_sha256(conn, req.capture_id);
    // B1: facts + watermark + audit commit atomically — a mid-run
    // failure never leaves content rows without their audit row.
    // Rejection rows ride the same transaction: one `analyze.rejected`
    // audit row per rejected record (kind, topic, field, value ≤200
    // chars, span).
    let (proposed, rejected, suppressed, suppressed_idx, usage) = in_transaction(conn, || {
        let mut proposed = 0;
        let mut suppressed = 0;
        let mut suppressed_idx = HashSet::new();
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
                        "cached_tokens": resp.usage.cached_tokens,
                        "reasoning_tokens": resp.usage.reasoning_tokens,
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
                suppressed_idx.insert(index);
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
            "proposed={proposed} rejected={rejected} suppressed={suppressed} input_tokens={} output_tokens={} cached_tokens={} reasoning_tokens={}",
            resp.usage.input_tokens,
            resp.usage.output_tokens,
            resp.usage.cached_tokens,
            resp.usage.reasoning_tokens
        );
        insert_analysis_audit(conn, req.at, req.actor, authority_id, &reason)?;
        // One `analyze.rejected` row per rejected record, same
        // transaction: the audit trail survives even when every record
        // was rejected (no fact rows to join against later).
        for (index, r) in rejection_idx.iter().zip(rejections.iter()) {
            let reason = serde_json::json!({
                "attempt": r.attempt,
                "kind": r.kind,
                "topic": r.topic,
                "field": r.field,
                "value": r.value,
                "span": [r.span.0, r.span.1],
            })
            .to_string();
            insert_audit(
                conn,
                req.at,
                req.actor,
                "analyze.rejected",
                "authority",
                authority_id,
                &capture_sha,
                &format!("attempt {} record {index}", r.attempt),
                &reason,
            )?;
        }
        Ok((
            proposed,
            rejected,
            suppressed,
            suppressed_idx,
            resp.usage.clone(),
        ))
    })
    .map_err(db_err)?;

    // Split proposed across attempts: each attempt's records minus its
    // failed and suppressed indices.
    let attempts = attempt_meta
        .into_iter()
        .map(|(n, start, end, kinds, usage)| {
            let rejected = failed.iter().filter(|i| start <= **i && **i < end).count();
            let suppressed_here = suppressed_idx
                .iter()
                .filter(|i| start <= **i && **i < end)
                .count();
            AttemptRecord {
                n,
                drafted: end - start,
                proposed: (end - start) - rejected - suppressed_here,
                rejected,
                kinds,
                usage,
            }
        })
        .collect();

    Ok(RunReport {
        authority_slug: req.slug.to_string(),
        proposed,
        rejected,
        suppressed,
        skipped: false,
        errors,
        usage,
        attempts,
        rejections,
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
                "evidence needs translation before analysis; no fact drafted (supported: en, es, fr)",
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
            &format!("skipped: unsupported lang {0} (supported: en, es, fr)", req.lang),
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
            cached_tokens: 0,
            reasoning_tokens: 0,
        },
        // Skipped paths make no model call: no attempts, no rejections.
        attempts: Vec::new(),
        rejections: Vec::new(),
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
            cached_tokens: 0,
            reasoning_tokens: 0,
        },
        // Skipped paths make no model call: no attempts, no rejections.
        attempts: Vec::new(),
        rejections: Vec::new(),
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
    use std::cell::RefCell;

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

    /// Client with a recorded usage block: prefix-cache hits plus
    /// thinking spend, as the Meta wire reports them.
    struct MeteredClient;
    impl ModelClient for MeteredClient {
        fn model_id(&self) -> &'static str {
            "stub"
        }

        fn draft_records(&self, req: &DraftRequest) -> Result<DraftResponse, ModelError> {
            let text = req.input_text.clone();
            let cite = |value: &str| match text.find(value) {
                Some(start) => (start as u32, (start + value.len()) as u32),
                None => (0, 0),
            };
            let (start, end) = cite("PO Box 40220");
            Ok(DraftResponse {
                records: vec![ProposedRecord {
                    topic: "address".to_string(),
                    field: "po_box".to_string(),
                    value: "PO Box 40220".to_string(),
                    citation: CitationRef {
                        doc_id: "fixture".to_string(),
                        byte_start: start,
                        byte_end: end,
                    },
                    confidence: 0.9,
                }],
                usage: Usage {
                    input_tokens: 1200,
                    output_tokens: 800,
                    cached_tokens: 1100,
                    reasoning_tokens: 640,
                },
                summary: String::new(),
            })
        }
    }

    #[test]
    fn report_and_audit_carry_cached_and_reasoning_tokens() {
        let (_dir, _store, conn) = migrated_with_authority("usa-wa-sos");
        let text = "PO Box 40220!";
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
        let report = run_analysis(&conn, &MeteredClient, &req).unwrap();
        assert_eq!(report.proposed, 1);
        assert_eq!(
            report.usage,
            Usage {
                input_tokens: 1200,
                output_tokens: 800,
                cached_tokens: 1100,
                reasoning_tokens: 640,
            }
        );
        let reason: String = conn
            .query_row(
                "SELECT reason FROM audit WHERE action = 'analysis_run'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(reason.contains("cached_tokens=1100"), "got: {reason}");
        assert!(reason.contains("reasoning_tokens=640"), "got: {reason}");
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
                    cached_tokens: 0,
                    reasoning_tokens: 0,
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
        // Single attempt, one rejection with record detail.
        assert_eq!(report.attempts.len(), 1);
        assert_eq!(
            (
                report.attempts[0].n,
                report.attempts[0].drafted,
                report.attempts[0].proposed,
                report.attempts[0].rejected
            ),
            (1, 1, 0, 1)
        );
        assert_eq!(report.attempts[0].kinds, vec!["unknown topic".to_string()]);
        assert_eq!(report.rejections.len(), 1);
        assert_eq!(report.rejections[0].kind, "UnknownTopic");
        assert_eq!(report.rejections[0].value, "PO Box 99999");
        let rejected_rows: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM audit WHERE action = 'analyze.rejected'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(rejected_rows, 1);
        assert_eq!(report.errors.len(), 1);
        // Usage rides along even when every record is rejected.
        assert_eq!(
            report.usage,
            Usage {
                input_tokens: 1,
                output_tokens: 1,
                cached_tokens: 0,
                reasoning_tokens: 0,
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

    /// First call drafts two quote-refused records (attempt 1), the
    /// narrowed retry drafts one valid record (attempt 2).
    struct RefuseThenSucceed;
    impl ModelClient for RefuseThenSucceed {
        fn model_id(&self) -> &'static str {
            "stub"
        }

        fn draft_records(&self, req: &DraftRequest) -> Result<DraftResponse, ModelError> {
            if req.input_text.contains("Validator refused") {
                return Ok(DraftResponse {
                    records: vec![ProposedRecord {
                        topic: "address".to_string(),
                        field: "po_box".to_string(),
                        value: "PO Box 40220".to_string(),
                        citation: CitationRef {
                            doc_id: "fixture".to_string(),
                            byte_start: 0,
                            byte_end: 12,
                        },
                        confidence: 0.9,
                    }],
                    usage: Usage {
                        input_tokens: 10,
                        output_tokens: 20,
                        cached_tokens: 5,
                        reasoning_tokens: 5,
                    },
                    summary: String::new(),
                });
            }
            // Values absent from their quotes: every record refused.
            let refused = |value: &str| ProposedRecord {
                topic: "address".to_string(),
                field: "po_box".to_string(),
                value: value.to_string(),
                citation: CitationRef {
                    doc_id: "fixture".to_string(),
                    byte_start: 0,
                    byte_end: 12,
                },
                confidence: 0.9,
            };
            Ok(DraftResponse {
                records: vec![refused("PO Box 11111"), refused("PO Box 22222")],
                usage: Usage {
                    input_tokens: 100,
                    output_tokens: 50,
                    cached_tokens: 80,
                    reasoning_tokens: 30,
                },
                summary: String::new(),
            })
        }
    }

    #[test]
    fn retry_records_both_attempts_and_rejection_audit_rows() {
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
        let report = run_analysis(&conn, &RefuseThenSucceed, &req).unwrap();
        assert_eq!((report.proposed, report.rejected), (1, 2));
        // Both attempts recorded, usage split per call.
        assert_eq!(report.attempts.len(), 2);
        let first = &report.attempts[0];
        assert_eq!(
            (first.n, first.drafted, first.proposed, first.rejected),
            (1, 2, 0, 2)
        );
        assert_eq!(first.kinds, vec!["value not in quote".to_string()]);
        assert_eq!(
            first.usage,
            Usage {
                input_tokens: 100,
                output_tokens: 50,
                cached_tokens: 80,
                reasoning_tokens: 30,
            }
        );
        let second = &report.attempts[1];
        assert_eq!(
            (second.n, second.drafted, second.proposed, second.rejected),
            (2, 1, 1, 0)
        );
        assert!(second.kinds.is_empty());
        // Totals still merge across the retry (audit + report unchanged).
        assert_eq!(
            report.usage,
            Usage {
                input_tokens: 110,
                output_tokens: 70,
                cached_tokens: 85,
                reasoning_tokens: 35,
            }
        );
        // Chronological rejections with record detail.
        assert_eq!(report.rejections.len(), 2);
        for r in &report.rejections {
            assert_eq!(r.attempt, 1);
            assert_eq!(r.kind, "ValueNotInQuote");
            assert_eq!((r.topic.as_str(), r.field.as_str()), ("address", "po_box"));
            assert_eq!(r.span, (0, 12));
        }
        assert_eq!(report.rejections[0].value, "PO Box 11111");
        assert_eq!(report.rejections[1].value, "PO Box 22222");
        // One `analyze.rejected` audit row per rejected record, inside
        // the run's transaction (plus the single `analysis_run` row).
        let rows: Vec<(String, String, String)> = conn
            .prepare(
                "SELECT action, after_ref, reason FROM audit WHERE action = 'analyze.rejected' ORDER BY id",
            )
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].1, "attempt 1 record 0");
        assert_eq!(rows[1].1, "attempt 1 record 1");
        for (action, _, reason) in &rows {
            assert_eq!(action, "analyze.rejected");
            let body: serde_json::Value = serde_json::from_str(reason).unwrap();
            assert_eq!(body["kind"], "ValueNotInQuote");
            assert_eq!(body["topic"], "address");
            assert_eq!(body["field"], "po_box");
            assert!(body["value"].as_str().unwrap().starts_with("PO Box"));
            assert_eq!(body["span"], serde_json::json!([0, 12]));
            assert_eq!(body["attempt"], 1);
        }
        let runs: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM audit WHERE action = 'analysis_run'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(runs, 1);
    }

    /// Refused attempt 1, then the retry call itself fails: attempt 2 is
    /// still recorded (nothing drafted, call failed).
    struct RefuseThenFail;
    impl ModelClient for RefuseThenFail {
        fn model_id(&self) -> &'static str {
            "stub"
        }

        fn draft_records(&self, req: &DraftRequest) -> Result<DraftResponse, ModelError> {
            if req.input_text.contains("Validator refused") {
                return Err(ModelError::Transport("retry boom".to_string()));
            }
            Ok(DraftResponse {
                records: vec![ProposedRecord {
                    topic: "address".to_string(),
                    field: "po_box".to_string(),
                    value: "PO Box 11111".to_string(),
                    citation: CitationRef {
                        doc_id: "fixture".to_string(),
                        byte_start: 0,
                        byte_end: 12,
                    },
                    confidence: 0.9,
                }],
                usage: Usage {
                    input_tokens: 100,
                    output_tokens: 50,
                    cached_tokens: 0,
                    reasoning_tokens: 0,
                },
                summary: String::new(),
            })
        }
    }

    #[test]
    fn failed_retry_call_still_records_attempt_two() {
        let (_dir, _store, conn) = migrated_with_authority("usa-wa-sos");
        let text = "PO Box 40220!";
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
        let report = run_analysis(&conn, &RefuseThenFail, &req).unwrap();
        assert_eq!((report.proposed, report.rejected), (0, 1));
        assert_eq!(report.attempts.len(), 2);
        assert_eq!((report.attempts[1].n, report.attempts[1].drafted), (2, 0));
        assert_eq!(report.attempts[1].kinds, vec!["call failed".to_string()]);
        assert!(report.errors.iter().any(|e| e.contains("retry: ")));
    }

    /// Records every call's `max_records`, so tests see the budget each
    /// draft asked the model for (attempt 1 first).
    struct BudgetSpy<'a> {
        inner: &'a dyn ModelClient,
        seen: RefCell<Vec<u32>>,
    }

    impl ModelClient for BudgetSpy<'_> {
        fn model_id(&self) -> &'static str {
            self.inner.model_id()
        }

        fn draft_records(&self, req: &DraftRequest) -> Result<DraftResponse, ModelError> {
            self.seen.borrow_mut().push(req.max_records);
            self.inner.draft_records(req)
        }
    }

    fn budget_spy(inner: &dyn ModelClient) -> BudgetSpy<'_> {
        BudgetSpy {
            inner,
            seen: RefCell::new(Vec::new()),
        }
    }

    #[test]
    fn max_records_override_reaches_the_draft_request() {
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
        // No override: the draft asks for the default.
        let spy = budget_spy(&StubClient);
        run_analysis(&conn, &spy, &req).unwrap();
        assert_eq!(spy.seen.into_inner(), vec![DEFAULT_MAX_RECORDS]);
        let spy = budget_spy(&StubClient);
        run_analysis_with_max_records(&conn, &spy, &req, None).unwrap();
        assert_eq!(spy.seen.into_inner(), vec![DEFAULT_MAX_RECORDS]);
        // `--max-records 24`: the draft asks for 24.
        let spy = budget_spy(&StubClient);
        run_analysis_with_max_records(&conn, &spy, &req, Some(24)).unwrap();
        assert_eq!(spy.seen.into_inner(), vec![24]);
        // The budget binds: the stub's two records cap at one.
        let report = run_analysis_with_max_records(&conn, &StubClient, &req, Some(1)).unwrap();
        assert_eq!((report.proposed, report.rejected), (1, 0));
    }

    #[test]
    fn refused_quote_retry_halves_the_override() {
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
        let spy = budget_spy(&RefuseThenSucceed);
        run_analysis(&conn, &spy, &req).unwrap();
        assert_eq!(
            spy.seen.into_inner(),
            vec![DEFAULT_MAX_RECORDS, DEFAULT_MAX_RECORDS / 2]
        );
        let spy = budget_spy(&RefuseThenSucceed);
        let report = run_analysis_with_max_records(&conn, &spy, &req, Some(24)).unwrap();
        assert_eq!(spy.seen.into_inner(), vec![24, 12]);
        assert_eq!((report.proposed, report.rejected), (1, 2));
        // A budget of one has nothing to halve: no retry.
        let spy = budget_spy(&RefuseThenSucceed);
        let report = run_analysis_with_max_records(&conn, &spy, &req, Some(1)).unwrap();
        assert_eq!(spy.seen.into_inner(), vec![1]);
        assert_eq!((report.proposed, report.attempts.len()), (0, 1));
    }

    /// A 300-char rejected value stores at most 200 chars, in both the
    /// report and the audit row.
    struct LongBadValue;
    impl ModelClient for LongBadValue {
        fn model_id(&self) -> &'static str {
            "stub"
        }

        fn draft_records(&self, _req: &DraftRequest) -> Result<DraftResponse, ModelError> {
            Ok(DraftResponse {
                records: vec![ProposedRecord {
                    topic: "not_a_topic".to_string(),
                    field: "po_box".to_string(),
                    value: "v".repeat(300),
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
                    cached_tokens: 0,
                    reasoning_tokens: 0,
                },
                summary: String::new(),
            })
        }
    }

    #[test]
    fn rejection_values_cap_at_200_chars() {
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
        let report = run_analysis(&conn, &LongBadValue, &req).unwrap();
        // One record, two failures (unknown topic + value not in quote):
        // both rejections cap the value.
        assert_eq!(report.rejections.len(), 2);
        for r in &report.rejections {
            assert_eq!(r.value.chars().count(), 200);
        }
        let reasons: Vec<String> = conn
            .prepare("SELECT reason FROM audit WHERE action = 'analyze.rejected' ORDER BY id")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(reasons.len(), 2);
        for reason in &reasons {
            let body: serde_json::Value = serde_json::from_str(reason).unwrap();
            assert_eq!(body["value"].as_str().unwrap().chars().count(), 200);
        }
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
    fn unsupported_lang_skip_advances_capture_watermark_without_facts() {
        let (_dir, _store, conn) = migrated_with_authority("usa-wa-sos");
        let id = seed_capture(&conn, "usa-wa-sos", "official");
        let mut sources = HashMap::new();
        sources.insert("fixture".to_string(), "x".to_string());
        let req = RunRequest {
            slug: "usa-wa-sos",
            lang: "de",
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

    /// SA-16: Spanish evidence drafts natively instead of skipping. The
    /// stub's fixture values ride inside Spanish prose, so a successful
    /// run proves the model was called and the quotes validated.
    #[test]
    fn es_evidence_is_analyzed_not_skipped() {
        let (_dir, _store, conn) = migrated_with_authority("usa-wa-sos");
        let text = "Envíe su solicitud al PO Box 40220. Horario de atención: Mon-Fri 9-5.";
        let mut sources = HashMap::new();
        sources.insert("fixture".to_string(), text.to_string());
        let req = RunRequest {
            slug: "usa-wa-sos",
            lang: "es",
            input_text: text,
            sources: &sources,
            actor: "test",
            at: 1_800_000_100,
            capture_id: UNATTRIBUTED_CAPTURE_ID,
        };
        let report = run_analysis(&conn, &StubClient, &req).unwrap();
        assert!(!report.skipped);
        assert_eq!((report.proposed, report.rejected), (2, 0));
    }

    /// SA-16: French evidence drafts natively instead of skipping (same
    /// shape as the Spanish case above).
    #[test]
    fn fr_evidence_is_analyzed_not_skipped() {
        let (_dir, _store, conn) = migrated_with_authority("usa-wa-sos");
        let text = "Envoyez votre dossier au PO Box 40220. Heures d'ouverture : Mon-Fri 9-5.";
        let mut sources = HashMap::new();
        sources.insert("fixture".to_string(), text.to_string());
        let req = RunRequest {
            slug: "usa-wa-sos",
            lang: "fr",
            input_text: text,
            sources: &sources,
            actor: "test",
            at: 1_800_000_100,
            capture_id: UNATTRIBUTED_CAPTURE_ID,
        };
        let report = run_analysis(&conn, &StubClient, &req).unwrap();
        assert!(!report.skipped);
        assert_eq!((report.proposed, report.rejected), (2, 0));
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
