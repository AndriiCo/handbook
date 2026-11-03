//! Model-client implementations for the analysis agent.
//!
//! The interface ([`ModelClient`](contracts::model::ModelClient) and its
//! request/response types) lives in `contracts`; this module keeps the
//! [`StubClient`] fixtures and the [`MetaRestClient`] skeleton.

use std::fmt;
use std::io::Read;
use std::sync::Arc;
use std::time::{Duration, Instant};

use callcache::{CacheError, MetaCache};
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
                cached_tokens: 0,
                reasoning_tokens: 0,
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

/// Second prompt generation: topic definitions + closed per-topic
/// field lists. Selected by `DraftRequest.prompt_version`; v1 stays
/// the default (see `runner::PROMPT_VERSION`).
pub const PROMPT_VERSION_V2: &str = "aa-prompt-v2";

/// Standing-rules template for a prompt version. Only the two named
/// generations exist; anything else reads as v1 (the only producers
/// are `runner::PROMPT_VERSION` and the eval `--prompt` flag, which
/// rejects unknown values, so the fallback is unreachable).
pub(crate) fn template_for(version: &str) -> &'static str {
    if version == PROMPT_VERSION_V2 {
        include_str!("../prompts/aa-prompt-v2.md")
    } else {
        include_str!("../prompts/aa-prompt-v1.md")
    }
}

/// Retry budget: up to 4 attempts per stream POST (error-handling
/// docs: 3–5 with jitter, honouring `Retry-After`).
const MAX_ATTEMPTS: u32 = 4;
/// First backoff step; doubles per retry (2s, 4s, 8s) plus jitter.
const RETRY_BASE_SECS: u64 = 2;
/// A server `Retry-After` beyond this is treated as this (minutes-long
/// sleeps help nobody in an operator batch).
const MAX_RETRY_AFTER_SECS: u64 = 60;

/// Backoff for retry `attempt_no` (1-based): exponential from the base
/// with up to +50% jitter, or the server's `Retry-After` when present.
fn retry_delay(base_secs: u64, attempt_no: u32, retry_after: Option<u64>) -> Duration {
    if let Some(after) = retry_after {
        return Duration::from_secs(after.min(MAX_RETRY_AFTER_SECS));
    }
    let backoff_secs = base_secs.saturating_mul(1u64 << attempt_no.saturating_sub(1).min(5));
    Duration::from_secs(backoff_secs) + Duration::from_millis(jitter_ms(backoff_secs * 1000))
}

/// Jitter without an rng dependency: the nanos clock, capped at half
/// the backoff (atlas transport precedent).
fn jitter_ms(range_ms: u64) -> u64 {
    if range_ms == 0 {
        return 0;
    }
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| u64::from(d.subsec_nanos()) % range_ms)
        .unwrap_or(0)
        .min(range_ms / 2)
}

/// One Meta HTTP attempt as telemetry sees it (M1): every POST the
/// client sends — retries included — yields exactly one record, so
/// daily caps and p50/p95 count real calls. No request or response
/// text rides here, only shape and spend.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallRecord {
    /// Model id from the request body (`muse-spark-1.3`, …).
    pub model: String,
    /// Which pipeline sent it (`draft`, `plan`, `factcheck`, …).
    pub pipeline: &'static str,
    /// Server-allocated run id, when the call belongs to a run.
    pub run_id: Option<u64>,
    /// 1-based attempt number within one logical request.
    pub attempt: u32,
    /// Wall time from send to the last body byte.
    pub latency_ms: u64,
    /// Wall time from send to the first body byte (the first SSE
    /// event), when a body arrived.
    pub ttft_ms: Option<u64>,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub cached_tokens: u32,
    pub reasoning_tokens: u32,
    /// `ok | refusal | length | parse_error | http_4xx | http_5xx |
    /// timeout | transport`.
    pub outcome: &'static str,
    /// HTTP status, when a response arrived.
    pub http_status: Option<u16>,
}

/// Sink for [`CallRecord`]s (the web crate writes `model_call` rows).
/// Called on the client's blocking thread; implementations must not
/// panic and should swallow their own storage errors.
pub trait CallObserver: Send + Sync {
    fn record(&self, call: &CallRecord);
}

