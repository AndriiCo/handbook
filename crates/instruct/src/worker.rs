//! Sweep orchestration: one authority end to end, or all due ones.
//!
//! [`run_authority`] ties the pipeline together: triggers →
//! condense → draft → diff → render → propose. Dry-run walks the
//! same path with zero writes. [`sweep_all`] runs every authority
//! with triggers past its cursor.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use rusqlite::Connection;

use atlas::model::ModelPort;
use atlas::summarize::SummaryCache;

use crate::draft::{LiveFact, draft_instructions};
use crate::error::{Error, Result};
use crate::propose::{ProposeReport, diff_sections, propose_changes};
use crate::render::render_instructions;
use crate::template::TemplatePack;
use crate::trigger::{TriggerSkip, collect_triggers, condense_triggers};

/// Outcome for one authority.
#[derive(Debug)]
pub struct AuthorityOutcome {
    pub slug: String,
    pub triggers: usize,
    pub skipped: Vec<TriggerSkip>,
    pub report: ProposeReport,
    pub instruction_md: String,
    /// Sections rendered (ids).
    pub rendered: Vec<String>,
}

/// Run one authority (writes unless `dry`).
#[allow(clippy::too_many_arguments)]
pub fn run_authority<P: ModelPort>(
    conn: &Connection,
    store_root: &Path,
    port: &P,
    pack: &TemplatePack,
    model_name: &str,
    slug: &str,
    actor: &str,
    now: i64,
    dry: bool,
) -> Result<AuthorityOutcome> {
    let model = pack
        .model(model_name)
        .ok_or_else(|| Error::Config(format!("unknown model '{model_name}'")))?;
    let (authority_id, authority_name) = authority_by_slug(conn, slug)?;
    let (mut triggers, skipped) = collect_triggers(
        conn,
        authority_id,
        slug,
        &store_root.join("text"),
        pack.worker.max_triggers,
    )?;
    if triggers.is_empty() {
        return Ok(AuthorityOutcome {
            slug: slug.to_string(),
            triggers: 0,
            skipped,
            report: ProposeReport::default(),
            instruction_md: String::new(),
            rendered: Vec::new(),
        });
    }
    let cache = SummaryCache::new(store_root.join("instruct").join("cache"));
    condense_triggers(
        port,
        &model.id,
        &cache,
        &mut triggers,
        pack.worker.summarize_at_tokens,
        8_000,
    )?;
    let live_facts = live_facts(conn, authority_id)?;
    let draft = draft_instructions(
        port,
        &model.id,
        model.context_window,
        model.max_output_tokens,
        pack,
        &live_facts,
        &triggers,
        dry,
    )?;
    let trigger_ids: Vec<i64> = triggers.iter().map(|t| t.capture_id).collect();
    let (proposals, errors, identical) =
        diff_sections(&live_facts, &draft.sections, &draft.overrides, &trigger_ids);
    let topics: HashSet<&str> = proposals.iter().map(|p| p.topic.as_str()).collect();
    let pending: HashSet<&str> = draft
        .sections
        .iter()
        .filter(|s| topics.contains(s.topic.as_str()))
        .map(|s| s.id.as_str())
        .collect();
    let instruction_md = render_instructions(&authority_name, pack, &draft.sections, &pending);
    let rendered = draft.sections.iter().map(|s| s.id.clone()).collect();
    if dry {
        return Ok(AuthorityOutcome {
            slug: slug.to_string(),
            triggers: triggers.len(),
            skipped,
            report: ProposeReport {
                proposed: proposals.len(),
                identical,
                errors,
                ..ProposeReport::default()
            },
            instruction_md,
            rendered,
        });
    }
    let mut report = propose_changes(
        conn,
        authority_id,
        slug,
        proposals,
        errors,
        &instruction_md,
        &trigger_ids,
        actor,
        now,
    )?;
    // diff_sections counted identicals; carry them over.
    report.identical = identical;
    Ok(AuthorityOutcome {
        slug: slug.to_string(),
        triggers: triggers.len(),
        skipped,
        report,
        instruction_md,
        rendered,
    })
}

