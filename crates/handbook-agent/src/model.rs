//! Model-client implementations for the analysis agent.
//!
//! The interface ([`ModelClient`](contracts::model::ModelClient) and its
//! request/response types) lives in `contracts`; this module keeps the
//! [`StubClient`] fixtures and the [`MetaRestClient`] skeleton.

use std::fmt;
use std::time::Duration;

use contracts::model::{
    CitationRef, DraftRequest, DraftResponse, ModelClient, ModelError, ProposedRecord, Usage,
};
use serde_json::json;

use crate::analyze::build_prompt;

/// Deterministic fixture client for tests and offline dev.
///
/// Returns the same fixed records for every call (capped at
/// `req.max_records`); citations reference `doc_id` `"fixture"`.
#[derive(Debug, Clone, Default)]
pub struct StubClient;

impl ModelClient for StubClient {
    fn model_id(&self) -> &'static str {
        "stub"
    }

    fn draft_records(&self, req: &DraftRequest) -> Result<DraftResponse, ModelError> {
        // Citations locate each value inside the request evidence: the
        // offsets are only valid because the validator checks the value
        // sits inside the quote. A value absent from the evidence cites
        // an empty range, which the validator rejects as BadRange.
        let cite = |value: &str| match req.input_text.find(value) {
            Some(start) => (start as u32, (start + value.len()) as u32),
            None => (0, 0),
        };
        let address_value = "PO Box 40220".to_string();
        let hours_value = "Mon-Fri 9-5".to_string();
        let (address_start, address_end) = cite(&address_value);
        let (hours_start, hours_end) = cite(&hours_value);
        let records = vec![
            ProposedRecord {
                topic: "address".to_string(),
                field: "po_box".to_string(),
                value: address_value,
                citation: CitationRef {
                    doc_id: "fixture".to_string(),
                    byte_start: address_start,
                    byte_end: address_end,
                },
                confidence: 0.9,
            },
            ProposedRecord {
                topic: "hours".to_string(),
                field: "weekday".to_string(),
                value: hours_value,
                citation: CitationRef {
                    doc_id: "fixture".to_string(),
                    byte_start: hours_start,
                    byte_end: hours_end,
                },
                confidence: 0.8,
            },
        ];
        let records = records.into_iter().take(req.max_records as usize).collect();
        Ok(DraftResponse {
            records,
            usage: Usage {
                input_tokens: 0,
                output_tokens: 0,
            },
            summary: "stub: two fixture records".to_string(),
        })
    }
}

/// Default Model API root; tests override with a stub URL. Keep in
/// sync with the `meta.base_url` default in the config crate.
pub const META_BASE_URL: &str = "https://api.meta.ai/v1";

/// Drafting model (`docs/meta-api.md`: default Spark on cost).
const DRAFT_MODEL: &str = "muse-spark-1.3";

/// Sleeps between the 3 attempts (spec §3: 2s/8s/32s + jitter; the
/// jitter is omitted — a single-operator batch has no thundering
/// herd — and the third sleep never runs: 3 attempts, then stop).
const RETRY_SLEEP_SECS: [u64; 2] = [2, 8];

/// Live Meta Models API client (blocking ureq, like the voice client).
///
/// Auth: Model API key auth only — the OAuth2 handshake cannot mint
/// Model API tokens. The key arrives from the sealed `meta.api_key`
/// row (never env); an empty key fails closed at call time. Shapes
/// follow `docs/meta-api-shapes.md` (verified 2026-10-28).
///
/// `Debug` is hand-written (M3): the key never appears in debug output,
/// so a stray `{:?}` in an error or log path cannot leak it.
#[derive(Clone)]
pub struct MetaRestClient {
    agent: ureq::Agent,
    stream_agent: ureq::Agent,
    base_url: String,
    api_key: String,
}

impl fmt::Debug for MetaRestClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MetaRestClient")
            .field("base_url", &self.base_url)
            .field("api_key", &"<redacted>")
            .finish()
    }
}

impl MetaRestClient {
    /// Build a client; an empty key fails closed at call time.
    pub fn new(base_url: String, api_key: String) -> Self {
        // Idempotent (ingest precedent): first install wins, a prior
        // install is fine. Without this the no-provider ureq backend
        // cannot dial TLS in xtask-driven processes.
        let provider = rustls::crypto::aws_lc_rs::default_provider();
        let _ = rustls::crypto::CryptoProvider::install_default(provider);
        let config = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_global(Some(Duration::from_secs(90)))
            .build();
        // Streams render while the model reasons, so a 32k-token draft
        // holds the connection far past the sync budget.
        let stream_config = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_global(Some(Duration::from_secs(300)))
            .build();
        Self {
            agent: ureq::Agent::new_with_config(config),
            stream_agent: ureq::Agent::new_with_config(stream_config),
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key,
        }
    }

