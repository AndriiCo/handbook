//! Blocking HTTP for the Meta Model API clients.
//!
//! Same shape as the labels transport (blocking ureq over rustls),
//! plus multipart for the Voice file endpoint. The web layer runs
//! these on `spawn_blocking`; `META_API_KEY` arrives per call and is
//! never logged or persisted here.

use std::fmt;
use std::time::Duration;

use serde_json::Value;

/// Raw HTTP response: status + bytes.
#[derive(Debug, Clone)]
pub struct HttpResponse {
    pub status: u16,
    pub body: Vec<u8>,
}

/// One file part of a multipart upload.
pub struct MultipartFile<'a> {
    pub field: &'a str,
    pub name: &'a str,
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

/// Minimal HTTP surface the Meta clients need.
pub trait HttpTransport {
    fn post_json(
        &self,
        url: &str,
        bearer: &str,
        body: &Value,
    ) -> Result<HttpResponse, TransportError>;

    fn post_multipart(
        &self,
        url: &str,
        bearer: &str,
        file: &MultipartFile,
        fields: &[MultipartField],
    ) -> Result<HttpResponse, TransportError>;
}

/// Live [`HttpTransport`] over ureq (rustls, pure Rust).
pub struct UreqTransport {
    agent: ureq::Agent,
}

impl UreqTransport {
    pub fn new() -> Self {
        let config = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_global(Some(Duration::from_secs(120)))
            .build();
        Self {
            agent: ureq::Agent::new_with_config(config),
        }
    }
}

impl Default for UreqTransport {
    fn default() -> Self {
        Self::new()
    }
}

impl HttpTransport for UreqTransport {
    fn post_json(
        &self,
        url: &str,
        bearer: &str,
        body: &Value,
    ) -> Result<HttpResponse, TransportError> {
        let response = self
            .agent
            .post(url)
            .header("Authorization", format!("Bearer {bearer}"))
            .header("Content-Type", "application/json")
            .send_json(body)
            .map_err(TransportError::from_ureq)?;
        finish(response)
    }

    fn post_multipart(
        &self,
        url: &str,
        bearer: &str,
        file: &MultipartFile,
        fields: &[MultipartField],
    ) -> Result<HttpResponse, TransportError> {
        let mut form = ureq::unversioned::multipart::Form::new();
        for field in fields {
            match field.content_type {
                Some(mime) => {
                    let part = ureq::unversioned::multipart::Part::text(field.value)
                        .mime_str(mime)
                        .map_err(TransportError::from_ureq)?;
                    form = form.part(field.name, part);
                }
                None => {
                    form = form.text(field.name, field.value);
                }
            }
        }
        let part = ureq::unversioned::multipart::Part::bytes(file.bytes)
            .file_name(file.name)
            .mime_str(file.content_type)
            .map_err(TransportError::from_ureq)?;
        form = form.part(file.field, part);
        let response = self
            .agent
            .post(url)
            .header("Authorization", format!("Bearer {bearer}"))
            .send(form)
            .map_err(TransportError::from_ureq)?;
        finish(response)
    }
}

/// Record/replay wrapper (operator decision 2026-11-03: reruns never
/// re-spend Meta budget). JSON bodies key on their canonical form,
/// multipart on the text fields plus the file's sha256; a hit returns
/// the stored status + raw bytes and never counts toward the daily cap.
pub struct CachedTransport<T: HttpTransport> {
    inner: T,
    cache: std::sync::Arc<callcache::MetaCache>,
}

impl<T: HttpTransport> CachedTransport<T> {
    pub fn new(inner: T, cache: std::sync::Arc<callcache::MetaCache>) -> Self {
        Self { inner, cache }
    }

    fn through(
        &self,
        key: &str,
        req: &callcache::meta::Request<'_>,
        live: impl FnOnce() -> Result<HttpResponse, TransportError>,
    ) -> Result<HttpResponse, TransportError> {
        let live = || live().map(|r| (r.status, r.body));
        match self.cache.call(key, req, live) {
            Ok(answer) => Ok(HttpResponse {
                status: answer.status,
                body: answer.body,
            }),
            Err(callcache::CallError::Live(e)) => Err(e),
            Err(callcache::CallError::Cache(e)) => Err(TransportError::new(e.to_string())),
        }
    }
}

impl<T: HttpTransport> HttpTransport for CachedTransport<T> {
    fn post_json(
        &self,
        url: &str,
        bearer: &str,
        body: &Value,
    ) -> Result<HttpResponse, TransportError> {
        let req = callcache::meta::Request {
            method: "POST",
            url,
            model: body.get("model").and_then(Value::as_str).unwrap_or(""),
        };
        let key = callcache::meta::json_key(url, body);
        self.through(&key, &req, || self.inner.post_json(url, bearer, body))
    }

