//! Meta Model API client: fast chat, background Responses, files.
//!
//! Wire shapes follow `docs/meta-api-shapes.md` (chat completions)
//! plus the pins from 2026-10-29 (Responses `background`,
//! `text.format` json_schema, Files API, `input_tokens`). Anything
//! marked `re-verify` below is conventional, not doc-pinned —
//! confirm at first live use.
//!
//! [`ModelPort`] is the trait the task runner programs to;
//! [`StubPort`] keeps tests and `--dry-run` offline.

use std::collections::HashMap;
use std::fmt;
use std::time::Duration;

use serde_json::{Value, json};

use crate::error::{Error, Result};

/// Default API base (overridable for tests/staging).
pub const BASE_URL: &str = "https://api.meta.ai/v1";

/// Background creation bodies over 1 MiB are rejected (413).
pub const BACKGROUND_BODY_LIMIT: usize = 1_048_576;

/// Inline media over 50 MB must go through the Files API.
pub const INLINE_MEDIA_LIMIT: usize = 50_000_000;

/// Transcription model id.
pub const VOICE_MODEL: &str = "muse-voice-transcribe-1.0";

/// Raw HTTP response: status + bytes + parsed Retry-After.
#[derive(Debug, Clone)]
pub struct HttpResponse {
    pub status: u16,
    pub body: Vec<u8>,
    pub retry_after_secs: Option<u64>,
}

/// One file part of a multipart upload.
pub struct MultipartFile<'a> {
    pub field: &'a str,
    pub filename: &'a str,
    pub content_type: &'a str,
    pub bytes: &'a [u8],
}

/// Minimal HTTP surface the client needs (stubbed in tests).
pub trait Transport {
    fn post_json(&self, url: &str, bearer: &str, body: &Value) -> Result<HttpResponse>;
    fn post_multipart(
        &self,
        url: &str,
        bearer: &str,
        file: &MultipartFile,
        fields: &[(&str, &str)],
    ) -> Result<HttpResponse>;
    fn get(&self, url: &str, bearer: &str) -> Result<HttpResponse>;
    fn delete(&self, url: &str, bearer: &str) -> Result<HttpResponse>;
}

/// Live [`Transport`] over blocking ureq.
pub struct UreqTransport {
    agent: ureq::Agent,
}

impl UreqTransport {
    pub fn new(timeout: Duration) -> Self {
        install_crypto_provider();
        Self {
            agent: ureq::Agent::new_with_config(
                ureq::Agent::config_builder()
                    .http_status_as_error(false)
                    .timeout_global(Some(timeout))
                    .build(),
            ),
        }
    }
}

impl Default for UreqTransport {
    fn default() -> Self {
        Self::new(Duration::from_secs(180))
    }
}

impl Transport for UreqTransport {
    fn post_json(&self, url: &str, bearer: &str, body: &Value) -> Result<HttpResponse> {
        let response = self
            .agent
            .post(url)
            .header("Authorization", format!("Bearer {bearer}"))
            .send_json(body)
            .map_err(|e| Error::Model(format!("transport: {e}")))?;
        finish(response)
    }

    fn post_multipart(
        &self,
        url: &str,
        bearer: &str,
        file: &MultipartFile,
        fields: &[(&str, &str)],
    ) -> Result<HttpResponse> {
        let mut form = ureq::unversioned::multipart::Form::new();
        for (name, value) in fields {
            form = form.text(name, value);
        }
        let part = ureq::unversioned::multipart::Part::bytes(file.bytes)
            .file_name(file.filename)
            .mime_str(file.content_type)
            .map_err(|e| Error::Model(format!("transport: {e}")))?;
        form = form.part(file.field, part);
        let response = self
            .agent
            .post(url)
            .header("Authorization", format!("Bearer {bearer}"))
            .send(form)
            .map_err(|e| Error::Model(format!("transport: {e}")))?;
        finish(response)
    }

    fn get(&self, url: &str, bearer: &str) -> Result<HttpResponse> {
        let response = self
            .agent
            .get(url)
            .header("Authorization", format!("Bearer {bearer}"))
            .call()
            .map_err(|e| Error::Model(format!("transport: {e}")))?;
        finish(response)
    }

    fn delete(&self, url: &str, bearer: &str) -> Result<HttpResponse> {
        let response = self
            .agent
            .delete(url)
            .header("Authorization", format!("Bearer {bearer}"))
            .call()
            .map_err(|e| Error::Model(format!("transport: {e}")))?;
        finish(response)
    }
}

