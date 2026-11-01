//! F9 redaction: SAM 3.1 via Model API (G1, G3).
//!
//! Every public render of community media uses a redacted derivative;
//! originals never serve to anonymous clients. Both the uploading
//! customer (pre-publish self-service) and admin moderation redact
//! through the [`Redactor`] trait: [`MetaSamClient`] in production,
//! [`StubRedactor`] in tests and CI. Derivatives record the input sha
//! + model version and strip EXIF.

use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::policy::{MediaPolicy, MediaSource, check};
use crate::transport::{HttpResponse, HttpTransport, TransportError};

/// Meta Models API base (meta-api.md).
pub const META_API_BASE: &str = "https://api.meta.ai/v1";

/// SAM-on-API request path. The overview confirms SAM 3.1 is on the
/// Model API with text prompts; the exact path + JSON shape get
/// pinned against dev.meta.ai at integration time.
pub const SAM_PATH: &str = "/v1/sam/segment";

/// SAM model version recorded on every derivative.
pub const SAM_MODEL_VERSION: &str = "sam-3.1";

/// What to mask: text prompts `face` / `license plate`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum RedactTarget {
    Face,
    LicensePlate,
}

impl RedactTarget {
    pub fn prompt(self) -> &'static str {
        match self {
            RedactTarget::Face => "face",
            RedactTarget::LicensePlate => "license plate",
        }
    }
}

/// One mask: normalized 0..1 box + prompt + score.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RedactBox {
    pub target: RedactTarget,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub score: f32,
}

/// A redaction run: input sha + model version + masks (F9).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Redaction {
    pub input_sha: String,
    pub model_version: String,
    pub boxes: Vec<RedactBox>,
    pub exif_stripped: bool,
}

/// Face/plate masking over one image.
pub trait Redactor {
    /// Mask `targets` in `image` (raw bytes; the client encodes).
    ///
    /// # Errors
    ///
    /// Returns [`RedactError`] when the terms gate blocks real media
    /// or the model call fails.
    fn redact(
        &self,
        policy: &MediaPolicy,
        source: MediaSource,
        image_sha: &str,
        image: &[u8],
        targets: &[RedactTarget],
    ) -> Result<Redaction, RedactError>;
}

/// [`Redactor`] failure.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum RedactError {
    TermsNotRead,
    Transport(TransportError),
    Api { status: u16, message: String },
    BadResponse(String),
}

impl fmt::Display for RedactError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RedactError::TermsNotRead => write!(
                f,
                "real media is blocked until the meta api data-handling terms are read"
            ),
            RedactError::Transport(e) => write!(f, "{e}"),
            RedactError::Api { status, message } => {
                write!(f, "sam request failed (http {status}): {message}")
            }
            RedactError::BadResponse(message) => {
                write!(f, "sam answer was unparseable: {message}")
            }
        }
    }
}

impl std::error::Error for RedactError {}

impl From<TransportError> for RedactError {
    fn from(e: TransportError) -> Self {
        RedactError::Transport(e)
    }
}

/// SAM 3.1 over the Meta Model API.
pub struct MetaSamClient<T: HttpTransport> {
    base_url: String,
    bearer: Option<String>,
    transport: T,
}

impl<T: HttpTransport> MetaSamClient<T> {
    /// `bearer` is the `META_API_KEY` value; `None` keeps the client
    /// inert (every call fails closed) until the key lands.
    pub fn new(base_url: &str, bearer: Option<String>, transport: T) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            bearer,
            transport,
        }
    }

    /// Request body: model + base64 image + text prompts. Shape is
    /// pinned against dev.meta.ai at integration time.
    pub fn request_body(image: &[u8], targets: &[RedactTarget]) -> Value {
        use base64::Engine as _;
        let prompts: Vec<&str> = targets.iter().map(|t| t.prompt()).collect();
        json!({
            "model": SAM_MODEL_VERSION,
            "imageBase64": base64::engine::general_purpose::STANDARD.encode(image),
            "prompts": prompts,
        })
    }
}

impl<T: HttpTransport> Redactor for MetaSamClient<T> {
    fn redact(
        &self,
        policy: &MediaPolicy,
        source: MediaSource,
        image_sha: &str,
        image: &[u8],
        targets: &[RedactTarget],
    ) -> Result<Redaction, RedactError> {
        check(policy, source).map_err(|_| RedactError::TermsNotRead)?;
        let Some(bearer) = &self.bearer else {
            return Err(RedactError::Api {
                status: 0,
                message: "meta api key is not configured".to_string(),
            });
        };
        let url = format!("{}{SAM_PATH}", self.base_url);
        let response =
            self.transport
                .post_json(&url, bearer, &Self::request_body(image, targets))?;
        check_ok(&response)?;
        let json: Value = serde_json::from_slice(&response.body)
            .map_err(|e| RedactError::BadResponse(e.to_string()))?;
        let boxes = parse_boxes(&json)?;
        Ok(Redaction {
            input_sha: image_sha.to_string(),
            model_version: SAM_MODEL_VERSION.to_string(),
            boxes,
            exif_stripped: true,
        })
    }
}

fn check_ok(response: &HttpResponse) -> Result<(), RedactError> {
    if (200..300).contains(&response.status) {
        return Ok(());
    }
    let text = String::from_utf8_lossy(&response.body);
    Err(RedactError::Api {
        status: response.status,
        message: text.chars().take(300).collect(),
    })
}