    /// Health/auth probe for `xtask analyze --check`: `GET /models`
    /// returns the catalog; the model ids are the proof the key works.
    ///
    /// # Errors
    ///
    /// [`ModelError::Auth`] on 401 or an empty key; [`ModelError::Transport`]
    /// on I/O or other non-2xx; [`ModelError::BadOutput`] when a 2xx
    /// body is not the `data[].id` catalog shape.
    pub fn check(&self) -> Result<Vec<String>, ModelError> {
        if self.api_key.is_empty() {
            return Err(ModelError::Auth("meta api key not configured".to_string()));
        }
        let url = format!("{}/models", self.base_url);
        let response = self.get(&url)?;
        if response.status == 401 {
            return Err(ModelError::Auth(
                "meta api: unauthorized (check meta.api_key)".to_string(),
            ));
        }
        if !(200..300).contains(&response.status) {
            return Err(short_status("meta api", response.status, &response.body));
        }
        let value: serde_json::Value = serde_json::from_slice(&response.body)
            .map_err(|e| ModelError::BadOutput(format!("models response: {e}")))?;
        let data = value["data"]
            .as_array()
            .ok_or_else(|| ModelError::BadOutput("models response: want data[]".to_string()))?;
        let mut ids = Vec::with_capacity(data.len());
        for entry in data {
            let id = entry["id"].as_str().ok_or_else(|| {
                ModelError::BadOutput("models response: data[] entry lacks id".to_string())
            })?;
            ids.push(id.to_string());
        }
        Ok(ids)
    }

    /// POST a streamed chat completion: server-sent events render
    /// while the model reasons, so long drafts never hit the
    /// non-streaming server time limit (HTTP 504 on expiry). A
    /// mid-stream transport failure restarts the whole stream.
    fn post_chat(&self, body: &serde_json::Value) -> Result<RawResponse, ModelError> {
        let url = format!("{}/chat/completions", self.base_url);
        let mut sleeps = RETRY_SLEEP_SECS.iter();
        loop {
            let attempt = self.post_sse(&url, body);
            let retryable = match &attempt {
                Err(ModelError::Transport(_)) => true,
                Ok(r) => matches!(r.status, 429 | 500 | 502 | 503 | 504),
                _ => false,
            };
            match sleeps.next() {
                Some(secs) if retryable => std::thread::sleep(Duration::from_secs(*secs)),
                _ => return attempt,
            }
        }
    }