fn finish(response: ureq::http::Response<ureq::Body>) -> Result<HttpResponse> {
    let status = response.status().as_u16();
    let retry_after_secs = response
        .headers()
        .get("Retry-After")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse().ok());
    let body = response
        .into_body()
        .read_to_vec()
        .map_err(|e| Error::Model(format!("transport: {e}")))?;
    Ok(HttpResponse {
        status,
        body,
        retry_after_secs,
    })
}

fn install_crypto_provider() {
    let provider = rustls::crypto::aws_lc_rs::default_provider();
    let _ = rustls::crypto::CryptoProvider::install_default(provider);
}

/// A JSON Schema attachment for structured output.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct JsonSchema {
    pub name: String,
    pub schema: Value,
    pub strict: bool,
}

/// Sync chat-completions call (fast path; text-only — media rides
/// the Responses API where the content blocks are doc-pinned).
#[derive(Debug, Clone)]
pub struct ChatRequest {
    pub model: String,
    pub instructions: String,
    pub user_text: String,
    pub schema: Option<JsonSchema>,
    pub max_output_tokens: u32,
    pub temperature: f32,
    /// `minimal|low|medium|high|max` (never `none` on Spark).
    pub reasoning_effort: Option<String>,
    /// Stable use-case key (prompt caching is automatic prefix
    /// matching — stable instructions first, evidence last).
    pub cache_key: Option<String>,
}

/// One Responses input block (typed; doc-pinned names).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum InputBlock {
    Text(String),
    /// Inline base64 (`data:<mime>;base64,…`, ≤50 MB).
    FileData {
        filename: String,
        data_url: String,
    },
    /// Public https URL the server fetches itself.
    FileUrl {
        url: String,
    },
    /// Previously uploaded file.
    FileId {
        id: String,
    },
    /// Base64 data URL or https URL (re-verify field at first
    /// live use — conventional `image_url`).
    ImageUrl {
        url: String,
    },
}

/// Responses call (sync fast-with-media, or `background` batch).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ResponsesRequest {
    pub model: String,
    /// Developer-level, current request only (resent each turn).
    pub instructions: String,
    pub input: Vec<InputBlock>,
    pub schema: Option<JsonSchema>,
    pub max_output_tokens: Option<u32>,
    pub reasoning_effort: Option<String>,
    pub background: bool,
    pub previous_response_id: Option<String>,
}

/// One finished completion.
#[derive(Debug, Clone, Default)]
pub struct Completion {
    pub text: String,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub cached_tokens: u32,
}

/// Background job state after create/retrieve.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BackgroundStatus {
    Queued,
    InProgress,
    Completed,
    Failed(String),
    Cancelled,
}

/// Created (possibly background) response.
#[derive(Debug, Clone)]
pub struct CreatedResponse {
    pub id: String,
    pub status: BackgroundStatus,
    /// Present when a sync call finished inline.
    pub completion: Option<Completion>,
}

/// Retrieved background response.
#[derive(Debug, Clone)]
pub struct RetrievedResponse {
    pub status: BackgroundStatus,
    pub completion: Option<Completion>,
}

/// Voice transcript: ordered turns (no word timestamps — the API
/// does not emit them).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transcript {
    pub turns: Vec<TranscriptTurn>,
}

/// One speaker turn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranscriptTurn {
    pub index: usize,
    pub speaker: Option<String>,
    pub text: String,
    pub start_ms: u64,
    pub end_ms: u64,
}

