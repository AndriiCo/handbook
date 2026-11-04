//! Eval harness (A5): deterministic draft-quality grading on the gold set.
//!
//! `xtask eval [--live [--record <corpus>]] [--replay <corpus>] [--planted] [--store <path>] [--prompt v1|v2]`:
//! for every case in `evals/authorities/*.json`, draft records from the
//! case evidence and
//! score the deterministic tier — quote found (sliceable, >= 8 chars),
//! value inside quote, topic match — each computed on its own, never
//! gated on the others. No database writes: grading calls
//! [`ModelClient::draft_records`](contracts::model::ModelClient) directly.
//! The summary lands in `<store>/quality.json` for `GET /api/v1/quality`.
//! `--planted` additionally grades deterministic planted faults (value
//! swap, moved span, digit change, empty/truncated value, dropped
//! `not`/`only`, wrong topic) through the validator, with a before-view
//! beside the after, and records the outcome in the new `planted` +
//! `misses` fields.
//!
//! Stub default (offline-safe); `--live` drafts via the Meta Model API
//! with the sealed key (same path as `analyze --live`). `--local` drafts
//! via an OpenAI-compatible endpoint on our own hardware serving a Meta
//! model (llama.cpp `llama-server` on plant): `LOCAL_MODEL_URL` (base,
//! e.g. `http://127.0.0.1:8080/v1`), `LOCAL_MODEL` (the id it serves;
//! from the environment, never the repo) and, if it wants one,
//! `LOCAL_MODEL_KEY`. Rule (D-4): a local model drafts only after it
//! matches Spark here on `quote_found_rate` and `value_in_quote_rate`
//! (same gold set, same prompt); until then it grades only. The LLM-judge
//! tier ("same meaning") stays future work until a judge agrees with
//! hand labels; the gold statements/values are already recorded for it.

use std::collections::HashMap;

use contracts::model::{
    CitationRef, DraftRequest, DraftResponse, ModelClient, ProposedRecord, Usage,
};
use handbook_agent::analyze::{MIN_QUOTE_CHARS, ValidationErrorKind, normalize, validate_response};

use crate::{flag, open_store, unix_now};

/// One gold case (subset of the JSON on disk).
struct GoldCase {
    id: String,
    authority: String,
    topic: String,
    value: String,
    evidence: String,
}

/// Scores over all drafted records.
#[derive(Default, PartialEq, Eq, Debug)]
struct Tallies {
    drafted: u64,
    quote_found: u64,
    value_in_quote: u64,
    topic_match: u64,
}

fn load_gold(dir: &std::path::Path) -> Vec<GoldCase> {
    let mut cases = Vec::new();
    let entries = std::fs::read_dir(dir).unwrap_or_else(|e| {
        eprintln!("eval: gold dir {}: {e}", dir.display());
        std::process::exit(1);
    });
    let mut files: Vec<_> = entries
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .collect();
    files.sort();
    if files.is_empty() {
        eprintln!("eval: no gold files in {}", dir.display());
        std::process::exit(1);
    }
    for path in files {
        let text = std::fs::read_to_string(&path).unwrap_or_else(|e| {
            eprintln!("eval: {}: {e}", path.display());
            std::process::exit(1);
        });
        let value: serde_json::Value = serde_json::from_str(&text).unwrap_or_else(|e| {
            eprintln!("eval: {}: {e}", path.display());
            std::process::exit(1);
        });
        let authority = value["authority"].as_str().unwrap_or("").to_string();
        let Some(list) = value["cases"].as_array() else {
            eprintln!(
                "eval: {}: want {{\"authority\", \"cases\"}}",
                path.display()
            );
            std::process::exit(1);
        };
        for case in list {
            cases.push(GoldCase {
                id: case["id"].as_str().unwrap_or("").to_string(),
                authority: authority.clone(),
                topic: case["topic"].as_str().unwrap_or("").to_string(),
                value: case["value"].as_str().unwrap_or("").to_string(),
                evidence: case["evidence"].as_str().unwrap_or("").to_string(),
            });
        }
    }
    cases
}

