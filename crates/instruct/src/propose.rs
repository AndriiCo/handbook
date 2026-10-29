//! Diff + propose: sections → reviewed fact updates.
//!
//! Each section statement becomes `field: value`; statements
//! identical to an accepted fact are dropped; the rest become
//! proposals with BEFORE/AFTER/REASON headers. Suppression is
//! consulted exactly like the analysis runner (paths A+B), and
//! the write — proposed facts + audits + one review task +
//! cursor — commits atomically per trigger set.

use std::collections::HashSet;

use rusqlite::{Connection, OptionalExtension};

use handbook_store::db::{
    NewProposedFact, SuppressionHit, check_suppression, insert_audit, insert_proposed_fact,
    insert_suppression, normalize_statement,
};

use crate::change::Change;
use crate::draft::{LiveFact, Override, SectionDraft};
use crate::error::{Error, Result};
use crate::trigger::set_cursor;

/// One validated proposal, ready to write.
#[derive(Debug)]
pub struct ChangeProposal {
    pub topic: String,
    pub statement: String,
    pub capture_id: i64,
    pub citation: Option<(u32, u32)>,
    pub change: Change,
}

/// Outcome of [`propose_changes`].
#[derive(Debug, Default)]
pub struct ProposeReport {
    pub proposed: usize,
    pub suppressed: usize,
    pub identical: usize,
    pub fact_ids: Vec<i64>,
    pub task_id: Option<i64>,
    pub errors: Vec<String>,
}

/// Diff drafted sections against live facts. Returns proposals,
/// per-statement errors (empty fields/values are skipped with an
/// error entry, never written), and the identical-statement count.
pub fn diff_sections(
    live_facts: &[LiveFact],
    sections: &[SectionDraft],
    overrides: &[Override],
    trigger_ids: &[i64],
) -> (Vec<ChangeProposal>, Vec<String>, usize) {
    let mut proposals = Vec::new();
    let mut errors = Vec::new();
    let mut identical = 0;
    for section in sections {
        let (capture_id, citation) = match section.citations.first() {
            Some(cite) => (cite.capture, Some((cite.start, cite.end))),
            None => (trigger_ids.first().copied().unwrap_or(0), None),
        };
        let reason_base = overrides
            .iter()
            .find(|o| o.section == section.id)
            .map(|o| o.reason.clone());
        for stmt in &section.statements {
            if stmt.field.trim().is_empty() || stmt.value.trim().is_empty() {
                errors.push(format!("section '{}': empty field or value", section.id));
                continue;
            }
            let statement = format!("{}: {}", stmt.field.trim(), stmt.value.trim());
            let norm = normalize_statement(&statement);
            // Identical to a live fact → nothing to propose.
            if live_facts.iter().any(|live| {
                live.topic == section.topic && normalize_statement(&live.statement) == norm
            }) {
                identical += 1;
                continue;
            }
            // Supersedes the live fact with the same field, if any.
            let field = stmt.field.trim();
            let target = live_facts.iter().find(|live| {
                live.topic == section.topic && field_of(&live.statement) == Some(field)
            });
            let reason = match reason_base.clone() {
                Some(base) => format!("{base} [section '{}']", section.id),
                None => format!("trigger evidence [section '{}']", section.id),
            };
            proposals.push(ChangeProposal {
                topic: section.topic.clone(),
                change: Change {
                    supersedes: target.map(|t| t.id).unwrap_or(0),
                    before: target.map(|t| t.statement.clone()),
                    after: statement.clone(),
                    reason,
                },
                statement,
                capture_id,
                citation,
            });
        }
    }
    (proposals, errors, identical)
}

/// Split `field: value` into its field half.
fn field_of(statement: &str) -> Option<&str> {
    statement.split_once(':').map(|(field, _)| field.trim())
}

/// Validate + write proposals atomically with audits, one review
/// task, and the cursor advance. `instruction_md` lands in the
/// task detail for reviewers.
#[allow(clippy::too_many_arguments)]
pub fn propose_changes(
    conn: &Connection,
    authority_id: i64,
    slug: &str,
    proposals: Vec<ChangeProposal>,
    errors: Vec<String>,
    instruction_md: &str,
    trigger_ids: &[i64],
    actor: &str,
    now: i64,
) -> Result<ProposeReport> {
    let allowed: HashSet<&str> = contracts::topics::Topic::ALL
        .iter()
        .map(|t| t.as_str())
        .collect();
    let created_by = format!("instruct/{}", crate::PROMPT_VERSION);
    let max_trigger = trigger_ids.iter().copied().max().unwrap_or(0);
    conn.execute_batch("BEGIN IMMEDIATE")?;
    let outcome = propose_inner(
        conn,
        authority_id,
        slug,
        &allowed,
        &proposals,
        errors,
        instruction_md,
        trigger_ids,
        max_trigger,
        actor,
        now,
        &created_by,
    );
    match outcome {
        Ok(report) => {
            conn.execute_batch("COMMIT")?;
            Ok(report)
        }
        Err(e) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(e)
        }
    }
}

