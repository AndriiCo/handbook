//! `xtask triage`: sort fetched pages with a local Meta model (D-4c).
//!
//! `cargo xtask triage --pages <dir> [--pack packs/world/<ISO2>.json]
//! [--out <file>]`: for every `*.html` in `--pages` (a discovery job's
//! `pages/`, or any saved pages), the local OpenAI-compatible endpoint
//! (`LOCAL_MODEL_URL`, `LOCAL_MODEL`, optional `LOCAL_MODEL_KEY`, as for
//! `xtask eval --local`) answers four questions: is the page about
//! apostilles or legalization (or the office that issues them), what is
//! its main language, which of the pack's authorities does it belong to,
//! and which handbook topics does it cover. Every answer is checked
//! against closed lists (an unknown slug or topic is dropped and listed)
//! and written to a report (default `<pages>/triage.json`).
//!
//! Report only: nothing reaches the store, a pack or a fact. Facts are
//! still drafted by Spark with the quote check and approved in
//! `/review`; a local model drafts only after it matches Spark on the
//! gold set (`docs/specs/work-queue.md` §4.1).

use std::path::Path;

use contracts::topics::Topic;
use serde_json::{Value, json};

use crate::flag;

/// Page text sent per call (characters); the head of a page carries
/// its title, navigation and lead, which is what sorting needs.
const MAX_PAGE_CHARS: usize = 12_000;
/// Reply budget: four short fields.
const MAX_REPLY_TOKENS: u32 = 512;
/// Stable instructions (the `system` message).
const INSTRUCTIONS: &str = "You sort web pages for a handbook on apostilles and the \
legalization of public documents. Read the page text and answer in JSON. relevant: true \
when the page is about apostilles, legalization or authentication of documents, or is the \
site of an office that issues them; otherwise false. language: the page's main language as \
an ISO 639-1 code. authority: the slug of the listed authority the page belongs to, or an \
empty string when none fits. topics: the listed topics the page gives information on, or an \
empty list. Judge only from the text; never guess.";

pub fn run(args: &[String]) {
    let Some(pages) = flag(args, "pages") else {
        eprintln!("triage: --pages <dir of *.html> is required");
        std::process::exit(2);
    };
    let pages = Path::new(&pages);
    let out = flag(args, "out").map_or_else(|| pages.join("triage.json"), Into::into);
    let authorities = match flag(args, "pack") {
        Some(pack) => authorities(Path::new(&pack)).unwrap_or_else(|e| {
            eprintln!("triage: {e}");
            std::process::exit(2);
        }),
        None => Vec::new(),
    };
    let (base, model, key) = crate::eval::local_endpoint(|var| std::env::var(var).ok())
        .unwrap_or_else(|e| {
            eprintln!("triage: {}", e.replace("--local", "triage"));
            std::process::exit(2);
        });
    let mut files: Vec<_> = std::fs::read_dir(pages)
        .unwrap_or_else(|e| {
            eprintln!("triage: {}: {e}", pages.display());
            std::process::exit(2);
        })
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|path| path.extension().is_some_and(|ext| ext == "html"))
        .collect();
    files.sort();
    let texts: Vec<(String, String)> = files
        .iter()
        .filter_map(|path| {
            let html = std::fs::read(path).ok()?;
            let name = path.file_name()?.to_string_lossy().into_owned();
            let text = crate::worker::research_nonparty::page_text(&String::from_utf8_lossy(&html));
            Some((name, text))
        })
        .collect();
    eprintln!("triage: {} page(s) on {base}, model {model}", texts.len());
    // One leak per process: the client's model id is `'static`.
    let model: &'static str = Box::leak(model.into_boxed_str());
    let client = handbook_agent::model::MetaRestClient::local(base, key, model);
    let schema = schema();
    let ask = |user: String| {
        client
            .chat_json(&handbook_agent::model::ChatJsonRequest {
                model,
                developer: INSTRUCTIONS,
                user,
                schema_name: "PageTriage",
                schema: schema.clone(),
                reasoning_effort: "low",
                prompt_cache_key: "triage-v1".to_string(),
                max_completion_tokens: MAX_REPLY_TOKENS,
                pipeline: "triage",
                run_id: None,
            })
            .map(|reply| reply.value)
            .map_err(|e| e.to_string())
    };
    let mut report = triage(&texts, &authorities, ask);
    report["model"] = json!(model);
    let text = serde_json::to_string_pretty(&report).expect("report serializes");
    std::fs::write(&out, text + "\n").unwrap_or_else(|e| {
        eprintln!("triage: write {}: {e}", out.display());
        std::process::exit(1);
    });
    println!("triage: {} -> {}", report["summary"], out.display());
}