/// Environment of `--local` (D-4): base URL, model id, optional key.
pub(crate) const LOCAL_ENV: [&str; 3] = ["LOCAL_MODEL_URL", "LOCAL_MODEL", "LOCAL_MODEL_KEY"];

/// True for a model id of a Meta family (`muse-…`, Llama): the organizer
/// rule holds for local endpoints too. A guard against pointing the eval
/// at the wrong model, not proof of the weights the endpoint serves.
pub(crate) fn is_meta_model(id: &str) -> bool {
    let id = id.trim().to_ascii_lowercase();
    !id.is_empty() && (id.starts_with("muse-") || id.contains("llama"))
}

/// `(base_url, model, key)` for `--local` from `get` (the environment),
/// or why not.
pub(crate) fn local_endpoint(
    get: impl Fn(&str) -> Option<String>,
) -> Result<(String, String, String), String> {
    let [url_var, model_var, key_var] = LOCAL_ENV;
    let base = get(url_var).unwrap_or_default();
    if !(base.starts_with("http://") || base.starts_with("https://")) {
        return Err(format!("--local needs {url_var} (http(s) base URL)"));
    }
    let model = get(model_var).unwrap_or_default();
    if !is_meta_model(&model) {
        return Err(format!(
            "--local needs {model_var} naming a Meta model (muse-… or Llama), not {model:?}"
        ));
    }
    Ok((
        base,
        model.trim().to_string(),
        get(key_var).unwrap_or_default(),
    ))
}

