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

/// Transcribe request-body cap: bodies over 32 MB are rejected (413).
pub const TRANSCRIBE_BODY_LIMIT: usize = 32 * 1024 * 1024;

/// Transcribe audio-duration cap: clips over 10 minutes are rejected (400).
pub const TRANSCRIBE_MAX_DURATION_MS: u64 = 10 * 60 * 1000;

/// Transcribe settings for one file upload.
///
/// `mode` is one of `PUSH_TO_TALK` (single turn; `turns` comes back
/// empty), `ENDPOINTING` (model-detected turn boundaries) or
/// `DIARIZATION` (plus speaker labels). `language_bias` lists language
/// names (`["English", "French"]`), never free-form context; `keywords`
/// biases recognition toward product names and domain terms. Both ride
/// inside the `request` JSON part.
#[derive(Debug, Clone)]
pub struct TranscribeOptions<'a> {
    pub mode: &'a str,
    pub language_bias: &'a [&'a str],
    pub keywords: &'a [&'a str],
}

impl Default for TranscribeOptions<'static> {
    fn default() -> Self {
        Self {
            mode: "ENDPOINTING",
            language_bias: &[],
            keywords: &[],
        }
    }
}

/// Build the `request` JSON part of the Transcribe multipart body.
///
/// Pinned to the speech-to-text docs: `model`, `audioEncoding: "WAV"`,
/// `mode`, plus `languageBias`/`keywords` only when non-empty.
pub fn transcribe_request_json(options: &TranscribeOptions) -> Value {
    let mut request = json!({
        "model": VOICE_MODEL,
        "audioEncoding": "WAV",
        "mode": options.mode,
    });
    if !options.language_bias.is_empty() {
        request["languageBias"] = Value::from(
            options
                .language_bias
                .iter()
                .map(|s| s.to_string())
                .collect::<Vec<_>>(),
        );
    }
    if !options.keywords.is_empty() {
        request["keywords"] = Value::from(
            options
                .keywords
                .iter()
                .map(|s| s.to_string())
                .collect::<Vec<_>>(),
        );
    }
    request
}

/// Build the `text` parameter of a Responses structured-output call.
///
/// Flat per the structured-output docs:
/// `text.format = {type: "json_schema", name, schema, strict}`. The
/// nested `json_schema` wrapper belongs to chat `response_format`.
pub fn responses_text_format(schema: &JsonSchema) -> Value {
    json!({
        "format": {
            "type": "json_schema",
            "name": schema.name,
            "schema": schema.schema,
            "strict": schema.strict,
        },
    })
}

/// Check the clip is a supported WAV file before uploading.
///
/// The endpoint accepts a RIFF/WAVE container holding mono 16-bit
/// integer PCM at 16 or 24 kHz. Returns the audio duration in ms.
/// Anything else is a 400 on the wire (`ffmpeg -ac 1 -ar 24000
/// -c:a pcm_s16le` converts); bodies over 32 MB are a 413.
pub fn validate_wav_audio(audio: &[u8]) -> Result<u64> {
    if audio.len() > TRANSCRIBE_BODY_LIMIT {
        return Err(Error::Model(format!(
            "audio is {} bytes, over the 32 MB body cap — split into shorter clips",
            audio.len()
        )));
    }
    let wav = WavInfo::parse(audio)?;
    let duration_ms = wav.duration_ms();
    if duration_ms > TRANSCRIBE_MAX_DURATION_MS {
        return Err(Error::Model(format!(
            "audio is {duration_ms} ms, over the 10-minute cap — split into shorter clips"
        )));
    }
    Ok(duration_ms)
}

struct WavInfo {
    sample_rate: u64,
    data_len: u64,
}

