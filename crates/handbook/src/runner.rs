//! Analysis-runner core (S2 lane): draft, validate, store as `proposed`.
//!
//! Pure orchestration around [`ModelClient`](crate::model::ModelClient):
//! build a draft request, draft records, validate each record against the
//! frozen [`Topic`](crate::topics::Topic) enum and source texts, and store
//! valid records as `proposed` facts. Never auto-accepts.
//!
//! Whole-response failures (client errors) return `Err` with no writes and
//! no watermark advance. Per-record validation failures are counted as
//! rejected and recorded in [`RunReport::errors`] as
//! `"record {index}: {kind:?}"`.

use std::collections::{HashMap, HashSet};

use rusqlite::{Connection, OptionalExtension};

use crate::analyze::{build_prompt, validate_response};
use crate::db::{NewProposedFact, insert_analysis_audit, insert_proposed_fact, upsert_watermark};
use crate::model::{DraftRequest, ModelClient, ModelError};
use crate::topics::Topic;

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
    pub errors: Vec<String>,
}

/// Draft records for one authority and store the valid ones as `proposed` facts.
///
/// `sources` maps citation `doc_id` to source text for byte-exact citation
/// validation. Always upserts the authority watermark and appends one
/// `action='analysis_run'` audit row on the success path (including
/// fully-rejected and empty runs); returns `Err` untouched-storage on client
/// or database failure.
pub fn run_analysis(
    conn: &Connection,
    client: &dyn ModelClient,
    slug: &str,
    input_text: &str,
    sources: &HashMap<String, String>,
    actor: &str,
    at: i64,
) -> Result<RunReport, ModelError> {
    let req = DraftRequest {
        authority_slug: slug.to_string(),
        prompt_version: PROMPT_VERSION,
        input_text: input_text.to_string(),
        max_records: DEFAULT_MAX_RECORDS,
    };
    // Rendered for template validation; the current ModelClient shape carries
    // the evidence via DraftRequest, so no consumer takes this string yet.
    let _ = build_prompt(include_str!("../prompts/aa-prompt-v1.md"), &req);

    // Whole-response failure: no writes, no watermark advance.
    let resp = client.draft_records(&req)?;

    let allowed: Vec<&str> = Topic::ALL.iter().map(|t| t.as_str()).collect();
    let mut errors = Vec::new();
    let mut failed = HashSet::new();
    if let Err(validation_errors) = validate_response(&resp, sources, &allowed) {
        for e in validation_errors {
            errors.push(format!("record {}: {:?}", e.record_index, e.kind));
            failed.insert(e.record_index);
        }
    }

    let authority_id: Option<i64> = conn
        .query_row("SELECT id FROM authority WHERE slug = ?1", [slug], |row| {
            row.get(0)
        })
        .optional()
        .map_err(db_err)?;
    let Some(authority_id) = authority_id else {
        return Err(ModelError::Transport(format!(
            "unknown authority slug: {slug}"
        )));
    };

    // The fact table has no field/citation/confidence columns: field folds
    // into the statement, citation/confidence are not stored (see
    // insert_proposed_fact docs).
    let created_by = format!("analysis-agent/{PROMPT_VERSION}");
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
                capture_id: UNATTRIBUTED_CAPTURE_ID,
                topic: &record.topic,
                statement: &statement,
                created_by: &created_by,
                created_at: at,
            },
        )
        .map_err(db_err)?;
        proposed += 1;
    }
    let rejected = failed.len();

    upsert_watermark(conn, slug, at, PROMPT_VERSION).map_err(db_err)?;
    let reason = format!(
        "proposed={proposed} rejected={rejected} input_tokens={} output_tokens={}",
        resp.usage.input_tokens, resp.usage.output_tokens
    );
    insert_analysis_audit(conn, at, actor, authority_id, &reason).map_err(db_err)?;

    Ok(RunReport {
        authority_slug: slug.to_string(),
        proposed,
        rejected,
        errors,
    })
}

/// The runner signature only carries [`ModelError`], so database failures
/// map there (a dedicated runner error is follow-up work).
fn db_err(e: rusqlite::Error) -> ModelError {
    ModelError::Transport(format!("runner db error: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{NewAuthority, add_authority, get_watermark, migrate, open};
    use crate::model::{CitationRef, DraftResponse, ProposedRecord, StubClient, Usage};
    use crate::store::Store;

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
        let report = run_analysis(
            &conn,
            &StubClient,
            "usa-wa-sos",
            text,
            &sources,
            "test",
            1_800_000_100,
        )
        .unwrap();
        assert_eq!(report.authority_slug, "usa-wa-sos");
        assert_eq!((report.proposed, report.rejected), (2, 0));
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
        let report = run_analysis(
            &conn,
            &BadTopicClient,
            "usa-wa-sos",
            "PO Box 99999!",
            &sources,
            "test",
            1_800_000_200,
        )
        .unwrap();
        assert_eq!((report.proposed, report.rejected), (0, 1));
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
}
