//! `xtask segments`: split HCCH competent-authority pages into labelled,
//! quoted segments with a Meta model (D-5).
//!
//! `cargo xtask segments [--pages <dir>] [--iso2 XX] [--out <file>]
//! [--spark]`: every competent-authority page in `--pages` (default
//! `evals/world`, the recorded `details-aid*.html` pages; with an explicit
//! dir, every `*.html`) is cut by the model into segments labelled
//! `authority` (an office that issues apostilles), `address`, `contact`,
//! `scope` (which documents an office handles) or `note`. Every segment
//! carries a verbatim quote, and one whose quote is not on the page
//! (folded comparison, 12–400 characters) is dropped and listed: the
//! positional parsing in `world-index` turned such lines into fake
//! authorities (P2-1), and a label without a quote is no better.
//!
//! A `scope` segment that names one of the closed document classes
//! becomes a pack v2 `scope` proposal (the `ScopeEntry` shape, quote
//! beside it) for staff to copy into a pack and review; `--iso2` fills
//! its `issued_in`. Model: the local endpoint (`LOCAL_MODEL_URL`,
//! `LOCAL_MODEL`, optional `LOCAL_MODEL_KEY`, as for `eval --local`), or
//! Muse Spark with `--spark` (`META_API_KEY`, `META_BASE_URL`).
//!
//! Report only: nothing reaches the store, a pack or a fact.

use serde_json::{Value, json};

use crate::flag;
use crate::packs::DOC_CLASSES;
use crate::worker::research_nonparty::{fold, page_text};

/// Segment labels (closed list).
const LABELS: [&str; 5] = ["authority", "address", "contact", "scope", "note"];
/// Quotes outside this length prove nothing (too short) or are not a
/// line (too long).
const QUOTE_CHARS: std::ops::RangeInclusive<usize> = 12..=400;
/// Page text sent per call (the designation block is near the top).
const MAX_PAGE_CHARS: usize = 16_000;
const MAX_REPLY_TOKENS: u32 = 2_048;
const SPARK: &str = "muse-spark-1.3";
const INSTRUCTIONS: &str = "You read one page of the HCCH Apostille Convention site that \
lists a country's competent authorities. Split the page's designation text into segments, \
in page order, and label each one: authority (the name of an office that issues apostilles), \
address (a postal or street address line), contact (phone, fax, e-mail, website), scope (which \
documents an office handles or does not handle), note (anything else: declarations, dates, \
headings). For a scope segment also give doc_class when it clearly names one of the listed \
classes, else an empty string. quote: copied verbatim from the page, one line or phrase of 12 \
to 300 characters, no ellipses, no paraphrase. text: the segment in plain words (the office \
name for an authority). Skip site navigation and footers. Never add anything the page does \
not say.";

pub fn run(args: &[String]) {
    let explicit = flag(args, "pages");
    let pages = explicit.clone().map_or_else(
        || crate::worker::repo_root().join("evals/world"),
        Into::into,
    );
    let out = flag(args, "out").map_or_else(|| pages.join("segments.json"), Into::into);
    let iso2 = flag(args, "iso2").unwrap_or_default().to_ascii_uppercase();
    let mut files: Vec<_> = std::fs::read_dir(&pages)
        .unwrap_or_else(|e| {
            eprintln!("segments: {}: {e}", pages.display());
            std::process::exit(2);
        })
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|path| {
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned());
            path.extension().is_some_and(|ext| ext == "html")
                && (explicit.is_some() || name.is_some_and(|n| n.starts_with("details-")))
        })
        .collect();
    files.sort();
    let texts: Vec<(String, String)> = files
        .iter()
        .filter_map(|path| {
            let html = std::fs::read(path).ok()?;
            let name = path.file_name()?.to_string_lossy().into_owned();
            Some((name, page_text(&String::from_utf8_lossy(&html))))
        })
        .collect();
    let (client, model) = if args.iter().any(|a| a == "--spark") {
        let Ok(key) = std::env::var("META_API_KEY") else {
            eprintln!("segments: --spark needs META_API_KEY in the environment");
            std::process::exit(2);
        };
        let base = std::env::var("META_BASE_URL")
            .unwrap_or_else(|_| handbook_agent::model::META_BASE_URL.to_string());
        (handbook_agent::model::MetaRestClient::new(base, key), SPARK)
    } else {
        let (base, model, key) = crate::eval::local_endpoint(|var| std::env::var(var).ok())
            .unwrap_or_else(|e| {
                eprintln!("segments: {}", e.replace("--local", "segments"));
                std::process::exit(2);
            });
        // One leak per process: the client's model id is `'static`.
        let model: &'static str = Box::leak(model.into_boxed_str());
        (
            handbook_agent::model::MetaRestClient::local(base, key, model),
            model,
        )
    };
    eprintln!("segments: {} page(s), model {model}", texts.len());
    let schema = schema();
    let ask = |user: String| {
        client
            .chat_json(&handbook_agent::model::ChatJsonRequest {
                model,
                developer: INSTRUCTIONS,
                user,
                schema_name: "AuthorityPageSegments",
                schema: schema.clone(),
                reasoning_effort: "low",
                prompt_cache_key: "segments-v1".to_string(),
                max_completion_tokens: MAX_REPLY_TOKENS,
                pipeline: "segments",
                run_id: None,
            })
            .map(|reply| reply.value)
            .map_err(|e| e.to_string())
    };
    let mut report = segments(&texts, &iso2, ask);
    report["model"] = json!(model);
    let text = serde_json::to_string_pretty(&report).expect("report serializes");
    std::fs::write(&out, text + "\n").unwrap_or_else(|e| {
        eprintln!("segments: write {}: {e}", out.display());
        std::process::exit(1);
    });
    println!("segments: {} -> {}", report["summary"], out.display());
}