/// The transactional body of [`propose_changes`].
#[allow(clippy::too_many_arguments)]
fn propose_inner(
    conn: &Connection,
    authority_id: i64,
    slug: &str,
    allowed: &std::collections::HashSet<&str>,
    proposals: &[ChangeProposal],
    errors: Vec<String>,
    instruction_md: &str,
    trigger_ids: &[i64],
    max_trigger: i64,
    actor: &str,
    now: i64,
    created_by: &str,
) -> Result<ProposeReport> {
    let mut report = ProposeReport {
        errors,
        ..ProposeReport::default()
    };
    for proposal in proposals {
        if !allowed.contains(proposal.topic.as_str()) {
            report
                .errors
                .push(format!("unknown topic '{}'", proposal.topic));
            continue;
        }
        let norm = normalize_statement(&proposal.statement);
        if let Some(hit) = check_suppression(conn, authority_id, &proposal.topic, &norm)? {
            let suppression_id = insert_suppression(
                conn,
                proposal.capture_id,
                authority_id,
                &proposal.topic,
                &proposal.statement,
                &norm,
                hit,
                now,
            )?;
            let (path, rule_id) = match hit {
                SuppressionHit::RejectedMemory => ("A", serde_json::json!(null)),
                SuppressionHit::Rule(id) => ("B", serde_json::json!(id)),
            };
            insert_audit(
                conn,
                now,
                created_by,
                "instruct.suppressed",
                "capture",
                proposal.capture_id,
                &capture_sha(conn, proposal.capture_id)?,
                &format!("suppression:{suppression_id}"),
                &serde_json::json!({
                    "path": path, "rule_id": rule_id,
                    "topic": proposal.topic, "statement_norm": norm,
                })
                .to_string(),
            )?;
            report.suppressed += 1;
            continue;
        }
        let header = proposal.change.header();
        let fact_id = insert_proposed_fact(
            conn,
            &NewProposedFact {
                authority_id,
                capture_id: proposal.capture_id,
                topic: &proposal.topic,
                statement: &proposal.statement,
                created_by,
                created_at: now,
                citation: proposal.citation,
                reasoning: &header,
            },
        )?;
        insert_audit(
            conn,
            now,
            actor,
            "instruct.proposed",
            "fact",
            fact_id,
            proposal.change.before.as_deref().unwrap_or("(none)"),
            &proposal.change.after,
            &proposal.change.reason,
        )?;
        report.fact_ids.push(fact_id);
        report.proposed += 1;
    }
    if report.proposed > 0 {
        let dedup = format!(
            "instruct:{slug}:{}-{}",
            trigger_ids.iter().copied().min().unwrap_or(0),
            max_trigger
        );
        let existing: Option<i64> = conn
            .query_row(
                "SELECT id FROM task WHERE dedup_key = ?1 AND status = 'open' LIMIT 1",
                [&dedup],
                |row| row.get(0),
            )
            .optional()?;
        let task_id = match existing {
            Some(id) => id,
            None => {
                conn.execute(
                        "INSERT INTO task(authority_id, kind, title, detail, status, dedup_key, created_by, created_at)
                         VALUES (?1, 'correction', ?2, ?3, 'open', ?4, ?5, ?6)",
                        rusqlite::params![
                            authority_id,
                            format!("Review instruction updates ({slug})"),
                            instruction_md,
                            dedup,
                            created_by,
                            now,
                        ],
                    )?;
                conn.last_insert_rowid()
            }
        };
        report.task_id = Some(task_id);
    }
    if max_trigger > 0 {
        set_cursor(conn, slug, max_trigger, now)?;
    }
    insert_audit(
        conn,
        now,
        actor,
        "instruct.sweep",
        "authority",
        authority_id,
        &format!("triggers={}", trigger_ids.len()),
        &format!(
            "proposed={} suppressed={}",
            report.proposed, report.suppressed
        ),
        &format!("task={:?} errors={}", report.task_id, report.errors.len()),
    )?;
    Ok(report)
}

fn capture_sha(conn: &Connection, capture_id: i64) -> Result<String> {
    conn.query_row(
        "SELECT sha256 FROM capture WHERE id = ?1",
        [capture_id],
        |row| row.get(0),
    )
    .map_err(Error::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::draft::{Citation, StatementDraft};

    fn live() -> Vec<LiveFact> {
        vec![LiveFact {
            id: 41,
            topic: "fees".to_string(),
            statement: "filing: $20 per apostille".to_string(),
        }]
    }

    fn section() -> SectionDraft {
        SectionDraft {
            id: "fees".to_string(),
            topic: "fees".to_string(),
            heading: "Fees".to_string(),
            body: "b".to_string(),
            statements: vec![
                StatementDraft {
                    field: "filing".to_string(),
                    value: "$30 per apostille".to_string(),
                },
                StatementDraft {
                    field: "".to_string(),
                    value: "bad".to_string(),
                },
            ],
            citations: vec![Citation {
                capture: 7,
                start: 0,
                end: 4,
            }],
        }
    }

    #[test]
    fn diff_supersedes_same_field_and_skips_identical() {
        let (proposals, errors, identical) = diff_sections(&live(), &[section()], &[], &[7]);
        assert_eq!(errors.len(), 1);
        assert_eq!(proposals.len(), 1);
        assert_eq!(identical, 0);
        assert_eq!(proposals[0].change.supersedes, 41);
        assert_eq!(
            proposals[0].change.before.as_deref(),
            Some("filing: $20 per apostille")
        );
        // Identical statement → no proposal.
        let mut same = section();
        same.statements[0].value = "$20 per apostille".to_string();
        let (proposals, _, identical) = diff_sections(&live(), &[same], &[], &[7]);
        assert!(proposals.is_empty());
        assert_eq!(identical, 1);
    }
}
