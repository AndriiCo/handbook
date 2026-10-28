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
    fn draft_records(&self, req: &DraftRequest) -> Result<DraftResponse, ModelError> {
        let records = vec![
            ProposedRecord {
                topic: "address".to_string(),
                field: "po_box".to_string(),
                value: "PO Box 40220".to_string(),
                citation: CitationRef {
                    doc_id: "fixture".to_string(),
                    byte_start: 0,
                    byte_end: 12,
                },
                confidence: 0.9,
            },
            ProposedRecord {
                topic: "hours".to_string(),
                field: "weekday".to_string(),
                value: "Mon-Fri 9-5".to_string(),
                citation: CitationRef {
                    doc_id: "fixture".to_string(),
                    byte_start: 13,
                    byte_end: 24,
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
        Self {
            agent: ureq::Agent::new_with_config(config),
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

    fn post_chat(&self, body: &serde_json::Value) -> Result<RawResponse, ModelError> {
        let url = format!("{}/chat/completions", self.base_url);
        let mut sleeps = RETRY_SLEEP_SECS.iter();
        loop {
            let attempt = self.post_json(&url, body);
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

    fn post_json(&self, url: &str, body: &serde_json::Value) -> Result<RawResponse, ModelError> {
        let response = self
            .agent
            .post(url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
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
/// lists all properties). The API enforces shape; our byte-exact
/// citation validator still enforces grounding.
fn response_format() -> serde_json::Value {
    json!({
        "type": "json_schema",
        "json_schema": {
            "name": "ProposedRecords",
            "schema": {
                "type": "object",
                "properties": {
                    "records": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "topic": {"type": "string"},
                                "field": {"type": "string"},
                                "value": {"type": "string"},
                                "citation": {
                                    "type": "object",
                                    "properties": {
                                        "doc_id": {"type": "string"},
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
                "required": ["records"],
                "additionalProperties": false
            }
        }
    })
}

impl ModelClient for MetaRestClient {
    fn draft_records(&self, req: &DraftRequest) -> Result<DraftResponse, ModelError> {
        if self.api_key.is_empty() {
            return Err(ModelError::Auth("meta api key not configured".to_string()));
        }
        let prompt = build_prompt(include_str!("../prompts/aa-prompt-v1.md"), req);
        let body = json!({
            "model": DRAFT_MODEL,
            "messages": [{"role": "developer", "content": prompt}],
            "temperature": 0,
            "max_completion_tokens": 2000,
            "response_format": response_format(),
            "prompt_cache_key": format!("analysis-agent-{}", req.prompt_version),
        });
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
        parse_completion(&response.body)
    }
}

/// Parse a `chat.completion` body into records + usage. Every failure
/// is [`ModelError::BadOutput`] with a short path message — the model
/// output is never echoed (it can be kilobytes of evidence-shaped prose).
fn parse_completion(body: &[u8]) -> Result<DraftResponse, ModelError> {
    let bad = |why: &str| ModelError::BadOutput(format!("completion: {why}"));
    let value: serde_json::Value =
        serde_json::from_slice(body).map_err(|e| bad(&format!("not json: {e}")))?;
    let choice = value["choices"]
        .as_array()
        .and_then(|c| c.first())
        .ok_or_else(|| bad("no choices"))?;
    if choice["message"]["refusal"].is_string() {
        return Err(bad("model refused"));
    }
    if choice["finish_reason"].as_str() == Some("length") {
        return Err(bad("truncated (finish_reason=length)"));
    }
    let content = choice["message"]["content"]
        .as_str()
        .ok_or_else(|| bad("no string content"))?;
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
        value["usage"][name]
            .as_u64()
            .map(|n| u32::try_from(n).unwrap_or(u32::MAX))
            .unwrap_or(0)
    };
    let usage = Usage {
        input_tokens: tokens("prompt_tokens"),
        output_tokens: tokens("completion_tokens"),
    };
    Ok(DraftResponse {
        records: out,
        usage,
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
        let records = r#"{"records": [
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
            assert_eq!(v["messages"][0]["role"], "developer");
            let prompt = v["messages"][0]["content"].as_str().unwrap();
            assert!(prompt.contains("wa-sos"), "slug substituted");
            assert!(prompt.contains("PO Box 40220"), "evidence inline");
            assert_eq!(v["temperature"], 0);
            assert_eq!(v["response_format"]["type"], "json_schema");
            assert_eq!(
                v["response_format"]["json_schema"]["schema"]["required"],
                serde_json::json!(["records"])
            );
            assert_eq!(v["prompt_cache_key"], "analysis-agent-aa-prompt-v1");
            (
                200,
                completion(
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
                    completion(
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
    fn draft_refusal_truncation_garbage_are_bad_output() {
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
            (
                "garbage",
                serde_json::json!({"role": "assistant", "content": "not json{{{"}),
                "stop",
            ),
        ] {
            let (base, seen, handle) =
                stub(1, move |_, _| (200, completion(message.clone(), finish)));
            let client = MetaRestClient::new(base, "stub-key".to_string());
            let err = client.draft_records(&sample_request()).expect_err(name);
            assert!(matches!(err, ModelError::BadOutput(_)), "{name}: {err}");
            let _ = seen.recv_timeout(Duration::from_secs(5)).unwrap();
            handle.join().unwrap();
        }
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

    /// One `chat.completion` envelope with a caller-built message.
    fn completion(message: serde_json::Value, finish_reason: &str) -> String {
        serde_json::json!({
            "id": "chatcmpl-test",
            "object": "chat.completion",
            "created": 1714502400,
            "model": "muse-spark-1.3",
            "choices": [{"index": 0, "message": message, "finish_reason": finish_reason}],
            "usage": {"prompt_tokens": 25, "completion_tokens": 19, "total_tokens": 44}
        })
        .to_string()
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