/// CLI entry: grade, print the audit line, write `quality.json`.
pub fn run(args: &[String]) {
    let store = open_store(args);
    let client: Box<dyn ModelClient> = if args.iter().any(|a| a == "--local") {
        if ["--live", "--record", "--replay"]
            .iter()
            .any(|f| args.iter().any(|a| a == f))
        {
            eprintln!("eval: --local drafts on its own endpoint; drop --live/--record/--replay");
            std::process::exit(2);
        }
        let (base, model, key) =
            local_endpoint(|var| std::env::var(var).ok()).unwrap_or_else(|e| {
                eprintln!("eval: {e}");
                std::process::exit(2);
            });
        eprintln!("eval: local endpoint {base}, model {model}");
        // One leak per process: `ModelClient::model_id` is `'static`.
        let model: &'static str = Box::leak(model.into_boxed_str());
        Box::new(handbook_agent::model::MetaRestClient::local(
            base, key, model,
        ))
    } else {
        crate::draft_client(args, || {
            let conn = crate::open_db(&store);
            if let Err(e) = handbook_store::db::migrate(&conn) {
                eprintln!("eval: migrate: {e}");
                std::process::exit(1);
            }
            crate::live_client(&conn)
        })
    };
    let model = client.model_id().to_string();
    // Gold lives at the workspace root; xtask's own manifest dir is one
    // level down, so prefer the caller's working directory.
    let cwd_gold = std::env::current_dir()
        .unwrap_or_else(|_| std::path::PathBuf::from("."))
        .join("evals/authorities");
    let manifest_gold = std::path::Path::new(
        &std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".to_string()),
    )
    .join("../evals/authorities");
    let gold_dir = if let Some(dir) = flag(args, "evals") {
        std::path::PathBuf::from(dir)
    } else if cwd_gold.is_dir() {
        cwd_gold
    } else {
        manifest_gold
    };
    let cases = load_gold(&gold_dir);

    // Prompt generation under test: `--prompt v2` drafts every case
    // with `aa-prompt-v2`; anything else (default) stays on v1, and
    // an unknown value fails fast instead of grading the wrong prompt.
    let prompt_flag = flag(args, "prompt");
    let prompt_version: &'static str = match prompt_flag.as_deref() {
        None | Some("v1") => handbook_agent::runner::PROMPT_VERSION,
        Some("v2") => handbook_agent::model::PROMPT_VERSION_V2,
        Some(other) => {
            eprintln!("eval: unknown --prompt {other:?} (want v1 or v2)");
            std::process::exit(1);
        }
    };

    let mut tallies = Tallies::default();
    let mut input_tokens: u64 = 0;
    let mut output_tokens: u64 = 0;
    let mut latency_ms: u64 = 0;
    for case in &cases {
        let req = DraftRequest {
            authority_slug: case.authority.clone(),
            prompt_version,
            input_text: case.evidence.clone(),
            max_records: handbook_agent::runner::DEFAULT_MAX_RECORDS,
            capture_id: handbook_agent::runner::UNATTRIBUTED_CAPTURE_ID,
            doc_ids: vec!["gold".to_string()],
        };
        let started = std::time::Instant::now();
        let resp = match client.draft_records(&req) {
            Ok(resp) => resp,
            Err(e) => {
                eprintln!("eval: draft: {e} (no writes)");
                std::process::exit(1);
            }
        };
        latency_ms += started.elapsed().as_millis() as u64;
        input_tokens += u64::from(resp.usage.input_tokens);
        output_tokens += u64::from(resp.usage.output_tokens);
        score_records(&resp.records, &case.evidence, &case.topic, &mut tallies);
    }

    let rate = |n: u64| {
        if tallies.drafted == 0 {
            0.0
        } else {
            n as f64 / tallies.drafted as f64
        }
    };
    // Planted-fault grading (`--planted`): deterministic mutations of
    // the gold set through the validator + the deterministic tiers,
    // with a before-view (the pre-gate validator) beside the after.
    let (planted_json, misses_json) = if args.iter().any(|a| a == "--planted") {
        match run_planted(&cases) {
            Ok(report) => {
                print_planted(&report);
                (
                    serde_json::json!({
                        "caught": report.caught_after(),
                        "total": report.total(),
                        "before_caught": report.caught_before(),
                    }),
                    serde_json::json!(report.misses),
                )
            }
            Err(e) => {
                eprintln!("eval: planted: {e}");
                std::process::exit(1);
            }
        }
    } else {
        (serde_json::Value::Null, serde_json::json!([]))
    };
    let now = unix_now();
    let summary = serde_json::json!({
        "run_at": iso8601(now),
        "gold_count": cases.len(),
        "quote_found_rate": rate(tallies.quote_found),
        "value_in_quote_rate": rate(tallies.value_in_quote),
        "topic_accuracy": rate(tallies.topic_match),
        "model": model,
        "prompt": prompt_version,
        "drafted_total": tallies.drafted,
        "planted": planted_json,
        "misses": misses_json,
        "input_tokens": input_tokens,
        "output_tokens": output_tokens,
        "latency_ms": latency_ms,
    });
    let dest = store.root().join("quality.json");
    if let Err(e) = std::fs::create_dir_all(store.root())
        .map_err(|e| e.to_string())
        .and_then(|()| {
            std::fs::write(&dest, serde_json::to_string_pretty(&summary).unwrap())
                .map_err(|e| e.to_string())
        })
    {
        eprintln!("eval: write {}: {e}", dest.display());
        std::process::exit(1);
    }
    println!(
        "eval: cases={} drafted={} quote_found_rate={:.3} value_in_quote_rate={:.3} topic_accuracy={:.3} model={} prompt={} input_tokens={} output_tokens={} latency_ms={} -> {}",
        cases.len(),
        tallies.drafted,
        rate(tallies.quote_found),
        rate(tallies.value_in_quote),
        rate(tallies.topic_match),
        model,
        prompt_version,
        input_tokens,
        output_tokens,
        latency_ms,
        dest.display()
    );
}

/// Deterministic tier over one draft batch: the evidence is the only
/// admissible source (`doc_id "gold"`). Validation errors still count
/// the record as drafted but failing every tier below.
fn score_records(
    records: &[contracts::model::ProposedRecord],
    evidence: &str,
    gold_topic: &str,
    tallies: &mut Tallies,
) {
    // a5-honest: each check stands alone. A right topic with a broken
    // citation still counts for topic_match; a long quote that omits the
    // value still counts for quote_found. Gating every tally on full
    // validation made the three rates identical by construction.
    let mut sources = HashMap::new();
    sources.insert("gold".to_string(), evidence.to_string());
    for record in records {
        tallies.drafted += 1;
        if record.topic == gold_topic {
            tallies.topic_match += 1;
        }
        let Some(source) = sources.get(&record.citation.doc_id) else {
            continue;
        };
        let (start, end) = (
            record.citation.byte_start as usize,
            record.citation.byte_end as usize,
        );
        let Some(quote) = source.get(start..end) else {
            continue;
        };
        if quote.chars().count() >= MIN_QUOTE_CHARS {
            tallies.quote_found += 1;
            if normalize(quote).contains(&normalize(&record.value)) {
                tallies.value_in_quote += 1;
            }
        }
    }
}

