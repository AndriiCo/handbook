//! `RenderPlanV1`: Spark composes the authority page from accepted
//! facts, `validate_plan` + a Wonder entailment pass gate it, staff
//! accept it, and `GET /api/v1/authorities/{slug}/plan` serves it
//! (always 200, model-free, re-filtered against current accepted facts).
//!
//! Owner: lane SA-22 plan (`docs/lanes/sa-22.md`). M2 first cut landed
//! by the builder (P323) while the lane session could not start; the
//! lane owns this file from here (`eval --plans`, live tuning).
//!
//! Every model output is a proposal: the deterministic validator drops
//! any block whose numbers, amounts, PO boxes, street numbers or times
//! are not written in the facts it cites, any block citing a fact that
//! is not accepted for this authority, and any URL or markup; Wonder
//! then drops blocks the cited facts do not entail. Nothing reaches the
//! public page until staff accept it, and the public read re-checks
//! every block against the facts accepted at read time.

use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, LazyLock};

use axum::{
    Router,
    extract::{Path, Request, State},
    http::StatusCode,
    response::{IntoResponse, Json, Response},
    routing::{get, post},
};
use handbook_agent::model::ChatJsonRequest;
use handbook_store::db;
use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::admin::AdminAuth;
use crate::approval::FactView;
use crate::{AppState, store_unavailable, unix_now};

/// Composer and entailment checker (`docs/meta-api.md`).
pub const PLAN_MODEL: &str = "muse-spark-1.3";
pub const ENTAIL_MODEL: &str = "muse-wonder-1.0";
/// Prompt generation (cache key + provenance).
pub const PROMPT_VERSION: &str = "plan-v1";
/// A plan needs at least this many accepted facts to compose.
pub const MIN_FACTS: usize = 3;

const PLAN_PROMPT: &str = include_str!("../prompts/plan-v1.md");
const ENTAIL_PROMPT: &str = include_str!("../prompts/entail-v1.md");
const PER_RUN_MARKER: &str = "<!-- PER-RUN -->";

const KINDS: &[&str] = &[
    "step", "callout", "office", "fees", "mail", "forms", "timing",
];
const MAX_TITLE: usize = 90;
const MAX_BODY: usize = 400;
const MAX_HEADLINE: usize = 160;
const BANNED: &[&str] = &[
    "placeholder",
    "not wired",
    "skeleton",
    "lorem",
    "todo",
    "coming soon",
];

/// The frozen fact topics (DB CHECK), for the `hide_topics` enum.
pub(crate) const TOPICS: &[&str] = &[
    "hours",
    "fees",
    "forms",
    "address",
    "parking",
    "route_photos",
    "rejections",
    "contacts",
    "po_box",
    "walkin",
    "mailing",
    "appointment",
    "payment",
    "processing_time",
    "jurisdiction",
    "legalization_chain",
    "website",
    "phone_script",
    "holiday_hours",
    "id_requirements",
    "copies",
    "translation",
    "notarization",
    "authentication",
    "dropbox",
];

/// Topics a block kind may rest on (`step`/`callout`: any but
/// `phone_script`). A block needs at least one cited fact in its set.
fn kind_topics(kind: &str) -> Option<&'static [&'static str]> {
    match kind {
        "office" => Some(&[
            "address",
            "hours",
            "walkin",
            "appointment",
            "parking",
            "holiday_hours",
            "contacts",
            "route_photos",
            "dropbox",
            "id_requirements",
        ]),
        "fees" => Some(&["fees", "payment"]),
        "mail" => Some(&[
            "mailing",
            "po_box",
            "address",
            "payment",
            "processing_time",
            "dropbox",
        ]),
        "forms" => Some(&[
            "forms",
            "id_requirements",
            "copies",
            "notarization",
            "translation",
            "authentication",
        ]),
        "timing" => Some(&[
            "processing_time",
            "hours",
            "walkin",
            "appointment",
            "holiday_hours",
        ]),
        _ => None,
    }
}

// ---------------------------------------------------------------- types

/// One document type a visitor can pick (`?path=`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanPath {
    pub id: String,
    pub label: String,
    pub fact_ids: Vec<i64>,
}

/// One fee line (`fees` blocks), amount exactly as a cited fact states.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeeLine {
    pub label: String,
    pub amount_cents: i64,
    pub per: String,
    pub fact_id: i64,
}

/// One allowlisted block (contracts.md "Plan block kind").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanBlock {
    pub id: String,
    pub kind: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub body: String,
    pub fact_ids: Vec<i64>,
    #[serde(default)]
    pub show_for: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tone: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lines: Vec<FeeLine>,
    /// Filled at read time from accepted offices (`office` blocks).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub office_ids: Option<Vec<i64>>,
    /// Filled at read time from published forms (`forms` blocks).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub form_ids: Option<Vec<i64>>,
}

/// The page plan (`docs/lanes/contracts/plan.json` → `plan`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenderPlanV1 {
    pub headline: String,
    #[serde(default)]
    pub paths: Vec<PlanPath>,
    pub blocks: Vec<PlanBlock>,
    #[serde(default)]
    pub hide_topics: Vec<String>,
}

/// One validator check line (admin Page plans tab).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Check {
    pub id: String,
    pub ok: bool,
    pub detail: String,
}

/// A block the gate removed, and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Dropped {
    pub id: String,
    pub reason: String,
}

/// Validator report stored beside every plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Report {
    pub passed: bool,
    pub checks: Vec<Check>,
    pub dropped_blocks: Vec<Dropped>,
}

/// What the validator needs to know about one accepted fact.
#[derive(Debug, Clone)]
pub struct Grounding {
    pub id: i64,
    pub topic: String,
    /// Statement plus every official quote: what a block may restate.
    pub text: String,
}

impl Grounding {
    pub fn from_view(f: &FactView) -> Self {
        let mut text = f.statement.clone();
        for c in &f.citations {
            text.push('\n');
            text.push_str(&c.quote);
        }
        Grounding {
            id: f.id,
            topic: f.topic.clone(),
            text,
        }
    }
}

// ------------------------------------------------------------ validator

fn has_markup_or_url(text: &str) -> bool {
    let lower = text.to_lowercase();
    lower.contains("http://")
        || lower.contains("https://")
        || lower.contains("www.")
        || text.contains('<')
        || text.contains('>')
}

fn banned_word(text: &str) -> Option<&'static str> {
    let lower = text.to_lowercase();
    BANNED.iter().copied().find(|w| lower.contains(w))
}

/// `$15` / `$15.50` for an amount in cents.
fn dollars(cents: i64) -> String {
    if cents % 100 == 0 {
        format!("${}", cents / 100)
    } else {
        format!("${}.{:02}", cents / 100, (cents % 100).abs())
    }
}