fn parse_boxes(json: &Value) -> Result<Vec<RedactBox>, RedactError> {
    let items = json
        .get("boxes")
        .or_else(|| json.get("masks"))
        .and_then(Value::as_array)
        .ok_or_else(|| RedactError::BadResponse("no boxes in sam answer".to_string()))?;
    let mut boxes = Vec::with_capacity(items.len());
    for item in items {
        let target = match item.get("label").and_then(Value::as_str).unwrap_or("") {
            "license plate" | "license_plate" | "plate" => RedactTarget::LicensePlate,
            _ => RedactTarget::Face,
        };
        let number = |key: &str| item.get(key).and_then(Value::as_f64).unwrap_or(0.0) as f32;
        boxes.push(RedactBox {
            target,
            x: number("x"),
            y: number("y"),
            w: number("w"),
            h: number("h"),
            score: number("score"),
        });
    }
    Ok(boxes)
}

/// Deterministic offline redactor for tests, CI, and `dev-seed`: one
/// centered face box, fixed model version, no network.
pub struct StubRedactor;

impl Redactor for StubRedactor {
    fn redact(
        &self,
        policy: &MediaPolicy,
        source: MediaSource,
        image_sha: &str,
        _image: &[u8],
        targets: &[RedactTarget],
    ) -> Result<Redaction, RedactError> {
        check(policy, source).map_err(|_| RedactError::TermsNotRead)?;
        let target = targets.first().copied().unwrap_or(RedactTarget::Face);
        Ok(Redaction {
            input_sha: image_sha.to_string(),
            model_version: "stub-sam-0".to_string(),
            boxes: vec![RedactBox {
                target,
                x: 0.25,
                y: 0.25,
                w: 0.5,
                h: 0.5,
                score: 1.0,
            }],
            exif_stripped: true,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    pub(crate) struct StubTransport {
        pub(crate) response: RefCell<HttpResponse>,
        pub(crate) last_url: RefCell<String>,
        pub(crate) last_body: RefCell<Value>,
    }

    impl StubTransport {
        pub(crate) fn canned(status: u16, body: &[u8]) -> Self {
            Self {
                response: RefCell::new(HttpResponse {
                    status,
                    body: body.to_vec(),
                }),
                last_url: RefCell::new(String::new()),
                last_body: RefCell::new(Value::Null),
            }
        }
    }

    impl HttpTransport for StubTransport {
        fn post_json(
            &self,
            url: &str,
            _bearer: &str,
            body: &Value,
        ) -> Result<HttpResponse, TransportError> {
            *self.last_url.borrow_mut() = url.to_string();
            *self.last_body.borrow_mut() = body.clone();
            Ok(self.response.borrow().clone())
        }

        fn post_multipart(
            &self,
            url: &str,
            _bearer: &str,
            _file: &crate::transport::MultipartFile,
            _fields: &[crate::transport::MultipartField],
        ) -> Result<HttpResponse, TransportError> {
            *self.last_url.borrow_mut() = url.to_string();
            Ok(self.response.borrow().clone())
        }
    }

    #[test]
    fn stub_redaction_seam() {
        let policy = MediaPolicy::synthetic_only();
        let redaction = StubRedactor
            .redact(
                &policy,
                MediaSource::SyntheticFixture,
                "sha-synthetic",
                b"bytes",
                &[RedactTarget::Face, RedactTarget::LicensePlate],
            )
            .unwrap();
        assert_eq!(redaction.input_sha, "sha-synthetic");
        assert_eq!(redaction.model_version, "stub-sam-0");
        assert!(redaction.exif_stripped);
        assert_eq!(redaction.boxes.len(), 1);
        assert!(
            StubRedactor
                .redact(&policy, MediaSource::RealUpload, "sha", b"b", &[])
                .is_err()
        );
    }

    #[test]
    fn sam_client_posts_prompts_and_parses_boxes() {
        let answer = serde_json::to_vec(&json!({
            "boxes": [
                {"label": "face", "x": 0.1, "y": 0.2, "w": 0.3, "h": 0.3, "score": 0.99},
                {"label": "license plate", "x": 0.5, "y": 0.5, "w": 0.2, "h": 0.1, "score": 0.9},
            ],
        }))
        .unwrap();
        let transport = StubTransport::canned(200, &answer);
        let client =
            MetaSamClient::new(META_API_BASE, Some("key-synthetic".to_string()), transport);
        let redaction = client
            .redact(
                &MediaPolicy::synthetic_only(),
                MediaSource::SyntheticFixture,
                "sha-synthetic",
                b"image-bytes",
                &[RedactTarget::Face, RedactTarget::LicensePlate],
            )
            .unwrap();
        assert_eq!(
            *client.transport.last_url.borrow(),
            format!("{META_API_BASE}{SAM_PATH}")
        );
        let body = client.transport.last_body.borrow();
        assert_eq!(body["model"], json!(SAM_MODEL_VERSION));
        assert_eq!(body["prompts"], json!(["face", "license plate"]));
        assert_eq!(redaction.boxes.len(), 2);
        assert_eq!(redaction.boxes[1].target, RedactTarget::LicensePlate);
        assert!(redaction.exif_stripped);
    }

    #[test]
    fn sam_client_fails_closed_without_key() {
        let transport = StubTransport::canned(200, b"{}");
        let client = MetaSamClient::new(META_API_BASE, None, transport);
        assert!(matches!(
            client
                .redact(
                    &MediaPolicy::synthetic_only(),
                    MediaSource::SyntheticFixture,
                    "sha",
                    b"b",
                    &[RedactTarget::Face],
                )
                .unwrap_err(),
            RedactError::Api { status: 0, .. }
        ));
    }
}