/// One strict-JSON chat call ([`MetaRestClient::chat_json`]): the
/// shared shape for every pipeline beyond drafting (plans, offices,
/// cross-checks, form maps, link triage).
#[derive(Debug, Clone)]
pub struct ChatJsonRequest<'a> {
    /// `muse-spark-1.3` or `muse-wonder-1.0`.
    pub model: &'a str,
    /// Stable standing rules (the cacheable `developer` prefix): no
    /// per-run values here.
    pub developer: &'a str,
    /// Per-run values (`user` message).
    pub user: String,
    /// `json_schema.name` (alphanumeric + `_`).
    pub schema_name: &'a str,
    /// Strict-subset JSON schema for the reply.
    pub schema: serde_json::Value,
    /// `low | medium | high` (never omitted: findings F4).
    pub reasoning_effort: &'a str,
    /// Stable per prompt version, so the prefix cache hits.
    pub prompt_cache_key: String,
    pub max_completion_tokens: u32,
    /// Telemetry labels for this call.
    pub pipeline: &'static str,
    pub run_id: Option<u64>,
}

/// A parsed strict-JSON reply plus its spend.
#[derive(Debug, Clone)]
pub struct ChatJsonResponse {
    pub value: serde_json::Value,
    pub usage: Usage,
    /// Wall time of the successful attempt.
    pub latency_ms: u64,
    /// Attempts it took (1 = first try).
    pub attempts: u32,
}

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
    observer: Option<Arc<dyn CallObserver>>,
    pipeline: &'static str,
    run_id: Option<u64>,
    /// Record/replay cache: a hit skips the network, the observer and
    /// the daily cap (`docs/ops/call-cache.md`).
    cache: Option<Arc<MetaCache>>,
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
            observer: None,
            pipeline: "draft",
            run_id: None,
            cache: None,
        }
    }

    /// Serve repeat requests from `cache` and count live calls in its
    /// daily ledger (operator decision 2026-11-03: reruns never
    /// re-spend budget).
    #[must_use]
    pub fn with_cache(mut self, cache: Arc<MetaCache>) -> Self {
        self.cache = Some(cache);
        self
    }

    /// Report every HTTP attempt to `observer`, labelled with
    /// `pipeline` and `run_id` (drafts default to pipeline `draft`).
    #[must_use]
    pub fn with_observer(
        mut self,
        observer: Arc<dyn CallObserver>,
        pipeline: &'static str,
        run_id: Option<u64>,
    ) -> Self {
        self.observer = Some(observer);
        self.pipeline = pipeline;
        self.run_id = run_id;
        self
    }

    fn observe(&self, call: CallRecord) {
        if let Some(observer) = &self.observer {
            observer.record(&call);
        }
    }

    /// Record one finished attempt: non-2xx and transport failures as
    /// their HTTP class, a 2xx as `ok` or the parse failure it hit.
    fn observe_attempt(
        &self,
        model: &str,
        pipeline: &'static str,
        run_id: Option<u64>,
        raw: &Result<RawResponse, ModelError>,
        parsed: Option<(&'static str, Usage)>,
    ) {
        // A cache hit is not a model call: no row, no cap spend.
        if self.observer.is_none() || matches!(raw, Ok(r) if r.cached) {
            return;
        }
        let zero = Usage {
            input_tokens: 0,
            output_tokens: 0,
            cached_tokens: 0,
            reasoning_tokens: 0,
        };
        let call = match raw {
            Ok(r) => {
                let (outcome, usage) = match parsed {
                    Some(p) => p,
                    None if (400..500).contains(&r.status) => ("http_4xx", zero),
                    None if r.status >= 500 => ("http_5xx", zero),
                    None => ("parse_error", zero),
                };
                CallRecord {
                    model: model.to_string(),
                    pipeline,
                    run_id,
                    attempt: r.attempt,
                    latency_ms: r.latency_ms,
                    ttft_ms: r.ttft_ms,
                    input_tokens: usage.input_tokens,
                    output_tokens: usage.output_tokens,
                    cached_tokens: usage.cached_tokens,
                    reasoning_tokens: usage.reasoning_tokens,
                    outcome,
                    http_status: Some(r.status),
                }
            }
            Err(e) => {
                let text = e.to_string();
                let timed_out = text.contains("timeout") || text.contains("timed out");
                CallRecord {
                    model: model.to_string(),
                    pipeline,
                    run_id,
                    attempt: 0,
                    latency_ms: 0,
                    ttft_ms: None,
                    input_tokens: 0,
                    output_tokens: 0,
                    cached_tokens: 0,
                    reasoning_tokens: 0,
                    outcome: if timed_out { "timeout" } else { "transport" },
                    http_status: None,
                }
            }
        };
        self.observe(call);
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
    ///
    /// Retry discipline (error-handling docs): up to 4 attempts with
    /// jitter, honouring `Retry-After` on 429/5xx. Terminal statuses
    /// (400/401/402/403/404/413) never retry. The only other retry in
    /// this crate is the narrowed re-draft on truncation/refusal, a
    /// different request (halved records) — never the same body twice.
    ///
    /// Telemetry: every retried attempt is recorded here; the attempt
    /// this returns is recorded by the caller once it knows the parse
    /// outcome (`observe_attempt`), so each POST counts exactly once.
    fn post_chat(
        &self,
        body: &serde_json::Value,
        pipeline: &'static str,
        run_id: Option<u64>,
    ) -> Result<RawResponse, ModelError> {
        let url = format!("{}/chat/completions", self.base_url);
        let model = body["model"].as_str().unwrap_or("unknown").to_string();
        let cached = self
            .cache
            .as_deref()
            .map(|cache| (cache, callcache::meta::json_key(&url, body)));
        if let Some((cache, key)) = &cached
            && let Some(hit) = cache.lookup(key).map_err(cache_error)?
        {
            return Ok(RawResponse {
                status: hit.status,
                body: hit.body,
                retry_after_secs: None,
                latency_ms: 0,
                ttft_ms: None,
                attempt: 1,
                cached: true,
            });
        }
        let request = callcache::meta::Request {
            method: "POST",
            url: &url,
            model: &model,
        };
        let mut attempt_no = 0u32;
        let result = loop {
            if let Some((cache, key)) = &cached {
                cache.admit(key).map_err(cache_error)?;
            }
            let started = Instant::now();
            let mut attempt = self.post_sse(&url, body, started);
            if let Some((cache, _)) = &cached {
                cache.spent(&request, attempt.as_ref().map_or(0, |r| r.status));
            }
            if let Ok(r) = attempt.as_mut() {
                r.attempt = attempt_no + 1;
            }
            let retry_after = match &attempt {
                Err(ModelError::Transport(_)) => None,
                Ok(r) if matches!(r.status, 429 | 500 | 502 | 503 | 504) => r.retry_after_secs,
                _ => break attempt,
            };
            // A server asking for longer than we will wait (or an HTTP
            // date we do not parse) gets its answer back, never an early
            // retry (review #53).
            if attempt_no >= MAX_ATTEMPTS - 1
                || retry_after.is_some_and(|s| s > MAX_RETRY_AFTER_SECS)
            {
                break attempt;
            }
            self.observe_attempt(&model, pipeline, run_id, &attempt, None);
            attempt_no += 1;
            std::thread::sleep(retry_delay(RETRY_BASE_SECS, attempt_no, retry_after));
        };
        if let (Some((cache, key)), Ok(r)) = (&cached, &result) {
            // A full disk must not discard an answer already paid for.
            let _ = cache.store(key, &request, r.status, &r.body);
        }
        result
    }

    fn post_sse(
        &self,
        url: &str,
        body: &serde_json::Value,
        started: Instant,
    ) -> Result<RawResponse, ModelError> {
        let response = self
            .stream_agent
            .post(url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .header("Accept", "text/event-stream")
            .send_json(body)
            .map_err(|e| ModelError::Transport(e.to_string()))?;
        let status = response.status().as_u16();
        let retry_after_secs = response
            .headers()
            .get("Retry-After")
            .and_then(|v| v.to_str().ok())
            // Seconds, or (HTTP-date) "later than we wait".
            .map(|v| v.trim().parse().unwrap_or(u64::MAX));
        // Read the stream in chunks so the first event's arrival time
        // (TTFT for telemetry) is known; same bytes as read_to_vec.
        let mut reader = response.into_body().into_reader();
        let mut out = Vec::new();
        let mut buf = [0u8; 16 * 1024];
        let mut ttft_ms = None;
        loop {
            let n = reader
                .read(&mut buf)
                .map_err(|e| ModelError::Transport(e.to_string()))?;
            if n == 0 {
                break;
            }
            if ttft_ms.is_none() {
                ttft_ms = Some(elapsed_ms(started));
            }
            out.extend_from_slice(&buf[..n]);
        }
        Ok(RawResponse {
            status,
            body: out,
            retry_after_secs,
            latency_ms: elapsed_ms(started),
            ttft_ms,
            attempt: 1,
            cached: false,
        })
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
        Ok(RawResponse {
            status,
            body,
            retry_after_secs: None,
            latency_ms: 0,
            ttft_ms: None,
            attempt: 1,
            cached: false,
        })
    }
}

/// Cache refusals as model errors: the cap is the budget error every
/// caller already handles; a replay-only miss reads as transport.
fn cache_error(e: CacheError) -> ModelError {
    match e {
        CacheError::OverBudget { .. } => ModelError::BudgetExceeded,
        other => ModelError::Transport(other.to_string()),
    }
}

/// Milliseconds since `started`, saturating.
fn elapsed_ms(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
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
    retry_after_secs: Option<u64>,
    /// Telemetry: send → last byte, send → first byte, 1-based attempt.
    latency_ms: u64,
    ttft_ms: Option<u64>,
    attempt: u32,
    /// Served from the call cache (no HTTP happened).
    cached: bool,
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
                                "value": {"type": "string", "minLength": 2},
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
    let template = template_for(req.prompt_version);
    let (developer, per_run) = template
        .split_once(PER_RUN_MARKER)
        .expect("prompt template carries a PER-RUN part");
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
        let raw = self.post_chat(&body, self.pipeline, self.run_id);
        let parsed = match &raw {
            Ok(r) if (200..300).contains(&r.status) => Some(parse_stream(&r.body)),
            _ => None,
        };
        self.observe_attempt(
            DRAFT_MODEL,
            self.pipeline,
            self.run_id,
            &raw,
            parsed.as_ref().map(|p| match p {
                Ok(resp) => ("ok", resp.usage.clone()),
                Err(e) => (failure_outcome(e), failed_usage(&raw)),
            }),
        );
        let response = raw?;
        if let Some(parsed) = parsed {
            return parsed;
        }
        if response.status == 401 {
            return Err(ModelError::Auth(
                "meta api: unauthorized (check meta.api_key)".to_string(),
            ));
        }
        if response.status == 402 {
            return Err(ModelError::BudgetExceeded);
        }
        Err(short_status("meta api", response.status, &response.body))
    }

    /// One strict-JSON chat call for any pipeline (plans, offices,
    /// cross-checks, form maps, link triage), per
    /// `docs/meta-api.md` (checklist): no temperature, explicit
    /// `reasoning_effort`, streamed with usage, stable developer prefix
    /// plus `prompt_cache_key`, jittered retries honouring Retry-After,
    /// refusal and truncation checks. Every HTTP attempt is recorded.
    ///
    /// # Errors
    ///
    /// [`ModelError::Auth`] (401 / empty key), [`ModelError::BudgetExceeded`]
    /// (402), [`ModelError::Transport`] (other non-2xx, I/O),
    /// [`ModelError::BadOutput`] (refusal, truncation, non-JSON).
    pub fn chat_json(&self, req: &ChatJsonRequest<'_>) -> Result<ChatJsonResponse, ModelError> {
        if self.api_key.is_empty() {
            return Err(ModelError::Auth("meta api key not configured".to_string()));
        }
        let body = json!({
            "model": req.model,
            "messages": [
                {"role": "developer", "content": req.developer.trim()},
                {"role": "user", "content": req.user},
            ],
            "reasoning_effort": req.reasoning_effort,
            "max_completion_tokens": req.max_completion_tokens,
            "response_format": {
                "type": "json_schema",
                "json_schema": {"name": req.schema_name, "strict": true, "schema": req.schema},
            },
            "prompt_cache_key": req.prompt_cache_key,
            "stream": true,
            "stream_options": {"include_usage": true},
        });
        let raw = self.post_chat(&body, req.pipeline, req.run_id);
        let parsed = match &raw {
            Ok(r) if (200..300).contains(&r.status) => {
                Some(collect_stream(&r.body).and_then(|(content, usage)| {
                    serde_json::from_str::<serde_json::Value>(&content)
                        .map(|value| (value, usage_from(&usage)))
                        .map_err(|e| {
                            ModelError::BadOutput(format!("completion: content not json: {e}"))
                        })
                }))
            }
            _ => None,
        };
        self.observe_attempt(
            req.model,
            req.pipeline,
            req.run_id,
            &raw,
            parsed.as_ref().map(|p| match p {
                Ok((_, usage)) => ("ok", usage.clone()),
                Err(e) => (failure_outcome(e), failed_usage(&raw)),
            }),
        );
        let response = raw?;
        if let Some(parsed) = parsed {
            let (value, usage) = parsed?;
            return Ok(ChatJsonResponse {
                value,
                usage,
                latency_ms: response.latency_ms,
                attempts: response.attempt,
            });
        }
        if response.status == 401 {
            return Err(ModelError::Auth(
                "meta api: unauthorized (check meta.api_key)".to_string(),
            ));
        }
        if response.status == 402 {
            return Err(ModelError::BudgetExceeded);
        }
        Err(short_status("meta api", response.status, &response.body))
    }
}

/// Token spend a failed (refused / truncated / unparseable) attempt
/// still carried in its stream's usage chunk (review #54).
fn failed_usage(raw: &Result<RawResponse, ModelError>) -> Usage {
    let Ok(r) = raw else { return zero_usage() };
    let text = String::from_utf8_lossy(&r.body);
    let mut usage = serde_json::Value::Null;
    for line in text.lines() {
        if let Some(data) = line.trim().strip_prefix("data:")
            && let Ok(chunk) = serde_json::from_str::<serde_json::Value>(data.trim())
            && let Some(u) = chunk.get("usage")
        {
            usage = u.clone();
        }
    }
    if usage.is_null() {
        zero_usage()
    } else {
        usage_from(&usage)
    }
}

/// Telemetry outcome for a 2xx whose body failed to parse.
fn failure_outcome(e: &ModelError) -> &'static str {
    match e {
        ModelError::BadOutput(reason) if reason.contains("model refused") => "refusal",
        ModelError::BadOutput(reason) if reason.contains("truncated") => "length",
        _ => "parse_error",
    }
}