/// Deterministic gate: returns the cleaned plan and its report. Blocks
/// that fail any check are dropped (never repaired); the plan passes
/// when its headline is grounded and at least one block survives.
pub fn validate_plan(plan: &RenderPlanV1, facts: &[Grounding]) -> (RenderPlanV1, Report) {
    let by_id: HashMap<i64, &Grounding> = facts.iter().map(|f| (f.id, f)).collect();
    let all_text: String = facts
        .iter()
        .map(|f| f.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let mut checks = Vec::new();
    let mut dropped = Vec::new();

    // Headline: grounded against every accepted fact (it summarises).
    let headline = plan.headline.trim().to_string();
    let headline_ok = !headline.is_empty()
        && headline.chars().count() <= MAX_HEADLINE
        && !has_markup_or_url(&headline)
        && banned_word(&headline).is_none()
        && crate::translate::guard_ok(&headline, &all_text);
    checks.push(Check {
        id: "headline_grounded".into(),
        ok: headline_ok,
        detail: if headline_ok {
            "every number in the headline is in an accepted fact".into()
        } else {
            "headline empty, too long, or states something no fact says".into()
        },
    });

    // Paths: known facts, slug ids, no markup.
    let mut paths = Vec::new();
    for p in plan.paths.iter().take(5) {
        let id_ok = !p.id.is_empty()
            && p.id.len() <= 40
            && p.id
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
        let facts_ok = p.fact_ids.iter().all(|id| by_id.contains_key(id));
        if id_ok && facts_ok && !has_markup_or_url(&p.label) && p.label.chars().count() <= 60 {
            paths.push(p.clone());
        }
    }
    let path_ids: BTreeSet<&str> = paths.iter().map(|p| p.id.as_str()).collect();

    let mut blocks = Vec::new();
    let mut counts: HashMap<&'static str, usize> = HashMap::new();
    let mut fail = |counts: &mut HashMap<&'static str, usize>,
                    id: &str,
                    check: &'static str,
                    reason: String| {
        *counts.entry(check).or_default() += 1;
        dropped.push(Dropped {
            id: id.to_string(),
            reason: format!("{check}: {reason}"),
        });
    };
    for (n, b) in plan.blocks.iter().take(12).enumerate() {
        let id = format!("b{}", n + 1);
        if !KINDS.contains(&b.kind.as_str()) {
            fail(
                &mut counts,
                &id,
                "kind_allowlisted",
                format!("unknown kind {:?}", b.kind),
            );
            continue;
        }
        if b.fact_ids.is_empty() {
            fail(
                &mut counts,
                &id,
                "fact_ids_accepted",
                "cites no fact".into(),
            );
            continue;
        }
        if let Some(bad) = b.fact_ids.iter().find(|fid| !by_id.contains_key(fid)) {
            fail(
                &mut counts,
                &id,
                "fact_ids_accepted",
                format!("fact {bad} is not accepted here"),
            );
            continue;
        }
        let cited: Vec<&Grounding> = b
            .fact_ids
            .iter()
            .filter_map(|fid| by_id.get(fid).copied())
            .collect();
        if cited.iter().any(|f| f.topic == "phone_script") {
            fail(
                &mut counts,
                &id,
                "kind_topics",
                "cites a phone script".into(),
            );
            continue;
        }
        if let Some(allowed) = kind_topics(&b.kind)
            && !cited.iter().any(|f| allowed.contains(&f.topic.as_str()))
        {
            fail(
                &mut counts,
                &id,
                "kind_topics",
                format!("no cited fact fits a {} block", b.kind),
            );
            continue;
        }
        let text = format!("{}\n{}", b.title.trim(), b.body.trim());
        if b.title.chars().count() > MAX_TITLE || b.body.chars().count() > MAX_BODY {
            fail(
                &mut counts,
                &id,
                "length_caps",
                "title or body too long".into(),
            );
            continue;
        }
        if has_markup_or_url(&text) {
            fail(
                &mut counts,
                &id,
                "no_urls_html",
                "URL or markup in text".into(),
            );
            continue;
        }
        if let Some(word) = banned_word(&text) {
            fail(
                &mut counts,
                &id,
                "no_banned_words",
                format!("contains {word:?}"),
            );
            continue;
        }
        let cited_text: String = cited
            .iter()
            .map(|f| f.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        if !crate::translate::guard_ok(&text, &cited_text) {
            fail(
                &mut counts,
                &id,
                "numbers_in_citations",
                "a number, amount, PO box, street or time is not in the cited facts".into(),
            );
            continue;
        }
        let mut lines = Vec::new();
        if b.kind == "fees" {
            let mut bad_line = None;
            for line in &b.lines {
                let fact_ok = b.fact_ids.contains(&line.fact_id);
                let amount_ok = line.amount_cents >= 0
                    && by_id.get(&line.fact_id).is_some_and(|f| {
                        crate::translate::guard_ok(&dollars(line.amount_cents), &f.text)
                    });
                if !fact_ok || !amount_ok || has_markup_or_url(&line.label) {
                    bad_line = Some(format!(
                        "fee line {} {} not in fact {}",
                        line.label,
                        dollars(line.amount_cents),
                        line.fact_id
                    ));
                    break;
                }
                lines.push(line.clone());
            }
            if let Some(reason) = bad_line {
                fail(&mut counts, &id, "fees_match_citations", reason);
                continue;
            }
            if lines.is_empty() {
                fail(
                    &mut counts,
                    &id,
                    "fees_match_citations",
                    "fees block without lines".into(),
                );
                continue;
            }
        }
        let tone = (b.kind == "callout").then(|| {
            if b.tone.as_deref() == Some("warning") {
                "warning"
            } else {
                "info"
            }
            .to_string()
        });
        blocks.push(PlanBlock {
            id,
            kind: b.kind.clone(),
            title: b.title.trim().to_string(),
            body: b.body.trim().to_string(),
            fact_ids: b.fact_ids.clone(),
            show_for: b
                .show_for
                .iter()
                .filter(|p| path_ids.contains(p.as_str()))
                .cloned()
                .collect(),
            tone,
            lines,
            office_ids: None,
            form_ids: None,
        });
    }
    for check in [
        "kind_allowlisted",
        "fact_ids_accepted",
        "kind_topics",
        "length_caps",
        "no_urls_html",
        "no_banned_words",
        "numbers_in_citations",
        "fees_match_citations",
    ] {
        let n = counts.get(check).copied().unwrap_or(0);
        checks.push(Check {
            id: check.into(),
            ok: n == 0,
            detail: if n == 0 {
                "all blocks pass".into()
            } else {
                format!("{n} block(s) dropped")
            },
        });
    }
    let mut hide: Vec<String> = plan
        .hide_topics
        .iter()
        .filter(|t| TOPICS.contains(&t.as_str()))
        .cloned()
        .collect();
    if !hide.iter().any(|t| t == "phone_script") {
        hide.push("phone_script".into());
    }
    let passed = headline_ok && !blocks.is_empty();
    (
        RenderPlanV1 {
            headline,
            paths,
            blocks,
            hide_topics: hide,
        },
        Report {
            passed,
            checks,
            dropped_blocks: dropped,
        },
    )
}

/// Read-time filter: keep blocks whose every cited fact is still
/// accepted; attach the accepted offices and forms; `None` when nothing
/// survives (the dossier falls back to the deterministic guide).
pub fn filter_for_read(
    plan: &RenderPlanV1,
    accepted: &BTreeSet<i64>,
    office_ids: &[i64],
    form_ids: &[i64],
) -> Option<RenderPlanV1> {
    let mut out = plan.clone();
    out.blocks
        .retain(|b| b.fact_ids.iter().all(|id| accepted.contains(id)));
    for b in &mut out.blocks {
        b.lines.retain(|l| accepted.contains(&l.fact_id));
        match b.kind.as_str() {
            "office" => b.office_ids = Some(office_ids.to_vec()),
            "forms" => b.form_ids = Some(form_ids.to_vec()),
            _ => {}
        }
    }
    out.blocks
        .retain(|b| !(b.kind == "fees" && b.lines.is_empty()));
    out.paths
        .retain(|p| p.fact_ids.iter().all(|id| accepted.contains(id)));
    (!out.blocks.is_empty()).then_some(out)
}

// --------------------------------------------------------------- models

/// Strict json_schema for the composer: fact ids are an enum of the
/// accepted ids, so the model cannot cite anything else.
pub fn plan_schema(fact_ids: &[i64]) -> serde_json::Value {
    let ids = json!({"type": "integer", "enum": fact_ids});
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["headline", "paths", "blocks", "hide_topics"],
        "properties": {
            "headline": {"type": "string"},
            "paths": {"type": "array", "items": {
                "type": "object", "additionalProperties": false,
                "required": ["id", "label", "fact_ids"],
                "properties": {
                    "id": {"type": "string"}, "label": {"type": "string"},
                    "fact_ids": {"type": "array", "items": ids},
                }}},
            "blocks": {"type": "array", "items": {
                "type": "object", "additionalProperties": false,
                "required": ["id", "kind", "title", "body", "fact_ids", "show_for", "tone", "lines"],
                "properties": {
                    "id": {"type": "string"},
                    "kind": {"type": "string", "enum": KINDS},
                    "title": {"type": "string"},
                    "body": {"type": "string"},
                    "fact_ids": {"type": "array", "items": ids},
                    "show_for": {"type": "array", "items": {"type": "string"}},
                    "tone": {"type": "string", "enum": ["info", "warning", "none"]},
                    "lines": {"type": "array", "items": {
                        "type": "object", "additionalProperties": false,
                        "required": ["label", "amount_cents", "per", "fact_id"],
                        "properties": {
                            "label": {"type": "string"},
                            "amount_cents": {"type": "integer"},
                            "per": {"type": "string", "enum": ["document", "request", "page", "copy", "none"]},
                            "fact_id": ids,
                        }}},
                }}},
            "hide_topics": {"type": "array", "items": {"type": "string", "enum": TOPICS}},
        }
    })
}

