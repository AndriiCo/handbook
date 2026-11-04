//! `cargo xtask ask "<question>" --slug <slug> [--base <url>] [--replay <fixture>]`
//!
//! A third-party agent asks the handbook a question; Muse Spark answers
//! with verbatim quotes cited to `source_url` + `captured_at`, or the
//! command refuses (non-zero exit) when no fact covers the question.
//!
//! Flow: `GET {base}/api/v1/authorities/{slug}/facts` loads the public
//! accepted facts; one chat-completions call carries exactly ONE tool,
//! `lookup_authority(slug)`, backed by that endpoint; the tool result
//! feeds a follow-up call; the final answer is quote-checked
//! byte-verbatim against the fetched payload before it prints.
//!
//! Meta call shape (https://dev.meta.ai/docs/protocols/chat-completions
//! with the envelope pinned in `docs/meta-api-shapes.md`): `developer`
//! standing rules first, per-run values in `user` (stable prefix
//! caches), no `temperature` (Spark is tuned for its defaults), explicit
//! `reasoning_effort`, `finish_reason: tool_calls` drives the tool loop,
//! refusals and `length` truncation fail closed. Retries follow the
//! error-handling docs (https://dev.meta.ai/docs/error-handling and
//! https://dev.meta.ai/docs/cookbook/error-handling-retry): up to 4
//! attempts with jitter honouring `Retry-After` on 429/5xx, never on
//! 400/401/402/403/404/413.
//!
//! Key: `$META_API_KEY` only (never a file, never a flag). Offline
//! replay (`--replay`) runs the recorded exchanges with zero network.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use atlas::cached::CachedTransport;
use atlas::model::{Transport, UreqTransport};
use serde_json::{Value, json};

/// Answering model: Meta only (model-gate).
pub const ASK_MODEL: &str = "muse-spark-1.3";
/// Default handbook base: local staging mount.
pub const DEFAULT_BASE: &str = "http://127.0.0.1:18318/apostille";
/// The single tool the model may call.
pub const TOOL_NAME: &str = "lookup_authority";
/// Prompt-cache key: stable use-case string, never per-user.
const CACHE_KEY: &str = "agent-cites-ask-v1";
/// Small answers: reasoning still counts inside this budget.
const MAX_COMPLETION_TOKENS: u32 = 4_000;
/// Upper bound on chat POSTs per question (first ask + tool rounds).
const MAX_POSTS: usize = 4;
/// Facts GET budget.
const FACTS_TIMEOUT: Duration = Duration::from_secs(30);
/// Retry budget per Meta POST (error-handling docs: 3-5 attempts).
const MAX_ATTEMPTS: u32 = 4;
/// First backoff step; doubles per retry plus jitter.
const RETRY_BASE_MS: u64 = 500;
/// A server `Retry-After` beyond this is treated as this.
const MAX_RETRY_AFTER_SECS: u64 = 60;

/// One public citation, exactly as the facts endpoint serves it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Citation {
    pub quote: String,
    pub source_url: String,
    pub captured_at: String,
}

/// One accepted fact with its citations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fact {
    pub id: i64,
    pub topic: String,
    pub statement: String,
    pub citations: Vec<Citation>,
}

/// Verified citation the answer rests on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verified {
    pub fact_id: i64,
    pub quote: String,
    pub source_url: String,
    pub captured_at: String,
}

/// Failure modes. Refusals are data (exit 1 with a clear message),
/// never silent empty output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AskError {
    Usage(String),
    Facts(String),
    Meta(String),
    Refused(String),
}

impl std::fmt::Display for AskError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AskError::Usage(m) | AskError::Facts(m) | AskError::Meta(m) | AskError::Refused(m) => {
                write!(f, "{m}")
            }
        }
    }
}