impl Transcript {
    /// Whole transcript as plain text (one turn per line).
    pub fn to_text(&self) -> String {
        self.turns
            .iter()
            .map(|t| match &t.speaker {
                Some(speaker) => format!("{speaker}: {}", t.text),
                None => t.text.clone(),
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// The model surface the task runner programs to.
pub trait ModelPort {
    fn complete_chat(&self, req: &ChatRequest) -> Result<Completion>;
    fn create_response(&self, req: &ResponsesRequest) -> Result<CreatedResponse>;
    fn retrieve_response(&self, id: &str) -> Result<RetrievedResponse>;
    fn cancel_response(&self, id: &str) -> Result<()>;
    /// Measure input tokens before sending (never estimated blind).
    fn count_tokens(&self, model: &str, input: &Value) -> Result<u32>;
    fn upload_file(&self, filename: &str, mime: &str, bytes: &[u8]) -> Result<String>;
    fn delete_file(&self, id: &str) -> Result<()>;
    fn transcribe(&self, filename: &str, mime: &str, bytes: &[u8]) -> Result<Transcript>;
    fn list_models(&self) -> Result<Vec<String>>;
}

/// Live Meta Model API client. The key is redacted in Debug.
pub struct MetaClient<T: Transport> {
    transport: T,
    base_url: String,
    api_key: String,
}

impl<T: Transport> fmt::Debug for MetaClient<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MetaClient")
            .field("base_url", &self.base_url)
            .field("api_key", &"<redacted>")
            .finish()
    }
}

impl<T: Transport> MetaClient<T> {
    /// Build a client; an empty key fails closed at call time.
    /// (`String` key + field shorthand: the secrets-scan forbids
    /// `api_key:` assignments with value-looking right sides.)
    pub fn new(transport: T, base_url: &str, api_key: String) -> Self {
        Self {
            transport,
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key,
        }
    }

    fn post(&self, path: &str, body: &Value) -> Result<HttpResponse> {
        if self.api_key.is_empty() {
            return Err(Error::Model("auth: empty api key".to_string()));
        }
        with_retry(|| {
            self.transport
                .post_json(&format!("{}{path}", self.base_url), &self.api_key, body)
        })
    }

    /// Classify an HTTP status into retry/terminal per the shapes doc.
    fn check(status: u16, body: &[u8], retry_after: Option<u64>) -> Result<Value> {
        if status == 404 && body.is_empty() {
            return Err(Error::Model("unknown api path (empty 404)".to_string()));
        }
        let json: Value = serde_json::from_slice(body).unwrap_or(Value::Null);
        let detail = json
            .get("error")
            .and_then(|e| e.get("message"))
            .and_then(Value::as_str)
            .unwrap_or("");
        match status {
            200..=299 => Ok(json),
            401 => Err(Error::Model(format!("auth: {detail}"))),
            402 => Err(Error::Model(format!("billing: {detail}"))),
            413 => Err(Error::Model("payload too large".to_string())),
            429 => Err(Error::Model(format!(
                "rate limited{}",
                retry_after.map_or(String::new(), |s| format!(" (retry after {s}s)"))
            ))),
            400 | 403 | 404 => Err(Error::Model(format!(
                "request rejected ({status}): {detail}"
            ))),
            _ => Err(Error::Model(format!("server error ({status}): {detail}"))),
        }
    }
}

impl<T: Transport> ModelPort for MetaClient<T> {
    fn complete_chat(&self, req: &ChatRequest) -> Result<Completion> {
        let mut body = json!({
            "model": req.model,
            "messages": [
                {"role": "developer", "content": req.instructions},
                {"role": "user", "content": req.user_text},
            ],
            "temperature": req.temperature,
            "max_completion_tokens": req.max_output_tokens,
            "stream": false,
        });
        if let Some(effort) = req.reasoning_effort.as_deref() {
            body["reasoning_effort"] = json!(effort);
        }
        if let Some(cache_key) = req.cache_key.as_deref() {
            body["prompt_cache_key"] = json!(cache_key);
        }
        if let Some(schema) = req.schema.as_ref() {
            body["response_format"] = json!({
                "type": "json_schema",
                "json_schema": {"name": schema.name, "schema": schema.schema, "strict": schema.strict},
            });
        }
        let res = self.post("/chat/completions", &body)?;
        let json = Self::check(res.status, &res.body, res.retry_after_secs)?;
        parse_chat_completion(&json)
    }

    fn create_response(&self, req: &ResponsesRequest) -> Result<CreatedResponse> {
        let input = req
            .input
            .iter()
            .map(|block| match block {
                InputBlock::Text(text) => json!({"type": "input_text", "text": text}),
                InputBlock::FileData { filename, data_url } => json!({
                    "type": "input_file", "filename": filename, "file_data": data_url,
                }),
                InputBlock::FileUrl { url } => json!({"type": "input_file", "file_url": url}),
                InputBlock::FileId { id } => json!({"type": "input_file", "file_id": id}),
                InputBlock::ImageUrl { url } => json!({"type": "input_image", "image_url": url}),
            })
            .collect::<Vec<_>>();
        let mut body = json!({
            "model": req.model,
            "instructions": req.instructions,
            "input": [{"role": "user", "content": input}],
        });
        if let Some(effort) = req.reasoning_effort.as_deref() {
            body["reasoning"] = json!({"effort": effort});
        }
        if let Some(max) = req.max_output_tokens {
            body["max_output_tokens"] = json!(max);
        }
        if let Some(schema) = req.schema.as_ref() {
            body["text"] = json!({
                "format": {"type": "json_schema",
                           "json_schema": {"name": schema.name, "schema": schema.schema,
                                           "strict": schema.strict}},
            });
        }
        if req.background {
            body["background"] = json!(true);
            let bytes = serde_json::to_vec(&body).expect("request serializes");
            if bytes.len() > BACKGROUND_BODY_LIMIT {
                return Err(Error::Model(format!(
                    "background creation body exceeds {BACKGROUND_BODY_LIMIT} bytes — upload media via the Files API first"
                )));
            }
        }
        if let Some(prev) = req.previous_response_id.as_deref() {
            body["previous_response_id"] = json!(prev);
        }
        let res = self.post("/responses", &body)?;
        let json = Self::check(res.status, &res.body, res.retry_after_secs)?;
        parse_created_response(&json)
    }

    fn retrieve_response(&self, id: &str) -> Result<RetrievedResponse> {
        if self.api_key.is_empty() {
            return Err(Error::Model("auth: empty api key".to_string()));
        }
        let res = with_retry(|| {
            self.transport
                .get(&format!("{}/responses/{id}", self.base_url), &self.api_key)
        })?;
        let json = Self::check(res.status, &res.body, res.retry_after_secs)?;
        parse_retrieved_response(&json)
    }

    fn cancel_response(&self, id: &str) -> Result<()> {
        let body = json!({});
        let res = self.post(&format!("/responses/{id}/cancel"), &body)?;
        Self::check(res.status, &res.body, res.retry_after_secs)?;
        Ok(())
    }

    fn count_tokens(&self, model: &str, input: &Value) -> Result<u32> {
        let res = self.post(
            "/responses/input_tokens",
            &json!({"model": model, "input": input}),
        )?;
        let json = Self::check(res.status, &res.body, res.retry_after_secs)?;
        json.get("input_tokens")
            .or_else(|| json.get("total_tokens"))
            .and_then(Value::as_u64)
            .and_then(|n| u32::try_from(n).ok())
            .ok_or_else(|| Error::Model("token count missing from response".to_string()))
    }

    fn upload_file(&self, filename: &str, mime: &str, bytes: &[u8]) -> Result<String> {
        if self.api_key.is_empty() {
            return Err(Error::Model("auth: empty api key".to_string()));
        }
        let file = MultipartFile {
            field: "file",
            filename,
            content_type: mime,
            bytes,
        };
        let res = with_retry(|| {
            self.transport.post_multipart(
                &format!("{}/files", self.base_url),
                &self.api_key,
                &file,
                &[("purpose", "assistants")],
            )
        })?;
        let json = Self::check(res.status, &res.body, res.retry_after_secs)?;
        json.get("id")
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| Error::Model("file id missing from upload response".to_string()))
    }

    fn delete_file(&self, id: &str) -> Result<()> {
        if self.api_key.is_empty() {
            return Err(Error::Model("auth: empty api key".to_string()));
        }
        let res = with_retry(|| {
            self.transport
                .delete(&format!("{}/files/{id}", self.base_url), &self.api_key)
        })?;
        Self::check(res.status, &res.body, res.retry_after_secs)?;
        Ok(())
    }

    fn transcribe(&self, filename: &str, mime: &str, bytes: &[u8]) -> Result<Transcript> {
        if self.api_key.is_empty() {
            return Err(Error::Model("auth: empty api key".to_string()));
        }
        let file = MultipartFile {
            field: "file",
            filename,
            content_type: mime,
            bytes,
        };
        let res = with_retry(|| {
            self.transport.post_multipart(
                &format!("{}/asr/transcribe", self.base_url),
                &self.api_key,
                &file,
                &[("model", VOICE_MODEL)],
            )
        })?;
        let json = Self::check(res.status, &res.body, res.retry_after_secs)?;
        parse_transcript(&json)
    }

    fn list_models(&self) -> Result<Vec<String>> {
        if self.api_key.is_empty() {
            return Err(Error::Model("auth: empty api key".to_string()));
        }
        let res = with_retry(|| {
            self.transport
                .get(&format!("{}/models", self.base_url), &self.api_key)
        })?;
        let json = Self::check(res.status, &res.body, res.retry_after_secs)?;
        Ok(json
            .get("data")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item.get("id").and_then(Value::as_str).map(str::to_string))
                    .collect()
            })
            .unwrap_or_default())
    }
}