fn split_prompt(template: &'static str) -> (&'static str, &'static str) {
    template
        .split_once(PER_RUN_MARKER)
        .expect("plan prompt carries a PER-RUN part")
}

/// Per-run composer input: one line per accepted fact.
pub fn plan_user(name: &str, slug: &str, facts: &[FactView]) -> String {
    let (_, per_run) = split_prompt(PLAN_PROMPT);
    let lines: Vec<String> = facts
        .iter()
        .filter(|f| f.topic != "phone_script")
        .map(|f| {
            let quote = f.citations.first().map(|c| c.quote.as_str()).unwrap_or("");
            format!("- {} · {} · {} · \"{}\"", f.id, f.topic, f.statement, quote)
        })
        .collect();
    per_run
        .replace("{{name}}", name)
        .replace("{{slug}}", slug)
        .replace("{{facts}}", &lines.join("\n"))
        .trim()
        .to_string()
}

fn entail_schema(block_ids: &[String]) -> serde_json::Value {
    json!({
        "type": "object", "additionalProperties": false, "required": ["verdicts"],
        "properties": {"verdicts": {"type": "array", "items": {
            "type": "object", "additionalProperties": false,
            "required": ["block_id", "supported", "reason"],
            "properties": {
                "block_id": {"type": "string", "enum": block_ids},
                "supported": {"type": "boolean"},
                "reason": {"type": "string"},
            }}}}
    })
}

/// Spend of one compose run (composer + entailment).
#[derive(Debug, Default, Clone, Copy, Serialize)]
pub struct Spend {
    pub latency_ms: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cached_tokens: u64,
    pub reasoning_tokens: u64,
}

impl Spend {
    fn add(&mut self, r: &handbook_agent::model::ChatJsonResponse) {
        self.latency_ms += r.latency_ms;
        self.input_tokens += u64::from(r.usage.input_tokens);
        self.output_tokens += u64::from(r.usage.output_tokens);
        self.cached_tokens += u64::from(r.usage.cached_tokens);
        self.reasoning_tokens += u64::from(r.usage.reasoning_tokens);
    }
}

/// Wonder entailment: drop blocks their cited facts do not support.
/// A failed check call keeps the deterministic result and says so.
fn entail(
    client: &handbook_agent::model::MetaRestClient,
    plan: &mut RenderPlanV1,
    report: &mut Report,
    facts: &[Grounding],
    run_id: u64,
    spend: &mut Spend,
) {
    if plan.blocks.is_empty() {
        return;
    }
    let by_id: HashMap<i64, &Grounding> = facts.iter().map(|f| (f.id, f)).collect();
    let (developer, per_run) = split_prompt(ENTAIL_PROMPT);
    let blocks_text: Vec<String> = plan
        .blocks
        .iter()
        .map(|b| {
            let cited: Vec<String> = b
                .fact_ids
                .iter()
                .filter_map(|id| by_id.get(id))
                .map(|f| format!("  - fact {}: {}", f.id, f.text.replace('\n', " / ")))
                .collect();
            format!("[{}] {} — {}\n{}", b.id, b.title, b.body, cited.join("\n"))
        })
        .collect();
    let ids: Vec<String> = plan.blocks.iter().map(|b| b.id.clone()).collect();
    let req = ChatJsonRequest {
        model: ENTAIL_MODEL,
        developer,
        user: per_run
            .replace("{{blocks}}", &blocks_text.join("\n\n"))
            .trim()
            .to_string(),
        schema_name: "EntailmentV1",
        schema: entail_schema(&ids),
        reasoning_effort: "medium",
        prompt_cache_key: "plan-entail-v1".to_string(),
        max_completion_tokens: 8_192,
        pipeline: "plan",
        run_id: Some(run_id),
    };
    match client.chat_json(&req) {
        Ok(resp) => {
            spend.add(&resp);
            let unsupported: HashMap<String, String> = resp.value["verdicts"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|v| v["supported"] == false)
                .filter_map(|v| {
                    Some((
                        v["block_id"].as_str()?.to_string(),
                        v["reason"].as_str().unwrap_or("").to_string(),
                    ))
                })
                .collect();
            let before = plan.blocks.len();
            plan.blocks.retain(|b| !unsupported.contains_key(&b.id));
            for (id, reason) in &unsupported {
                report.dropped_blocks.push(Dropped {
                    id: id.clone(),
                    reason: format!(
                        "entailment: {}",
                        reason.chars().take(120).collect::<String>()
                    ),
                });
            }
            let kept = plan.blocks.len();
            report.checks.push(Check {
                id: "entailment".into(),
                ok: unsupported.is_empty(),
                detail: format!("{ENTAIL_MODEL}: {kept}/{before} blocks supported"),
            });
            report.passed = report.passed && kept > 0;
        }
        Err(e) => report.checks.push(Check {
            id: "entailment".into(),
            ok: false,
            detail: format!("{ENTAIL_MODEL} check unavailable ({e}); deterministic gate only"),
        }),
    }
}