/// Planted-fault grading (`--planted`): deterministic mutations of
/// the gold set, each scored through the analysis validator plus the
/// deterministic tiers. A plant is *caught* when the validator
/// rejects it or any tier fails. The *before* view replays the same
/// record through the pre-gate validator ([`ValidationErrorKind::BadValue`]
/// ignored), so the table shows what the new value gate adds. No
/// RNG anywhere: every mutation is a fixed function of the case.
const PLANTED_WINDOW: usize = 40;

/// One mutated gold record under test.
struct Plant {
    case_id: String,
    /// Record topic under test (mutated by `wrong_topic`).
    topic: String,
    /// Tier reference (always the gold topic).
    gold_topic: String,
    value: String,
    evidence: String,
    start: usize,
    end: usize,
}

impl Plant {
    fn new(case: &GoldCase, topic: String, value: String, span: (usize, usize)) -> Plant {
        Plant {
            case_id: case.id.clone(),
            topic,
            gold_topic: case.topic.clone(),
            value,
            evidence: case.evidence.clone(),
            start: span.0,
            end: span.1,
        }
    }
}

struct PlantedRow {
    class: &'static str,
    total: usize,
    caught_after: usize,
    caught_before: usize,
}

struct PlantedReport {
    rows: Vec<PlantedRow>,
    misses: Vec<String>,
}

impl PlantedReport {
    fn total(&self) -> usize {
        self.rows.iter().map(|row| row.total).sum()
    }

    fn caught_after(&self) -> usize {
        self.rows.iter().map(|row| row.caught_after).sum()
    }

    fn caught_before(&self) -> usize {
        self.rows.iter().map(|row| row.caught_before).sum()
    }
}

/// Byte-exact span of `value` inside `evidence` (case-sensitive
/// first, case-insensitive fallback); `None` when absent.
fn byte_span(evidence: &str, value: &str) -> Option<(usize, usize)> {
    if let Some(start) = evidence.find(value) {
        return Some((start, start + value.len()));
    }
    let lower = evidence.to_lowercase();
    let needle = value.to_lowercase();
    lower
        .find(needle.as_str())
        .map(|start| (start, start + value.len()))
}

/// Byte range of a `chars`-character window at the evidence tail;
/// char-boundary safe by construction.
fn window_from_end(text: &str, chars: usize) -> (usize, usize) {
    let total: usize = text.chars().count();
    let skip = total.saturating_sub(chars);
    let start = text
        .char_indices()
        .nth(skip)
        .map(|(i, _)| i)
        .unwrap_or(text.len());
    (start, text.len())
}

/// Byte range of a `chars`-character window at the evidence head.
fn window_from_start(text: &str, chars: usize) -> (usize, usize) {
    let end = text
        .char_indices()
        .nth(chars)
        .map(|(i, _)| i)
        .unwrap_or(text.len());
    (0, end)
}

/// Bump the first ASCII digit (`9` wraps to `0`); digitless values
/// gain a trailing ` 9`.
fn digit_bump(value: &str) -> String {
    let mut out = value.to_string();
    if let Some(i) = out.find(|c: char| c.is_ascii_digit()) {
        let d = out.as_bytes()[i];
        let bumped = (b'0' + (d - b'0' + 1) % 10) as char;
        out.replace_range(i..=i, &bumped.to_string());
    } else {
        out.push_str(" 9");
    }
    out
}