/// Retry 429/5xx with exponential backoff + jitter (4 tries max);
/// terminal statuses return immediately.
fn with_retry(op: impl Fn() -> Result<HttpResponse>) -> Result<HttpResponse> {
    let mut wait_ms = 500u64;
    for attempt in 0..4 {
        let res = op()?;
        if !matches!(res.status, 429 | 500 | 502 | 503 | 504) {
            return Ok(res);
        }
        if attempt == 3 {
            return Ok(res); // caller reports the terminal status
        }
        if res.status == 429
            && let Some(after) = res.retry_after_secs
        {
            std::thread::sleep(Duration::from_secs(after.min(60)));
            continue;
        }
        // Jitter without a rng dependency: mix in the nanos clock.
        let jitter = (std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos() % 500)
            .unwrap_or(0) as u64)
            .min(wait_ms / 2);
        std::thread::sleep(Duration::from_millis(wait_ms + jitter));
        wait_ms *= 2;
    }
    unreachable!("loop returns by attempt 3")
}

/// Parse a chat completion (pinned shape): content string + usage.
/// Refusals and `length` truncation are errors, never partial data.
fn parse_chat_completion(json: &Value) -> Result<Completion> {
    let choice = json
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
        .ok_or_else(|| Error::Model("chat completion has no choices".to_string()))?;
    if choice
        .get("finish_reason")
        .and_then(Value::as_str)
        .is_some_and(|reason| reason == "length")
    {
        return Err(Error::Model(
            "chat completion truncated (finish_reason=length)".to_string(),
        ));
    }
    let message = choice
        .get("message")
        .ok_or_else(|| Error::Model("chat completion has no message".to_string()))?;
    if message.get("refusal").and_then(Value::as_str).is_some() {
        return Err(Error::Model("model refused the request".to_string()));
    }
    let text = message
        .get("content")
        .and_then(Value::as_str)
        .ok_or_else(|| Error::Model("chat completion has no content".to_string()))?;
    Ok(Completion {
        text: text.to_string(),
        input_tokens: usage(json, &["prompt_tokens"]),
        output_tokens: usage(json, &["completion_tokens"]),
        cached_tokens: json
            .get("usage")
            .and_then(|u| u.get("prompt_tokens_details"))
            .and_then(|d| d.get("cached_tokens"))
            .and_then(Value::as_u64)
            .and_then(|n| u32::try_from(n).ok())
            .unwrap_or(0),
    })
}