// ------------------------------------------------------------------ DB

/// Boot DDL on the metadata DB (`handbook.sqlite`): `CREATE TABLE IF
/// NOT EXISTS` only — no ALTER, DROP or RENAME (AGENTS.md §7).
pub fn migrate_tables(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS page_plan (
            id INTEGER PRIMARY KEY,
            authority_slug TEXT NOT NULL,
            version INTEGER NOT NULL,
            status TEXT NOT NULL,
            plan_json TEXT NOT NULL,
            validator_json TEXT NOT NULL,
            provenance_json TEXT NOT NULL,
            run_id INTEGER,
            created_at INTEGER NOT NULL,
            decided_at INTEGER,
            decided_by_role TEXT
        );
        CREATE INDEX IF NOT EXISTS page_plan_slug ON page_plan(authority_slug, status);",
    )
}

/// One stored plan row.
#[derive(Debug, Clone)]
pub struct PlanRow {
    pub id: i64,
    pub slug: String,
    pub version: i64,
    pub status: String,
    pub plan: RenderPlanV1,
    pub validator: serde_json::Value,
    pub provenance: serde_json::Value,
    pub run_id: Option<i64>,
    pub created_at: i64,
    pub decided_at: Option<i64>,
}

const ROW_COLS: &str = "id, authority_slug, version, status, plan_json, validator_json,
                        provenance_json, run_id, created_at, decided_at";

fn row_of(r: &rusqlite::Row<'_>) -> rusqlite::Result<PlanRow> {
    let plan: String = r.get(4)?;
    let validator: String = r.get(5)?;
    let provenance: String = r.get(6)?;
    Ok(PlanRow {
        id: r.get(0)?,
        slug: r.get(1)?,
        version: r.get(2)?,
        status: r.get(3)?,
        plan: serde_json::from_str(&plan).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(4, rusqlite::types::Type::Text, Box::new(e))
        })?,
        validator: serde_json::from_str(&validator).unwrap_or(json!({})),
        provenance: serde_json::from_str(&provenance).unwrap_or(json!({})),
        run_id: r.get(7)?,
        created_at: r.get(8)?,
        decided_at: r.get(9)?,
    })
}

/// Store a proposed plan; returns (id, version).
pub fn insert_proposed(
    conn: &Connection,
    slug: &str,
    plan: &RenderPlanV1,
    report: &Report,
    provenance: &serde_json::Value,
    run_id: Option<u64>,
    now: i64,
) -> rusqlite::Result<(i64, i64)> {
    let version: i64 = conn.query_row(
        "SELECT COALESCE(MAX(version), 0) + 1 FROM page_plan WHERE authority_slug = ?1",
        [slug],
        |r| r.get(0),
    )?;
    conn.execute(
        "INSERT INTO page_plan(authority_slug, version, status, plan_json, validator_json,
                               provenance_json, run_id, created_at)
         VALUES (?1, ?2, 'proposed', ?3, ?4, ?5, ?6, ?7)",
        rusqlite::params![
            slug,
            version,
            serde_json::to_string(plan).unwrap_or_default(),
            serde_json::to_string(report).unwrap_or_default(),
            provenance.to_string(),
            run_id.and_then(|id| i64::try_from(id).ok()),
            now,
        ],
    )?;
    Ok((conn.last_insert_rowid(), version))
}

/// The accepted plan for `slug`, if any.
pub fn accepted_for(conn: &Connection, slug: &str) -> rusqlite::Result<Option<PlanRow>> {
    conn.query_row(
        &format!("SELECT {ROW_COLS} FROM page_plan WHERE authority_slug = ?1 AND status = 'accepted' ORDER BY version DESC LIMIT 1"),
        [slug],
        row_of,
    )
    .optional()
}

/// Accept or reject a proposed plan in one transaction (accept
/// supersedes the previous accepted version). Returns the row, or
/// `Ok(None)` when the plan is not proposed.
pub fn decide(
    conn: &Connection,
    id: i64,
    accept: bool,
    actor: &str,
    now: i64,
) -> rusqlite::Result<Option<PlanRow>> {
    let tx = conn.unchecked_transaction()?;
    let row: Option<PlanRow> = tx
        .query_row(
            &format!("SELECT {ROW_COLS} FROM page_plan WHERE id = ?1"),
            [id],
            row_of,
        )
        .optional()?;
    let Some(row) = row.filter(|r| r.status == "proposed") else {
        return Ok(None);
    };
    let to = if accept { "accepted" } else { "rejected" };
    if accept {
        tx.execute(
            "UPDATE page_plan SET status = 'superseded' WHERE authority_slug = ?1 AND status = 'accepted'",
            [&row.slug],
        )?;
    }
    let mut provenance = row.provenance.clone();
    if accept {
        // Role label only: the public page never names a staff member.
        provenance["approved"] = json!({"by": "Staff reviewer", "at": now});
    }
    tx.execute(
        "UPDATE page_plan SET status = ?1, decided_at = ?2, decided_by_role = 'staff',
                              provenance_json = ?3 WHERE id = ?4",
        rusqlite::params![to, now, provenance.to_string(), id],
    )?;
    tx.execute(
        "INSERT INTO audit(at, actor, action, entity, entity_id, before_ref, after_ref, reason)
         VALUES (?1, ?2, ?3, 'page_plan', ?4, 'proposed', ?5, ?6)",
        rusqlite::params![
            now,
            actor,
            if accept { "plan_accept" } else { "plan_reject" },
            id,
            to,
            format!("{} v{}", row.slug, row.version),
        ],
    )?;
    tx.commit()?;
    Ok(Some(PlanRow {
        status: to.to_string(),
        decided_at: Some(now),
        provenance,
        ..row
    }))
}

// ---------------------------------------------------------- public read

static PLAN_LIMITS: LazyLock<crate::passkey::RateBuckets> =
    LazyLock::new(crate::passkey::RateBuckets::default);