/// Drop a standalone `not`/`only` word; `None` when the value
/// carries neither (the class then skips the case).
fn drop_negation(value: &str) -> Option<String> {
    fn is_negation(word: &str) -> bool {
        let core: String = word.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
        matches!(core.to_lowercase().as_str(), "not" | "only")
    }
    let words: Vec<&str> = value.split_whitespace().collect();
    if !words.iter().any(|w| is_negation(w)) {
        return None;
    }
    Some(
        words
            .into_iter()
            .filter(|w| !is_negation(w))
            .collect::<Vec<_>>()
            .join(" "),
    )
}

/// A frozen topic that is valid but wrong for the case.
fn wrong_topic(gold: &str) -> String {
    contracts::topics::Topic::ALL
        .iter()
        .map(|t| t.as_str())
        .find(|t| *t != gold)
        .unwrap_or("unfiled")
        .to_string()
}

/// Grade one plant: caught (after) when the validator rejects it or
/// any deterministic tier fails; the before-view ignores the new
/// `BadValue` gate.
fn score_plant(plant: &Plant, allowed: &[&str]) -> (bool, bool) {
    let record = ProposedRecord {
        topic: plant.topic.clone(),
        field: "planted".to_string(),
        value: plant.value.clone(),
        citation: CitationRef {
            doc_id: "gold".to_string(),
            byte_start: plant.start as u32,
            byte_end: plant.end as u32,
        },
        confidence: 1.0,
    };
    let resp = DraftResponse {
        records: vec![record],
        usage: Usage {
            input_tokens: 0,
            output_tokens: 0,
            cached_tokens: 0,
            reasoning_tokens: 0,
        },
        summary: String::new(),
    };
    let mut sources = HashMap::new();
    sources.insert("gold".to_string(), plant.evidence.clone());
    let errors = validate_response(&resp, &sources, allowed);
    let mut tallies = Tallies::default();
    score_records(
        &resp.records,
        &plant.evidence,
        &plant.gold_topic,
        &mut tallies,
    );
    let tiers_pass = tallies.drafted == 1
        && tallies.quote_found == 1
        && tallies.value_in_quote == 1
        && tallies.topic_match == 1;
    let after = errors.is_err() || !tiers_pass;
    let legacy_open = match &errors {
        Ok(()) => false,
        Err(errs) => errs.iter().any(|e| e.kind != ValidationErrorKind::BadValue),
    };
    (after, legacy_open || !tiers_pass)
}

/// Grade one mutation class over its plants.
fn grade(class: &'static str, plants: Vec<Plant>, allowed: &[&str]) -> (PlantedRow, Vec<String>) {
    let mut row = PlantedRow {
        class,
        total: plants.len(),
        caught_after: 0,
        caught_before: 0,
    };
    let mut misses = Vec::new();
    for plant in &plants {
        let (after, before) = score_plant(plant, allowed);
        if after {
            row.caught_after += 1;
        } else {
            misses.push(format!("{class}:{}", plant.case_id));
        }
        if before {
            row.caught_before += 1;
        }
    }
    (row, misses)
}