fn schema() -> Value {
    let mut classes: Vec<&str> = DOC_CLASSES.to_vec();
    classes.push("");
    json!({
        "type": "object",
        "required": ["segments"],
        "properties": {"segments": {"type": "array", "items": {
            "type": "object",
            "required": ["label", "text", "quote"],
            "properties": {
                "label": {"type": "string", "enum": LABELS},
                "text": {"type": "string"},
                "quote": {"type": "string"},
                "doc_class": {"type": "string", "enum": classes}
            }
        }}}
    })
}

/// The HCCH page a recorded `details-aid<N>.html` copies.
fn source_uri(file: &str) -> String {
    file.strip_prefix("details-aid")
        .and_then(|rest| rest.strip_suffix(".html"))
        .filter(|aid| !aid.is_empty() && aid.bytes().all(|b| b.is_ascii_digit()))
        .map_or_else(
            || format!("file:{file}"),
            |aid| format!("https://www.hcch.net/en/states/authorities/details3/?aid={aid}"),
        )
}

/// Keep the segments whose label is known and whose quote is on the
/// page; scope segments naming a document class become proposals.
fn checked(answer: &Value, page: &str, file: &str, iso2: &str) -> Result<Value, String> {
    let segments = answer["segments"]
        .as_array()
        .ok_or("the answer has no segments list")?;
    let folded_page = fold(page);
    let (mut kept, mut dropped, mut scope) = (Vec::new(), Vec::new(), Vec::new());
    for segment in segments {
        let label = segment["label"].as_str().unwrap_or("");
        let quote = segment["quote"].as_str().unwrap_or("").trim();
        let reason = if !LABELS.contains(&label) {
            Some("unknown label")
        } else if !QUOTE_CHARS.contains(&quote.chars().count()) {
            Some("quote is not 12 to 400 characters")
        } else if !folded_page.contains(&fold(quote)) {
            Some("quote is not on the page")
        } else {
            None
        };
        if let Some(reason) = reason {
            dropped.push(json!({"segment": segment, "reason": reason}));
            continue;
        }
        let text = segment["text"].as_str().map(str::trim).unwrap_or("");
        let text = if text.is_empty() { quote } else { text };
        kept.push(json!({"label": label, "text": text, "quote": quote}));
        let doc_class = segment["doc_class"].as_str().unwrap_or("");
        if label == "scope" && DOC_CLASSES.contains(&doc_class) {
            let mut entry = json!({"doc_class": doc_class, "source_uri": source_uri(file)});
            if !iso2.is_empty() {
                entry["issued_in"] = json!(iso2);
            }
            scope.push(json!({"scope": entry, "quote": quote}));
        }
    }
    Ok(json!({"file": file, "segments": kept, "dropped": dropped, "scope_proposals": scope}))
}

