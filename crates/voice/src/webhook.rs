//! Telnyx webhook envelope parsing plus Ed25519 verification.
//!
//! Verification needs the exact body bytes Telnyx sent — parse only
//! after the signature checks out.

use std::collections::HashMap;
use std::fmt;

/// Voice API webhook event types this slice handles; anything else
/// parses as [`EventType::Other`] and is logged by type only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EventType {
    Initiated,
    Answered,
    Hangup,
    RecordingSaved,
    Transcription,
    MachineDetectionEnded,
    Other(String),
}

impl EventType {
    /// Parse a Telnyx `event_type` string (unknown → `Other`).
    pub fn parse(input: &str) -> Self {
        match input {
            "call.initiated" => EventType::Initiated,
            "call.answered" => EventType::Answered,
            "call.hangup" => EventType::Hangup,
            "call.recording.saved" => EventType::RecordingSaved,
            "call.transcription" => EventType::Transcription,
            "call.machine.detection.ended" => EventType::MachineDetectionEnded,
            other => EventType::Other(other.to_string()),
        }
    }

    /// Canonical spelling (round-trips through [`EventType::parse`]).
    pub fn as_str(&self) -> &str {
        match self {
            EventType::Initiated => "call.initiated",
            EventType::Answered => "call.answered",
            EventType::Hangup => "call.hangup",
            EventType::RecordingSaved => "call.recording.saved",
            EventType::Transcription => "call.transcription",
            EventType::MachineDetectionEnded => "call.machine.detection.ended",
            EventType::Other(name) => name,
        }
    }
}

/// Top-level webhook envelope (`data` + `meta`).
#[derive(Debug, Clone, serde::Deserialize)]
pub struct WebhookEnvelope {
    pub data: EventData,
}

/// One event: id (dedup key), type, payload.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct EventData {
    pub id: String,
    pub event_type: String,
    pub occurred_at: Option<String>,
    #[serde(default)]
    pub payload: EventPayload,
}

/// Call payload. Every field is optional: Telnyx reuses this shape
/// across events and only some keys appear on each one.
#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct EventPayload {
    pub call_control_id: Option<String>,
    pub call_leg_id: Option<String>,
    pub call_session_id: Option<String>,
    pub state: Option<String>,
    pub hangup_cause: Option<String>,
    pub hangup_source: Option<String>,
    pub recording_urls: Option<HashMap<String, String>>,
    /// Live segment text. Telnyx documents the event but example
    /// payloads disagree on the key, so both spellings parse and
    /// [`EventPayload::segment_text`] prefers `transcript`.
    pub transcript: Option<String>,
    pub text: Option<String>,
    pub transcription_data: Option<TranscriptionData>,
}

impl EventPayload {
    /// Best-effort live segment text (`None` when the payload
    /// carries no recognized text key).
    pub fn segment_text(&self) -> Option<&str> {
        self.transcript
            .as_deref()
            .or(self
                .transcription_data
                .as_ref()
                .and_then(|d| d.transcript.as_deref()))
            .or(self.text.as_deref())
    }
}

/// Nested transcription detail some payloads carry.
#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct TranscriptionData {
    pub transcript: Option<String>,
    pub confidence: Option<f64>,
}

/// Parse a verified webhook body into its envelope.
///
/// # Errors
///
/// [`WebhookError::Parse`] when the body is not a webhook envelope.
pub fn parse_envelope(body: &[u8]) -> Result<WebhookEnvelope, WebhookError> {
    serde_json::from_slice(body).map_err(|e| WebhookError::Parse(format!("webhook body: {e}")))
}

/// Verify a Telnyx Ed25519 webhook signature.
///
/// The signed message is `{timestamp}.{raw-body}` — the
/// Standard-Webhooks shape minus the message id, which Telnyx does
/// not send. This construction is inferred from the Telnyx docs
/// (headers + Standard-Webhooks compatibility note), NOT quoted
/// from a byte-level spec: confirm against a portal test webhook
/// before trusting production traffic (spec OQ-1).
///
/// # Errors
///
/// [`WebhookError::BadSignature`] on malformed inputs or a mismatch
/// (all failures look identical — no oracle for key probing).
pub fn verify_signature(
    public_key_hex: &str,
    timestamp: &str,
    body: &[u8],
    signature_hex: &str,
) -> Result<(), WebhookError> {
    use ed25519_dalek::{Signature, Verifier, VerifyingKey};

    let key_bytes: [u8; 32] = parse_hex(public_key_hex).ok_or(WebhookError::BadSignature)?;
    let sig_bytes: [u8; 64] = parse_hex(signature_hex).ok_or(WebhookError::BadSignature)?;
    let key = VerifyingKey::from_bytes(&key_bytes).map_err(|_| WebhookError::BadSignature)?;
    let signature = Signature::from_bytes(&sig_bytes);
    let mut message = Vec::with_capacity(timestamp.len() + 1 + body.len());
    message.extend_from_slice(timestamp.as_bytes());
    message.push(b'.');
    message.extend_from_slice(body);
    key.verify(&message, &signature)
        .map_err(|_| WebhookError::BadSignature)
}