/// Parse the public facts payload (`{"authority": slug, "facts": [...]}`)
/// into facts. Unknown fields are ignored; missing citations stay empty
/// (a fact without citations can never verify a quote).
pub fn parse_facts(payload: &Value) -> Vec<Fact> {
    payload["facts"]
        .as_array()
        .map(|facts| {
            facts
                .iter()
                .map(|fact| Fact {
                    id: fact["id"].as_i64().unwrap_or(0),
                    topic: fact["topic"].as_str().unwrap_or("").to_string(),
                    statement: fact["statement"].as_str().unwrap_or("").to_string(),
                    citations: fact["citations"]
                        .as_array()
                        .map(|cites| {
                            cites
                                .iter()
                                .map(|cite| Citation {
                                    quote: cite["quote"].as_str().unwrap_or("").to_string(),
                                    source_url: cite["source_url"]
                                        .as_str()
                                        .unwrap_or("")
                                        .to_string(),
                                    captured_at: cite["captured_at"]
                                        .as_str()
                                        .unwrap_or("")
                                        .to_string(),
                                })
                                .collect()
                        })
                        .unwrap_or_default(),
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Handbook facts URL for any base (staging, production, localhost):
/// trailing slashes are trimmed, the mount path is kept as given.
pub fn facts_url(base: &str, slug: &str) -> String {
    format!(
        "{}/api/v1/authorities/{slug}/facts",
        base.trim_end_matches('/')
    )
}

/// Standing rules (stable prefix: cacheable). Per-run values
/// (question, slug) ride in the `user` message only.
fn developer_rules() -> &'static str {
    "You answer questions about apostille authorities using ONLY the lookup_authority tool. \
     Call lookup_authority with the authority slug, then answer from the returned facts. \
     Every factual claim in your answer must quote the fact statement or a citation quote \
     EXACTLY, character for character. Cite each used fact on its own line in this exact form: \
     [fact <id>] \"<exact quote>\" (source: <source_url>; captured: <captured_at>). \
     If no returned fact covers the question, say exactly: I do not have a fact covering that question. \
     Never invent quotes, URLs, or dates."
}

/// The single tool definition: `lookup_authority(slug)`.
fn lookup_tool() -> Value {
    json!({
        "type": "function",
        "function": {
            "name": TOOL_NAME,
            "description": "Fetch the handbook's accepted facts for one authority slug, with verbatim quotes, source URLs and capture timestamps.",
            "parameters": {
                "type": "object",
                "properties": {
                    "slug": {
                        "type": "string",
                        "description": "Authority slug, e.g. usa-wa-sos."
                    }
                },
                "required": ["slug"],
                "additionalProperties": false
            }
        }
    })
}

/// Chat body: stable rules first, per-run values last; sampling
/// untouched; reasoning explicit; one tool.
fn chat_body(messages: &[Value], reasoning_effort: &str) -> Value {
    json!({
        "model": ASK_MODEL,
        "messages": messages,
        "tools": [lookup_tool()],
        "tool_choice": "auto",
        "reasoning_effort": reasoning_effort,
        "max_completion_tokens": MAX_COMPLETION_TOKENS,
        "prompt_cache_key": CACHE_KEY,
        "stream": false,
    })
}

/// Retryable Meta statuses: 429/5xx honour `Retry-After`; terminal
/// 400/401/402/403/404/413 (and anything else) never retry.
pub(crate) fn retryable(status: u16) -> bool {
    matches!(status, 429 | 500 | 502 | 503 | 504)
}

/// Backoff before retry `attempt_no` (1-based): exponential from the
/// base plus nanos-clock jitter (no rng dependency), or the server's
/// `Retry-After` when present.
pub(crate) fn retry_delay(attempt_no: u32, retry_after: Option<u64>) -> Duration {
    if let Some(after) = retry_after {
        return Duration::from_secs(after.min(MAX_RETRY_AFTER_SECS));
    }
    let backoff_ms = RETRY_BASE_MS.saturating_mul(1u64 << attempt_no.saturating_sub(1).min(5));
    let jitter = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| u64::from(d.subsec_nanos()) % (backoff_ms / 2).max(1))
        .unwrap_or(0);
    Duration::from_millis(backoff_ms + jitter)
}

/// One chat POST against the Meta API with the findings retry
/// discipline. Returns the decoded envelope.
fn post_chat(
    transport: &CachedTransport<UreqTransport>,
    meta_base: &str,
    key: &str,
    body: &Value,
) -> Result<(Value, atlas::model::HttpResponse), AskError> {
    let url = format!("{}/chat/completions", meta_base.trim_end_matches('/'));
    let mut attempt_no = 0u32;
    loop {
        let response = transport
            .post_json(&url, key, body)
            .map_err(|e| AskError::Meta(format!("ask: meta transport: {e}")))?;
        if !retryable(response.status) {
            return decode_chat(response);
        }
        if attempt_no >= MAX_ATTEMPTS - 1 {
            return decode_chat(response);
        }
        attempt_no += 1;
        std::thread::sleep(retry_delay(attempt_no, response.retry_after_secs));
    }
}

/// Decode one non-streamed chat envelope: terminal statuses become
/// errors (short phrase only — bodies can echo request evidence),
/// 2xx yields the JSON body.
fn decode_chat(
    response: atlas::model::HttpResponse,
) -> Result<(Value, atlas::model::HttpResponse), AskError> {
    let status = response.status;
    if status == 401 {
        return Err(AskError::Meta(
            "ask: meta api unauthorized (check META_API_KEY)".to_string(),
        ));
    }
    if status == 402 {
        return Err(AskError::Meta("ask: meta api budget exceeded".to_string()));
    }
    if !(200..300).contains(&status) {
        let text = String::from_utf8_lossy(&response.body);
        let trimmed: String = text.chars().take(120).collect();
        return Err(AskError::Meta(format!(
            "ask: meta api status {status}: {}",
            trimmed.trim()
        )));
    }
    let json: Value = serde_json::from_slice(&response.body)
        .map_err(|e| AskError::Meta(format!("ask: meta api bad body: {e}")))?;
    Ok((json, response))
}

/// The model's next move after one chat round.
#[derive(Debug)]
enum Turn {
    /// Call the tool with these arguments (raw JSON string).
    CallTool(String),
    /// Final text (may still fail verification).
    Answer(String),
}

/// Read one chat round: refusal and `length` truncation fail closed;
/// `tool_calls` continues the loop; `stop` with text is the answer.
fn read_turn(envelope: &Value) -> Result<(Turn, Value), AskError> {
    let choice = envelope["choices"]
        .as_array()
        .and_then(|choices| choices.first())
        .ok_or_else(|| AskError::Meta("ask: meta api: no choices[]".to_string()))?;
    let message = &choice["message"];
    if !message["refusal"].is_null() {
        return Err(AskError::Refused(
            "ask: refused — the model declined to answer".to_string(),
        ));
    }
    match choice["finish_reason"].as_str() {
        Some("tool_calls") => {
            let call = message["tool_calls"]
                .as_array()
                .and_then(|calls| calls.first())
                .ok_or_else(|| AskError::Meta("ask: meta api: empty tool_calls".to_string()))?;
            let name = call["function"]["name"].as_str().unwrap_or("");
            if name != TOOL_NAME {
                return Err(AskError::Meta(format!(
                    "ask: meta api: unexpected tool {name:?}"
                )));
            }
            Ok((
                Turn::CallTool(call["function"]["arguments"].to_string()),
                usage_of(envelope),
            ))
        }
        Some("stop") => {
            let text = message["content"].as_str().unwrap_or("").to_string();
            Ok((Turn::Answer(text), usage_of(envelope)))
        }
        Some("length") => Err(AskError::Refused(
            "ask: refused — the answer was truncated before any citation".to_string(),
        )),
        other => Err(AskError::Meta(format!(
            "ask: meta api: finish_reason={other:?}"
        ))),
    }
}

/// Usage accumulator (optional on the wire; record when present).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cached_tokens: u64,
    pub reasoning_tokens: u64,
}

fn usage_of(envelope: &Value) -> Value {
    envelope["usage"].clone()
}

fn add_usage(total: &mut Usage, envelope_usage: &Value) {
    let n = |name: &str| envelope_usage[name].as_u64().unwrap_or(0);
    total.input_tokens += n("prompt_tokens");
    total.output_tokens += n("completion_tokens");
    total.cached_tokens += envelope_usage["prompt_tokens_details"]["cached_tokens"]
        .as_u64()
        .unwrap_or(n("cached_tokens"));
    total.reasoning_tokens += envelope_usage["completion_tokens_details"]["reasoning_tokens"]
        .as_u64()
        .unwrap_or(n("reasoning_tokens"));
}

/// Parse one citation line of the enforced form:
/// `[fact <id>] "<quote>" (source: <url>; captured: <ts>)`
fn parse_citation(line: &str) -> Option<(i64, String, String, String)> {
    let rest = line.trim().strip_prefix("[fact ")?;
    let (id_text, rest) = rest.split_once(']')?;
    let id = id_text.trim().parse().ok()?;
    let rest = rest.trim_start();
    let quoted = rest.strip_prefix('"')?;
    let (quote, rest) = quoted.split_once('"')?;
    let rest = rest.trim_start().strip_prefix('(')?.strip_suffix(')')?;
    let (source_part, captured_part) = rest.split_once(';')?;
    let source_url = source_part
        .trim()
        .strip_prefix("source:")?
        .trim()
        .to_string();
    let captured_at = captured_part
        .trim()
        .strip_prefix("captured:")?
        .trim()
        .to_string();
    if quote.is_empty() || source_url.is_empty() || captured_at.is_empty() {
        return None;
    }
    Some((id, quote.to_string(), source_url, captured_at))
}

/// Quote-check the final text byte-verbatim against the fetched
/// facts: each citation line must name a real fact id, carry a quote
/// that appears character-for-character in that fact's statement or
/// one of its citation quotes, and repeat that citation's
/// `source_url` + `captured_at` exactly. Returns the verified set,
/// empty when nothing checks out.
pub fn verify_answer(text: &str, facts: &[Fact]) -> Vec<Verified> {
    let mut out = Vec::new();
    for line in text.lines() {
        let Some((id, quote, source_url, captured_at)) = parse_citation(line) else {
            continue;
        };
        let Some(fact) = facts.iter().find(|f| f.id == id) else {
            continue;
        };
        // Candidate (text, url, ts) pairs: the statement may be cited
        // under any of the fact's citations; a citation quote only
        // under its own URL + timestamp.
        let mut candidates: Vec<(&str, &str, &str)> = fact
            .citations
            .iter()
            .map(|c| {
                (
                    fact.statement.as_str(),
                    c.source_url.as_str(),
                    c.captured_at.as_str(),
                )
            })
            .collect();
        for cite in &fact.citations {
            candidates.push((
                cite.quote.as_str(),
                cite.source_url.as_str(),
                cite.captured_at.as_str(),
            ));
        }
        let ok = candidates
            .iter()
            .any(|(text, url, ts)| *text == quote && *url == source_url && *ts == captured_at);
        if ok {
            out.push(Verified {
                fact_id: id,
                quote,
                source_url,
                captured_at,
            });
        }
    }
    out
}

/// Facts source: live GET or recorded fixture (zero network).
pub enum FactsSource {
    Live { base: String },
    Replay { payload: Value },
}

impl FactsSource {
    fn load(&self, slug: &str) -> Result<Value, AskError> {
        match self {
            FactsSource::Live { base } => {
                let url = facts_url(base, slug);
                match ingest::fetch::fetch_url(&url, FACTS_TIMEOUT) {
                    Ok(fetched) => serde_json::from_slice(&fetched.bytes).map_err(|e| {
                        AskError::Facts(format!("ask: facts at {url} are not JSON: {e}"))
                    }),
                    Err(ingest::fetch::FetchError::HttpStatus(404)) => Err(AskError::Refused(
                        format!("ask: refused — unknown authority {slug:?}"),
                    )),
                    Err(e) => Err(AskError::Facts(format!("ask: facts GET {url}: {e}"))),
                }
            }
            FactsSource::Replay { payload } => Ok(payload.clone()),
        }
    }
}

/// Meta source: live POSTs or recorded request/response exchanges
/// (zero network; requests are recorded for shape assertions).
pub enum MetaSource {
    Live {
        transport: CachedTransport<UreqTransport>,
        meta_base: String,
        key: String,
    },
    Replay {
        exchanges: VecDeque<(Value, Value)>,
        seen_requests: Vec<Value>,
    },
}

impl MetaSource {
    fn chat(&mut self, body: &Value) -> Result<(Value, Value), AskError> {
        match self {
            MetaSource::Live {
                transport,
                meta_base,
                key,
            } => {
                let (envelope, _raw) = post_chat(transport, meta_base, key, body)?;
                let usage = envelope["usage"].clone();
                Ok((envelope, usage))
            }
            MetaSource::Replay {
                exchanges,
                seen_requests,
            } => {
                seen_requests.push(body.clone());
                exchanges
                    .pop_front()
                    .map(|(_, response)| {
                        let usage = response["usage"].clone();
                        (response, usage)
                    })
                    .ok_or_else(|| {
                        AskError::Meta("ask: replay fixture has no more exchanges".to_string())
                    })
            }
        }
    }
}

/// Trim a facts payload to what the model needs (id, topic,
/// statement, citations) — the tool result.
fn tool_result(payload: &Value) -> String {
    let facts = parse_facts(payload);
    let trimmed: Vec<Value> = facts
        .iter()
        .map(|fact| {
            json!({
                "id": fact.id,
                "topic": fact.topic,
                "statement": fact.statement,
                "citations": fact.citations.iter().map(|c| {
                    json!({
                        "quote": c.quote,
                        "source_url": c.source_url,
                        "captured_at": c.captured_at,
                    })
                }).collect::<Vec<_>>(),
            })
        })
        .collect();
    serde_json::to_string(&json!({"facts": trimmed})).unwrap_or_else(|_| "{\"facts\":[]}".into())
}

/// Run the ask loop. Returns the model's text plus the verified
/// citations. Errors when the tool is never consulted, when no
/// citation verifies, or when the model refuses.
pub fn answer(
    question: &str,
    slug: &str,
    facts: &FactsSource,
    meta: &mut MetaSource,
    usage_out: &mut Usage,
) -> Result<(String, Vec<Verified>), AskError> {
    let mut messages = vec![
        json!({"role": "developer", "content": developer_rules()}),
        json!({"role": "user", "content": format!("Authority slug: {slug}\nQuestion: {question}")}),
    ];
    let mut tool_calls = 0u32;
    for _ in 0..MAX_POSTS {
        let body = chat_body(&messages, "low");
        let (envelope, usage) = meta.chat(&body)?;
        add_usage(usage_out, &usage);
        match read_turn(&envelope)? {
            (Turn::CallTool(args_text), _) => {
                tool_calls += 1;
                let args: Value = serde_json::from_str(&args_text).unwrap_or(json!({}));
                let want = args["slug"].as_str().unwrap_or(slug);
                let payload = facts.load(want)?;
                let result = tool_result(&payload);
                let call_id = envelope["choices"][0]["message"]["tool_calls"][0]["id"]
                    .as_str()
                    .unwrap_or("call-0");
                messages.push(json!({
                    "role": "assistant",
                    "tool_calls": envelope["choices"][0]["message"]["tool_calls"],
                }));
                messages.push(json!({
                    "role": "tool",
                    "tool_call_id": call_id,
                    "content": result,
                }));
            }
            (Turn::Answer(text), _) => {
                if tool_calls == 0 {
                    return Err(AskError::Refused(
                        "ask: refused — answered without consulting lookup_authority".to_string(),
                    ));
                }
                let payload = facts.load(slug)?;
                let verified = verify_answer(&text, &parse_facts(&payload));
                if verified.is_empty() {
                    // The model may state it has no covering fact in
                    // prose; either way, no verified quote = refuse.
                    return Err(AskError::Refused(
                        "ask: refused — no fact covers the question".to_string(),
                    ));
                }
                return Ok((text, verified));
            }
        }
    }
    Err(AskError::Refused(
        "ask: refused — tool loop did not converge".to_string(),
    ))
}

/// Replay fixture on disk: question + slug + facts payload +
/// recorded Meta exchanges + run metadata.
#[derive(Debug, Clone)]
pub struct Fixture {
    pub question: String,
    pub slug: String,
    pub payload: Value,
    pub exchanges: Vec<(Value, Value)>,
}

pub fn load_fixture(path: &str) -> Result<Fixture, AskError> {
    let bytes = std::fs::read(path)
        .map_err(|e| AskError::Usage(format!("ask: cannot read --replay {path}: {e}")))?;
    let fixture: Value = serde_json::from_slice(&bytes)
        .map_err(|e| AskError::Usage(format!("ask: bad --replay {path}: {e}")))?;
    let exchanges = fixture["exchanges"]
        .as_array()
        .map(|list| {
            list.iter()
                .map(|entry| (entry["request"].clone(), entry["response"].clone()))
                .collect()
        })
        .unwrap_or_default();
    Ok(Fixture {
        question: fixture["question"].as_str().unwrap_or("").to_string(),
        slug: fixture["slug"].as_str().unwrap_or("").to_string(),
        payload: fixture["facts"].clone(),
        exchanges,
    })
}

/// CLI entry. Exit 0 prints the cited answer; exit 1 refuses or
/// reports a runtime failure; exit 2 is a usage error.
pub fn run(args: &[String]) {
    let mut question: Option<String> = None;
    for arg in args {
        if arg.starts_with("--") {
            continue;
        }
        if question.is_none() {
            question = Some(arg.clone());
        }
    }
    let Some(question) = question.filter(|q| !q.trim().is_empty()) else {
        eprintln!(
            "usage: cargo xtask ask \"<question>\" --slug <slug> [--base <url>] [--replay <fixture>]"
        );
        std::process::exit(2);
    };
    let slug = crate::flag(args, "slug").unwrap_or_else(|| {
        eprintln!("ask: missing required flag --slug");
        std::process::exit(2);
    });
    let base = crate::flag(args, "base").unwrap_or_else(|| DEFAULT_BASE.to_string());

    // Offline replay resolves the "?" question and "fixture" slug
    // from the recording, so one command replays any fixture.
    let replay = if let Some(path) = crate::flag(args, "replay") {
        match load_fixture(&path) {
            Ok(fixture) => Some(fixture),
            Err(e) => {
                eprintln!("{e}");
                std::process::exit(2);
            }
        }
    } else {
        None
    };
    let (question, slug) = match &replay {
        Some(fixture) => (
            if question == "?" {
                fixture.question.clone()
            } else {
                question
            },
            if slug == "fixture" {
                fixture.slug.clone()
            } else {
                slug
            },
        ),
        None => (question, slug),
    };

    let started = Instant::now();
    let mut usage = Usage::default();
    let outcome = if let Some(fixture) = replay {
        let facts = FactsSource::Replay {
            payload: fixture.payload,
        };
        let mut meta = MetaSource::Replay {
            exchanges: VecDeque::from(fixture.exchanges),
            seen_requests: Vec::new(),
        };
        answer(&question, &slug, &facts, &mut meta, &mut usage)
            .map(|(text, verified)| (text, verified, meta_seen(&meta)))
    } else {
        let key = std::env::var("META_API_KEY").unwrap_or_default();
        if key.is_empty() {
            eprintln!("ask: META_API_KEY is not set (key via env only; --replay runs offline)");
            std::process::exit(2);
        }
        let meta_base = crate::flag(args, "meta-base")
            .unwrap_or_else(|| handbook_agent::model::META_BASE_URL.to_string());
        let facts = FactsSource::Live { base: base.clone() };
        let mut meta = MetaSource::Live {
            transport: CachedTransport::new(
                UreqTransport::default(),
                crate::call_cache::meta_cache(args),
            ),
            meta_base,
            key,
        };
        answer(&question, &slug, &facts, &mut meta, &mut usage)
            .map(|(text, verified)| (text, verified, Vec::new()))
    };

    match outcome {
        Ok((text, verified, _seen)) => {
            println!("{text}");
            println!();
            println!("Citations verified: {}", verified.len());
            for item in &verified {
                println!(
                    "- [fact {}] source: {} captured: {}",
                    item.fact_id, item.source_url, item.captured_at
                );
            }
            eprintln!(
                "ask: model={} slug={slug} latency_ms={} input_tokens={} output_tokens={} reasoning_tokens={} cached_tokens={}",
                ASK_MODEL,
                started.elapsed().as_millis(),
                usage.input_tokens,
                usage.output_tokens,
                usage.reasoning_tokens,
                usage.cached_tokens,
            );
        }
        Err(e @ AskError::Refused(_)) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    }
}

/// Requests the replay leg issued (test hook; live runs nothing).
fn meta_seen(meta: &MetaSource) -> Vec<Value> {
    match meta {
        MetaSource::Replay { seen_requests, .. } => seen_requests.clone(),
        MetaSource::Live { .. } => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Facts payload in the exact `GET .../facts` shape, built from
    /// the synthetic `evals/authorities/usa-wa-sos` gold rows (public
    /// text only — never staging captures).
    fn facts_payload() -> Value {
        json!({
            "authority": "usa-wa-sos",
            "facts": [
                {"id": 2, "authority_slug": "usa-wa-sos", "topic": "fees",
                 "statement": "$15 per document",
                 "status": "accepted", "created_by": "synthetic",
                 "drafted_by": "fixture", "created_at": 1_700_000_000,
                 "citations": [
                     {"quote": "The fee table shows $15 per document for standard authentication.",
                      "source_url": "https://www.sos.wa.gov/authentication/fees",
                      "captured_at": "2026-10-30T04:29:00Z"}
                 ],
                 "conflict": false, "conflicts": []},
                {"id": 8, "authority_slug": "usa-wa-sos", "topic": "processing_time",
                 "statement": "five business days",
                 "status": "accepted", "created_by": "synthetic",
                 "drafted_by": "fixture", "created_at": 1_700_000_000,
                 "citations": [
                     {"quote": "Standard processing is five business days from receipt of a complete order.",
                      "source_url": "https://www.sos.wa.gov/authentication/processing",
                      "captured_at": "2026-10-30T04:29:00Z"}
                 ],
                 "conflict": false, "conflicts": []}
            ]
        })
    }

    fn tool_response(slug: &str) -> Value {
        json!({
            "id": "chatcmpl-ask-1",
            "model": ASK_MODEL,
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": serde_json::Value::Null,
                    "tool_calls": [{
                        "id": "call-lookup-1",
                        "type": "function",
                        "function": {
                            "name": TOOL_NAME,
                            "arguments": format!("{{\"slug\":\"{slug}\"}}"),
                        }
                    }]
                },
                "finish_reason": "tool_calls"
            }],
            "usage": {
                "prompt_tokens": 900, "completion_tokens": 60, "total_tokens": 960,
                "prompt_tokens_details": {"cached_tokens": 400},
                "completion_tokens_details": {"reasoning_tokens": 40}
            }
        })
    }

    fn final_response(text: &str) -> Value {
        json!({
            "id": "chatcmpl-ask-2",
            "model": ASK_MODEL,
            "choices": [{
                "index": 0,
                "message": {"role": "assistant", "content": text},
                "finish_reason": "stop"
            }],
            "usage": {
                "prompt_tokens": 1500, "completion_tokens": 120, "total_tokens": 1620,
                "prompt_tokens_details": {"cached_tokens": 400},
                "completion_tokens_details": {"reasoning_tokens": 80}
            }
        })
    }

    fn replay_meta(responses: Vec<Value>) -> MetaSource {
        MetaSource::Replay {
            exchanges: responses.into_iter().map(|r| (json!({}), r)).collect(),
            seen_requests: Vec::new(),
        }
    }

    #[test]
    fn request_body_uses_one_tool_and_no_temperature() {
        let body = chat_body(&[json!({"role": "developer", "content": "rules"})], "low");
        assert_eq!(body["model"], ASK_MODEL);
        let tools = body["tools"].as_array().expect("tools[]");
        assert_eq!(tools.len(), 1, "exactly one tool");
        assert_eq!(tools[0]["function"]["name"], TOOL_NAME);
        assert!(
            tools[0]["function"]["parameters"]["properties"]["slug"].is_object(),
            "slug parameter declared"
        );
        assert!(body.get("temperature").is_none(), "sampling untouched");
        assert_eq!(body["reasoning_effort"], "low");
        assert_eq!(body["tool_choice"], "auto");
        assert_eq!(body["messages"][0]["role"], "developer");
        assert_eq!(body["prompt_cache_key"], CACHE_KEY);
        assert_eq!(body["stream"], false);
    }

    #[test]
    fn replay_fee_question_answers_with_verified_quote() {
        let text = "The fee is $15 per document.\n\
            [fact 2] \"$15 per document\" \
            (source: https://www.sos.wa.gov/authentication/fees; captured: 2026-10-30T04:29:00Z)";
        let mut meta = replay_meta(vec![tool_response("usa-wa-sos"), final_response(text)]);
        let facts = FactsSource::Replay {
            payload: facts_payload(),
        };
        let mut usage = Usage::default();
        let (answer_text, verified) = answer(
            "What is the per-document authentication fee?",
            "usa-wa-sos",
            &facts,
            &mut meta,
            &mut usage,
        )
        .expect("fee question answers");
        assert!(answer_text.contains("$15 per document"));
        assert_eq!(verified.len(), 1);
        assert_eq!(verified[0].fact_id, 2);
        assert_eq!(verified[0].quote, "$15 per document");
        assert_eq!(
            verified[0].source_url,
            "https://www.sos.wa.gov/authentication/fees"
        );
        assert_eq!(verified[0].captured_at, "2026-10-30T04:29:00Z");
        // Usage accumulates across both legs, details included.
        assert_eq!(usage.input_tokens, 2400);
        assert_eq!(usage.output_tokens, 180);
        assert_eq!(usage.reasoning_tokens, 120);
        assert_eq!(usage.cached_tokens, 800);
        // Both legs went out as Spark tool-calling requests.
        let seen = meta_seen(&meta);
        assert_eq!(seen.len(), 2);
        for request in &seen {
            assert_eq!(request["model"], ASK_MODEL);
            assert_eq!(request["tools"][0]["function"]["name"], TOOL_NAME);
            assert!(request.get("temperature").is_none());
        }
    }

    #[test]
    fn uncovered_question_refuses() {
        let text = "I do not have a fact covering that question.";
        let mut meta = replay_meta(vec![tool_response("usa-wa-sos"), final_response(text)]);
        let facts = FactsSource::Replay {
            payload: facts_payload(),
        };
        let mut usage = Usage::default();
        let err = answer(
            "What are the consular fees of a country with no facts here?",
            "usa-wa-sos",
            &facts,
            &mut meta,
            &mut usage,
        )
        .expect_err("no covering fact refuses");
        assert_eq!(
            err,
            AskError::Refused("ask: refused — no fact covers the question".to_string())
        );
    }

    #[test]
    fn paraphrased_quote_does_not_verify() {
        // One changed word: byte-verbatim check must reject it.
        let text = "The fee is fifteen dollars per document.\n\
            [fact 2] \"$15 per paper\" \
            (source: https://www.sos.wa.gov/authentication/fees; captured: 2026-10-30T04:29:00Z)";
        let mut meta = replay_meta(vec![tool_response("usa-wa-sos"), final_response(text)]);
        let facts = FactsSource::Replay {
            payload: facts_payload(),
        };
        let mut usage = Usage::default();
        let err = answer("fee?", "usa-wa-sos", &facts, &mut meta, &mut usage)
            .expect_err("near-miss quote refuses");
        assert!(matches!(err, AskError::Refused(_)), "got: {err}");
    }

    #[test]
    fn wrong_source_url_does_not_verify() {
        let text = "Fee below.\n[fact 2] \"$15 per document\" \
            (source: https://example.invalid/other; captured: 2026-10-30T04:29:00Z)";
        let verified = verify_answer(text, &parse_facts(&facts_payload()));
        assert!(verified.is_empty(), "URL mismatch must not verify");
    }

    #[test]
    fn answer_without_tool_call_refuses() {
        let text = "[fact 2] \"$15 per document\" \
            (source: https://www.sos.wa.gov/authentication/fees; captured: 2026-10-30T04:29:00Z)";
        let mut meta = replay_meta(vec![final_response(text)]);
        let facts = FactsSource::Replay {
            payload: facts_payload(),
        };
        let mut usage = Usage::default();
        let err = answer("fee?", "usa-wa-sos", &facts, &mut meta, &mut usage)
            .expect_err("direct answer refuses");
        assert!(
            format!("{err}").contains("without consulting"),
            "got: {err}"
        );
    }

    #[test]
    fn model_refusal_fails_closed() {
        let envelope = json!({
            "choices": [{
                "index": 0,
                "message": {"role": "assistant", "refusal": "declined"},
                "finish_reason": "stop"
            }]
        });
        let err = read_turn(&envelope).expect_err("refusal fails closed");
        assert!(matches!(err, AskError::Refused(_)), "got: {err}");
    }

    #[test]
    fn facts_url_keeps_mount_and_trims_slash() {
        assert_eq!(
            facts_url("http://127.0.0.1:18318/apostille", "usa-wa-sos"),
            "http://127.0.0.1:18318/apostille/api/v1/authorities/usa-wa-sos/facts"
        );
        assert_eq!(
            facts_url("https://andrii.ai/apostille/", "usa-wa-sos"),
            "https://andrii.ai/apostille/api/v1/authorities/usa-wa-sos/facts"
        );
        assert_eq!(
            facts_url("http://localhost:18318/apostille", "usa-wa-sos"),
            "http://localhost:18318/apostille/api/v1/authorities/usa-wa-sos/facts"
        );
    }

    #[test]
    fn retryable_statuses_match_findings() {
        for status in [429, 500, 502, 503, 504] {
            assert!(retryable(status), "{status} retries");
        }
        for status in [200, 400, 401, 402, 403, 404, 413] {
            assert!(!retryable(status), "{status} never retries");
        }
    }

    #[test]
    fn retry_delay_honours_retry_after() {
        assert_eq!(
            retry_delay(1, Some(7)),
            Duration::from_secs(7),
            "server Retry-After wins"
        );
        assert_eq!(
            retry_delay(1, Some(9_999)),
            Duration::from_secs(MAX_RETRY_AFTER_SECS),
            "long sleeps capped"
        );
        let delay = retry_delay(1, None);
        assert!(
            delay >= Duration::from_millis(RETRY_BASE_MS)
                && delay < Duration::from_millis(RETRY_BASE_MS * 2),
            "base plus jitter, got: {delay:?}"
        );
    }

    #[test]
    fn committed_fixture_replays_with_zero_network() {
        let path = format!(
            "{}/../examples/agent-cites/fixtures/ask-usa-wa-sos-fee.json",
            env!("CARGO_MANIFEST_DIR")
        );
        let fixture = load_fixture(&path).expect("fixture loads");
        assert_eq!(fixture.slug, "usa-wa-sos");
        assert!(!fixture.exchanges.is_empty(), "fixture records exchanges");
        let facts = FactsSource::Replay {
            payload: fixture.payload,
        };
        let mut meta = MetaSource::Replay {
            exchanges: VecDeque::from(fixture.exchanges),
            seen_requests: Vec::new(),
        };
        let mut usage = Usage::default();
        let (text, verified) =
            answer("?", "fixture", &facts, &mut meta, &mut usage).expect("fixture replays");
        assert!(!verified.is_empty(), "fixture answer verifies");
        assert!(text.contains(&verified[0].quote));
    }
}