fn run_planted(cases: &[GoldCase]) -> Result<PlantedReport, String> {
    let allowed: Vec<&str> = contracts::topics::Topic::ALL
        .iter()
        .map(|t| t.as_str())
        .collect();
    let mut rows = Vec::new();
    let mut misses = Vec::new();
    let mut take = |row: PlantedRow, mut m: Vec<String>| {
        rows.push(row);
        misses.append(&mut m);
    };

    // value_swap: each case takes the next case's value over its own
    // span (citations untouched, values estranged).
    if cases.len() >= 2 {
        let mut plants = Vec::new();
        for (i, case) in cases.iter().enumerate() {
            let donor = &cases[(i + 1) % cases.len()];
            let Some(span) = byte_span(&case.evidence, &case.value) else {
                return Err(format!("value_swap: no byte span for {}", case.id));
            };
            plants.push(Plant::new(
                case,
                case.topic.clone(),
                donor.value.clone(),
                span,
            ));
        }
        let (row, m) = grade("value_swap", plants, &allowed);
        take(row, m);
    }
    // span_moved: the value is right but the citation window slid to
    // a value-free slice (tail preferred, head as fallback).
    {
        let mut plants = Vec::new();
        for case in cases {
            let picked = [
                window_from_end(&case.evidence, PLANTED_WINDOW),
                window_from_start(&case.evidence, PLANTED_WINDOW),
            ]
            .into_iter()
            .find(|(s, e)| {
                case.evidence[*s..*e].chars().count() >= MIN_QUOTE_CHARS
                    && !normalize(&case.evidence[*s..*e]).contains(&normalize(&case.value))
            });
            let Some(span) = picked else {
                return Err(format!("span_moved: no clean window for {}", case.id));
            };
            plants.push(Plant::new(
                case,
                case.topic.clone(),
                case.value.clone(),
                span,
            ));
        }
        let (row, m) = grade("span_moved", plants, &allowed);
        take(row, m);
    }
    // digit_change: one digit bumped (digitless values gain ` 9`).
    {
        let mut plants = Vec::new();
        for case in cases {
            let Some(span) = byte_span(&case.evidence, &case.value) else {
                return Err(format!("digit_change: no byte span for {}", case.id));
            };
            plants.push(Plant::new(
                case,
                case.topic.clone(),
                digit_bump(&case.value),
                span,
            ));
        }
        let (row, m) = grade("digit_change", plants, &allowed);
        take(row, m);
    }
    // empty_value: the pre-gate validator waves `""` through
    // (`contains("")` is always true); the value gate must catch it.
    {
        let mut plants = Vec::new();
        for case in cases {
            let Some(span) = byte_span(&case.evidence, &case.value) else {
                return Err(format!("empty_value: no byte span for {}", case.id));
            };
            plants.push(Plant::new(case, case.topic.clone(), String::new(), span));
        }
        let (row, m) = grade("empty_value", plants, &allowed);
        take(row, m);
    }
    // truncated_value: a single surviving character usually sits
    // somewhere in the quote, so only the length gate catches it.
    {
        let mut plants = Vec::new();
        for case in cases {
            let Some(span) = byte_span(&case.evidence, &case.value) else {
                return Err(format!("truncated_value: no byte span for {}", case.id));
            };
            let one: String = case.value.chars().take(1).collect();
            plants.push(Plant::new(case, case.topic.clone(), one, span));
        }
        let (row, m) = grade("truncated_value", plants, &allowed);
        take(row, m);
    }
    // dropped_negation: `not`/`only` removed; cases without either
    // word are skipped (deterministic: fixed function of the value).
    {
        let mut plants = Vec::new();
        for case in cases {
            let Some(dropped) = drop_negation(&case.value) else {
                continue;
            };
            let Some(span) = byte_span(&case.evidence, &case.value) else {
                return Err(format!("dropped_negation: no byte span for {}", case.id));
            };
            plants.push(Plant::new(case, case.topic.clone(), dropped, span));
        }
        let (row, m) = grade("dropped_negation", plants, &allowed);
        take(row, m);
    }
    // wrong_topic: a valid but wrong frozen topic; only the
    // topic-match tier can catch it.
    {
        let mut plants = Vec::new();
        for case in cases {
            let Some(span) = byte_span(&case.evidence, &case.value) else {
                return Err(format!("wrong_topic: no byte span for {}", case.id));
            };
            plants.push(Plant::new(
                case,
                wrong_topic(&case.topic),
                case.value.clone(),
                span,
            ));
        }
        let (row, m) = grade("wrong_topic", plants, &allowed);
        take(row, m);
    }
    Ok(PlantedReport { rows, misses })
}

fn print_planted(report: &PlantedReport) {
    for row in &report.rows {
        println!(
            "planted {}: {}/{} caught (before {}/{})",
            row.class, row.caught_after, row.total, row.caught_before, row.total
        );
    }
    println!(
        "planted total: {}/{} caught, {} missed (before {}/{})",
        report.caught_after(),
        report.total(),
        report.misses.len(),
        report.caught_before(),
        report.total()
    );
    if report.misses.is_empty() {
        println!("planted misses: none");
    } else {
        println!("planted misses: {}", report.misses.join(", "));
    }
}