/// Replay window: the `telnyx-timestamp` header must be within
/// `window_secs` of `now` (both unix seconds).
pub fn timestamp_fresh(timestamp: &str, now: i64, window_secs: i64) -> bool {
    timestamp
        .parse::<i64>()
        .is_ok_and(|at| (now - at).abs() <= window_secs)
}

fn parse_hex<const N: usize>(input: &str) -> Option<[u8; N]> {
    if input.len() != N * 2 {
        return None;
    }
    let mut out = [0u8; N];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&input[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(out)
}

/// Webhook parse/verification failure. `Display` stays opaque on
/// purpose: callers must not distinguish "bad key" from "bad body"
/// in responses (no oracle for key probing).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WebhookError {
    BadSignature,
    Parse(String),
}

impl fmt::Display for WebhookError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WebhookError::BadSignature => f.write_str("invalid webhook signature"),
            WebhookError::Parse(_) => f.write_str("invalid webhook body"),
        }
    }
}

impl std::error::Error for WebhookError {}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    #[test]
    fn signature_round_trip_and_tamper_rejection() {
        let signing = SigningKey::from_bytes(&[7u8; 32]);
        let public = hex(signing.verifying_key().as_bytes());
        let timestamp = "1761571200";
        let body = br#"{"data":{"id":"wh-1"}}"#;
        let mut message = Vec::new();
        message.extend_from_slice(timestamp.as_bytes());
        message.push(b'.');
        message.extend_from_slice(body);
        let signature = hex(&signing.sign(&message).to_bytes());

        assert!(verify_signature(&public, timestamp, body, &signature).is_ok());
        // Tampered body, timestamp, signature, or key all fail alike.
        assert!(verify_signature(&public, timestamp, b"{}", &signature).is_err());
        assert!(verify_signature(&public, "1761571201", body, &signature).is_err());
        let mut bad_sig = signature.clone();
        bad_sig.replace_range(0..2, "ff");
        assert!(verify_signature(&public, timestamp, body, &bad_sig).is_err());
        assert!(verify_signature(&public, timestamp, body, "zz").is_err());
        assert!(verify_signature("zz", timestamp, body, &signature).is_err());
    }

    #[test]
    fn replay_window() {
        assert!(timestamp_fresh("1000", 1000, 300));
        assert!(timestamp_fresh("900", 1000, 300));
        assert!(!timestamp_fresh("100", 1000, 300));
        assert!(!timestamp_fresh("not-a-time", 1000, 300));
    }

    #[test]
    fn envelope_fixtures_parse() {
        let body = br#"{
            "data": {"record_type": "event", "event_type": "call.recording.saved",
                     "id": "wh-9", "occurred_at": "2026-10-27T00:00:00Z",
                     "payload": {"call_control_id": "leg-1", "call_session_id": "sess-1",
                                 "recording_urls": {"mp3": "https://example.test/r.mp3"}}},
            "meta": {"attempt": 1, "delivered_to": "https://example.test/wh"}}"#;
        let envelope = parse_envelope(body).unwrap();
        assert_eq!(envelope.data.id, "wh-9");
        assert_eq!(
            EventType::parse(&envelope.data.event_type),
            EventType::RecordingSaved
        );
        let urls = envelope.data.payload.recording_urls.unwrap();
        assert_eq!(urls["mp3"], "https://example.test/r.mp3");

        let segment = br#"{"data": {"id": "wh-10", "event_type": "call.transcription",
            "payload": {"call_control_id": "leg-1", "transcript": "hello"}}
        }"#;
        let envelope = parse_envelope(segment).unwrap();
        assert_eq!(envelope.data.payload.segment_text(), Some("hello"));

        assert!(parse_envelope(b"not json").is_err());
        assert!(parse_envelope(br#"{"data":{"id":"x"}}"#).is_err());
    }

    #[test]
    fn unknown_events_stay_transparent() {
        assert_eq!(
            EventType::parse("call.sparkle"),
            EventType::Other("call.sparkle".to_string())
        );
        assert_eq!(
            EventType::Other("call.sparkle".to_string()).as_str(),
            "call.sparkle"
        );
    }
}