impl WavInfo {
    fn parse(audio: &[u8]) -> Result<Self> {
        let invalid = |why: String| Error::Model(format!("not a supported WAV file: {why}"));
        if audio.len() < 12 || &audio[0..4] != b"RIFF" || &audio[8..12] != b"WAVE" {
            return Err(invalid(
                "not a RIFF/WAVE file — convert to mono 16-bit WAV at 16 or 24 kHz".to_string(),
            ));
        }
        let mut offset = 12;
        let mut fmt: Option<(u16, u16, u32, u16)> = None;
        let mut data_len: Option<u64> = None;
        while offset + 8 <= audio.len() {
            let id = &audio[offset..offset + 4];
            let size =
                u32::from_le_bytes(audio[offset + 4..offset + 8].try_into().expect("u32")) as usize;
            let body = offset + 8;
            if id == b"fmt " {
                if audio.len() < body + 16 {
                    return Err(invalid("WAV fmt chunk is truncated".to_string()));
                }
                let tag = u16::from_le_bytes(audio[body..body + 2].try_into().expect("u16"));
                let channels =
                    u16::from_le_bytes(audio[body + 2..body + 4].try_into().expect("u16"));
                let rate = u32::from_le_bytes(audio[body + 4..body + 8].try_into().expect("u32"));
                let bits = u16::from_le_bytes(audio[body + 14..body + 16].try_into().expect("u16"));
                fmt = Some((tag, channels, rate, bits));
            } else if id == b"data" {
                // Declared size: the duration guard judges what the
                // header claims. A truncated file still fails on the
                // wire (400); the server re-validates regardless.
                data_len = Some(size as u64);
            }
            offset = body + size + (size % 2);
        }
        let Some((tag, channels, rate, bits)) = fmt else {
            return Err(invalid("WAV has no fmt chunk".to_string()));
        };
        if tag != 1 || channels != 1 || bits != 16 || (rate != 16000 && rate != 24000) {
            return Err(invalid(format!(
                "must be mono 16-bit PCM at 16 or 24 kHz (got format={tag} channels={channels} bits={bits} rate={rate})"
            )));
        }
        let Some(data_len) = data_len else {
            return Err(invalid("WAV has no data chunk".to_string()));
        };
        Ok(Self {
            sample_rate: u64::from(rate),
            data_len,
        })
    }

    fn duration_ms(&self) -> u64 {
        self.data_len.saturating_mul(1000) / (self.sample_rate.saturating_mul(2).max(1))
    }
}

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

/// One text part of a multipart upload. A part whose `content_type`
/// is `Some` (e.g. the Transcribe `request` JSON part, sent as
/// `application/json` per the speech-to-text docs) is sent with that
/// MIME type; `None` sends a plain text part.
pub struct MultipartField<'a> {
    pub name: &'a str,
    pub value: &'a str,
    pub content_type: Option<&'a str>,
}

impl<'a> MultipartField<'a> {
    pub fn text(name: &'a str, value: &'a str) -> Self {
        Self {
            name,
            value,
            content_type: None,
        }
    }

    pub fn typed(name: &'a str, value: &'a str, content_type: &'a str) -> Self {
        Self {
            name,
            value,
            content_type: Some(content_type),
        }
    }
}

/// Minimal HTTP surface the client needs (stubbed in tests).
pub trait Transport {
    fn post_json(&self, url: &str, bearer: &str, body: &Value) -> Result<HttpResponse>;
    fn post_multipart(
        &self,
        url: &str,
        bearer: &str,
        file: &MultipartFile,
        fields: &[MultipartField],
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
        fields: &[MultipartField],
    ) -> Result<HttpResponse> {
        let mut form = ureq::unversioned::multipart::Form::new();
        for field in fields {
            match field.content_type {
                Some(mime) => {
                    let part = ureq::unversioned::multipart::Part::text(field.value)
                        .mime_str(mime)
                        .map_err(|e| Error::Model(format!("transport: {e}")))?;
                    form = form.part(field.name, part);
                }
                None => {
                    form = form.text(field.name, field.value);
                }
            }
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
    /// Meta API data-handling terms read: voice uploads stay
    /// fail-closed until the owner opts in (same tripwire as the
    /// media crate's `MediaPolicy::terms_read`).
    terms_read: bool,
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
            terms_read: false,
        }
    }

