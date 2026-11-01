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