    fn post_sse(&self, url: &str, body: &serde_json::Value) -> Result<RawResponse, ModelError> {
        let response = self
            .stream_agent
            .post(url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .header("Accept", "text/event-stream")
            .send_json(body)
            .map_err(|e| ModelError::Transport(e.to_string()))?;
        let status = response.status().as_u16();
        let body = response
            .into_body()
            .read_to_vec()
            .map_err(|e| ModelError::Transport(e.to_string()))?;
        Ok(RawResponse { status, body })
    }

    fn get(&self, url: &str) -> Result<RawResponse, ModelError> {
        let response = self
            .agent
            .get(url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .call()
            .map_err(|e| ModelError::Transport(e.to_string()))?;
        let status = response.status().as_u16();
        let body = response
            .into_body()
            .read_to_vec()
            .map_err(|e| ModelError::Transport(e.to_string()))?;
        Ok(RawResponse { status, body })
    }
}

/// Short server phrase only (voice precedent): bodies can echo request
/// data, and request data here holds evidence text. Never the key.
fn short_status(who: &str, status: u16, body: &[u8]) -> ModelError {
    let text = String::from_utf8_lossy(body);
    let trimmed: String = text.chars().take(120).collect();
    ModelError::Transport(format!("{who} status {status}: {}", trimmed.trim()))
}

struct RawResponse {
    status: u16,
    body: Vec<u8>,
}

/// `json_schema` response format for [`DraftResponse`]: schema-constrained
/// decoding (not post-processing), strict-subset compliant (root plain
/// object, every object `additionalProperties: false`, `required`
/// lists all properties). The topic is the frozen [`Topic`](contracts::topics::Topic)
/// enum, not a free string, so off-vocabulary topics are refused by the
/// API before our byte-exact citation validator enforces grounding.
/// Schema-constrained decoding with the admissible citation doc ids
/// as an enum. Empty `doc_ids` keeps the free-string shape (used by
/// tests); live calls always pass the request's doc ids.
fn response_format_for(doc_ids: &[String]) -> serde_json::Value {
    let topics: Vec<&str> = contracts::topics::Topic::ALL
        .iter()
        .map(|t| t.as_str())
        .collect();
    let doc_id = if doc_ids.is_empty() {
        json!({"type": "string"})
    } else {
        json!({"type": "string", "enum": doc_ids})
    };
    json!({
        "type": "json_schema",
        "json_schema": {
            "name": "ProposedRecords",
            "schema": {
                "type": "object",
                "properties": {
                    "summary": {"type": "string"},
                    "records": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "topic": {"type": "string", "enum": topics},
                                "field": {"type": "string"},
                                "value": {"type": "string"},
                                "citation": {
                                    "type": "object",
                                    "properties": {
                                        "doc_id": doc_id,
                                        "byte_start": {"type": "integer"},
                                        "byte_end": {"type": "integer"}
                                    },
                                    "required": ["doc_id", "byte_start", "byte_end"],
                                    "additionalProperties": false
                                },
                                "confidence": {"type": "number"}
                            },
                            "required": ["topic", "field", "value", "citation", "confidence"],
                            "additionalProperties": false
                        }
                    }
                },
                "required": ["summary", "records"],
                "additionalProperties": false
            }
        }
    })
}

/// Completion budget per draft call. Spark's reasoning tokens count
/// inside it (a two-character reply already costs ~385), so an 8k cap
/// truncates real drafts into `finish_reason = length` on staging.
const MAX_COMPLETION_TOKENS: u32 = 32_000;

impl ModelClient for MetaRestClient {
    fn model_id(&self) -> &'static str {
        DRAFT_MODEL
    }

    fn draft_records(&self, req: &DraftRequest) -> Result<DraftResponse, ModelError> {
        if self.api_key.is_empty() {
            return Err(ModelError::Auth("meta api key not configured".to_string()));
        }
        match self.attempt(req, None) {
            Err(ModelError::BadOutput(reason))
                if is_truncation_or_refusal(&reason) && req.max_records > 1 =>
            {
                // One retry with fewer records and the refusal reason, so
                // a long draft still yields grounded records instead of
                // nothing.
                let narrowed = DraftRequest {
                    max_records: (req.max_records / 2).max(1),
                    ..req.clone()
                };
                self.attempt(&narrowed, Some(reason.as_str()))
            }
            other => other,
        }
    }
}

/// Retryable model-side failures: truncation (`finish_reason = length`)
/// or an explicit refusal. Schema/garbage errors are terminal — asking
/// again for the same shape rarely helps.
fn is_truncation_or_refusal(reason: &str) -> bool {
    reason.contains("truncated") || reason.contains("model refused")
}

/// Split between the stable standing rules and the per-run values in
/// `aa-prompt-v1.md`. Everything before the marker ships as the
/// `developer` message (stable prefix: cacheable, outranks `user`);
/// everything after renders per run into the `user` message.
const PER_RUN_MARKER: &str = "<!-- PER-RUN -->";

/// Chat request body: stable rules first (`developer`), per-run values
/// last (`user`). A lone `developer` message is rejected with HTTP 400
/// (`` `messages` must contain at least one message with role `user`
/// or `tool` ``), so the `user` message is always present.
fn draft_body(req: &DraftRequest, previous_failure: Option<&str>) -> serde_json::Value {
    let template = include_str!("../prompts/aa-prompt-v1.md");
    let (developer, per_run) = template
        .split_once(PER_RUN_MARKER)
        .expect("aa-prompt-v1.md carries a PER-RUN part");
    debug_assert!(
        !developer.contains("{{"),
        "standing rules must not hold per-run placeholders"
    );
    let mut user = build_prompt(per_run, req);
    if let Some(reason) = previous_failure {
        user.push_str(&format!(
            "\n\nPrevious attempt failed ({reason}). Draft fewer records \
             and keep every citation on the cited page."
        ));
    }
    // No `temperature`: Spark is tuned for its defaults, and
    // structured extraction wants sampling untouched. Explicit
    // `reasoning_effort: medium`: drafts ground claims in evidence
    // (measured: even trivial replies burn ~60 reasoning tokens, and
    // verdict-scale JSON runs to ~1.5k — both inside the 32k budget).
    json!({
        "model": DRAFT_MODEL,
        "messages": [
            {"role": "developer", "content": developer.trim()},
            {"role": "user", "content": user},
        ],
        "reasoning_effort": "medium",
        "max_completion_tokens": MAX_COMPLETION_TOKENS,
        "response_format": response_format_for(&req.doc_ids),
        "prompt_cache_key": format!("analysis-agent-{}", req.prompt_version),
        // Streamed: deltas render while the model reasons, so long
        // drafts never hit the non-streaming server time limit.
        // `include_usage` appends the usage chunk the SSE lane omits
        // by default.
        "stream": true,
        "stream_options": {"include_usage": true},
    })
}

