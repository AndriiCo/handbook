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

/// SAM 3.1 runs on the Responses endpoint: one call carries the prompt
/// plus the image, and streams a box and a mask per object
/// (sam/segmenting in the Meta docs). The old `/v1/sam/segment` path
/// never existed — joining it onto the `/v1` base produced `/v1/v1/…`.
pub const SAM_PATH: &str = "/responses";

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
    /// SAM object id of the record this box came from (stable within
    /// one response; ids are sparse, never positional).
    pub object_id: u32,
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

    /// Request body for one concept ([`sam_request_body`]).
    pub fn request_body(prompt: &str, image: &[u8]) -> Value {
        sam_request_body(prompt, image)
    }
}

/// Request body for one concept: model + one bare noun phrase in
/// `input_text` + the image in `input_image` (sam/segmenting).
/// Concept prompts name one thing per request — lists return zero
/// matches — so callers send one body per target. Shared by
/// [`MetaSamClient`] and the A2 recorder (`xtask a2-record`).
pub fn sam_request_body(prompt: &str, image: &[u8]) -> Value {
    use base64::Engine as _;
    json!({
        "model": SAM_MODEL_VERSION,
        "input": [{
            "type": "message",
            "role": "user",
            "content": [
                {"type": "input_text", "text": prompt},
                {"type": "input_image",
                 "image_url": format!("data:{};base64,{}",
                    image_mime(image),
                    base64::engine::general_purpose::STANDARD.encode(image))},
            ],
        }],
        "stream": false,
        "metadata": {"include_confidence": "true"},
    })
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
        // One bare noun phrase per request; merge the matches.
        let url = format!("{}{SAM_PATH}", self.base_url);
        let mut boxes = Vec::new();
        for target in targets {
            let response = self.transport.post_json(
                &url,
                bearer,
                &MetaSamClient::<T>::request_body(target.prompt(), image),
            )?;
            check_ok(&response)?;
            let json: Value = serde_json::from_slice(&response.body)
                .map_err(|e| RedactError::BadResponse(e.to_string()))?;
            boxes.extend(parse_segmentation(&json, *target)?);
        }
        Ok(Redaction {
            input_sha: image_sha.to_string(),
            model_version: SAM_MODEL_VERSION.to_string(),
            boxes,
            exif_stripped: true,
        })
    }
}

