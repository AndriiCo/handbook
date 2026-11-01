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
use crate::transport::{
    HttpResponse, HttpTransport, MultipartField, MultipartFile, TransportError,
};

/// Voice file-endpoint path (meta-api.md, from the speech-to-text docs).
pub const TRANSCRIBE_PATH: &str = "/v1/asr/transcribe";

/// Transcription model id.
pub const VOICE_MODEL: &str = "muse-voice-transcribe-1.0";

/// Request-body size cap: bodies over 32 MB are rejected (413).
pub const TRANSCRIBE_BODY_LIMIT: usize = 32 * 1024 * 1024;

/// Audio-duration cap: clips over 10 minutes are rejected (400).
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

/// Build the `request` JSON part of the multipart body.
///
/// Pinned to the speech-to-text docs: `model`, `audioEncoding: "WAV"`,
/// `mode`, plus `languageBias`/`keywords` only when non-empty.
pub fn transcribe_request_json(options: &TranscribeOptions) -> Value {
    let mut request = serde_json::json!({
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

/// Check the clip is a supported WAV file before uploading.
///
/// The endpoint accepts a RIFF/WAVE container holding mono 16-bit
/// integer PCM at 16 or 24 kHz. Returns the audio duration in ms.
/// Anything else is a 400 on the wire (`ffmpeg -ac 1 -ar 24000
/// -c:a pcm_s16le` converts); bodies over 32 MB are a 413.
pub fn validate_wav_audio(audio: &[u8]) -> Result<u64, TranscribeError> {
    if audio.len() > TRANSCRIBE_BODY_LIMIT {
        return Err(TranscribeError::InvalidAudio(format!(
            "audio is {} bytes, over the 32 MB body cap — split into shorter clips",
            audio.len()
        )));
    }
    let wav = WavInfo::parse(audio)?;
    let duration_ms = wav.duration_ms();
    if duration_ms > TRANSCRIBE_MAX_DURATION_MS {
        return Err(TranscribeError::InvalidAudio(format!(
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
    fn parse(audio: &[u8]) -> Result<Self, TranscribeError> {
        let invalid = |why: &str| TranscribeError::InvalidAudio(why.to_string());
        if audio.len() < 12 || &audio[0..4] != b"RIFF" || &audio[8..12] != b"WAVE" {
            return Err(invalid(
                "audio is not a RIFF/WAVE file — convert to mono 16-bit WAV at 16 or 24 kHz",
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
                    return Err(invalid("WAV fmt chunk is truncated"));
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
            return Err(invalid("WAV has no fmt chunk"));
        };
        if tag != 1 || channels != 1 || bits != 16 || (rate != 16000 && rate != 24000) {
            return Err(invalid(&format!(
                "WAV must be mono 16-bit PCM at 16 or 24 kHz (got format={tag} channels={channels} bits={bits} rate={rate})"
            )));
        }
        let Some(data_len) = data_len else {
            return Err(invalid("WAV has no data chunk"));
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

    /// Transcribe one audio file: two multipart parts, `request`
    /// (JSON settings) and `audio` (the WAV clip), per the
    /// speech-to-text docs. `audio` must pass [`validate_wav_audio`];
    /// the part is sent as `audio/wav` whatever the file name says.
    ///
    /// # Errors
    ///
    /// Returns [`TranscribeError`] when the terms gate blocks real
    /// media, the key is missing, the clip is not a supported WAV
    /// file, or the call/parse fails.
    pub fn transcribe(
        &self,
        policy: &MediaPolicy,
        source: MediaSource,
        file_name: &str,
        audio: &[u8],
        options: &TranscribeOptions,
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
        validate_wav_audio(audio)?;
        let url = format!("{}{TRANSCRIBE_PATH}", self.base_url);
        let request = transcribe_request_json(options);
        let request_body = serde_json::to_string(&request)
            .map_err(|e| TranscribeError::BadResponse(e.to_string()))?;
        let fields = [MultipartField::typed(
            "request",
            &request_body,
            "application/json",
        )];
        let file = MultipartFile {
            field: "audio",
            name: file_name,
            content_type: "audio/wav",
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
    // Official `turns` array first (`turnId`, `startMs`, `endMs`,
    // `transcript`, `speaker`); legacy `segments`/`text` aliases for
    // older replays. `PUSH_TO_TALK` returns no turns at all, so a
    // top-level `transcript` string falls back to one turn.
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
                .ok_or_else(|| TranscribeError::BadResponse("turn without text".to_string()))?;
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
                start_ms: turn_millis(item, &["startMs", "start_ms"], &["start"]),
                end_ms: turn_millis(item, &["endMs", "end_ms"], &["end"]),
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
    Err(TranscribeError::BadResponse(
        "no turns in transcript answer".to_string(),
    ))
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
    InvalidAudio(String),
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
            TranscribeError::InvalidAudio(message) => {
                write!(f, "audio is not a supported WAV file: {message}")
            }
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
        last_file: RefCell<(String, String, String, Vec<u8>)>,
        last_fields: RefCell<Vec<(String, String, Option<String>)>>,
    }

    impl StubTransport {
        fn canned(status: u16, body: &[u8]) -> Self {
            Self {
                response: RefCell::new(HttpResponse {
                    status,
                    body: body.to_vec(),
                }),
                last_url: RefCell::new(String::new()),
                last_file: RefCell::new((String::new(), String::new(), String::new(), Vec::new())),
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
            fields: &[MultipartField],
        ) -> Result<HttpResponse, TransportError> {
            *self.last_url.borrow_mut() = url.to_string();
            *self.last_file.borrow_mut() = (
                file.field.to_string(),
                file.name.to_string(),
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
            Ok(self.response.borrow().clone())
        }
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

    #[test]
    fn file_ingest_posts_request_and_audio_parts() {
        // Contract: `request` JSON part (application/json) + `audio`
        // WAV part (audio/wav), per the speech-to-text docs.
        let answer = serde_json::to_vec(&serde_json::json!({
            "sessionId": "s-1",
            "transcript": "Morning, how can I help? I need an apostille.",
            "audioDurationMs": 4000,
            "turns": [
                {"turnId": 1, "speaker": "A", "transcript": "Morning, how can I help?", "startMs": 0, "endMs": 2100},
                {"turnId": 2, "transcript": "I need an apostille.", "startMs": 2100, "endMs": 4000},
            ],
        }))
        .unwrap();
        let transport = StubTransport::canned(200, &answer);
        let client =
            VoiceFileClient::new(META_API_BASE, Some("key-synthetic".to_string()), transport);
        let audio = wav_bytes(24000, 1, 16, 24000);
        let transcript = client
            .transcribe(
                &MediaPolicy::synthetic_only(),
                MediaSource::SyntheticFixture,
                "call.wav",
                &audio,
                &TranscribeOptions {
                    mode: "DIARIZATION",
                    language_bias: &["English"],
                    keywords: &["apostille"],
                },
            )
            .unwrap();
        assert_eq!(
            *client.transport.last_url.borrow(),
            format!("{META_API_BASE}{TRANSCRIBE_PATH}")
        );
        let file = client.transport.last_file.borrow();
        assert_eq!(file.0, "audio");
        assert_eq!(file.1, "call.wav");
        assert_eq!(file.2, "audio/wav");
        assert_eq!(file.3, audio);
        let fields = client.transport.last_fields.borrow();
        assert_eq!(fields.len(), 1);
        assert_eq!(fields[0].0, "request");
        assert_eq!(fields[0].2.as_deref(), Some("application/json"));
        let request: Value = serde_json::from_str(&fields[0].1).unwrap();
        assert_eq!(
            request,
            serde_json::json!({
                "model": VOICE_MODEL,
                "audioEncoding": "WAV",
                "mode": "DIARIZATION",
                "languageBias": ["English"],
                "keywords": ["apostille"],
            })
        );
        assert_eq!(transcript.turns.len(), 2);
        assert_eq!(transcript.turns[0].speaker.as_deref(), Some("A"));
        assert_eq!(transcript.turns[1].end_ms, 4000);
    }

    #[test]
    fn default_request_json_pins_official_shape() {
        let request = transcribe_request_json(&TranscribeOptions::default());
        assert_eq!(
            request,
            serde_json::json!({
                "model": VOICE_MODEL,
                "audioEncoding": "WAV",
                "mode": "ENDPOINTING",
            })
        );
    }

    #[test]
    fn wav_validator_accepts_16_and_24k_mono() {
        assert_eq!(
            validate_wav_audio(&wav_bytes(16000, 1, 16, 16000)).unwrap(),
            1000
        );
        assert_eq!(
            validate_wav_audio(&wav_bytes(24000, 1, 16, 48000)).unwrap(),
            2000
        );
    }

    #[test]
    fn wav_validator_rejects_wrong_format() {
        for (why, audio) in [
            ("mp3 bytes", b"fake-audio".to_vec()),
            ("stereo", wav_bytes(24000, 2, 16, 100)),
            ("8-bit", wav_bytes(24000, 1, 8, 100)),
            ("44.1 kHz", wav_bytes(44100, 1, 16, 100)),
        ] {
            let err = format!("{}", validate_wav_audio(&audio).unwrap_err());
            assert!(err.contains("WAV"), "{why}: {err}");
        }
    }

    #[test]
    fn wav_validator_rejects_overlong_clips() {
        // Declared data size claims >10 minutes; no big allocation.
        let mut audio = wav_bytes(16000, 1, 16, 100);
        let overlong = (TRANSCRIBE_MAX_DURATION_MS + 1000) * 16000 * 2 / 1000;
        audio[40..44].copy_from_slice(&(overlong as u32).to_le_bytes());
        let err = format!("{}", validate_wav_audio(&audio).unwrap_err());
        assert!(err.contains("10-minute"), "{err}");
    }

    #[test]
    fn real_media_stays_gated() {
        let transport = StubTransport::canned(200, b"{}");
        let client =
            VoiceFileClient::new(META_API_BASE, Some("key-synthetic".to_string()), transport);
        let audio = wav_bytes(16000, 1, 16, 100);
        assert!(matches!(
            client
                .transcribe(
                    &MediaPolicy::synthetic_only(),
                    MediaSource::RealUpload,
                    "call.wav",
                    &audio,
                    &TranscribeOptions::default(),
                )
                .unwrap_err(),
            TranscribeError::TermsNotRead
        ));
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