impl MetaRestClient {
    fn attempt(
        &self,
        req: &DraftRequest,
        previous_failure: Option<&str>,
    ) -> Result<DraftResponse, ModelError> {
        let body = draft_body(req, previous_failure);
        let response = self.post_chat(&body)?;
        if response.status == 401 {
            return Err(ModelError::Auth(
                "meta api: unauthorized (check meta.api_key)".to_string(),
            ));
        }
        if response.status == 402 {
            return Err(ModelError::BudgetExceeded);
        }
        if !(200..300).contains(&response.status) {
            return Err(short_status("meta api", response.status, &response.body));
        }
        parse_stream(&response.body)
    }
}

/// Assemble one streamed completion: `data:` JSON chunks carry
/// `choices[0].delta.content` slices (skipped when absent — the final
/// chunk's `content` is null), the terminal `finish_reason`, and, with
/// `stream_options.include_usage`, a trailing `usage` chunk. `[DONE]`
/// ends the lane; anything else is ignored (keep-alives, comments).
fn parse_stream(body: &[u8]) -> Result<DraftResponse, ModelError> {
    let bad = |why: &str| ModelError::BadOutput(format!("stream: {why}"));
    let text = std::str::from_utf8(body).map_err(|e| bad(&format!("not utf-8: {e}")))?;
    let mut content = String::new();
    let mut refusal = String::new();
    let mut finish: Option<String> = None;
    let mut usage = serde_json::json!({});
    let mut saw_done = false;
    for line in text.lines() {
        let line = line.trim();
        let Some(data) = line.strip_prefix("data:") else {
            continue;
        };
        let data = data.trim();
        if data == "[DONE]" {
            saw_done = true;
            break;
        }
        if data.is_empty() {
            continue;
        }
        let chunk: serde_json::Value =
            serde_json::from_str(data).map_err(|e| bad(&format!("chunk not json: {e}")))?;
        if let Some(u) = chunk.get("usage") {
            usage = u.clone();
        }
        let Some(choice) = chunk
            .get("choices")
            .and_then(|c| c.as_array())
            .and_then(|c| c.first())
        else {
            continue;
        };
        if let Some(reason) = choice.get("finish_reason").and_then(|r| r.as_str()) {
            finish = Some(reason.to_string());
        }
        let delta = &choice["delta"];
        if let Some(slice) = delta.get("content").and_then(|c| c.as_str()) {
            content.push_str(slice);
        }
        if let Some(slice) = delta.get("refusal").and_then(|r| r.as_str()) {
            refusal.push_str(slice);
        }
    }
    if !saw_done {
        return Err(bad("stream ended without [DONE]"));
    }
    if !refusal.is_empty() {
        return Err(bad("model refused"));
    }
    match finish.as_deref() {
        Some("stop") => {}
        Some("length") => return Err(bad("truncated (finish_reason=length)")),
        other => return Err(bad(&format!("finish_reason={other:?}"))),
    }
    records_from_content(&content, &usage)
}

