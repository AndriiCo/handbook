//! Eval harness (A5): deterministic draft-quality grading on the gold set.
//!
//! `xtask eval [--live] [--store <path>] [--prompt v1|v2]`: for every
//! case in `evals/authorities/*.json`, draft records from the case evidence and
//! score the deterministic tier — quote found (sliceable, >= 8 chars),
//! value inside quote, topic match — each computed on its own, never
//! gated on the others. No database writes: grading calls
//! [`ModelClient::draft_records`](contracts::model::ModelClient) directly.
//! The summary lands in `<store>/quality.json` for `GET /api/v1/quality`.
//!
//! Stub default (offline-safe); `--live` drafts via the Meta Model API
//! with the sealed key (same path as `analyze --live`). The LLM-judge
//! tier ("same meaning") stays future work until a judge agrees with
//! hand labels; the gold statements/values are already recorded for it.

use std::collections::HashMap;

use contracts::model::{DraftRequest, ModelClient};
use handbook_agent::analyze::{MIN_QUOTE_CHARS, normalize};
use handbook_agent::model::StubClient;

use crate::{flag, open_store, unix_now};

/// One gold case (subset of the JSON on disk).
struct GoldCase {
    authority: String,
    topic: String,
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
                authority: authority.clone(),
                topic: case["topic"].as_str().unwrap_or("").to_string(),
                evidence: case["evidence"].as_str().unwrap_or("").to_string(),
            });
        }
    }
    cases
}

/// CLI entry: grade, print the audit line, write `quality.json`.
pub fn run(args: &[String]) {
    let live = args.iter().any(|a| a == "--live");
    let store = open_store(args);
    let client: Box<dyn ModelClient> = if live {
        let conn = crate::open_db(&store);
        if let Err(e) = handbook_store::db::migrate(&conn) {
            eprintln!("eval: migrate: {e}");
            std::process::exit(1);
        }
        Box::new(crate::live_client(&conn))
    } else {
        Box::new(StubClient)
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
}
