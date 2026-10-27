//! Voice Transcribe file ingest (G3, P1 call console).
//!
//! `POST /v1/asr/transcribe` (multipart upload, meta-api.md) turns
//! call audio into stored `capture(kind='transcript')` rows. Output
//! is transcript text with **turn-level** timing (+ optional speaker
//! labels) — the API offers no word-level timestamps, and neither
//! does [`TranscriptTurn`] by design. The realtime WebSocket is the
//! console's later concern, not this module's.

use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::policy::{MediaPolicy, MediaSource, check};
use crate::transport::{HttpResponse, HttpTransport, MultipartFile, TransportError};

/// Voice file-endpoint path (meta-api.md, from the speech-to-text docs).
pub const TRANSCRIBE_PATH: &str = "/v1/asr/transcribe";

/// Transcription model id.
pub const VOICE_MODEL: &str = "muse-voice-transcribe-1.0";

/// One speaker turn: text + turn-level timing. No word timestamps —
/// the API does not emit them and this type cannot carry them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TranscriptTurn {
    pub index: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speaker: Option<String>,
    pub text: String,
    pub start_ms: u64,
    pub end_ms: u64,
}

/// A whole-file transcript: ordered turns.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Transcript {
    pub turns: Vec<TranscriptTurn>,
}

/// Voice Transcribe file client (multipart upload).
pub struct VoiceFileClient<T: HttpTransport> {
    base_url: String,
    bearer: Option<String>,
    transport: T,
}

impl<T: HttpTransport> VoiceFileClient<T> {
    /// `bearer` is the `META_API_KEY` value; `None` fails closed.
    pub fn new(base_url: &str, bearer: Option<String>, transport: T) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            bearer,
            transport,
        }
    }

    /// Transcribe one audio file. `language_bias` is the optional
    /// BCP-47 hint (immutable per session on the realtime API; a
    /// plain hint on the file endpoint).
    ///
    /// # Errors
    ///
    /// Returns [`TranscribeError`] when the terms gate blocks real
    /// media, the key is missing, or the call/parse fails.
    pub fn transcribe(
        &self,
        policy: &MediaPolicy,
        source: MediaSource,
        file_name: &str,
        content_type: &str,
        audio: &[u8],
        language_bias: Option<&str>,
    ) -> Result<Transcript, TranscribeError> {
        check(policy, source).map_err(|_| TranscribeError::TermsNotRead)?;
        let Some(bearer) = &self.bearer else {
            return Err(TranscribeError::Api {
                status: 0,
                message: "meta api key is not configured".to_string(),
            });
        };
        if audio.is_empty() {
            return Err(TranscribeError::EmptyAudio);
        }
        let url = format!("{}{TRANSCRIBE_PATH}", self.base_url);
        let mut fields: Vec<(&str, &str)> = vec![("model", VOICE_MODEL)];
        if let Some(bias) = language_bias {
            fields.push(("languageBias", bias));
        }
        let file = MultipartFile {
            field: "file",
            name: file_name,
            content_type,
            bytes: audio,
        };
        let response = self
            .transport
            .post_multipart(&url, bearer, &file, &fields)?;
        check_ok(&response)?;
        let json: Value = serde_json::from_slice(&response.body)
            .map_err(|e| TranscribeError::BadResponse(e.to_string()))?;
        parse_transcript(&json)
    }
}

fn check_ok(response: &HttpResponse) -> Result<(), TranscribeError> {
    if (200..300).contains(&response.status) {
        return Ok(());
    }
    let text = String::from_utf8_lossy(&response.body);
    Err(TranscribeError::Api {
        status: response.status,
        message: text.chars().take(300).collect(),
    })
}

fn parse_transcript(json: &Value) -> Result<Transcript, TranscribeError> {
    let items = json
        .get("turns")
        .or_else(|| json.get("segments"))
        .and_then(Value::as_array)
        .ok_or_else(|| TranscribeError::BadResponse("no turns in transcript answer".to_string()))?;
    let mut turns = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        let text = item
            .get("text")
            .and_then(Value::as_str)
            .ok_or_else(|| TranscribeError::BadResponse("turn without text".to_string()))?;
        turns.push(TranscriptTurn {
            index,
            speaker: item
                .get("speaker")
                .and_then(Value::as_str)
                .map(str::to_string),
            text: text.to_string(),
            start_ms: turn_millis(item, &["startMs", "start_ms"], &["start"]),
            end_ms: turn_millis(item, &["endMs", "end_ms"], &["end"]),
        });
    }
    Ok(Transcript { turns })
}