fn zero_usage() -> Usage {
    Usage {
        input_tokens: 0,
        output_tokens: 0,
        cached_tokens: 0,
        reasoning_tokens: 0,
    }
}

/// Assemble one streamed completion: `data:` JSON chunks carry
/// `choices[0].delta.content` slices (skipped when absent — the final
/// chunk's `content` is null), the terminal `finish_reason`, and, with
/// `stream_options.include_usage`, a trailing `usage` chunk. `[DONE]`
/// ends the lane; anything else is ignored (keep-alives, comments).
fn parse_stream(body: &[u8]) -> Result<DraftResponse, ModelError> {
    let (content, usage) = collect_stream(body)?;
    records_from_content(&content, &usage)
}

/// The content string + raw usage object of one complete stream, after
/// the `[DONE]`, refusal and `finish_reason` checks.
fn collect_stream(body: &[u8]) -> Result<(String, serde_json::Value), ModelError> {
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
    Ok((content, usage))
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
    let counted = usage_from(usage);
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

/// Token counts from one stream's `usage` object.
fn usage_from(usage: &serde_json::Value) -> Usage {
    // `usage` is optional on the wire; record when present.
    // Counts above u32::MAX saturate (4B tokens/call is impossible).
    let tokens = |name: &str| {
        usage[name]
            .as_u64()
            .map(|n| u32::try_from(n).unwrap_or(u32::MAX))
            .unwrap_or(0)
    };
    // Prefix-cache hits and thinking spend ride in the details
    // sub-objects (`docs/meta-api-shapes.md`); a flat key wins as a
    // fallback so replay fixtures can spell them either way.
    let nested = |group: &str, name: &str| {
        usage[group]
            .get(name)
            .and_then(|n| n.as_u64())
            .map(|n| u32::try_from(n).unwrap_or(u32::MAX))
    };
    Usage {
        input_tokens: tokens("prompt_tokens"),
        output_tokens: tokens("completion_tokens"),
        cached_tokens: nested("prompt_tokens_details", "cached_tokens")
            .or_else(|| (tokens("cached_tokens") != 0).then_some(tokens("cached_tokens")))
            .unwrap_or(0),
        reasoning_tokens: nested("completion_tokens_details", "reasoning_tokens")
            .or_else(|| (tokens("reasoning_tokens") != 0).then_some(tokens("reasoning_tokens")))
            .unwrap_or(0),
    }
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
    fn prompt_versions_split_on_marker_with_clean_prefix() {
        // Both generations render: standing rules up front (no
        // per-run placeholders — the stable cacheable prefix), values
        // after the marker. v2 teaches topics + closed field lists.
        for version in ["aa-prompt-v1", PROMPT_VERSION_V2] {
            let (developer, per_run) = template_for(version)
                .split_once(PER_RUN_MARKER)
                .expect("PER-RUN part");
            assert!(!developer.contains("{{"), "version: {version}");
            assert!(per_run.contains("{{slug}}"), "version: {version}");
        }
        let (v2, _) = template_for(PROMPT_VERSION_V2)
            .split_once(PER_RUN_MARKER)
            .expect("PER-RUN part");
        assert!(v2.contains("`po_box`"), "v2 defines topics");
        assert!(v2.contains("per_document"), "v2 lists closed fields");
        assert!(template_for("aa-prompt-v1").contains("Reply in the source language"));
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
            // Gate: single-character and empty values are refused by
            // the API before our own BadValue validator runs.
            assert_eq!(
                v["response_format"]["json_schema"]["schema"]["properties"]["records"]["items"]["properties"]
                    ["value"],
                serde_json::json!({"type": "string", "minLength": 2})
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
                output_tokens: 19,
                cached_tokens: 5,
                reasoning_tokens: 7,
            }
        );
        let _ = seen.recv_timeout(Duration::from_secs(5)).unwrap();
        handle.join().unwrap();
    }

    #[test]
    fn draft_replay_fixture_carries_cached_and_reasoning_tokens() {
        // Recorded wire shape (`docs/meta-api-shapes.md`): the usage
        // chunk carries prefix-cache hits and thinking spend in the
        // details sub-objects. The console renders both per call.
        let body = concat!(
            "data: {\"choices\": [{\"index\": 0, \"delta\": {\"content\": \"{\\\"summary\\\": \\\"s\\\", \\\"records\\\": []}\"}}]}\n\n",
            "data: {\"choices\": [{\"index\": 0, \"delta\": {}, \"finish_reason\": \"stop\"}],",
            " \"usage\": {\"prompt_tokens\": 1200, \"completion_tokens\": 800, \"total_tokens\": 2000,",
            " \"prompt_tokens_details\": {\"cached_tokens\": 1100},",
            " \"completion_tokens_details\": {\"reasoning_tokens\": 640}}}\n\n",
            "data: [DONE]\n",
        );
        let resp = parse_stream(body.as_bytes()).expect("replay parses");
        assert_eq!(
            resp.usage,
            Usage {
                input_tokens: 1200,
                output_tokens: 800,
                cached_tokens: 1100,
                reasoning_tokens: 640,
            }
        );
    }

    #[test]
    fn draft_replay_fixture_without_details_is_zero() {
        // Older recordings carry no details blocks: both new counts
        // read as zero, never a parse failure.
        let body = concat!(
            "data: {\"choices\": [{\"index\": 0, \"delta\": {\"content\": \"{\\\"summary\\\": \\\"s\\\", \\\"records\\\": []}\"}}]}\n\n",
            "data: {\"choices\": [{\"index\": 0, \"delta\": {}, \"finish_reason\": \"stop\"}],",
            " \"usage\": {\"prompt_tokens\": 25, \"completion_tokens\": 19, \"total_tokens\": 44}}\n\n",
            "data: [DONE]\n",
        );
        let resp = parse_stream(body.as_bytes()).expect("replay parses");
        assert_eq!(
            resp.usage,
            Usage {
                input_tokens: 25,
                output_tokens: 19,
                cached_tokens: 0,
                reasoning_tokens: 0,
            }
        );
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
    fn cached_redraft_makes_zero_http_calls_and_spends_nothing() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let calls = std::sync::Arc::new(AtomicUsize::new(0));
        let seen_calls = calls.clone();
        let records = r#"{"records": []}"#.to_string();
        // One exchange only: a second HTTP call would find no stub.
        let (base, seen, handle) = stub(1, move |_, _| {
            seen_calls.fetch_add(1, Ordering::SeqCst);
            let delta = serde_json::json!({"role": "assistant", "content": records.clone()});
            (200, sse_stream(delta, "stop"))
        });
        let dir = tempfile::tempdir().unwrap();
        let cache =
            Arc::new(MetaCache::new(dir.path(), callcache::CacheMode::Record).with_budget(5));
        let observer = Arc::new(Recorder::default());
        let client = MetaRestClient::new(base, "stub-key".to_string())
            .with_cache(cache.clone())
            .with_observer(observer.clone(), "draft", None);
        let live = client.draft_records(&sample_request()).expect("live draft");
        seen.recv_timeout(Duration::from_secs(10)).unwrap();
        handle.join().unwrap();
        let again = client
            .draft_records(&sample_request())
            .expect("cached draft");
        assert_eq!(again, live);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        // One telemetry row and one ledger line: the hit counted nowhere.
        assert_eq!(observer.0.lock().unwrap().len(), 1);
        assert_eq!(cache.budget().unwrap().used(callcache::unix_now()), 1);
        assert_eq!(cache.counts(), (1, 1));

        // Offline: replay-only answers from disk with no server at all,
        // and a request it never saw fails closed instead of dialing.
        let offline = Arc::new(MetaCache::new(dir.path(), callcache::CacheMode::ReplayOnly));
        let client = MetaRestClient::new("http://127.0.0.1:9".to_string(), "k".to_string())
            .with_cache(offline);
        assert_eq!(client.draft_records(&sample_request()).unwrap(), live);
        let mut other = sample_request();
        other.input_text.push_str(" (revised)");
        assert!(matches!(
            client.draft_records(&other),
            Err(ModelError::Transport(msg)) if msg.contains("replay-only")
        ));
    }

    #[test]
    fn daily_cap_in_the_cache_ledger_stops_live_calls() {
        let dir = tempfile::tempdir().unwrap();
        let cache =
            Arc::new(MetaCache::new(dir.path(), callcache::CacheMode::Record).with_budget(0));
        let client = MetaRestClient::new("http://127.0.0.1:9".to_string(), "k".to_string())
            .with_cache(cache);
        assert_eq!(
            client.draft_records(&sample_request()),
            Err(ModelError::BudgetExceeded)
        );
    }

    /// Collects every telemetry record (M1).
    #[derive(Default)]
    struct Recorder(std::sync::Mutex<Vec<CallRecord>>);

    impl CallObserver for Recorder {
        fn record(&self, call: &CallRecord) {
            self.0.lock().unwrap().push(call.clone());
        }
    }

    #[test]
    fn observer_records_every_attempt_including_the_retried_429() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let calls = std::sync::Arc::new(AtomicUsize::new(0));
        let seen_calls = calls.clone();
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
                        serde_json::json!({"role": "assistant", "content": r#"{"records": []}"#}),
                        "stop",
                    ),
                )
            }
        });
        let recorder = std::sync::Arc::new(Recorder::default());
        let client = MetaRestClient::new(base, "stub-key".to_string()).with_observer(
            recorder.clone(),
            "draft",
            Some(7),
        );
        client.draft_records(&sample_request()).expect("retry wins");
        for _ in 0..2 {
            seen.recv_timeout(Duration::from_secs(10)).unwrap();
        }
        handle.join().unwrap();
        let rows = recorder.0.lock().unwrap().clone();
        assert_eq!(rows.len(), 2, "one record per HTTP attempt");
        assert_eq!(
            (rows[0].outcome, rows[0].http_status, rows[0].attempt),
            ("http_4xx", Some(429), 1)
        );
        assert_eq!(
            (rows[1].outcome, rows[1].http_status, rows[1].attempt),
            ("ok", Some(200), 2)
        );
        assert!(
            rows.iter()
                .all(|r| r.model == DRAFT_MODEL && r.run_id == Some(7))
        );
        assert!(rows[1].ttft_ms.is_some());
    }

    #[test]
    fn chat_json_sends_strict_schema_and_parses_the_reply() {
        let (base, seen, handle) = stub(1, |_, _| {
            (
                200,
                sse_stream(
                    serde_json::json!({"role": "assistant", "content": r#"{"verdict": "supported"}"#}),
                    "stop",
                ),
            )
        });
        let recorder = std::sync::Arc::new(Recorder::default());
        let client = MetaRestClient::new(base, "stub-key".to_string()).with_observer(
            recorder.clone(),
            "draft",
            None,
        );
        let resp = client
            .chat_json(&ChatJsonRequest {
                model: "muse-wonder-1.0",
                developer: "Check the claim against the text.",
                user: "claim + text".to_string(),
                schema_name: "FactCheckV1",
                schema: serde_json::json!({"type": "object", "properties": {"verdict": {"type": "string"}},
                    "required": ["verdict"], "additionalProperties": false}),
                reasoning_effort: "low",
                prompt_cache_key: "factcheck-v1".to_string(),
                max_completion_tokens: 2048,
                pipeline: "factcheck",
                run_id: Some(3),
            })
            .expect("parsed");
        assert_eq!(resp.value["verdict"], "supported");
        assert_eq!(resp.attempts, 1);
        let sent = seen.recv_timeout(Duration::from_secs(10)).unwrap();
        handle.join().unwrap();
        let json_start = sent.find("\n{").expect("body after head") + 1;
        let v: serde_json::Value = serde_json::from_str(&sent[json_start..]).unwrap();
        assert_eq!(v["reasoning_effort"], "low");
        assert_eq!(v["prompt_cache_key"], "factcheck-v1");
        assert_eq!(v["response_format"]["json_schema"]["strict"], true);
        assert_eq!(v["messages"][0]["role"], "developer");
        assert!(v.get("temperature").is_none(), "sampling untouched");
        let rows = recorder.0.lock().unwrap().clone();
        assert_eq!(rows.len(), 1);
        assert_eq!(
            (rows[0].model.as_str(), rows[0].pipeline, rows[0].outcome),
            ("muse-wonder-1.0", "factcheck", "ok")
        );
    }

    #[test]
    fn chat_json_refusal_is_bad_output_and_recorded_as_refusal() {
        let (base, _seen, handle) = stub(1, |_, _| {
            (
                200,
                sse_stream(
                    serde_json::json!({"role": "assistant", "refusal": "no"}),
                    "stop",
                ),
            )
        });
        let recorder = std::sync::Arc::new(Recorder::default());
        let client = MetaRestClient::new(base, "stub-key".to_string()).with_observer(
            recorder.clone(),
            "plan",
            None,
        );
        let err = client
            .chat_json(&ChatJsonRequest {
                model: "muse-spark-1.3",
                developer: "Compose.",
                user: "facts".to_string(),
                schema_name: "RenderPlanV1",
                schema: serde_json::json!({"type": "object"}),
                reasoning_effort: "high",
                prompt_cache_key: "plan-v1".to_string(),
                max_completion_tokens: 8192,
                pipeline: "plan",
                run_id: None,
            })
            .expect_err("refusal");
        handle.join().unwrap();
        assert!(matches!(err, ModelError::BadOutput(_)));
        assert_eq!(recorder.0.lock().unwrap()[0].outcome, "refusal");
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
                "usage": {"prompt_tokens": 25, "completion_tokens": 19, "total_tokens": 44,
                    "prompt_tokens_details": {"cached_tokens": 5},
                    "completion_tokens_details": {"reasoning_tokens": 7}},
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

    #[test]
    fn retry_delay_backs_off_with_jitter_and_retry_after() {
        for (attempt, low, high) in [(1, 2, 3), (2, 4, 6), (3, 8, 12)] {
            let got = retry_delay(2, attempt, None);
            assert!(
                got >= Duration::from_secs(low) && got <= Duration::from_secs(high),
                "attempt {attempt}: {got:?}"
            );
        }
        assert_eq!(retry_delay(2, 1, Some(30)), Duration::from_secs(30));
        assert_eq!(retry_delay(2, 1, Some(3600)), Duration::from_secs(60));
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