/// Segment every `(file, text)` with `ask` (the model call; tests pass
/// a closure) and roll the answers into the report.
fn segments(
    pages: &[(String, String)],
    iso2: &str,
    ask: impl Fn(String) -> Result<Value, String>,
) -> Value {
    let (mut rows, mut kept, mut dropped, mut proposals, mut errors) = (Vec::new(), 0, 0, 0, 0);
    for (file, text) in pages {
        let page: String = text.chars().take(MAX_PAGE_CHARS).collect();
        match ask(page.clone()).and_then(|a| checked(&a, &page, file, iso2)) {
            Ok(row) => {
                kept += row["segments"].as_array().map_or(0, Vec::len);
                dropped += row["dropped"].as_array().map_or(0, Vec::len);
                proposals += row["scope_proposals"].as_array().map_or(0, Vec::len);
                rows.push(row);
            }
            Err(error) => {
                errors += 1;
                rows.push(json!({"file": file, "error": error}));
            }
        }
    }
    json!({
        "format": "segments.v1",
        "pages": rows,
        "summary": {"pages": pages.len(), "segments": kept, "dropped": dropped,
                    "scope_proposals": proposals, "errors": errors},
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: &str = "Competent Authority: Ministry of Foreign Affairs, Consular \
                        Department. Address: 12 Main Street, Capital City. Telephone: +1 555 \
                        0100. Apostilles for court documents are issued by the Supreme Court \
                        registry. Declaration of 2 August 2004.";

    #[test]
    fn quotes_must_be_on_the_page_and_labels_known() {
        let answer = json!({"segments": [
            {"label": "authority", "text": "Ministry of Foreign Affairs",
             "quote": "Ministry of Foreign Affairs, Consular Department"},
            {"label": "address", "text": "", "quote": "12 Main Street, Capital City"},
            {"label": "contact", "text": "phone", "quote": "Telephone: +1 555 0100"},
            {"label": "scope", "text": "court documents: Supreme Court",
             "quote": "Apostilles for court documents are issued by the Supreme Court registry",
             "doc_class": "state_court"},
            {"label": "authority", "text": "Invented office",
             "quote": "The Ministry of Magic issues apostilles"},
            {"label": "office", "text": "x", "quote": "Declaration of 2 August 2004"},
            {"label": "note", "text": "x", "quote": "short"},
            {"label": "scope", "text": "y", "quote": "Declaration of 2 August 2004",
             "doc_class": "pets"}
        ]});
        let row = checked(&answer, PAGE, "details-aid1240.html", "xx").unwrap();
        let labels: Vec<&str> = row["segments"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["label"].as_str().unwrap())
            .collect();
        assert_eq!(
            labels,
            ["authority", "address", "contact", "scope", "scope"]
        );
        assert_eq!(row["segments"][1]["text"], "12 Main Street, Capital City");
        let reasons: Vec<&str> = row["dropped"]
            .as_array()
            .unwrap()
            .iter()
            .map(|d| d["reason"].as_str().unwrap())
            .collect();
        assert_eq!(
            reasons,
            [
                "quote is not on the page",
                "unknown label",
                "quote is not 12 to 400 characters"
            ]
        );
        // Only the scope segment naming a known class is a proposal.
        assert_eq!(
            row["scope_proposals"],
            json!([{"scope": {"doc_class": "state_court", "issued_in": "xx",
                "source_uri": "https://www.hcch.net/en/states/authorities/details3/?aid=1240"},
                "quote": "Apostilles for court documents are issued by the Supreme Court registry"}])
        );
        assert!(checked(&json!({"x": 1}), PAGE, "a.html", "").is_err());
        assert_eq!(source_uri("saved.html"), "file:saved.html");
    }

    #[test]
    fn every_page_is_reported_and_errors_counted() {
        let pages = vec![
            ("a.html".to_string(), PAGE.to_string()),
            ("b.html".to_string(), "Timeout".to_string()),
        ];
        let report = segments(&pages, "", |user: String| {
            if user.starts_with("Competent") {
                Ok(json!({"segments": [{"label": "authority", "text": "MFA",
                    "quote": "Ministry of Foreign Affairs, Consular Department"}]}))
            } else {
                Err("transport: timed out".to_string())
            }
        });
        assert_eq!(report["format"], "segments.v1");
        assert_eq!(
            report["summary"],
            json!({"pages": 2, "segments": 1, "dropped": 0, "scope_proposals": 0, "errors": 1})
        );
        assert_eq!(report["pages"][1]["error"], "transport: timed out");
        // The schema offers exactly the labels and the closed classes.
        let schema = schema();
        let item = &schema["properties"]["segments"]["items"]["properties"];
        assert_eq!(item["label"]["enum"], json!(LABELS));
        assert_eq!(item["doc_class"]["enum"][0], DOC_CLASSES[0]);
    }

    /// The recorded HCCH detail pages are the default input.
    #[test]
    fn recorded_detail_pages_have_text_to_segment() {
        let dir = crate::worker::repo_root().join("evals/world");
        let html = std::fs::read_to_string(dir.join("details-aid1240.html")).unwrap();
        assert!(page_text(&html).len() > 200);
    }
}