/// The `/plan` body (contracts `plan.json`): never an error shape.
pub fn read_body(
    slug: &str,
    row: Option<PlanRow>,
    facts: Vec<FactView>,
    offices: Vec<serde_json::Value>,
    forms: Vec<serde_json::Value>,
) -> serde_json::Value {
    let accepted: BTreeSet<i64> = facts.iter().map(|f| f.id).collect();
    let office_ids: Vec<i64> = offices.iter().filter_map(|o| o["id"].as_i64()).collect();
    let form_ids: Vec<i64> = forms.iter().filter_map(|f| f["id"].as_i64()).collect();
    let (plan, provenance) = match row {
        Some(row) => match filter_for_read(&row.plan, &accepted, &office_ids, &form_ids) {
            Some(plan) => {
                let mut body = serde_json::to_value(&plan).unwrap_or(json!({}));
                body["id"] = json!(row.id);
                body["version"] = json!(row.version);
                body["status"] = json!("accepted");
                body["accepted_at"] = json!(row.decided_at);
                (body, row.provenance)
            }
            None => (serde_json::Value::Null, serde_json::Value::Null),
        },
        None => (serde_json::Value::Null, serde_json::Value::Null),
    };
    json!({
        "slug": slug,
        "plan": plan,
        "facts": facts,
        "offices": offices,
        "forms": forms,
        "provenance": provenance,
    })
}

/// `GET /apostille/api/v1/authorities/{slug}/plan` — anyone, 60/min/IP,
/// model-free, always 200 (an unknown slug reads as no plan).
async fn public_plan(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    req: Request,
) -> Response {
    let now = unix_now();
    let ip =
        crate::admin_ip::peer_ip(&req).map_or_else(|| "unknown".to_string(), |ip| ip.to_string());
    if !PLAN_LIMITS.check(&format!("plan:{ip}"), now, 60, 60) {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            Json(json!({"error": "rate limited"})),
        )
            .into_response();
    }
    let store = handbook_store::store::Store::new(&state.store_root);
    let Ok(conn) = db::open(&store) else {
        return store_unavailable();
    };
    let facts =
        crate::approval::accepted_fact_views(&conn, &state.store_root, &slug).unwrap_or_default();
    let offices = crate::offices::accepted_for_slug(&conn, &slug).unwrap_or_default();
    let forms = crate::forms_map::accepted_for_slug(&conn, &slug).unwrap_or_default();
    let row = accepted_for(&conn, &slug).unwrap_or(None);
    Json(read_body(&slug, row, facts, offices, forms)).into_response()
}

/// Public routes, merged into the `/apostille` API router (paths start
/// `/api/v1/…`). Rate-limit every handler; no model calls on a public read.
pub fn public_routes() -> Router<AppState> {
    Router::new().route("/api/v1/authorities/{slug}/plan", get(public_plan))
}

// ------------------------------------------------------------- compose

#[derive(Debug, Deserialize)]
pub struct ComposeBody {
    pub slug: String,
}

/// The compose run: Spark composes, the gate validates, Wonder checks
/// entailment, and the result is stored as a proposed plan.
fn compose_blocking(
    store_root: &std::path::Path,
    kek: Option<&config::Kek>,
    slug: &str,
    run_id: u64,
) -> Result<serde_json::Value, String> {
    let store = handbook_store::store::Store::new(store_root);
    let conn = db::open(&store).map_err(|e| format!("store unavailable: {e}"))?;
    let name: String = conn
        .query_row("SELECT name FROM authority WHERE slug = ?1", [slug], |r| {
            r.get(0)
        })
        .map_err(|_| "unknown authority".to_string())?;
    let facts = crate::approval::accepted_fact_views(&conn, store_root, slug).unwrap_or_default();
    let facts: Vec<FactView> = facts
        .into_iter()
        .filter(|f| f.topic != "phone_script")
        .collect();
    if facts.len() < MIN_FACTS {
        return Err(format!(
            "needs at least {MIN_FACTS} accepted facts (has {})",
            facts.len()
        ));
    }
    let grounding: Vec<Grounding> = facts.iter().map(Grounding::from_view).collect();
    let ids: Vec<i64> = facts.iter().map(|f| f.id).collect();
    let client = crate::draft::live_client(&conn, kek)?.with_observer(
        crate::telemetry::DbObserver::new(store_root.to_path_buf()),
        "plan",
        Some(run_id),
    );
    let (developer, _) = split_prompt(PLAN_PROMPT);
    let req = ChatJsonRequest {
        model: PLAN_MODEL,
        developer,
        user: plan_user(&name, slug, &facts),
        schema_name: "RenderPlanV1",
        schema: plan_schema(&ids),
        reasoning_effort: "high",
        prompt_cache_key: format!("authority-plan-{PROMPT_VERSION}"),
        max_completion_tokens: 16_384,
        pipeline: "plan",
        run_id: Some(run_id),
    };
    let mut spend = Spend::default();
    let resp = client
        .chat_json(&req)
        .map_err(|e| format!("compose failed: {e}"))?;
    spend.add(&resp);
    let drafted: RenderPlanV1 =
        serde_json::from_value(resp.value).map_err(|e| format!("compose: plan shape: {e}"))?;
    let (mut plan, mut report) = validate_plan(&drafted, &grounding);
    entail(
        &client,
        &mut plan,
        &mut report,
        &grounding,
        run_id,
        &mut spend,
    );
    let checks = crate::factcheck::checks_for_slug(&conn, slug).unwrap_or_default();
    let count = |v: &str| checks.iter().filter(|c| c["verdict"] == v).count();
    let mut sources: Vec<serde_json::Value> = Vec::new();
    for f in &facts {
        if !plan.blocks.iter().any(|b| b.fact_ids.contains(&f.id)) {
            continue;
        }
        for c in &f.citations {
            if !sources.iter().any(|s| s["url"] == c.source_url.as_str()) {
                sources.push(json!({"url": c.source_url, "captured_at": c.captured_at}));
            }
        }
    }
    let provenance = json!({
        "models": [
            {"model": facts.first().map(|f| f.drafted_by.split(" via ").next().unwrap_or(PLAN_MODEL).to_string()).unwrap_or_else(|| PLAN_MODEL.to_string()), "role": "extract"},
            {"model": crate::plan::ENTAIL_MODEL, "role": "cross_check"},
            {"model": PLAN_MODEL, "role": "compose"},
        ],
        "validator": {
            "passed": report.passed,
            "checks": report.checks.len(),
            "dropped_blocks": report.dropped_blocks.len(),
        },
        "cross_check": {
            "supported": count("supported"),
            "contradicted": count("contradicted"),
            "not_found": count("not_found"),
            "checked_before_accept": !checks.is_empty(),
        },
        "run": {
            "run_id": run_id,
            "latency_ms": spend.latency_ms,
            "input_tokens": spend.input_tokens,
            "output_tokens": spend.output_tokens,
            "cached_tokens": spend.cached_tokens,
            "reasoning_tokens": spend.reasoning_tokens,
        },
        "sources": sources,
    });
    let (plan_id, version) = insert_proposed(
        &conn,
        slug,
        &plan,
        &report,
        &provenance,
        Some(run_id),
        unix_now(),
    )
    .map_err(|e| format!("store plan: {e}"))?;
    Ok(json!({
        "plan_id": plan_id, "version": version, "passed": report.passed,
        "blocks": plan.blocks.len(), "dropped": report.dropped_blocks.len(),
        "latency_ms": spend.latency_ms,
        "input_tokens": spend.input_tokens, "output_tokens": spend.output_tokens,
        "cached_tokens": spend.cached_tokens, "reasoning_tokens": spend.reasoning_tokens,
    }))
}

