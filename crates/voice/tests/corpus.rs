//! Contract tests for the webhook fixture corpus.
//!
//! Spec: `docs/specs/voice-telnyx.md` §11.2. Every file the replay
//! harness (`cargo xtask voice-replay`) references must exist, parse
//! as a §1 envelope, carry synthetic-only values, and honor the
//! dedupe-id relationships (duplicates share `data.id`).

use std::path::PathBuf;

use voice::EventType;

fn webhooks_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("webhooks")
}

fn read(name: &str) -> Vec<u8> {
    std::fs::read(webhooks_dir().join(name))
        .unwrap_or_else(|e| panic!("missing fixture {name}: {e}"))
}

/// Envelope plus raw value (for keys the typed parser drops).
fn parse(name: &str) -> (voice::WebhookEnvelope, serde_json::Value) {
    let body = read(name);
    let envelope = voice::webhook::parse_envelope(&body)
        .unwrap_or_else(|e| panic!("{name} does not parse as a webhook envelope: {e}"));
    let value: serde_json::Value =
        serde_json::from_slice(&body).unwrap_or_else(|e| panic!("{name} is not JSON: {e}"));
    (envelope, value)
}

fn id_of(name: &str) -> String {
    parse(name).0.data.id
}

/// §1 envelope keys plus synthetic-only senders.
fn assert_envelope(name: &str, event: EventType) -> voice::WebhookEnvelope {
    let (envelope, value) = parse(name);
    assert_eq!(value["data"]["record_type"], "event", "{name}");
    assert!(
        value["data"]["occurred_at"].is_string(),
        "{name} lacks occurred_at"
    );
    assert_eq!(EventType::parse(&envelope.data.event_type), event, "{name}");
    assert!(!envelope.data.id.is_empty(), "{name} lacks data.id");
    let payload = &value["data"]["payload"];
    for key in [
        "call_control_id",
        "connection_id",
        "call_leg_id",
        "call_session_id",
        "client_state",
        "from",
        "to",
        "direction",
        "state",
    ] {
        assert!(payload.get(key).is_some(), "{name} payload lacks {key}");
    }
    // Synthetic numbers only: the cohort fixtures rule bans canonical
    // pilot values from checked-in fixtures.
    for key in ["from", "to"] {
        let number = payload[key].as_str().unwrap_or("");
        assert!(
            number.starts_with("+1555"),
            "{name} {key} is not a synthetic +1555 number: {number}"
        );
    }
    assert!(
        !String::from_utf8_lossy(&read(name)).contains("api.telnyx.com"),
        "{name} references the real API host"
    );
    envelope
}

#[test]
fn corpus_files_match_spec_names() {
    for name in [
        "initiated.json",
        "answered.json",
        "transcription-1.json",
        "transcription-2.json",
        "recording-saved.json",
        "hangup.json",
        "duplicate-answered.json",
        "out-of-order-hangup.json",
        "late-answered.json",
        "replay-recording-saved.json",
        "unknown-leg.json",
        "bad-signature.json",
        "machine-detection.json",
        "unknown-event.json",
    ] {
        assert!(
            webhooks_dir().join(name).is_file(),
            "missing corpus file {name}"
        );
    }
}

#[test]
fn happy_path_envelopes() {
    assert_envelope("initiated.json", EventType::Initiated);
    assert_envelope("answered.json", EventType::Answered);
    let first = assert_envelope("transcription-1.json", EventType::Transcription);
    let second = assert_envelope("transcription-2.json", EventType::Transcription);
    // Two distinct live segments (ascending delivery order).
    let a = first.data.payload.segment_text().unwrap_or("");
    let b = second.data.payload.segment_text().unwrap_or("");
    assert!(!a.is_empty() && !b.is_empty());
    assert_ne!(a, b);
    assert_ne!(first.data.id, second.data.id);
    let saved = assert_envelope("recording-saved.json", EventType::RecordingSaved);
    let urls = saved
        .data
        .payload
        .recording_urls
        .expect("recording-saved lacks recording_urls");
    assert!(urls.contains_key("mp3"), "recording-saved lacks an mp3 URL");
    assert_envelope("hangup.json", EventType::Hangup);
}

#[test]
fn duplicate_and_replay_ids_match() {
    // Same `data.id`, so the handler's INSERT OR IGNORE treats the
    // second delivery as a no-op; bodies differ (redelivery attempt),
    // so the dedupe provably keys on the id, not the bytes.
    assert_eq!(id_of("answered.json"), id_of("duplicate-answered.json"));
    assert_ne!(read("answered.json"), read("duplicate-answered.json"));
    assert_eq!(
        id_of("recording-saved.json"),
        id_of("replay-recording-saved.json")
    );
    assert_ne!(
        read("recording-saved.json"),
        read("replay-recording-saved.json")
    );
    // The out-of-order pair carries distinct ids (both apply; state
    // moves forward only).
    assert_ne!(
        id_of("out-of-order-hangup.json"),
        id_of("late-answered.json")
    );
    assert_envelope("out-of-order-hangup.json", EventType::Hangup);
    assert_envelope("late-answered.json", EventType::Answered);
}

#[test]
fn negative_and_edge_envelopes() {
    let unknown = assert_envelope("unknown-leg.json", EventType::Answered);
    let leg = unknown.data.payload.call_control_id.unwrap_or_default();
    assert!(!leg.is_empty());
    assert!(
        !leg.contains("__CALL_CONTROL_ID__"),
        "unknown-leg must keep a fixed unknown leg (no replay token)"
    );
    assert_envelope("bad-signature.json", EventType::Answered);
    assert_envelope("machine-detection.json", EventType::MachineDetectionEnded);
    let edge = assert_envelope(
        "unknown-event.json",
        EventType::Other("call.whisper.started".to_string()),
    );
    assert!(matches!(
        EventType::parse(&edge.data.event_type),
        EventType::Other(_)
    ));
}

#[test]
fn stub_mp3_is_small_and_present() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("stub-recording.mp3");
    let bytes = std::fs::read(&path).expect("missing stub-recording.mp3");
    assert!(!bytes.is_empty());
    assert!(
        bytes.len() < 64 * 1024,
        "stub mp3 exceeds the 64 KiB spec cap: {}",
        bytes.len()
    );
    assert_eq!(&bytes[0..3], b"ID3", "stub mp3 lacks an ID3 header");
}