/// Turn timing in ms: integer millisecond keys first, then float
/// second keys. Missing timing is 0 (the turn still ingests).
fn turn_millis(item: &Value, ms_keys: &[&str], sec_keys: &[&str]) -> u64 {
    for key in ms_keys {
        if let Some(ms) = item.get(*key).and_then(Value::as_u64) {
            return ms;
        }
    }
    for key in sec_keys {
        if let Some(secs) = item.get(*key).and_then(Value::as_f64) {
            return (secs * 1000.0) as u64;
        }
    }
    0
}

/// [`VoiceFileClient::transcribe`] failure.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum TranscribeError {
    TermsNotRead,
    EmptyAudio,
    Transport(TransportError),
    Api { status: u16, message: String },
    BadResponse(String),
}

impl fmt::Display for TranscribeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TranscribeError::TermsNotRead => write!(
                f,
                "real media is blocked until the meta api data-handling terms are read"
            ),
            TranscribeError::EmptyAudio => write!(f, "audio is empty"),
            TranscribeError::Transport(e) => write!(f, "{e}"),
            TranscribeError::Api { status, message } => {
                write!(f, "transcribe request failed (http {status}): {message}")
            }
            TranscribeError::BadResponse(message) => {
                write!(f, "transcript answer was unparseable: {message}")
            }
        }
    }
}

impl std::error::Error for TranscribeError {}

impl From<TransportError> for TranscribeError {
    fn from(e: TransportError) -> Self {
        TranscribeError::Transport(e)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::redact::META_API_BASE;
    use std::cell::RefCell;

    struct StubTransport {
        response: RefCell<HttpResponse>,
        last_url: RefCell<String>,
        last_file: RefCell<(String, String, Vec<u8>)>,
        last_fields: RefCell<Vec<(String, String)>>,
    }

    impl StubTransport {
        fn canned(status: u16, body: &[u8]) -> Self {
            Self {
                response: RefCell::new(HttpResponse {
                    status,
                    body: body.to_vec(),
                }),
                last_url: RefCell::new(String::new()),
                last_file: RefCell::new((String::new(), String::new(), Vec::new())),
                last_fields: RefCell::new(Vec::new()),
            }
        }
    }

    impl HttpTransport for StubTransport {
        fn post_json(
            &self,
            _url: &str,
            _bearer: &str,
            _body: &Value,
        ) -> Result<HttpResponse, TransportError> {
            panic!("voice file ingest uses multipart, not json");
        }

        fn post_multipart(
            &self,
            url: &str,
            _bearer: &str,
            file: &MultipartFile,
            fields: &[(&str, &str)],
        ) -> Result<HttpResponse, TransportError> {
            *self.last_url.borrow_mut() = url.to_string();
            *self.last_file.borrow_mut() = (
                format!("{}:{}:{}", file.field, file.name, file.content_type),
                file.name.to_string(),
                file.bytes.to_vec(),
            );
            *self.last_fields.borrow_mut() = fields
                .iter()
                .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
                .collect();
            Ok(self.response.borrow().clone())
        }
    }

    #[test]
    fn file_ingest_posts_multipart_and_parses_turns() {
        let answer = serde_json::to_vec(&serde_json::json!({
            "turns": [
                {"speaker": "staff", "text": "Morning, how can I help?", "startMs": 0, "endMs": 2100},
                {"text": "I need an apostille.", "startMs": 2100, "endMs": 4000},
            ],
        }))
        .unwrap();
        let transport = StubTransport::canned(200, &answer);
        let client =
            VoiceFileClient::new(META_API_BASE, Some("key-synthetic".to_string()), transport);
        let transcript = client
            .transcribe(
                &MediaPolicy::synthetic_only(),
                MediaSource::SyntheticFixture,
                "call.mp3",
                "audio/mpeg",
                b"fake-audio",
                Some("en"),
            )
            .unwrap();
        assert_eq!(
            *client.transport.last_url.borrow(),
            format!("{META_API_BASE}{TRANSCRIBE_PATH}")
        );
        assert_eq!(client.transport.last_file.borrow().1, "call.mp3");
        assert!(
            client
                .transport
                .last_fields
                .borrow()
                .contains(&("model".to_string(), VOICE_MODEL.to_string()))
        );
        assert_eq!(transcript.turns.len(), 2);
        assert_eq!(transcript.turns[0].speaker.as_deref(), Some("staff"));
        assert_eq!(transcript.turns[1].end_ms, 4000);
    }

    #[test]
    fn turns_carry_no_word_timestamps() {
        let turn = TranscriptTurn {
            index: 0,
            speaker: None,
            text: "hello".to_string(),
            start_ms: 0,
            end_ms: 500,
        };
        let json = serde_json::to_string(&turn).unwrap();
        for banned in ["word", "token", "phoneme", "confidence"] {
            assert!(!json.contains(banned), "turn leaks {banned}: {json}");
        }
    }
}