fn publish_run(bus: &crate::ws_bus::Bus, run_id: u64, kind: &str, body: serde_json::Value) {
    let mut event =
        json!({"type": kind, "topic": format!("run:{run_id}"), "run_id": run_id, "at": unix_now()});
    if let (Some(map), Some(extra)) = (event.as_object_mut(), body.as_object()) {
        for (k, v) in extra {
            map.insert(k.clone(), v.clone());
        }
    }
    bus.publish(&format!("run:{run_id}"), event.to_string());
}

/// `POST /apostille/api/v1/admin/plans/compose` — staff. Starts one
/// compose run and returns its `run:{id}` topic at once.
async fn compose(
    _auth: AdminAuth,
    State(state): State<AppState>,
    Json(body): Json<ComposeBody>,
) -> Response {
    let store = handbook_store::store::Store::new(&state.store_root);
    let Ok(conn) = db::open(&store) else {
        return store_unavailable();
    };
    let known: bool = conn
        .query_row(
            "SELECT 1 FROM authority WHERE slug = ?1",
            [&body.slug],
            |_| Ok(true),
        )
        .optional()
        .unwrap_or(None)
        .unwrap_or(false);
    if !known {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "unknown authority"})),
        )
            .into_response();
    }
    let now = unix_now();
    if crate::telemetry::over_daily_cap(&conn, &state, now) {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            Json(json!({"error": "daily model-call cap reached (meta.daily_call_cap)"})),
        )
            .into_response();
    }
    drop(conn);
    let run_id = state.draft_runs.reserve_id();
    let bus = state.bus.clone();
    let store_root = state.store_root.clone();
    let kek: Option<Arc<config::Kek>> = state.kek.clone();
    let slug = body.slug.clone();
    tokio::spawn(async move {
        publish_run(
            &bus,
            run_id,
            "compose.started",
            json!({"slug": slug, "model": PLAN_MODEL}),
        );
        let job_slug = slug.clone();
        let outcome = tokio::task::spawn_blocking(move || {
            compose_blocking(&store_root, kek.as_deref(), &job_slug, run_id)
        })
        .await;
        match outcome {
            Ok(Ok(summary)) => {
                publish_run(&bus, run_id, "compose.done", summary.clone());
                crate::ws_bus::publish_ops(
                    &bus,
                    "plan_proposed",
                    json!({"slug": slug, "plan_id": summary["plan_id"], "run_id": run_id}),
                    unix_now(),
                );
            }
            Ok(Err(error)) => publish_run(&bus, run_id, "compose.failed", json!({"error": error})),
            Err(join) => publish_run(
                &bus,
                run_id,
                "compose.failed",
                json!({"error": format!("compose task failed: {join}")}),
            ),
        }
    });
    (
        StatusCode::ACCEPTED,
        Json(json!({"run_id": run_id, "topic": format!("run:{run_id}")})),
    )
        .into_response()
}

#[derive(Debug, Deserialize)]
pub struct ListQuery {
    pub status: Option<String>,
    pub slug: Option<String>,
}

/// `GET /apostille/api/v1/admin/plans` — staff (contracts
/// `admin-plan-proposals.json`).
async fn list(
    _auth: AdminAuth,
    State(state): State<AppState>,
    axum::extract::Query(q): axum::extract::Query<ListQuery>,
) -> Response {
    let store = handbook_store::store::Store::new(&state.store_root);
    let Ok(conn) = db::open(&store) else {
        return store_unavailable();
    };
    let status = q.status.unwrap_or_else(|| "proposed".to_string());
    let slug = q.slug.unwrap_or_default();
    let rows: Vec<PlanRow> = conn
        .prepare(&format!(
            "SELECT {ROW_COLS} FROM page_plan
             WHERE (?1 = 'all' OR status = ?1) AND (?2 = '' OR authority_slug = ?2)
             ORDER BY id DESC LIMIT 50"
        ))
        .and_then(|mut s| {
            s.query_map(rusqlite::params![status, slug], row_of)?
                .collect()
        })
        .unwrap_or_default();
    let plans: Vec<serde_json::Value> = rows
        .into_iter()
        .map(|r| {
            let current: Option<i64> = conn
                .query_row(
                    "SELECT version FROM page_plan WHERE authority_slug = ?1 AND status = 'accepted' ORDER BY version DESC LIMIT 1",
                    [&r.slug],
                    |x| x.get(0),
                )
                .optional()
                .unwrap_or(None);
            let mut plan = serde_json::to_value(&r.plan).unwrap_or(json!({}));
            plan["id"] = json!(r.id);
            plan["version"] = json!(r.version);
            plan["status"] = json!(r.status);
            json!({
                "id": r.id, "authority_slug": r.slug, "version": r.version, "status": r.status,
                "created_at": r.created_at, "run_id": r.run_id, "plan": plan,
                "validator": r.validator, "provenance": r.provenance, "current_version": current,
            })
        })
        .collect();
    Json(json!({"plans": plans})).into_response()
}

async fn decide_route(auth: AdminAuth, state: AppState, id: i64, accept: bool) -> Response {
    let store = handbook_store::store::Store::new(&state.store_root);
    let Ok(conn) = db::open(&store) else {
        return store_unavailable();
    };
    let now = unix_now();
    match decide(&conn, id, accept, &auth.user.user_id, now) {
        Ok(Some(row)) => {
            if accept {
                crate::ws_bus::publish_authority_event(
                    &state.bus,
                    &row.slug,
                    "plan_accepted",
                    json!({"plan_id": row.id, "version": row.version}),
                    now,
                );
                crate::ws_bus::publish_ops(
                    &state.bus,
                    "plan_accepted",
                    json!({"slug": row.slug, "plan_id": row.id, "version": row.version}),
                    now,
                );
            }
            Json(json!({"id": row.id, "authority_slug": row.slug, "version": row.version, "status": row.status}))
                .into_response()
        }
        Ok(None) => (
            StatusCode::CONFLICT,
            Json(json!({"error": "plan is not proposed"})),
        )
            .into_response(),
        Err(_) => store_unavailable(),
    }
}