    /// Opt in after the Meta API data-handling terms are read.
    /// Voice transcription stays fail-closed without this.
    pub fn with_terms_read(mut self, terms_read: bool) -> Self {
        self.terms_read = terms_read;
        self
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
        // Flat `text.format` per the structured-output docs
        // (`text.format.{type,name,schema,strict}` — the nested
        // `json_schema` wrapper is the chat `response_format` shape).
        if let Some(schema) = req.schema.as_ref() {
            body["text"] = responses_text_format(schema);
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
                &[MultipartField::text("purpose", "assistants")],
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
        if !self.terms_read {
            return Err(Error::Model(
                "real media is blocked until the meta api data-handling terms are read".to_string(),
            ));
        }
        if bytes.is_empty() {
            return Err(Error::Model("audio is empty".to_string()));
        }
        validate_wav_audio(bytes)?;
        // The caller passes through whatever container the source had;
        // the wire part is always the validated WAV clip.
        let _ = mime;
        let request = transcribe_request_json(&TranscribeOptions::default());
        let request_body = serde_json::to_string(&request)
            .map_err(|e| Error::Model(format!("transcribe request: {e}")))?;
        let file = MultipartFile {
            field: "audio",
            filename,
            content_type: "audio/wav",
            bytes,
        };
        let res = with_retry(|| {
            self.transport.post_multipart(
                &format!("{}/asr/transcribe", self.base_url),
                &self.api_key,
                &file,
                &[MultipartField::typed(
                    "request",
                    &request_body,
                    "application/json",
                )],
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
        // Honour Retry-After on any retried status, not just 429.
        if let Some(after) = res.retry_after_secs {
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
        // Truncation (e.g. max_output_tokens hit) surfaces as
        // `status: "incomplete"` with `incomplete_details`, not as an
        // empty completion — never treat it as success.
        Some("incomplete") => BackgroundStatus::Failed(format!(
            "response incomplete: {}",
            json.get("incomplete_details")
                .and_then(|d| d.get("reason"))
                .and_then(Value::as_str)
                .unwrap_or("truncated"),
        )),
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

/// Find a model refusal in a Responses `output` array: content
/// items shaped `{"type": "refusal", "refusal": "..."}`.
fn refusal_text(json: &Value) -> Option<String> {
    fn walk(value: &Value) -> Option<String> {
        match value {
            Value::Object(map) => {
                if map.get("type").and_then(Value::as_str) == Some("refusal") {
                    return map
                        .get("refusal")
                        .and_then(Value::as_str)
                        .map(str::to_string);
                }
                map.values().find_map(walk)
            }
            Value::Array(items) => items.iter().find_map(walk),
            _ => None,
        }
    }
    json.get("output").and_then(walk)
}

fn completion_from_responses(json: &Value) -> Result<Option<Completion>> {
    if refusal_text(json).is_some() {
        return Err(Error::Model("model refused the request".to_string()));
    }
    let text = collect_output_text(json);
    if text.is_empty() {
        return Ok(None);
    }
    Ok(Some(Completion {
        text,
        input_tokens: usage(json, &["input_tokens", "prompt_tokens"]),
        output_tokens: usage(json, &["output_tokens", "completion_tokens"]),
        cached_tokens: 0,
    }))
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
        completion_from_responses(json)?
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
    if let BackgroundStatus::Failed(reason) = &status {
        return Err(Error::Model(format!("response failed: {reason}")));
    }
    let completion = if status == BackgroundStatus::Completed {
        completion_from_responses(json)?
    } else {
        None
    };
    Ok(RetrievedResponse { status, completion })
}

/// Parse a turn-level transcript.
///
/// Official `turns` array first (`turnId`, `startMs`, `endMs`,
/// `transcript`, `speaker`); legacy `segments`/`text` aliases for older
/// replays. `PUSH_TO_TALK` returns no turns at all, so a top-level
/// `transcript` string falls back to one turn.
fn parse_transcript(json: &Value) -> Result<Transcript> {
    if let Some(items) = json
        .get("turns")
        .or_else(|| json.get("segments"))
        .and_then(Value::as_array)
        .filter(|items| !items.is_empty())
    {
        let mut turns = Vec::with_capacity(items.len());
        for item in items.iter() {
            let text = item
                .get("transcript")
                .or_else(|| item.get("text"))
                .and_then(Value::as_str)
                .ok_or_else(|| Error::Model("transcript turn without text".to_string()))?;
            turns.push(TranscriptTurn {
                index: item
                    .get("turnId")
                    .and_then(Value::as_u64)
                    .map(|n| n as usize)
                    .unwrap_or(turns.len()),
                speaker: item
                    .get("speaker")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                text: text.to_string(),
                start_ms: millis(item, "start"),
                end_ms: millis(item, "end"),
            });
        }
        return Ok(Transcript { turns });
    }
    if let Some(text) = json.get("transcript").and_then(Value::as_str) {
        return Ok(Transcript {
            turns: vec![TranscriptTurn {
                index: 0,
                speaker: None,
                text: text.to_string(),
                start_ms: 0,
                end_ms: 0,
            }],
        });
    }
    Err(Error::Model("transcript has no turns".to_string()))
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
        _fields: &[MultipartField],
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
            temperature: 1.0,
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

    /// Minimal mono 16-bit WAV at `rate` Hz with `samples` frames.
    fn wav_bytes(rate: u32, channels: u16, bits: u16, samples: u32) -> Vec<u8> {
        let bytes_per_frame = u32::from(channels) * u32::from(bits) / 8;
        let data_len = samples * bytes_per_frame;
        let mut out = Vec::new();
        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&(36 + data_len).to_le_bytes());
        out.extend_from_slice(b"WAVEfmt ");
        out.extend_from_slice(&16u32.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&channels.to_le_bytes());
        out.extend_from_slice(&rate.to_le_bytes());
        out.extend_from_slice(&(rate * bytes_per_frame).to_le_bytes());
        out.extend_from_slice(&(channels * bits / 8).to_le_bytes());
        out.extend_from_slice(&bits.to_le_bytes());
        out.extend_from_slice(b"data");
        out.extend_from_slice(&data_len.to_le_bytes());
        out.resize(out.len() + data_len as usize, 0);
        out
    }

    /// Capturing transport: records the JSON body or the multipart
    /// parts of one call.
    struct CaptureTransport {
        status: u16,
        body: Vec<u8>,
        last_json: std::cell::RefCell<Value>,
        last_file: std::cell::RefCell<(String, String, String, Vec<u8>)>,
        last_fields: std::cell::RefCell<Vec<(String, String, Option<String>)>>,
    }

    impl CaptureTransport {
        fn canned(status: u16, body: &str) -> Self {
            Self {
                status,
                body: body.as_bytes().to_vec(),
                last_json: std::cell::RefCell::new(Value::Null),
                last_file: std::cell::RefCell::new((
                    String::new(),
                    String::new(),
                    String::new(),
                    Vec::new(),
                )),
                last_fields: std::cell::RefCell::new(Vec::new()),
            }
        }
    }

    impl Transport for CaptureTransport {
        fn post_json(&self, _url: &str, _bearer: &str, body: &Value) -> Result<HttpResponse> {
            *self.last_json.borrow_mut() = body.clone();
            Ok(HttpResponse {
                status: self.status,
                body: self.body.clone(),
                retry_after_secs: None,
            })
        }

        fn post_multipart(
            &self,
            _url: &str,
            _bearer: &str,
            file: &MultipartFile,
            fields: &[MultipartField],
        ) -> Result<HttpResponse> {
            *self.last_file.borrow_mut() = (
                file.field.to_string(),
                file.filename.to_string(),
                file.content_type.to_string(),
                file.bytes.to_vec(),
            );
            *self.last_fields.borrow_mut() = fields
                .iter()
                .map(|f| {
                    (
                        f.name.to_string(),
                        f.value.to_string(),
                        f.content_type.map(str::to_string),
                    )
                })
                .collect();
            Ok(HttpResponse {
                status: self.status,
                body: self.body.clone(),
                retry_after_secs: None,
            })
        }

        fn get(&self, _url: &str, _bearer: &str) -> Result<HttpResponse> {
            panic!("transcribe uses multipart, not get");
        }

        fn delete(&self, _url: &str, _bearer: &str) -> Result<HttpResponse> {
            panic!("transcribe uses multipart, not delete");
        }
    }

    #[test]
    fn transcript_parses_official_turns() {
        let stub = StubTransport::default().route(
            "POST",
            "/asr/transcribe",
            200,
            r#"{"sessionId":"s-1","transcript":"hello hi","audioDurationMs":2000,
                "turns":[{"turnId":1,"transcript":"hello","speaker":"A","startMs":0,"endMs":900},
                         {"turnId":2,"transcript":"hi","startMs":1000,"endMs":2000}]}"#,
        );
        let client = MetaClient::new(stub, BASE_URL, "k".to_string()).with_terms_read(true);
        let audio = wav_bytes(16000, 1, 16, 16000);
        let out = client.transcribe("v.wav", "audio/wav", &audio).unwrap();
        assert_eq!(out.turns.len(), 2);
        assert_eq!(out.turns[0].speaker.as_deref(), Some("A"));
        assert_eq!(out.turns[1].start_ms, 1000);
        assert!(out.to_text().contains("A: hello"));
    }

    #[test]
    fn transcript_parses_legacy_aliases_and_push_to_talk() {
        // Older replays used `segments`/`text` with float seconds.
        let stub = StubTransport::default().route(
            "POST",
            "/asr/transcribe",
            200,
            r#"{"turns":[{"text":"hello","speaker":"A","startMs":0,"endMs":900},
                          {"text":"hi","start":1.0,"end":2.0}]}"#,
        );
        let client = MetaClient::new(stub, BASE_URL, "k".to_string()).with_terms_read(true);
        let audio = wav_bytes(16000, 1, 16, 16000);
        let out = client.transcribe("v.wav", "audio/wav", &audio).unwrap();
        assert_eq!(out.turns[1].start_ms, 1000);
        // PUSH_TO_TALK returns no turns: top-level transcript is one turn.
        let stub = StubTransport::default().route(
            "POST",
            "/asr/transcribe",
            200,
            r#"{"sessionId":"s-2","transcript":"hello there","audioDurationMs":900,"turns":[]}"#,
        );
        let client = MetaClient::new(stub, BASE_URL, "k".to_string()).with_terms_read(true);
        let out = client.transcribe("v.wav", "audio/wav", &audio).unwrap();
        assert_eq!(out.turns.len(), 1);
        assert_eq!(out.turns[0].text, "hello there");
    }

    #[test]
    fn transcribe_posts_request_and_audio_parts() {
        // Contract: `request` JSON part (application/json) + `audio`
        // WAV part (audio/wav), per the speech-to-text docs.
        let transport = CaptureTransport::canned(
            200,
            r#"{"sessionId":"s-1","transcript":"hi","audioDurationMs":500,"turns":[]}"#,
        );
        let client = MetaClient::new(transport, BASE_URL, "k".to_string()).with_terms_read(true);
        let audio = wav_bytes(24000, 1, 16, 12000);
        client.transcribe("v.wav", "audio/wav", &audio).unwrap();
        let file = client.transport.last_file.borrow();
        assert_eq!(file.0, "audio");
        assert_eq!(file.1, "v.wav");
        assert_eq!(file.2, "audio/wav");
        assert_eq!(file.3, audio);
        let fields = client.transport.last_fields.borrow();
        assert_eq!(fields.len(), 1);
        assert_eq!(fields[0].0, "request");
        assert_eq!(fields[0].2.as_deref(), Some("application/json"));
        let request: Value = serde_json::from_str(&fields[0].1).unwrap();
        assert_eq!(
            request,
            json!({
                "model": VOICE_MODEL,
                "audioEncoding": "WAV",
                "mode": "ENDPOINTING",
            })
        );
    }

    #[test]
    fn transcribe_request_json_pins_official_shape() {
        assert_eq!(
            transcribe_request_json(&TranscribeOptions::default()),
            json!({
                "model": VOICE_MODEL,
                "audioEncoding": "WAV",
                "mode": "ENDPOINTING",
            })
        );
        let options = TranscribeOptions {
            mode: "DIARIZATION",
            language_bias: &["English", "French"],
            keywords: &["Acme Mobile"],
        };
        assert_eq!(
            transcribe_request_json(&options),
            json!({
                "model": VOICE_MODEL,
                "audioEncoding": "WAV",
                "mode": "DIARIZATION",
                "languageBias": ["English", "French"],
                "keywords": ["Acme Mobile"],
            })
        );
    }

    #[test]
    fn transcribe_validates_wav_before_sending() {
        let client = MetaClient::new(StubTransport::default(), BASE_URL, "k".to_string())
            .with_terms_read(true);
        for (why, audio) in [
            ("mp3 bytes", b"fake-audio".to_vec()),
            ("stereo", wav_bytes(24000, 2, 16, 100)),
            ("44.1 kHz", wav_bytes(44100, 1, 16, 100)),
        ] {
            let err = format!(
                "{}",
                client.transcribe("v", "audio/mpeg", &audio).unwrap_err()
            );
            assert!(err.contains("WAV"), "{why}: {err}");
        }
        // Declared data size claims >10 minutes; no big allocation.
        let mut overlong = wav_bytes(16000, 1, 16, 100);
        let too_much = TRANSCRIBE_MAX_DURATION_MS * 32 + 2000;
        overlong[40..44].copy_from_slice(&(too_much as u32).to_le_bytes());
        let err = format!(
            "{}",
            client.transcribe("v", "audio/wav", &overlong).unwrap_err()
        );
        assert!(err.contains("10-minute"), "{err}");
    }

    #[test]
    fn transcribe_fails_closed_until_terms_read() {
        let client = MetaClient::new(StubTransport::default(), BASE_URL, "k".to_string());
        let audio = wav_bytes(16000, 1, 16, 100);
        let err = format!(
            "{}",
            client.transcribe("v", "audio/wav", &audio).unwrap_err()
        );
        assert!(err.contains("terms are read"), "{err}");
    }

    fn table_schema() -> JsonSchema {
        JsonSchema {
            name: "rows".to_string(),
            schema: json!({
                "type": "object",
                "properties": {"rows": {"type": "array"}},
                "required": ["rows"],
                "additionalProperties": false,
            }),
            strict: true,
        }
    }

    #[test]
    fn responses_create_pins_flat_text_format() {
        // Contract: `text.format` is flat
        // ({type,name,schema,strict}) — the nested `json_schema`
        // wrapper is the chat `response_format` shape.
        let transport = CaptureTransport::canned(
            200,
            r#"{"id":"resp_1","status":"completed",
                "output":[{"type":"message","content":[{"type":"output_text","text":"{\"rows\":[]}"}]}],
                "usage":{"input_tokens":3,"output_tokens":4}}"#,
        );
        let client = MetaClient::new(transport, BASE_URL, "k".to_string());
        let created = client
            .create_response(&ResponsesRequest {
                model: "muse-spark-1.3".to_string(),
                instructions: "i".to_string(),
                input: vec![InputBlock::Text("hi".to_string())],
                schema: Some(table_schema()),
                max_output_tokens: None,
                reasoning_effort: None,
                background: false,
                previous_response_id: None,
            })
            .unwrap();
        assert_eq!(created.completion.unwrap().text, "{\"rows\":[]}");
        let body = client.transport.last_json.borrow();
        assert_eq!(
            body["text"],
            json!({
                "format": {
                    "type": "json_schema",
                    "name": "rows",
                    "schema": {
                        "type": "object",
                        "properties": {"rows": {"type": "array"}},
                        "required": ["rows"],
                        "additionalProperties": false,
                    },
                    "strict": true,
                },
            })
        );
        assert!(body["text"]["format"].get("json_schema").is_none());
    }

    #[test]
    fn responses_incomplete_is_an_error() {
        let stub = StubTransport::default().route(
            "GET",
            "/responses/resp_9",
            200,
            r#"{"id":"resp_9","status":"incomplete",
                "incomplete_details":{"reason":"max_output_tokens"},
                "output":[]}"#,
        );
        let client = MetaClient::new(stub, BASE_URL, "k".to_string());
        let err = format!("{}", client.retrieve_response("resp_9").unwrap_err());
        assert!(err.contains("incomplete"), "{err}");
        assert!(err.contains("max_output_tokens"), "{err}");
    }

    #[test]
    fn responses_refusal_is_an_error() {
        let stub = StubTransport::default().route(
            "POST",
            "/responses",
            200,
            r#"{"id":"resp_2","status":"completed",
                "output":[{"type":"message","content":[
                    {"type":"refusal","refusal":"I cannot help with that"}]}],
                "usage":{"input_tokens":3,"output_tokens":4}}"#,
        );
        let client = MetaClient::new(stub, BASE_URL, "k".to_string());
        let err = format!(
            "{}",
            client
                .create_response(&ResponsesRequest {
                    model: "m".to_string(),
                    instructions: "i".to_string(),
                    input: vec![InputBlock::Text("hi".to_string())],
                    schema: None,
                    max_output_tokens: None,
                    reasoning_effort: None,
                    background: false,
                    previous_response_id: None,
                })
                .unwrap_err()
        );
        assert!(err.contains("refused"), "{err}");
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