/// Parse records + usage out of one complete content string. Every
/// failure is [`ModelError::BadOutput`] with a short path message —
/// the model output is never echoed (it can be kilobytes of
/// evidence-shaped prose).
fn records_from_content(
    content: &str,
    usage: &serde_json::Value,
) -> Result<DraftResponse, ModelError> {
    let bad = |why: &str| ModelError::BadOutput(format!("completion: {why}"));
    let inner: serde_json::Value =
        serde_json::from_str(content).map_err(|e| bad(&format!("content not json: {e}")))?;
    let records = inner["records"]
        .as_array()
        .ok_or_else(|| bad("no records[]"))?;
    let mut out = Vec::with_capacity(records.len());
    for (index, record) in records.iter().enumerate() {
        let field = |name: &str| {
            record[name]
                .as_str()
                .filter(|s| !s.is_empty())
                .ok_or_else(|| bad(&format!("records[{index}].{name}: want non-empty string")))
        };
        let citation = &record["citation"];
        let u32_field = |name: &str| {
            citation[name]
                .as_u64()
                .and_then(|n| u32::try_from(n).ok())
                .ok_or_else(|| bad(&format!("records[{index}].citation.{name}: want u32")))
        };
        let confidence = record["confidence"]
            .as_f64()
            .ok_or_else(|| bad(&format!("records[{index}].confidence: want number")))?;
        #[allow(clippy::cast_possible_truncation)]
        let confidence = confidence as f32;
        let doc_id = citation["doc_id"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                bad(&format!(
                    "records[{index}].citation.doc_id: want non-empty string"
                ))
            })?;
        out.push(ProposedRecord {
            topic: field("topic")?.to_string(),
            field: field("field")?.to_string(),
            value: field("value")?.to_string(),
            citation: CitationRef {
                doc_id: doc_id.to_string(),
                byte_start: u32_field("byte_start")?,
                byte_end: u32_field("byte_end")?,
            },
            confidence,
        });
    }
    // `usage` is optional on the wire; record when present.
    // Counts above u32::MAX saturate (4B tokens/call is impossible).
    let tokens = |name: &str| {
        usage[name]
            .as_u64()
            .map(|n| u32::try_from(n).unwrap_or(u32::MAX))
            .unwrap_or(0)
    };
    let counted = Usage {
        input_tokens: tokens("prompt_tokens"),
        output_tokens: tokens("completion_tokens"),
    };
    // Required by the schema, but tolerate absence (a summary must
    // never fail a draft — it quarantines in the UI, unused here).
    // Truncated to 500 chars on a char boundary.
    let summary: String = inner["summary"]
        .as_str()
        .unwrap_or("")
        .chars()
        .take(500)
        .collect();
    Ok(DraftResponse {
        records: out,
        usage: counted,
        summary,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    fn sample_request() -> DraftRequest {
        DraftRequest {
            authority_slug: "wa-sos".to_string(),
            prompt_version: "aa-prompt-v1",
            input_text: "PO Box 40220".to_string(),
            max_records: 4,
            capture_id: 0,
            doc_ids: vec!["fixture".to_string()],
        }
    }

    #[test]
    fn stub_is_deterministic() {
        let client = StubClient;
        let req = sample_request();
        let first = client.draft_records(&req).expect("stub drafts");
        let second = client.draft_records(&req).expect("stub drafts");
        assert_eq!(first, second);
        assert!(!first.records.is_empty());
        assert!(
            first.records.iter().all(|r| r.citation.doc_id == "fixture"),
            "stub cites fixture doc only"
        );
    }

    #[test]
    fn stub_respects_max_records() {
        let client = StubClient;
        let mut req = sample_request();
        req.max_records = 1;
        let resp = client.draft_records(&req).expect("stub drafts");
        assert_eq!(resp.records.len(), 1);
    }

    #[test]
    fn meta_client_new_stores_config() {
        let client =
            MetaRestClient::new("https://api.meta.ai/v1".to_string(), "sekrit".to_string());
        assert_eq!(client.base_url, "https://api.meta.ai/v1");
        assert_eq!(client.api_key, "sekrit");
    }

    #[test]
    fn meta_client_debug_redacts_key() {
        let client =
            MetaRestClient::new("https://api.meta.ai/v1".to_string(), "sekrit".to_string());
        let shown = format!("{client:?}");
        assert!(!shown.contains("sekrit"), "got: {shown}");
        assert!(shown.contains("api_key"), "got: {shown}");
    }

    #[test]
    fn draft_empty_key_fails_closed_without_network() {
        // No stub: an empty key must fail before any dial.
        let client = MetaRestClient::new("http://127.0.0.1:9".to_string(), String::new());
        let err = client
            .draft_records(&sample_request())
            .expect_err("empty key fails closed");
        assert_eq!(
            err,
            ModelError::Auth("meta api key not configured".to_string())
        );
    }

    #[test]
    fn draft_posts_chat_and_parses_records() {
        let records = r#"{"summary": "two address records", "records": [
            {"topic": "address", "field": "po_box", "value": "PO Box 40220",
             "citation": {"doc_id": "fixture", "byte_start": 0, "byte_end": 12},
             "confidence": 0.9},
            {"topic": "hours", "field": "weekday", "value": "Mon-Fri 9-5",
             "citation": {"doc_id": "fixture", "byte_start": 13, "byte_end": 24},
             "confidence": 0.8}]}"#;
        let (base, seen, handle) = stub(1, move |head, body| {
            assert!(head.starts_with("POST /chat/completions "), "{head}");
            assert!(
                head.to_ascii_lowercase()
                    .contains("authorization: bearer stub-key"),
                "{head}"
            );
            let v: serde_json::Value = serde_json::from_str(body).unwrap();
            assert_eq!(v["model"], "muse-spark-1.3");
            // Stable rules first, per-run values last.
            assert_eq!(v["messages"][0]["role"], "developer");
            let rules = v["messages"][0]["content"].as_str().unwrap();
            assert!(!rules.contains("{{"), "no placeholders in standing rules");
            assert!(!rules.contains("wa-sos"), "no per-run slug in rules");
            assert!(!rules.contains("PO Box 40220"), "no evidence in rules");
            assert_eq!(v["messages"][1]["role"], "user");
            let prompt = v["messages"][1]["content"].as_str().unwrap();
            assert!(prompt.contains("wa-sos"), "slug substituted");
            assert!(prompt.contains("PO Box 40220"), "evidence inline");
            assert!(prompt.contains("fixture"), "doc ids named");
            assert_eq!(
                v["response_format"]["json_schema"]["schema"]["properties"]["records"]["items"]["properties"]
                    ["citation"]["properties"]["doc_id"],
                serde_json::json!({"type": "string", "enum": ["fixture"]})
            );
            assert!(v.get("temperature").is_none(), "sampling untouched");
            assert_eq!(v["reasoning_effort"], "medium");
            assert_eq!(v["response_format"]["type"], "json_schema");
            assert_eq!(
                v["response_format"]["json_schema"]["schema"]["required"],
                serde_json::json!(["summary", "records"])
            );
            assert_eq!(v["prompt_cache_key"], "analysis-agent-aa-prompt-v1");
            assert_eq!(v["stream"], true);
            assert_eq!(
                v["stream_options"],
                serde_json::json!({"include_usage": true})
            );
            (
                200,
                sse_stream(
                    serde_json::json!({"role": "assistant", "content": records}),
                    "stop",
                ),
            )
        });
        let client = MetaRestClient::new(base, "stub-key".to_string());
        let resp = client.draft_records(&sample_request()).expect("drafts");
        assert_eq!(resp.records.len(), 2);
        assert_eq!(resp.records[0].topic, "address");
        assert_eq!(resp.records[0].citation.byte_end, 12);
        assert_eq!(resp.records[1].value, "Mon-Fri 9-5");
        assert_eq!(resp.summary, "two address records");
        assert_eq!(
            resp.usage,
            Usage {
                input_tokens: 25,
                output_tokens: 19
            }
        );
        let _ = seen.recv_timeout(Duration::from_secs(5)).unwrap();
        handle.join().unwrap();
    }

    #[test]
    fn draft_retries_429_then_succeeds() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let calls = std::sync::Arc::new(AtomicUsize::new(0));
        let seen_calls = calls.clone();
        let records = r#"{"records": []}"#.to_string();
        let (base, seen, handle) = stub(2, move |_, _| {
            if seen_calls.fetch_add(1, Ordering::SeqCst) == 0 {
                (
                    429,
                    r#"{"error": {"type": "rate_limit_error"}}"#.to_string(),
                )
            } else {
                (
                    200,
                    sse_stream(
                        serde_json::json!({"role": "assistant", "content": records.clone()}),
                        "stop",
                    ),
                )
            }
        });
        let client = MetaRestClient::new(base, "stub-key".to_string());
        let resp = client.draft_records(&sample_request()).expect("retry wins");
        assert!(resp.records.is_empty());
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        for _ in 0..2 {
            seen.recv_timeout(Duration::from_secs(10)).unwrap();
        }
        handle.join().unwrap();
    }

    #[test]
    fn draft_unauthorized_is_auth_not_retry() {
        // One exchange: 401 is terminal, never retried.
        let (base, seen, handle) = stub(1, |_, _| {
            (
                401,
                r#"{"error": {"type": "authentication_error"}}"#.to_string(),
            )
        });
        let client = MetaRestClient::new(base, "stub-key".to_string());
        let err = client.draft_records(&sample_request()).expect_err("401");
        assert!(matches!(err, ModelError::Auth(_)), "{err}");
        assert!(!err.to_string().contains("stub-key"), "{err}");
        let _ = seen.recv_timeout(Duration::from_secs(5)).unwrap();
        handle.join().unwrap();
    }

    #[test]
    fn draft_billing_is_budget_exceeded() {
        let (base, seen, handle) = stub(1, |_, _| {
            (402, r#"{"error": {"type": "billing_error"}}"#.to_string())
        });
        let client = MetaRestClient::new(base, "stub-key".to_string());
        let err = client.draft_records(&sample_request()).expect_err("402");
        assert_eq!(err, ModelError::BudgetExceeded);
        let _ = seen.recv_timeout(Duration::from_secs(5)).unwrap();
        handle.join().unwrap();
    }

    #[test]
    fn draft_garbage_is_terminal_bad_output() {
        // Schema violations never retry: one exchange, then BadOutput.
        let (base, seen, handle) = stub(1, |_, _| {
            (
                200,
                sse_stream(
                    serde_json::json!({"role": "assistant", "content": "not json{{{"}),
                    "stop",
                ),
            )
        });
        let client = MetaRestClient::new(base, "stub-key".to_string());
        let err = client
            .draft_records(&sample_request())
            .expect_err("garbage");
        assert!(matches!(err, ModelError::BadOutput(_)), "{err}");
        let _ = seen.recv_timeout(Duration::from_secs(5)).unwrap();
        handle.join().unwrap();
    }

    #[test]
    fn draft_truncation_then_garbage_is_bad_output() {
        // Length/refusal retries once; a second failure is terminal.
        for (name, message, finish) in [
            (
                "refused",
                serde_json::json!({"role": "assistant", "refusal": "sorry"}),
                "stop",
            ),
            (
                "truncated",
                serde_json::json!({"role": "assistant", "content": r#"{"records": []}"#}),
                "length",
            ),
        ] {
            let (base, seen, handle) =
                stub(2, move |_, _| (200, sse_stream(message.clone(), finish)));
            let client = MetaRestClient::new(base, "stub-key".to_string());
            let err = client.draft_records(&sample_request()).expect_err(name);
            assert!(matches!(err, ModelError::BadOutput(_)), "{name}: {err}");
            for _ in 0..2 {
                seen.recv_timeout(Duration::from_secs(5)).unwrap();
            }
            handle.join().unwrap();
        }
    }

    #[test]
    fn draft_truncation_retries_with_fewer_records() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let calls = std::sync::Arc::new(AtomicUsize::new(0));
        let seen_calls = calls.clone();
        let records = r#"{"summary": "one", "records": []}"#.to_string();
        let (base, seen, handle) = stub(2, move |_, body| {
            let n = seen_calls.fetch_add(1, Ordering::SeqCst);
            let v: serde_json::Value = serde_json::from_str(body).unwrap();
            if n == 0 {
                assert_eq!(v["max_completion_tokens"], 32_000, "raised budget");
                let first = v["messages"][1]["content"].as_str().unwrap();
                assert!(first.contains("MAX RECORDS: 4"), "full max_records");
                assert!(!first.contains("Previous attempt failed"), "no note yet");
                (
                    200,
                    sse_stream(
                        serde_json::json!({"role": "assistant", "content": r#"{"records": []}"#}),
                        "length",
                    ),
                )
            } else {
                let prompt = v["messages"][1]["content"].as_str().unwrap();
                assert!(prompt.contains("Previous attempt failed"), "reason carried");
                assert!(prompt.contains("MAX RECORDS: 2"), "halved max_records");
                (
                    200,
                    sse_stream(
                        serde_json::json!({"role": "assistant", "content": records.clone()}),
                        "stop",
                    ),
                )
            }
        });
        let client = MetaRestClient::new(base, "stub-key".to_string());
        let resp = client.draft_records(&sample_request()).expect("retry wins");
        assert_eq!(resp.summary, "one");
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        for _ in 0..2 {
            seen.recv_timeout(Duration::from_secs(5)).unwrap();
        }
        handle.join().unwrap();
    }

    #[test]
    fn check_lists_models() {
        let (base, seen, handle) = stub(1, |head, _| {
            assert!(head.starts_with("GET /models "), "{head}");
            (
                200,
                r#"{"data": [{"id": "muse-spark-1.3"}, {"id": "muse-wonder-1.0"}]}"#.to_string(),
            )
        });
        let client = MetaRestClient::new(base, "stub-key".to_string());
        let ids = client.check().expect("catalog");
        assert_eq!(ids, vec!["muse-spark-1.3", "muse-wonder-1.0"]);
        let _ = seen.recv_timeout(Duration::from_secs(5)).unwrap();
        handle.join().unwrap();
    }

    #[test]
    fn check_unauthorized_is_auth() {
        let (base, seen, handle) = stub(1, |_, _| (401, String::new()));
        let client = MetaRestClient::new(base, "stub-key".to_string());
        let err = client.check().expect_err("401");
        assert!(matches!(err, ModelError::Auth(_)), "{err}");
        let _ = seen.recv_timeout(Duration::from_secs(5)).unwrap();
        handle.join().unwrap();
    }

    /// One SSE lane for a caller-built message: content arrives in
    /// two slices (proving assembly), then the terminal chunk carries
    /// `finish_reason` + `usage`, then `[DONE]`.
    fn sse_stream(message: serde_json::Value, finish_reason: &str) -> String {
        fn chunk(delta: serde_json::Value) -> String {
            format!(
                "data: {}\n\n",
                serde_json::json!({"choices": [{"index": 0, "delta": delta}]})
            )
        }
        let mut out = String::new();
        if let Some(content) = message.get("content").and_then(|c| c.as_str()) {
            let mid = content
                .char_indices()
                .nth(content.chars().count() / 2)
                .map(|(i, _)| i)
                .unwrap_or(0);
            for slice in [&content[..mid], &content[mid..]] {
                out.push_str(&chunk(serde_json::json!({"content": slice})));
            }
        }
        if let Some(refusal) = message.get("refusal").and_then(|r| r.as_str()) {
            out.push_str(&chunk(serde_json::json!({"refusal": refusal})));
        }
        out.push_str(&format!(
            "data: {}\n\n",
            serde_json::json!({
                "choices": [{"index": 0, "delta": {}, "finish_reason": finish_reason}],
                "usage": {"prompt_tokens": 25, "completion_tokens": 19, "total_tokens": 44},
            })
        ));
        out.push_str("data: [DONE]\n");
        out
    }

    #[test]
    fn draft_stream_cut_before_done_is_bad_output() {
        let err = parse_stream(b"data: {\"choices\":[]}\n\n").expect_err("no DONE");
        assert!(err.to_string().contains("without [DONE]"), "{err}");
        let err = parse_stream(b"not events at all\n").expect_err("no lane");
        assert!(matches!(err, ModelError::BadOutput(_)), "{err}");
    }

    /// Canned HTTP/1.1 exchanges against a port-0 stub (voice precedent):
    /// the test hands the stub an exchange count + responder, the stub
    /// binds 127.0.0.1:0 and reports its base URL.
    fn stub<F>(
        exchanges: usize,
        respond: F,
    ) -> (String, mpsc::Receiver<String>, std::thread::JoinHandle<()>)
    where
        F: Fn(&str, &str) -> (u16, String) + Send + 'static,
    {
        use std::io::{BufRead, BufReader, Read, Write};
        use std::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
        let (tx, rx) = mpsc::channel();
        let handle = std::thread::spawn(move || {
            for _ in 0..exchanges {
                let (stream, _) = listener.accept().unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut head = String::new();
                reader.read_line(&mut head).unwrap();
                let mut len = 0usize;
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    let trimmed = line.trim_end();
                    if trimmed.is_empty() {
                        break;
                    }
                    head.push_str(&line);
                    if let Some(n) = trimmed
                        .to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .and_then(|v| v.trim().parse().ok())
                    {
                        len = n;
                    }
                }
                let mut body = vec![0u8; len];
                reader.read_exact(&mut body).unwrap();
                let body = String::from_utf8_lossy(&body).to_string();
                let (status, reply) = respond(&head, &body);
                tx.send(format!("{head}\n{body}")).unwrap();
                let mut stream = reader.into_inner();
                let text = format!(
                    "HTTP/1.1 {status} OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}",
                    reply.len()
                );
                stream.write_all(text.as_bytes()).unwrap();
            }
        });
        (base, rx, handle)
    }
}