async fn accept(auth: AdminAuth, State(state): State<AppState>, Path(id): Path<i64>) -> Response {
    decide_route(auth, state, id, true).await
}

async fn reject(auth: AdminAuth, State(state): State<AppState>, Path(id): Path<i64>) -> Response {
    decide_route(auth, state, id, false).await
}

/// Staff routes merged onto the ADMIN listener only (the builder wraps
/// them in the admin guard stack; handlers take `AdminAuth`).
pub fn admin_router(state: AppState) -> Router {
    Router::<AppState>::new()
        .route("/apostille/api/v1/admin/plans", get(list))
        .route("/apostille/api/v1/admin/plans/compose", post(compose))
        .route("/apostille/api/v1/admin/plans/{id}/accept", post(accept))
        .route("/apostille/api/v1/admin/plans/{id}/reject", post(reject))
        .with_state(state)
}

#[cfg(test)]
mod live {
    //! Live compose on a scratch copy of a store (plant only, staging
    //! key): `LIVE_STORE=<dir> LIVE_SLUGS=usa-wa-sos,vietnam
    //! cargo test --release -p web plan::live -- --ignored --nocapture`.
    //! Prints one `LIVE` line per run plus the stored plan and report
    //! (the replay fixture) and the model-call summary.
    use super::*;