/// Data-URL MIME sniff: PNG magic wins, JPEG SOI second, otherwise
/// JPEG (the service sniffs content; the label only routes decoding).
fn image_mime(image: &[u8]) -> &'static str {
    if image.starts_with(b"\x89PNG\r\n\x1a\n") {
        "image/png"
    } else {
        "image/jpeg"
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

/// Collect the `output_text` lane of a Responses answer.
fn output_text(json: &Value) -> String {
    json.get("output")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.get("content")?.as_array())
                .flatten()
                .filter(|block| block.get("type").and_then(Value::as_str) == Some("output_text"))
                .filter_map(|block| block.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}

/// Parse a SAM 3.1 segmentation answer into boxes for `target`.
fn parse_segmentation(json: &Value, target: RedactTarget) -> Result<Vec<RedactBox>, RedactError> {
    Ok(parse_sam_answer(json)?
        .into_iter()
        .map(|b| RedactBox {
            target,
            object_id: b.object_id,
            x: b.x as f32,
            y: b.y as f32,
            w: b.w as f32,
            h: b.h as f32,
            score: b.score as f32,
        })
        .collect())
}

/// One SAM 3.1 box record, label-free: object id, box as fractions of
/// the source image, confidence (1.0 when the answer carries none).
/// [`Redactor`] tags it with its target; the A2 recorder with its
/// concept prompt.
#[derive(Debug, Clone, PartialEq)]
pub struct SamBox {
    pub object_id: u32,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub score: f64,
}

/// Parse a SAM 3.1 Responses answer into its box records.
///
/// Reads the documented token grammar
/// (`<Nf>id<|box;x1;y1;x2;y2;w;h[;c]|><|mask;…|>,…`): box corners are
/// source pixels, normalized by the record's own `w`/`h`; records key
/// by object id, never by position. An empty lane (zero matches) is a
/// normal result, not an error. `incomplete` and refusals are errors.
///
/// # Errors
///
/// [`RedactError::BadResponse`] on a refusal, an `incomplete` answer
/// or any status other than `completed`.
pub fn parse_sam_answer(json: &Value) -> Result<Vec<SamBox>, RedactError> {
    if json
        .get("output")
        .and_then(Value::as_array)
        .map(|items| {
            items.iter().flat_map(|item| {
                item.get("content")
                    .and_then(Value::as_array)
                    .map(Vec::as_slice)
                    .unwrap_or(&[])
            })
        })
        .is_some_and(|mut blocks| {
            blocks.any(|b| b.get("type").and_then(Value::as_str) == Some("refusal"))
        })
    {
        return Err(RedactError::BadResponse(
            "sam refused the request".to_string(),
        ));
    }
    match json.get("status").and_then(Value::as_str) {
        Some("completed") | None => {}
        Some("incomplete") => {
            return Err(RedactError::BadResponse(
                "sam response incomplete (truncated)".to_string(),
            ));
        }
        Some(other) => {
            return Err(RedactError::BadResponse(format!("sam status: {other}")));
        }
    }
    let text = output_text(json);
    Ok(parse_records(&text))
}

/// Scan `output_text` lines for box records. Fields parse by key, so
/// tokens read the same with or without confidence `c`.
fn parse_records(text: &str) -> Vec<SamBox> {
    let mut boxes = Vec::new();
    for line in text.lines() {
        let mut rest = line.trim();
        // Frame marker `<Nf>` opens the line; frames never feed boxes.
        if let Some(end) = rest.strip_prefix('<').and_then(|r| r.find('>')) {
            rest = rest[end + 1..].trim_start();
        }
        for record in split_records(rest) {
            if let Some(parsed) = parse_record(record)
                && parsed.w > 0.0
                && parsed.h > 0.0
            {
                boxes.push(parsed);
            }
        }
    }
    boxes
}

/// Split a frame line into `id<|box;…|>…` records. Mask payloads are
/// base85 and may hold commas and `>`, so records anchor on the
/// box-token opener with the object id scanned backwards from it —
/// never on comma or bracket splits.
fn split_records(line: &str) -> Vec<&str> {
    let bytes = line.as_bytes();
    let mut box_at = Vec::new();
    let mut cursor = 0;
    while let Some(found) = line[cursor..].find("<|box;") {
        box_at.push(cursor + found);
        cursor += found + 6;
    }
    // Start of the record owning the box token at `box_pos`: the
    // run of digits (the object id) right before it.
    fn record_start(bytes: &[u8], box_pos: usize) -> usize {
        let mut end = box_pos;
        while end > 0 && (bytes[end - 1].is_ascii_whitespace() || bytes[end - 1] == b',') {
            end -= 1;
        }
        let mut start = end;
        while start > 0 && bytes[start - 1].is_ascii_digit() {
            start -= 1;
        }
        start
    }
    box_at
        .iter()
        .enumerate()
        .filter_map(|(i, &pos)| {
            let start = record_start(bytes, pos);
            if start == pos {
                return None;
            }
            let end = box_at
                .get(i + 1)
                .map(|&next| record_start(bytes, next))
                .unwrap_or(line.len());
            if end <= start {
                return None;
            }
            Some(&line[start..end])
        })
        .collect()
}

/// Parse one `id<|box;x1=..;…|><|mask;…|>` record.
fn parse_record(record: &str) -> Option<SamBox> {
    let (id_part, box_part) = record.split_once("<|box;")?;
    let object_id: u32 = id_part.trim().trim_start_matches(',').trim().parse().ok()?;
    let inner = box_part.split_once("|>")?.0;
    let mut x1 = None;
    let mut y1 = None;
    let mut x2 = None;
    let mut y2 = None;
    let mut w = None;
    let mut h = None;
    let mut score = 1.0f64;
    for field in inner.split(';') {
        let (key, value) = field.split_once('=')?;
        match key.trim() {
            "x1" => x1 = value.trim().parse().ok(),
            "y1" => y1 = value.trim().parse().ok(),
            "x2" => x2 = value.trim().parse().ok(),
            "y2" => y2 = value.trim().parse().ok(),
            "w" => w = value.trim().parse().ok(),
            "h" => h = value.trim().parse().ok(),
            "c" => {
                if let Ok(c) = value.trim().parse::<f64>() {
                    score = c;
                }
            }
            _ => {}
        }
    }
    let (x1, y1, x2, y2, w, h): (f64, f64, f64, f64, f64, f64) = (x1?, y1?, x2?, y2?, w?, h?);
    if w <= 0.0 || h <= 0.0 || x2 < x1 || y2 < y1 {
        return None;
    }
    Some(SamBox {
        object_id,
        x: x1 / w,
        y: y1 / h,
        w: (x2 - x1) / w,
        h: (y2 - y1) / h,
        score,
    })
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
                object_id: 0,
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
        pub(crate) bodies: RefCell<Vec<Value>>,
    }

    impl StubTransport {
        pub(crate) fn canned(status: u16, body: &[u8]) -> Self {
            Self {
                response: RefCell::new(HttpResponse {
                    status,
                    body: body.to_vec(),
                }),
                last_url: RefCell::new(String::new()),
                bodies: RefCell::new(Vec::new()),
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
            self.bodies.borrow_mut().push(body.clone());
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

    /// A completed Responses answer carrying one `output_text` lane.
    fn sam_answer(text: &str) -> Vec<u8> {
        serde_json::to_vec(&json!({
            "id": "resp_sam_1",
            "status": "completed",
            "output": [{
                "type": "message",
                "content": [{"type": "output_text", "text": text}],
            }],
        }))
        .unwrap()
    }

    #[test]
    fn sam_request_pins_responses_shape() {
        // Contract: one bare noun phrase in `input_text`, the image in
        // `input_image`, on POST {base}/responses — never a list, never
        // a `/v1/sam/segment` path.
        let body = MetaSamClient::<StubTransport>::request_body("face", b"image-bytes");
        assert_eq!(body["model"], json!(SAM_MODEL_VERSION));
        let content = &body["input"][0]["content"];
        assert_eq!(content[0], json!({"type": "input_text", "text": "face"}));
        let url = content[1]["image_url"].as_str().unwrap();
        assert!(url.starts_with("data:image/jpeg;base64,"), "{url}");
        assert_eq!(body["metadata"], json!({"include_confidence": "true"}));
        assert!(body.get("prompts").is_none());
        assert!(body.get("imageBase64").is_none());
    }

    #[test]
    fn sam_client_segments_each_target_separately() {
        let answer = sam_answer(
            "<0f>0<|box;x1=181;y1=365;x2=556;y2=486;w=1516;h=1600;c=0.99|>\
             <|mask;x=0;y=0;c=0.99;data=122,376,~payload|>",
        );
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
        // No doubled path: {base}/responses, one call per concept.
        assert_eq!(
            *client.transport.last_url.borrow(),
            format!("{META_API_BASE}{SAM_PATH}")
        );
        assert!(!client.transport.last_url.borrow().contains("/v1/v1/"));
        let bodies = client.transport.bodies.borrow();
        assert_eq!(bodies.len(), 2);
        assert_eq!(bodies[0]["input"][0]["content"][0]["text"], json!("face"));
        assert_eq!(
            bodies[1]["input"][0]["content"][0]["text"],
            json!("license plate")
        );
        // Same canned lane per request, tagged by requesting target.
        assert_eq!(redaction.boxes.len(), 2);
        assert_eq!(redaction.boxes[0].target, RedactTarget::Face);
        assert_eq!(redaction.boxes[1].target, RedactTarget::LicensePlate);
        let first = &redaction.boxes[0];
        assert_eq!(first.object_id, 0);
        assert!((first.x - 181.0 / 1516.0).abs() < 1e-4, "{first:?}");
        assert!(
            (first.w - (556.0 - 181.0) / 1516.0).abs() < 1e-4,
            "{first:?}"
        );
        assert!((first.score - 0.99).abs() < 1e-6, "{first:?}");
        assert!(redaction.exif_stripped);
    }

    #[test]
    fn sam_records_parse_by_object_id() {
        // Sparse ids (0, 2), confidence, and a mask payload holding
        // commas must not disturb the parse.
        let text = "<0f>0<|box;x1=211;y1=228;x2=270;y2=254;w=320;h=334|>\
            <|mask;x=0;y=0;data=27,60,~a,b|>,2<|box;x1=155;y1=228;x2=202;y2=254;\
            w=320;h=334|><|mask;x=0;y=0;data=27,48,~c,d|>";
        let boxes = parse_records(text);
        assert_eq!(boxes.len(), 2);
        assert_eq!(boxes[0].object_id, 0);
        assert_eq!(boxes[1].object_id, 2);
        assert!((boxes[1].x - 155.0 / 320.0).abs() < 1e-4);
        // Zero matches is a normal result, not an error.
        assert!(parse_records("").is_empty());
        assert!(parse_records("<0f>").is_empty());
    }

    #[test]
    fn sam_incomplete_and_refusal_are_errors() {
        let incomplete = serde_json::to_vec(&json!({
            "id": "resp_x", "status": "incomplete",
            "incomplete_details": {"reason": "max_output_tokens"},
            "output": [],
        }))
        .unwrap();
        assert!(
            parse_segmentation(
                &serde_json::from_slice(&incomplete).unwrap(),
                RedactTarget::Face,
            )
            .is_err()
        );
        let refusal = serde_json::to_vec(&json!({
            "id": "resp_y", "status": "completed",
            "output": [{"type": "message",
                        "content": [{"type": "refusal", "refusal": "no"}]}],
        }))
        .unwrap();
        assert!(
            parse_segmentation(
                &serde_json::from_slice(&refusal).unwrap(),
                RedactTarget::Face,
            )
            .is_err()
        );
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