/// `(slug, name)` of the pack's visible authorities: the ones a page
/// may belong to (hidden rows are page fragments, not offices).
fn authorities(pack: &Path) -> Result<Vec<(String, String)>, String> {
    let bytes = std::fs::read(pack).map_err(|e| format!("{}: {e}", pack.display()))?;
    let file =
        crate::packs::PackFile::parse(&bytes).map_err(|e| format!("{}: {e}", pack.display()))?;
    Ok(file
        .pack
        .authorities
        .iter()
        .enumerate()
        .filter(|(i, _)| file.ext_of(*i).hidden != Some(true))
        .map(|(_, a)| (a.slug.clone(), a.name.clone()))
        .collect())
}

/// The reply schema: topics from the closed topic list.
fn schema() -> Value {
    let topics: Vec<&str> = Topic::ALL.iter().map(|t| t.as_str()).collect();
    json!({
        "type": "object",
        "properties": {
            "relevant": {"type": "boolean"},
            "language": {"type": "string"},
            "authority": {"type": "string"},
            "topics": {"type": "array", "items": {"type": "string", "enum": topics}},
        },
        "required": ["relevant", "language", "authority", "topics"],
        "additionalProperties": false,
    })
}

/// The per-page `user` message: the authority and topic lists, then the
/// page text (clipped to [`MAX_PAGE_CHARS`]).
fn user_text(text: &str, authorities: &[(String, String)]) -> String {
    let listed = if authorities.is_empty() {
        "(none listed)".to_string()
    } else {
        authorities
            .iter()
            .map(|(slug, name)| format!("{slug} | {name}"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let topics: Vec<&str> = Topic::ALL.iter().map(|t| t.as_str()).collect();
    let clipped: String = text.chars().take(MAX_PAGE_CHARS).collect();
    format!(
        "Authorities (slug | name):\n{listed}\n\nTopics: {}\n\nPage text:\n{clipped}",
        topics.join(", ")
    )
}

/// One answer checked against the closed lists: an unknown slug or
/// topic is dropped (and listed), a language that is not an ISO 639
/// code reads `und`. `Err` when the answer has no relevance flag.
fn checked(answer: &Value, authorities: &[(String, String)]) -> Result<Value, String> {
    let relevant = answer["relevant"]
        .as_bool()
        .ok_or("the answer has no relevant flag")?;
    let mut dropped = Vec::new();
    let language = answer["language"]
        .as_str()
        .unwrap_or("")
        .trim()
        .to_lowercase();
    let iso = (2..=3).contains(&language.len()) && language.bytes().all(|b| b.is_ascii_lowercase());
    let authority = match answer["authority"].as_str().map(str::trim) {
        None | Some("") => None,
        Some(slug) if authorities.iter().any(|(s, _)| s == slug) => Some(slug.to_string()),
        Some(slug) => {
            dropped.push(json!({"authority": slug}));
            None
        }
    };
    let mut topics: Vec<&str> = Vec::new();
    for topic in answer["topics"].as_array().into_iter().flatten() {
        let name = topic.as_str().unwrap_or("");
        match Topic::parse(name) {
            Ok(_) if !topics.contains(&name) => topics.push(name),
            Ok(_) => {}
            Err(_) => dropped.push(json!({"topic": topic})),
        }
    }
    Ok(json!({
        "relevant": relevant,
        "language": if iso { language } else { "und".to_string() },
        "authority": authority,
        "topics": topics,
        "dropped": dropped,
    }))
}

/// Sort every `(file, text)` with `ask` (the model call; tests pass a
/// closure) and roll the answers into the report.
fn triage(
    pages: &[(String, String)],
    authorities: &[(String, String)],
    ask: impl Fn(String) -> Result<Value, String>,
) -> Value {
    let mut rows = Vec::new();
    let (mut relevant, mut errors) = (0, 0);
    for (file, text) in pages {
        let answer = ask(user_text(text, authorities)).and_then(|a| checked(&a, authorities));
        rows.push(match answer {
            Ok(mut row) => {
                if row["relevant"] == true {
                    relevant += 1;
                }
                row["file"] = json!(file);
                row
            }
            Err(error) => {
                errors += 1;
                json!({"file": file, "error": error})
            }
        });
    }
    json!({
        "format": "triage.v1",
        "pages": rows,
        "summary": {"pages": pages.len(), "relevant": relevant, "errors": errors},
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mh() -> Vec<(String, String)> {
        authorities(&crate::worker::repo_root().join("packs/world/MH.json")).unwrap()
    }

    #[test]
    fn authorities_are_the_visible_rows() {
        let rows = mh();
        // 16 Marshall Islands rows, 9 hidden by curation.
        assert_eq!(rows.len(), 7);
        assert_eq!(rows[0].0, "mh-ca-1");
        assert!(!rows.iter().any(|(slug, _)| slug == "mh-ca-11"));
        let text = user_text(&"x".repeat(MAX_PAGE_CHARS + 50), &rows);
        assert!(text.contains("mh-ca-1 | Minister of Foreign Affairs of the Marshall Islands"));
        assert!(text.contains("legalization_chain"));
        assert!(text.ends_with(&"x".repeat(MAX_PAGE_CHARS)));
        assert!(!text.contains(&"x".repeat(MAX_PAGE_CHARS + 1)));
        assert_eq!(
            schema()["properties"]["topics"]["items"]["enum"][0],
            Topic::ALL[0].as_str()
        );
    }

    #[test]
    fn answers_are_checked_against_closed_lists() {
        let rows = mh();
        let row = checked(
            &json!({"relevant": true, "language": "EN", "authority": "mh-ca-1",
                    "topics": ["fees", "fees", "made_up"]}),
            &rows,
        )
        .unwrap();
        assert_eq!(row["language"], "en");
        assert_eq!(row["authority"], "mh-ca-1");
        assert_eq!(row["topics"], json!(["fees"]));
        assert_eq!(row["dropped"], json!([{"topic": "made_up"}]));
        let row = checked(
            &json!({"relevant": false, "language": "English", "authority": "mh-ca-99",
                    "topics": []}),
            &rows,
        )
        .unwrap();
        assert_eq!(row["language"], "und");
        assert!(row["authority"].is_null());
        assert_eq!(row["dropped"], json!([{"authority": "mh-ca-99"}]));
        let none = checked(&json!({"relevant": true, "authority": ""}), &rows).unwrap();
        assert!(none["authority"].is_null());
        assert!(checked(&json!({"language": "en"}), &rows).is_err());
    }

    #[test]
    fn triage_reports_every_page() {
        let rows = mh();
        let pages = vec![
            ("a.html".to_string(), "Apostille requests".to_string()),
            ("b.html".to_string(), "Weather".to_string()),
            ("c.html".to_string(), "Timeout".to_string()),
        ];
        let report = triage(&pages, &rows, |user: String| {
            if user.ends_with("Apostille requests") {
                Ok(
                    json!({"relevant": true, "language": "en", "authority": "mh-ca-1",
                          "topics": ["authentication"]}),
                )
            } else if user.ends_with("Weather") {
                Ok(json!({"relevant": false, "language": "en", "authority": "", "topics": []}))
            } else {
                Err("transport: timed out".to_string())
            }
        });
        assert_eq!(report["format"], "triage.v1");
        assert_eq!(
            report["summary"],
            json!({"pages": 3, "relevant": 1, "errors": 1})
        );
        assert_eq!(report["pages"][0]["file"], "a.html");
        assert_eq!(report["pages"][0]["topics"], json!(["authentication"]));
        assert_eq!(report["pages"][2]["error"], "transport: timed out");
    }
}