fn usage(json: &Value, keys: &[&str]) -> u32 {
    keys.iter()
        .filter_map(|key| {
            json.get("usage")
                .and_then(|u| u.get(key))
                .and_then(Value::as_u64)
        })
        .next()
        .and_then(|n| u32::try_from(n).ok())
        .unwrap_or(0)
}

fn parse_status(json: &Value) -> BackgroundStatus {
    match json.get("status").and_then(Value::as_str) {
        Some("queued") => BackgroundStatus::Queued,
        Some("in_progress") => BackgroundStatus::InProgress,
        Some("completed") => BackgroundStatus::Completed,
        Some("cancelled") => BackgroundStatus::Cancelled,
        Some("failed") => BackgroundStatus::Failed(
            json.get("error")
                .and_then(|e| e.get("message"))
                .and_then(Value::as_str)
                .unwrap_or("background response failed")
                .to_string(),
        ),
        other => BackgroundStatus::Failed(format!("unknown status: {other:?}")),
    }
}

/// Collect `output_text` from a Responses `output` array
/// (defensive walk — re-verify the item shape at first live use).
fn collect_output_text(json: &Value) -> String {
    let mut texts = Vec::new();
    fn walk(value: &Value, texts: &mut Vec<String>) {
        match value {
            Value::Object(map) => {
                if map.get("type").and_then(Value::as_str) == Some("output_text")
                    && let Some(text) = map.get("text").and_then(Value::as_str)
                {
                    texts.push(text.to_string());
                }
                for value in map.values() {
                    walk(value, texts);
                }
            }
            Value::Array(items) => {
                for item in items {
                    walk(item, texts);
                }
            }
            _ => {}
        }
    }
    if let Some(output) = json.get("output") {
        walk(output, &mut texts);
    }
    texts.join("\n")
}