/// Unix seconds → `YYYY-MM-DDTHH:MM:SSZ` (same civil-from-days shape as
/// the web tier's citation dates; every quality consumer parses ISO).
fn iso8601(unix: i64) -> String {
    let days = unix.div_euclid(86_400);
    let secs = unix.rem_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        secs / 3_600,
        (secs % 3_600) / 60,
        secs % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use contracts::model::{CitationRef, ProposedRecord};

    fn record(topic: &str, value: &str, start: u32, end: u32) -> ProposedRecord {
        ProposedRecord {
            topic: topic.to_string(),
            field: "f".to_string(),
            value: value.to_string(),
            citation: CitationRef {
                doc_id: "gold".to_string(),
                byte_start: start,
                byte_end: end,
            },
            confidence: 0.9,
        }
    }

    #[test]
    fn full_marks_for_grounded_record() {
        let evidence = "Counter hours are Mon-Fri 8-5 excluding state holidays.";
        // "Mon-Fri 8-5" sits at bytes 18..29.
        assert_eq!(&evidence[18..29], "Mon-Fri 8-5");
        let mut tallies = Tallies::default();
        score_records(
            &[record("hours", "Mon-Fri 8-5", 18, 29)],
            evidence,
            "hours",
            &mut tallies,
        );
        assert_eq!(
            tallies,
            Tallies {
                drafted: 1,
                quote_found: 1,
                value_in_quote: 1,
                topic_match: 1,
            }
        );
    }

    #[test]
    fn failures_count_as_drafted_only() {
        let evidence = "Counter hours are Mon-Fri 8-5 excluding state holidays.";
        let mut bad_doc = record("hours", "Mon-Fri 8-5", 18, 29);
        bad_doc.citation.doc_id = "elsewhere".to_string();
        let mut tallies = Tallies::default();
        score_records(
            &[
                bad_doc,                                // MissingDoc
                record("fees", "Mon-Fri 8-5", 18, 29),  // wrong topic
                record("hours", "nope", 18, 29),        // value outside quote
                record("hours", "Mon-Fri 8-5", 18, 21), // short quote ("Mon")
            ],
            evidence,
            "hours",
            &mut tallies,
        );
        // a5-honest: independent checks. Right topic counts even with a
        // broken citation (3/4); a long quote counts even for the wrong
        // topic or a missing value (2/4 found, 1/4 with value).
        assert_eq!(
            tallies,
            Tallies {
                drafted: 4,
                quote_found: 2,
                value_in_quote: 1,
                topic_match: 3,
            }
        );
    }

    #[test]
    fn iso8601_shapes_dates() {
        assert_eq!(iso8601(1_800_000_050), "2027-01-15T08:00:50Z");
        assert_eq!(iso8601(0), "1970-01-01T00:00:00Z");
    }

    #[test]
    fn digit_bump_rotates_first_digit() {
        assert_eq!(digit_bump("PO Box 40228"), "PO Box 50228");
        assert_eq!(digit_bump("$9 flat"), "$0 flat");
        assert_eq!(digit_bump("no digits here"), "no digits here 9");
    }

    #[test]
    fn drop_negation_skips_clean_values() {
        assert_eq!(
            drop_negation("Certificate of Authentication, not an apostille"),
            Some("Certificate of Authentication, an apostille".to_string())
        );
        assert_eq!(drop_negation("mornings only"), Some("mornings".to_string()));
        assert_eq!(drop_negation("$15 per document"), None);
    }

    #[test]
    fn windows_stay_on_char_boundaries() {
        let text = "Montréal open Mon-Fri 8-5 for walk-in filing every week.";
        let (s, e) = window_from_end(text, 40);
        assert!(text.is_char_boundary(s) && text.is_char_boundary(e));
        assert_eq!(text[s..e].chars().count(), 40);
        let (s, e) = window_from_start(text, 40);
        assert_eq!(
            (s, e),
            (0, text.char_indices().nth(40).map(|(i, _)| i).unwrap())
        );
    }

    #[test]
    fn byte_span_prefers_exact_then_case_insensitive() {
        assert_eq!(
            byte_span("to PO Box 40228 with", "PO Box 40228"),
            Some((3, 15))
        );
        assert_eq!(
            byte_span("to po box 40228 with", "PO Box 40228"),
            Some((3, 15))
        );
        assert_eq!(byte_span("nothing here", "PO Box 40228"), None);
    }

    fn planted_case(id: &str, topic: &str, value: &str, evidence: &str) -> GoldCase {
        GoldCase {
            id: id.to_string(),
            authority: "test".to_string(),
            topic: topic.to_string(),
            value: value.to_string(),
            evidence: evidence.to_string(),
        }
    }

    #[test]
    fn planted_run_grades_every_class() {
        let cases = vec![
            planted_case(
                "t-a",
                "fees",
                "$15 per document",
                "The published fee table shows $15 per document for standard authentication service today.",
            ),
            planted_case(
                "t-b",
                "hours",
                "Mon-Fri 8-5",
                "Customer service counters stay open Mon-Fri 8-5 except on weekends and public holidays here.",
            ),
        ];
        let report = run_planted(&cases).expect("synthetic plants grade");
        // Six classes plant both cases; dropped_negation skips both
        // (no `not`/`only` in either value).
        assert_eq!(report.total(), 12);
        assert_eq!(report.caught_after(), 12);
        assert!(report.misses.is_empty());
        // Before the value gate, the empty and truncated plants walk
        // through: their single characters sit inside their quotes.
        assert_eq!(report.caught_before(), 8);
        let dropped = report
            .rows
            .iter()
            .find(|row| row.class == "dropped_negation")
            .expect("dropped row present");
        assert_eq!(dropped.total, 0);
    }

    #[test]
    fn empty_and_truncated_plants_show_the_gate_gap() {
        let allowed: Vec<&str> = contracts::topics::Topic::ALL
            .iter()
            .map(|t| t.as_str())
            .collect();
        let case = planted_case(
            "t-a",
            "fees",
            "$15 per document",
            "The published fee table shows $15 per document for standard authentication service today.",
        );
        let (start, end) = byte_span(&case.evidence, &case.value).expect("span");
        for value in [String::new(), "$".to_string()] {
            let plant = Plant::new(&case, case.topic.clone(), value, (start, end));
            let (after, before) = score_plant(&plant, &allowed);
            assert!(after, "gate catches the short value");
            assert!(!before, "pre-gate validator misses it");
        }
    }

    /// D-4: `--local` reads its endpoint from the environment and takes
    /// Meta model ids only.
    #[test]
    fn local_endpoint_needs_a_url_and_a_meta_model() {
        let env = |pairs: &'static [(&'static str, &'static str)]| {
            move |var: &str| {
                pairs
                    .iter()
                    .find(|(k, _)| *k == var)
                    .map(|(_, v)| (*v).to_string())
            }
        };
        let ok = local_endpoint(env(&[
            ("LOCAL_MODEL_URL", "http://127.0.0.1:8080/v1"),
            ("LOCAL_MODEL", " Llama-3.3-70B-Instruct "),
        ]))
        .unwrap();
        assert_eq!(
            ok,
            (
                "http://127.0.0.1:8080/v1".to_string(),
                "Llama-3.3-70B-Instruct".to_string(),
                String::new()
            )
        );
        assert!(local_endpoint(env(&[("LOCAL_MODEL", "llama-3")])).is_err());
        assert!(
            local_endpoint(env(&[
                ("LOCAL_MODEL_URL", "ftp://x"),
                ("LOCAL_MODEL", "llama-3")
            ]))
            .is_err()
        );
        let wrong = local_endpoint(env(&[
            ("LOCAL_MODEL_URL", "http://127.0.0.1:8080/v1"),
            ("LOCAL_MODEL", "acme-7b-instruct"),
        ]))
        .unwrap_err();
        assert!(wrong.contains("Meta model"), "{wrong}");
        assert!(is_meta_model("muse-spark-1.3") && !is_meta_model(""));
    }
}