/// Sweep every authority that has triggers past its cursor.
#[allow(clippy::too_many_arguments)]
pub fn sweep_all<P: ModelPort>(
    conn: &Connection,
    store_root: &Path,
    port: &P,
    pack: &TemplatePack,
    model_name: &str,
    actor: &str,
    now: i64,
    dry: bool,
    limit_slugs: Option<&[String]>,
) -> Result<Vec<AuthorityOutcome>> {
    let mut stmt = conn.prepare("SELECT slug FROM authority ORDER BY slug")?;
    let slugs: Vec<String> = stmt
        .query_map([], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    let mut outcomes = Vec::new();
    for slug in &slugs {
        if let Some(only) = limit_slugs
            && !only.iter().any(|s| s == slug)
        {
            continue;
        }
        let outcome = run_authority(
            conn, store_root, port, pack, model_name, slug, actor, now, dry,
        )?;
        if outcome.triggers > 0 {
            outcomes.push(outcome);
        }
    }
    Ok(outcomes)
}

/// Instruction cache/artifacts dir for one store.
pub fn instruct_dir(store_root: &Path) -> PathBuf {
    store_root.join("instruct")
}

fn authority_by_slug(conn: &Connection, slug: &str) -> Result<(i64, String)> {
    conn.query_row(
        "SELECT id, name FROM authority WHERE slug = ?1",
        [slug],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .map_err(|_| Error::Trigger(format!("unknown authority '{slug}'")))
}

fn live_facts(conn: &Connection, authority_id: i64) -> Result<Vec<LiveFact>> {
    let mut stmt = conn.prepare(
        "SELECT id, topic, statement FROM fact
         WHERE authority_id = ?1 AND status = 'accepted' ORDER BY id",
    )?;
    stmt.query_map([authority_id], |row| {
        Ok(LiveFact {
            id: row.get(0)?,
            topic: row.get(1)?,
            statement: row.get(2)?,
        })
    })?
    .collect::<rusqlite::Result<_>>()
    .map_err(Error::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::change::Change;
    use crate::fixture::{WA_SLUG, seed_wa_demo, wa_script};
    use crate::template::TemplatePack;

    /// Washington-state end-to-end: synthetic authority + three
    /// triggers → instructions markdown + proposed rows with
    /// parsed before/after headers + one review task. Offline
    /// (scripted StubPort); `xtask instruct-demo` prints the
    /// same drill for humans.
    #[test]
    fn washington_demo_end_to_end() {
        let tmp = tempfile::tempdir().unwrap();
        let store = handbook_store::store::Store::new(tmp.path());
        let conn = handbook_store::db::open(&store).unwrap();
        handbook_store::db::migrate(&conn).unwrap();
        let demo = seed_wa_demo(&conn, tmp.path(), 1_700_000_000).unwrap();
        let pack = TemplatePack::load(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("packs/general.toml"),
        )
        .unwrap();
        let port = atlas::model::StubPort::with_script(wa_script(&demo.capture_ids));
        let outcome = run_authority(
            &conn,
            tmp.path(),
            &port,
            &pack,
            "spark",
            WA_SLUG,
            "test",
            1_700_000_001,
            false,
        )
        .unwrap();
        assert_eq!(outcome.triggers, 3);
        assert_eq!(outcome.report.proposed, 3);
        assert_eq!(outcome.report.suppressed, 0);
        assert!(outcome.report.task_id.is_some());
        // Instructions carry the refined Friday hours + pending flags.
        assert!(outcome.instruction_md.contains("Fri 8:00am-4:00pm"));
        assert!(outcome.instruction_md.contains("(pending review)"));
        // Proposed rows parse back to before/after headers.
        let mut stmt = conn
            .prepare("SELECT topic, statement, reasoning FROM fact WHERE status = 'proposed'")
            .unwrap();
        let rows: Vec<(String, String, String)> = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(rows.len(), 3);
        let fees = rows.iter().find(|(t, _, _)| t == "fees").unwrap();
        let change = Change::parse(&fees.2).unwrap();
        assert_eq!(change.before.as_deref(), Some("filing: $20 per apostille"));
        assert_eq!(change.after, "filing: $30 per document");
        assert!(change.supersedes > 0);
        let appt = rows.iter().find(|(t, _, _)| t == "appointment").unwrap();
        assert_eq!(Change::parse(&appt.2).unwrap().supersedes, 0);
        // The Friday override moved the walkin fact with its reason.
        let walkin = rows.iter().find(|(t, _, _)| t == "walkin").unwrap();
        let change = Change::parse(&walkin.2).unwrap();
        assert_eq!(
            change.after,
            "hours: Mon-Thu 8:00am-5:00pm, Fri 8:00am-4:00pm"
        );
        assert!(change.reason.contains("Friday"), "{}", change.reason);
        // Audit trail: one instruct.proposed per fact + sweep row.
        let audits: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM audit WHERE action = 'instruct.proposed'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(audits, 3);
        // Second sweep finds nothing (cursor advanced).
        let port = atlas::model::StubPort::with_script(vec![]);
        let again = run_authority(
            &conn,
            tmp.path(),
            &port,
            &pack,
            "spark",
            WA_SLUG,
            "test",
            1_700_000_002,
            false,
        )
        .unwrap();
        assert_eq!(again.triggers, 0);
    }

    #[test]
    fn dry_run_writes_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let store = handbook_store::store::Store::new(tmp.path());
        let conn = handbook_store::db::open(&store).unwrap();
        handbook_store::db::migrate(&conn).unwrap();
        let demo = seed_wa_demo(&conn, tmp.path(), 1_700_000_000).unwrap();
        let pack = TemplatePack::load(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("packs/general.toml"),
        )
        .unwrap();
        let port = atlas::model::StubPort::with_script(wa_script(&demo.capture_ids));
        let outcome = run_authority(
            &conn,
            tmp.path(),
            &port,
            &pack,
            "spark",
            WA_SLUG,
            "test",
            1_700_000_001,
            true,
        )
        .unwrap();
        assert_eq!(outcome.triggers, 3);
        assert_eq!(outcome.report.proposed, 3);
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM fact WHERE status = 'proposed'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 0);
    }
}