fn completion_from_responses(json: &Value) -> Option<Completion> {
    let text = collect_output_text(json);
    if text.is_empty() {
        return None;
    }
    Some(Completion {
        text,
        input_tokens: usage(json, &["input_tokens", "prompt_tokens"]),
        output_tokens: usage(json, &["output_tokens", "completion_tokens"]),
        cached_tokens: 0,
    })
}

fn parse_created_response(json: &Value) -> Result<CreatedResponse> {
    let id = json
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| Error::Model("response has no id".to_string()))?;
    let status = parse_status(json);
    if let BackgroundStatus::Failed(reason) = &status {
        return Err(Error::Model(format!("response failed: {reason}")));
    }
    let completion = if status == BackgroundStatus::Completed {
        completion_from_responses(json)
    } else {
        None
    };
    Ok(CreatedResponse {
        id: id.to_string(),
        status,
        completion,
    })
}

fn parse_retrieved_response(json: &Value) -> Result<RetrievedResponse> {
    let status = parse_status(json);
    let completion = if status == BackgroundStatus::Completed {
        completion_from_responses(json)
    } else {
        None
    };
    Ok(RetrievedResponse { status, completion })
}

/// Parse a turn-level transcript (`turns`, or `segments` alias).
fn parse_transcript(json: &Value) -> Result<Transcript> {
    let items = json
        .get("turns")
        .or_else(|| json.get("segments"))
        .and_then(Value::as_array)
        .ok_or_else(|| Error::Model("transcript has no turns".to_string()))?;
    let mut turns = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        let text = item
            .get("text")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::Model("transcript turn without text".to_string()))?;
        turns.push(TranscriptTurn {
            index,
            speaker: item
                .get("speaker")
                .and_then(Value::as_str)
                .map(str::to_string),
            text: text.to_string(),
            start_ms: millis(item, "start"),
            end_ms: millis(item, "end"),
        });
    }
    Ok(Transcript { turns })
}

fn millis(item: &Value, side: &str) -> u64 {
    for key in [format!("{side}Ms"), format!("{side}_ms")] {
        if let Some(ms) = item.get(&key).and_then(Value::as_u64) {
            return ms;
        }
    }
    // Seconds (float) fallback.
    item.get(side)
        .and_then(Value::as_f64)
        .map(|s| (s * 1000.0) as u64)
        .unwrap_or(0)
}

/// Canned transport for tests: fixed responses per `METHOD path`.
#[derive(Debug, Default)]
pub struct StubTransport {
    routes: HashMap<String, HttpResponse>,
}

impl StubTransport {
    pub fn route(mut self, method: &str, path: &str, status: u16, body: &str) -> Self {
        self.routes.insert(
            format!("{method} {path}"),
            HttpResponse {
                status,
                body: body.as_bytes().to_vec(),
                retry_after_secs: None,
            },
        );
        self
    }

    fn lookup(&self, method: &str, url: &str) -> Result<HttpResponse> {
        let path = url.split_once("/v1/").map_or(url, |(_, rest)| rest);
        self.routes
            .get(&format!("{method} /{path}"))
            .or_else(|| self.routes.get(&format!("{method} {path}")))
            .cloned()
            .ok_or_else(|| Error::Model(format!("stub has no route for {method} {url}")))
    }
}

impl Transport for StubTransport {
    fn post_json(&self, url: &str, _bearer: &str, _body: &Value) -> Result<HttpResponse> {
        self.lookup("POST", url)
    }

    fn post_multipart(
        &self,
        url: &str,
        _bearer: &str,
        _file: &MultipartFile,
        _fields: &[(&str, &str)],
    ) -> Result<HttpResponse> {
        self.lookup("POST", url)
    }

    fn get(&self, url: &str, _bearer: &str) -> Result<HttpResponse> {
        self.lookup("GET", url)
    }

    fn delete(&self, url: &str, _bearer: &str) -> Result<HttpResponse> {
        self.lookup("DELETE", url)
    }
}

/// Canned model port for task/batch tests.
#[derive(Debug, Default)]
pub struct StubPort {
    pub chat_text: String,
    pub response_text: String,
    script: std::cell::RefCell<std::collections::VecDeque<String>>,
}

impl StubPort {
    pub fn new(chat_text: &str, response_text: &str) -> Self {
        Self {
            chat_text: chat_text.to_string(),
            response_text: response_text.to_string(),
            script: std::cell::RefCell::new(std::collections::VecDeque::new()),
        }
    }