    #[test]
    #[ignore = "live: needs META_API_KEY and LIVE_STORE (a scratch store copy)"]
    fn live_compose() {
        let root = std::path::PathBuf::from(std::env::var("LIVE_STORE").expect("LIVE_STORE"));
        let slugs = std::env::var("LIVE_SLUGS").unwrap_or_else(|_| "usa-wa-sos".to_string());
        let store = handbook_store::store::Store::new(&root);
        let conn = db::open(&store).expect("open store");
        crate::telemetry::migrate_tables(&conn).unwrap();
        migrate_tables(&conn).unwrap();
        crate::offices::migrate_tables(&conn).unwrap();
        crate::factcheck::migrate_tables(&conn).unwrap();
        // M3: cross-check every capture of each slug first, so the plan
        // provenance carries real Wonder verdicts.
        for slug in slugs.split(',') {
            let captures: Vec<i64> = conn
                .prepare(
                    "SELECT DISTINCT f.capture_id FROM fact f JOIN authority a ON a.id = f.authority_id
                     WHERE a.slug = ?1 AND f.status IN ('proposed', 'accepted')",
                )
                .unwrap()
                .query_map([slug], |r| r.get(0))
                .unwrap()
                .flatten()
                .collect();
            for (j, capture) in captures.into_iter().enumerate() {
                match crate::factcheck::run_blocking(&root, None, capture, 9_200 + j as u64) {
                    Ok(summary) => println!("LIVE-CHECK {slug} {summary}"),
                    Err(e) => println!("LIVE-CHECK {slug} capture {capture} FAILED {e}"),
                }
            }
            println!(
                "CHECKS {slug} {}",
                serde_json::Value::Array(crate::factcheck::latest_for_slug(&conn, slug).unwrap())
            );
        }
        for (i, slug) in slugs.split(',').enumerate() {
            match crate::offices::compose_blocking(&root, None, slug, 9_100 + i as u64) {
                Ok(summary) => {
                    println!("LIVE-OFFICES {slug} {summary}");
                    for (id, office, guards) in conn
                        .prepare("SELECT id, office_json, guards_json FROM office_record WHERE run_id = ?1")
                        .unwrap()
                        .query_map([9_100 + i as i64], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)))
                        .unwrap()
                        .flatten()
                    {
                        println!("OFFICE {slug} {id} {office}");
                        println!("GUARDS {slug} {id} {guards}");
                        // Accept so the plan compose below sees offices.
                        let _ = crate::offices::decide(&conn, id, true, None, "live-harness", unix_now());
                    }
                }
                Err(e) => println!("LIVE-OFFICES {slug} FAILED {e}"),
            }
        }
        for (i, slug) in slugs.split(',').enumerate() {
            let run_id = 9_000 + i as u64;
            match compose_blocking(&root, None, slug, run_id) {
                Ok(summary) => {
                    println!("LIVE {slug} {summary}");
                    let row: (String, String, String) = conn
                        .query_row(
                            "SELECT plan_json, validator_json, provenance_json FROM page_plan
                             WHERE run_id = ?1",
                            [run_id as i64],
                            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                        )
                        .unwrap();
                    println!("PLAN {slug} {}", row.0);
                    println!("REPORT {slug} {}", row.1);
                    println!("PROVENANCE {slug} {}", row.2);
                    // Staging stand only: accept the plan so the dossier shows it.
                    if std::env::var("LIVE_ACCEPT_PLAN").as_deref() == Ok("1")
                        && let Ok(Some(id)) = conn
                            .query_row(
                                "SELECT id FROM page_plan WHERE run_id = ?1",
                                [run_id as i64],
                                |r| r.get::<_, i64>(0),
                            )
                            .optional()
                    {
                        println!(
                            "ACCEPT {slug} plan {id} {:?}",
                            decide(&conn, id, true, "live-harness", unix_now())
                                .map(|r| r.map(|r| r.status))
                        );
                    }
                }
                Err(e) => println!("LIVE {slug} FAILED {e}"),
            }
        }
        let q = crate::telemetry::CallsQuery {
            pipeline: Some("plan".into()),
            since: Some(0),
            run_id: None,
            limit: Some(50),
        };
        println!(
            "CALLS {}",
            crate::telemetry::calls_view(&conn, &q, unix_now()).unwrap()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn g(id: i64, topic: &str, text: &str) -> Grounding {
        Grounding {
            id,
            topic: topic.into(),
            text: text.into(),
        }
    }

    fn facts() -> Vec<Grounding> {
        vec![
            g(
                201,
                "fees",
                "The apostille fee is $15 per document.\nThe fee is $15 per document.",
            ),
            g(
                202,
                "hours",
                "The counter is open Monday to Friday, 8:00 AM to 4:30 PM.",
            ),
            g(203, "po_box", "Mailed requests go to PO Box 4000."),
            g(
                204,
                "address",
                "Walk-in service is at 100 Fixture Way, Olympia, WA 98501.",
            ),
            g(
                205,
                "walkin",
                "Walk-in requests are processed the same day.",
            ),
            g(209, "phone_script", "Say: I need an apostille."),
        ]
    }

    fn block(kind: &str, title: &str, body: &str, ids: &[i64]) -> PlanBlock {
        PlanBlock {
            id: "x".into(),
            kind: kind.into(),
            title: title.into(),
            body: body.into(),
            fact_ids: ids.to_vec(),
            show_for: vec![],
            tone: None,
            lines: vec![],
            office_ids: None,
            form_ids: None,
        }
    }

    fn plan(blocks: Vec<PlanBlock>) -> RenderPlanV1 {
        RenderPlanV1 {
            headline: "Get it in person the same day or by mail.".into(),
            paths: vec![PlanPath {
                id: "birth-certificate".into(),
                label: "Birth certificate".into(),
                fact_ids: vec![201],
            }],
            blocks,
            hide_topics: vec![],
        }
    }

    #[test]
    fn grounded_blocks_pass_and_get_server_ids() {
        let mut fees = block("fees", "It costs $15 per document", "", &[201]);
        fees.lines = vec![FeeLine {
            label: "Apostille".into(),
            amount_cents: 1500,
            per: "document".into(),
            fact_id: 201,
        }];
        let p = plan(vec![
            block(
                "office",
                "Visit the counter",
                "Open Monday to Friday, 8:00 AM to 4:30 PM.",
                &[202, 204],
            ),
            fees,
            block("mail", "Or mail it", "Send it to PO Box 4000.", &[203]),
        ]);
        let (clean, report) = validate_plan(&p, &facts());
        assert!(report.passed, "{report:?}");
        assert_eq!(clean.blocks.len(), 3, "{:#?}", report.dropped_blocks);
        assert_eq!(clean.blocks[0].id, "b1");
        assert!(report.dropped_blocks.is_empty());
        assert!(clean.hide_topics.contains(&"phone_script".to_string()));
    }

    #[test]
    fn planted_faults_are_dropped() {
        // Foreign fact id, invented fee, invented PO box, a URL, a bad
        // hour, a phone script, a kind that no cited topic fits.
        let mut bad_fee = block("fees", "It costs $15", "", &[201]);
        bad_fee.lines = vec![FeeLine {
            label: "Apostille".into(),
            amount_cents: 2000,
            per: "document".into(),
            fact_id: 201,
        }];
        let p = plan(vec![
            block("step", "Bring it", "Bring the document.", &[999]),
            block("step", "Pay", "Pay $25 at the counter.", &[201]),
            bad_fee,
            block("mail", "Mail it", "Send it to PO Box 5000.", &[203]),
            block(
                "step",
                "Read more",
                "See https://example.test for details.",
                &[205],
            ),
            block("timing", "Hours", "Open until 6:00 PM.", &[202]),
            block("step", "Call", "Say the script.", &[209]),
            block("fees", "Fees", "", &[205]),
            block(
                "step",
                "Go same day",
                "Walk-in requests are processed the same day.",
                &[205],
            ),
        ]);
        let (clean, report) = validate_plan(&p, &facts());
        assert_eq!(clean.blocks.len(), 1, "{:#?}", report.dropped_blocks);
        assert_eq!(clean.blocks[0].title, "Go same day");
        assert_eq!(report.dropped_blocks.len(), 8);
        let reasons: Vec<&str> = report
            .dropped_blocks
            .iter()
            .map(|d| d.reason.split(':').next().unwrap())
            .collect();
        for check in [
            "fact_ids_accepted",
            "numbers_in_citations",
            "fees_match_citations",
            "no_urls_html",
            "kind_topics",
        ] {
            assert!(reasons.contains(&check), "{check} in {reasons:?}");
        }
        assert!(report.passed, "one grounded block survives");
    }

    #[test]
    fn ungrounded_headline_fails_the_plan() {
        let mut p = plan(vec![block(
            "step",
            "Go same day",
            "Walk-in requests are processed the same day.",
            &[205],
        )]);
        p.headline = "It costs $99 and takes 3 days.".into();
        let (_, report) = validate_plan(&p, &facts());
        assert!(!report.passed);
        assert!(!report.checks[0].ok);
    }

    #[test]
    fn read_filter_drops_blocks_whose_facts_left_and_attaches_offices() {
        let p = plan(vec![
            block("office", "Visit", "Open Monday to Friday.", &[202, 204]),
            block("mail", "Mail", "Send it to PO Box 4000.", &[203]),
        ]);
        let accepted: BTreeSet<i64> = [202, 204].into_iter().collect();
        let out = filter_for_read(&p, &accepted, &[31], &[]).expect("office survives");
        assert_eq!(out.blocks.len(), 1);
        assert_eq!(out.blocks[0].office_ids, Some(vec![31]));
        assert!(out.paths.is_empty(), "path fact 201 left");
        assert!(filter_for_read(&p, &BTreeSet::new(), &[], &[]).is_none());
    }

    #[test]
    fn contract_seed_deserializes_and_composer_schema_enumerates_ids() {
        let seed: serde_json::Value =
            serde_json::from_str(include_str!("../../../docs/lanes/contracts/plan.json")).unwrap();
        let parsed: RenderPlanV1 = serde_json::from_value(seed["plan"].clone()).unwrap();
        assert_eq!(parsed.blocks.len(), 7);
        assert!(
            parsed
                .blocks
                .iter()
                .all(|b| KINDS.contains(&b.kind.as_str()))
        );
        let schema = plan_schema(&[201, 202]);
        assert_eq!(
            schema["properties"]["blocks"]["items"]["properties"]["fact_ids"]["items"]["enum"],
            json!([201, 202])
        );
        assert_eq!(schema["additionalProperties"], false);
        let (developer, per_run) = split_prompt(PLAN_PROMPT);
        assert!(!developer.contains("{{"), "stable cacheable prefix");
        assert!(per_run.contains("{{facts}}"));
    }

    #[test]
    fn accept_supersedes_and_audits_reject_leaves_accepted() {
        let conn = Connection::open_in_memory().unwrap();
        migrate_tables(&conn).unwrap();
        conn.execute_batch(
            "CREATE TABLE audit(id INTEGER PRIMARY KEY, at INTEGER, actor TEXT, action TEXT,
             entity TEXT, entity_id INTEGER, before_ref TEXT, after_ref TEXT, reason TEXT);",
        )
        .unwrap();
        let p = plan(vec![block(
            "step",
            "Go same day",
            "Walk-in requests are processed the same day.",
            &[205],
        )]);
        let report = Report {
            passed: true,
            checks: vec![],
            dropped_blocks: vec![],
        };
        let (a, va) =
            insert_proposed(&conn, "usa-wa-sos", &p, &report, &json!({}), Some(4), 10).unwrap();
        let (b, vb) =
            insert_proposed(&conn, "usa-wa-sos", &p, &report, &json!({}), Some(5), 11).unwrap();
        assert_eq!((va, vb), (1, 2));
        decide(&conn, a, true, "staff-1", 20).unwrap().unwrap();
        let row = decide(&conn, b, true, "staff-1", 30).unwrap().unwrap();
        assert_eq!(row.provenance["approved"]["by"], "Staff reviewer");
        let now = accepted_for(&conn, "usa-wa-sos").unwrap().unwrap();
        assert_eq!(now.id, b);
        let old: String = conn
            .query_row("SELECT status FROM page_plan WHERE id = ?1", [a], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(old, "superseded");
        assert!(
            decide(&conn, b, false, "staff-1", 40).unwrap().is_none(),
            "only proposed plans decide"
        );
        let audits: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM audit WHERE action = 'plan_accept'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(audits, 2);
    }
}