    fn post_multipart(
        &self,
        url: &str,
        bearer: &str,
        file: &MultipartFile,
        fields: &[MultipartField],
    ) -> Result<HttpResponse, TransportError> {
        let parts: Vec<callcache::meta::TextPart<'_>> = fields
            .iter()
            .map(|f| callcache::meta::TextPart {
                name: f.name,
                value: f.value,
                content_type: f.content_type,
            })
            .collect();
        let key =
            callcache::meta::multipart_key(url, file.field, file.content_type, file.bytes, &parts);
        let model = fields
            .iter()
            .find(|f| f.name == "request")
            .and_then(|f| serde_json::from_str::<Value>(f.value).ok())
            .and_then(|v| v.get("model").and_then(Value::as_str).map(str::to_string))
            .unwrap_or_default();
        let req = callcache::meta::Request {
            method: "POST",
            url,
            model: &model,
        };
        self.through(&key, &req, || {
            self.inner.post_multipart(url, bearer, file, fields)
        })
    }
}

fn finish(response: ureq::http::Response<ureq::Body>) -> Result<HttpResponse, TransportError> {
    let status = response.status().as_u16();
    let body = response
        .into_body()
        .read_to_vec()
        .map_err(TransportError::from_ureq)?;
    Ok(HttpResponse { status, body })
}

/// Transport failure (DNS, TLS, timeouts, protocol). The message never
/// includes the bearer token or the request body.
#[derive(Debug, Clone)]
pub struct TransportError {
    message: String,
}

impl TransportError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    fn from_ureq(error: ureq::Error) -> Self {
        Self {
            message: error.to_string(),
        }
    }
}

impl fmt::Display for TransportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "meta http transport: {}", self.message)
    }
}

impl std::error::Error for TransportError {}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::sync::Arc;

    use super::*;

    #[derive(Default)]
    struct Counting(Cell<u32>);

    impl HttpTransport for &Counting {
        fn post_json(&self, _: &str, _: &str, _: &Value) -> Result<HttpResponse, TransportError> {
            self.0.set(self.0.get() + 1);
            Ok(HttpResponse {
                status: 200,
                body: br#"{"boxes":[]}"#.to_vec(),
            })
        }

        fn post_multipart(
            &self,
            _: &str,
            _: &str,
            _: &MultipartFile,
            _: &[MultipartField],
        ) -> Result<HttpResponse, TransportError> {
            self.0.set(self.0.get() + 1);
            Ok(HttpResponse {
                status: 200,
                body: br#"{"text":"hello"}"#.to_vec(),
            })
        }
    }

    fn audio(bytes: &[u8]) -> MultipartFile<'_> {
        MultipartFile {
            field: "audio",
            name: "clip-1234.wav",
            content_type: "audio/wav",
            bytes,
        }
    }

    #[test]
    fn repeat_upload_and_json_calls_skip_the_transport() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Arc::new(
            callcache::MetaCache::new(dir.path(), callcache::CacheMode::Record).with_budget(9),
        );
        let counting = Counting::default();
        let transport = CachedTransport::new(&counting, cache.clone());
        let url = "https://api.meta.ai/v1/asr/transcribe";
        let request = r#"{"model":"muse-voice-transcribe-1.0"}"#;
        let fields = [MultipartField::typed(
            "request",
            request,
            "application/json",
        )];
        for _ in 0..2 {
            let r = transport
                .post_multipart(url, "k", &audio(b"RIFF1"), &fields)
                .unwrap();
            assert_eq!(r.body, br#"{"text":"hello"}"#);
        }
        assert_eq!(counting.0.get(), 1);
        // Other audio bytes are another request.
        transport
            .post_multipart(url, "k", &audio(b"RIFF2"), &fields)
            .unwrap();
        assert_eq!(counting.0.get(), 2);
        let body = serde_json::json!({"model": "sam-3.1", "prompt": "face"});
        for _ in 0..2 {
            transport
                .post_json("https://api.meta.ai/v1/sam", "k", &body)
                .unwrap();
        }
        assert_eq!(counting.0.get(), 3);
        assert_eq!(cache.budget().unwrap().used(callcache::unix_now()), 3);

        let offline = Arc::new(callcache::MetaCache::new(
            dir.path(),
            callcache::CacheMode::ReplayOnly,
        ));
        let idle = Counting::default();
        let transport = CachedTransport::new(&idle, offline);
        transport
            .post_multipart(url, "k", &audio(b"RIFF1"), &fields)
            .unwrap();
        assert!(
            transport
                .post_multipart(url, "k", &audio(b"RIFF3"), &fields)
                .is_err()
        );
        assert_eq!(idle.0.get(), 0);
    }
}