    /// Scripted replies: each `complete_chat` pops the front;
    /// exhausted scripts fall back to `chat_text`. Multi-step
    /// workers (draft → refine) script one entry per call.
    pub fn with_script(responses: Vec<String>) -> Self {
        Self {
            chat_text: "{}".to_string(),
            response_text: "{}".to_string(),
            script: std::cell::RefCell::new(responses.into()),
        }
    }
}

impl ModelPort for StubPort {
    fn complete_chat(&self, _req: &ChatRequest) -> Result<Completion> {
        let text = self
            .script
            .borrow_mut()
            .pop_front()
            .unwrap_or_else(|| self.chat_text.clone());
        Ok(Completion {
            text,
            input_tokens: 10,
            output_tokens: 5,
            cached_tokens: 0,
        })
    }

    fn create_response(&self, req: &ResponsesRequest) -> Result<CreatedResponse> {
        if req.background {
            Ok(CreatedResponse {
                id: "resp_stub_1".to_string(),
                status: BackgroundStatus::Queued,
                completion: None,
            })
        } else {
            Ok(CreatedResponse {
                id: "resp_stub_0".to_string(),
                status: BackgroundStatus::Completed,
                completion: Some(Completion {
                    text: self.response_text.clone(),
                    input_tokens: 10,
                    output_tokens: 5,
                    cached_tokens: 0,
                }),
            })
        }
    }

    fn retrieve_response(&self, _id: &str) -> Result<RetrievedResponse> {
        Ok(RetrievedResponse {
            status: BackgroundStatus::Completed,
            completion: Some(Completion {
                text: self.response_text.clone(),
                input_tokens: 10,
                output_tokens: 5,
                cached_tokens: 0,
            }),
        })
    }

    fn cancel_response(&self, _id: &str) -> Result<()> {
        Ok(())
    }

    fn count_tokens(&self, _model: &str, _input: &Value) -> Result<u32> {
        Ok(100)
    }

    fn upload_file(&self, _filename: &str, _mime: &str, _bytes: &[u8]) -> Result<String> {
        Ok("file_stub_1".to_string())
    }

    fn delete_file(&self, _id: &str) -> Result<()> {
        Ok(())
    }

    fn transcribe(&self, _filename: &str, _mime: &str, _bytes: &[u8]) -> Result<Transcript> {
        Ok(Transcript { turns: Vec::new() })
    }

    fn list_models(&self) -> Result<Vec<String>> {
        Ok(vec!["muse-spark-1.3".to_string()])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chat_body() -> Value {
        json!({
            "id": "chatcmpl-1", "object": "chat.completion", "created": 1,
            "model": "muse-spark-1.3",
            "choices": [{"index": 0,
                "message": {"role": "assistant", "content": "{\"rows\":[]}",
                            "refusal": null, "tool_calls": []},
                "finish_reason": "stop"}],
            "usage": {"prompt_tokens": 25, "completion_tokens": 19, "total_tokens": 44,
                      "prompt_tokens_details": {"cached_tokens": 5}},
        })
    }

    fn chat_req() -> ChatRequest {
        ChatRequest {
            model: "muse-spark-1.3".to_string(),
            instructions: "i".to_string(),
            user_text: "u".to_string(),
            schema: None,
            max_output_tokens: 100,
            temperature: 0.1,
            reasoning_effort: Some("low".to_string()),
            cache_key: Some("k".to_string()),
        }
    }

    #[test]
    fn chat_completion_parses_usage() {
        let stub = StubTransport::default().route(
            "POST",
            "/chat/completions",
            200,
            &chat_body().to_string(),
        );
        let client = MetaClient::new(stub, BASE_URL, "test-key".to_string());
        let out = client.complete_chat(&chat_req()).unwrap();
        assert_eq!(out.text, "{\"rows\":[]}");
        assert_eq!(out.input_tokens, 25);
        assert_eq!(out.output_tokens, 19);
        assert_eq!(out.cached_tokens, 5);
    }

    #[test]
    fn chat_truncation_and_refusal_are_errors() {
        let mut body = chat_body();
        body["choices"][0]["finish_reason"] = json!("length");
        let stub =
            StubTransport::default().route("POST", "/chat/completions", 200, &body.to_string());
        let client = MetaClient::new(stub, BASE_URL, "k".to_string());
        assert!(
            format!("{}", client.complete_chat(&chat_req()).unwrap_err()).contains("truncated")
        );
        let mut body = chat_body();
        body["choices"][0]["message"]["refusal"] = json!("no");
        let stub =
            StubTransport::default().route("POST", "/chat/completions", 200, &body.to_string());
        let client = MetaClient::new(stub, BASE_URL, "k".to_string());
        assert!(format!("{}", client.complete_chat(&chat_req()).unwrap_err()).contains("refused"));
    }

    #[test]
    fn error_statuses_classify() {
        for (status, body, needle) in [
            (401, r#"{"error":{"message":"bad key"}}"#, "auth"),
            (402, r#"{"error":{"message":"unpaid"}}"#, "billing"),
            (413, r#"{}"#, "too large"),
            (429, r#"{"error":{"message":"slow"}}"#, "rate limited"),
            (400, r#"{"error":{"message":"top_p"}}"#, "rejected"),
        ] {
            let stub = StubTransport::default().route("POST", "/chat/completions", status, body);
            let client = MetaClient::new(stub, BASE_URL, "k".to_string());
            let err = format!("{}", client.complete_chat(&chat_req()).unwrap_err());
            assert!(err.contains(needle), "{status}: {err}");
        }
        // Empty-body 404 = unknown path.
        let stub = StubTransport::default().route("POST", "/chat/completions", 404, "");
        let client = MetaClient::new(stub, BASE_URL, "k".to_string());
        assert!(
            format!("{}", client.complete_chat(&chat_req()).unwrap_err())
                .contains("unknown api path")
        );
    }

    #[test]
    fn background_create_and_retrieve() {
        let stub = StubTransport::default()
            .route(
                "POST",
                "/responses",
                200,
                r#"{"id":"resp_1","status":"queued","background":true}"#,
            )
            .route(
                "GET",
                "/responses/resp_1",
                200,
                r#"{"id":"resp_1","status":"completed",
                    "output":[{"type":"message","content":[{"type":"output_text","text":"done"}]}],
                    "usage":{"input_tokens":3,"output_tokens":4}}"#,
            );
        let client = MetaClient::new(stub, BASE_URL, "k".to_string());
        let created = client
            .create_response(&ResponsesRequest {
                model: "m".to_string(),
                instructions: "i".to_string(),
                input: vec![InputBlock::Text("hi".to_string())],
                schema: None,
                max_output_tokens: None,
                reasoning_effort: None,
                background: true,
                previous_response_id: None,
            })
            .unwrap();
        assert_eq!(created.id, "resp_1");
        assert_eq!(created.status, BackgroundStatus::Queued);
        let got = client.retrieve_response("resp_1").unwrap();
        assert_eq!(got.status, BackgroundStatus::Completed);
        assert_eq!(got.completion.unwrap().text, "done");
    }

    #[test]
    fn oversized_background_body_refuses_locally() {
        let client = MetaClient::new(StubTransport::default(), BASE_URL, "k".to_string());
        let big = "x".repeat(BACKGROUND_BODY_LIMIT + 1);
        let err = client
            .create_response(&ResponsesRequest {
                model: "m".to_string(),
                instructions: "i".to_string(),
                input: vec![InputBlock::Text(big)],
                schema: None,
                max_output_tokens: None,
                reasoning_effort: None,
                background: true,
                previous_response_id: None,
            })
            .unwrap_err();
        assert!(format!("{err}").contains("Files API"), "{err}");
    }

    #[test]
    fn transcript_parses_turns() {
        let stub = StubTransport::default().route(
            "POST",
            "/asr/transcribe",
            200,
            r#"{"turns":[{"text":"hello","speaker":"A","startMs":0,"endMs":900},
                          {"text":"hi","start":1.0,"end":2.0}]}"#,
        );
        let client = MetaClient::new(stub, BASE_URL, "k".to_string());
        let out = client.transcribe("v.mp3", "audio/mpeg", b"bytes").unwrap();
        assert_eq!(out.turns.len(), 2);
        assert_eq!(out.turns[0].speaker.as_deref(), Some("A"));
        assert_eq!(out.turns[1].start_ms, 1000);
        assert!(out.to_text().contains("A: hello"));
    }

    #[test]
    fn key_is_redacted_in_debug() {
        let client = MetaClient::new(
            StubTransport::default(),
            BASE_URL,
            "test-secret-9".to_string(),
        );
        let debug = format!("{client:?}");
        assert!(debug.contains("<redacted>"));
        assert!(!debug.contains("secret"));
    }
}
