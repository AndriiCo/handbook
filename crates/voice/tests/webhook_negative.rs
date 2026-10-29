//! Hermetic negative probes for the Telnyx webhook lane.
//!
//! Spec: `docs/specs/voice-telnyx.md` §§7–8. Each probe drives the
//! real `voice` primitives in the handler order from §3 — freshness
//! → signature → parse envelope → `INSERT OR IGNORE` webhook id →
//! load call by leg → forward-only apply — against in-memory SQLite
//! with in-test Ed25519 keys. No network, no fixtures from disk.
//!
//! Consent (`consent_confirmed`) and the `audit` table live in the
//! web layer; the refusal path is asserted by
//! `crates/web/src/admin_voice.rs`
//! (`dial_refuses_without_consent_and_audits`).

use ed25519_dalek::{Signer, SigningKey};
use rusqlite::Connection;

use voice::{CallStatus, EventType};

/// Same replay window the handler enforces (spec §Signature).
const WINDOW_SECS: i64 = 300;

fn b64(bytes: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// Sign `{timestamp}|{body}` exactly like the Telnyx SDK; returns
/// `(public_key_b64, signature_b64)`.
fn signed(key: &SigningKey, timestamp: &str, body: &[u8]) -> (String, String) {
    let mut message = Vec::with_capacity(timestamp.len() + 1 + body.len());
    message.extend_from_slice(timestamp.as_bytes());
    message.push(b'|');
    message.extend_from_slice(body);
    (
        b64(key.verifying_key().as_bytes()),
        b64(&key.sign(&message).to_bytes()),
    )
}

/// Synthetic fixture key — never a real key.
fn signing_key(seed: u8) -> SigningKey {
    SigningKey::from_bytes(&[seed; 32])
}

fn event_body(id: &str, event: &str, payload: serde_json::Value) -> Vec<u8> {
    serde_json::json!({
        "data": {"record_type": "event", "event_type": event, "id": id,
                 "occurred_at": "2026-10-27T00:00:00Z", "payload": payload},
        "meta": {"attempt": 1},
    })
    .to_string()
    .into_bytes()
}

fn memory_call() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    voice::store::migrate(&conn).unwrap();
    voice::store::insert_call(
        &conn,
        &voice::store::NewCall {
            id: "c-1",
            call_control_id: "leg-1",
            call_leg_id: "leg-1",
            call_session_id: "sess-1",
            to_hash: "abc",
            authority: Some("usa-wa-sos"),
            questions_json: "[]",
            script_id: "inline",
            created_by: "staff-1",
            now: 1,
        },
    )
    .unwrap();
    conn
}

/// Spec §3 order: dedupe on `data.id` → load by leg → forward-only
/// apply. Returns the call's status after the webhook, or `None`
/// when the leg is unknown and the webhook is dropped.
fn deliver(conn: &Connection, envelope: &voice::WebhookEnvelope, now: i64) -> Option<String> {
    let event = EventType::parse(&envelope.data.event_type);
    let leg = envelope
        .data
        .payload
        .call_control_id
        .clone()
        .unwrap_or_default();
    let call = if leg.is_empty() {
        None
    } else {
        voice::store::get_call_by_leg(conn, &leg).unwrap()
    };
    let fresh = voice::store::record_webhook(
        conn,
        &envelope.data.id,
        call.as_ref().map(|c| c.id.as_str()),
        event.as_str(),
        now,
    )
    .unwrap();
    if !fresh {
        // Retried delivery: the receipt already exists, the effect
        // ran once.
        return call.map(|c| c.status);
    }
    let Some(call) = call else {
        // Unknown leg: receipt kept, nothing applied.
        return None;
    };
    if let Some(next) = CallStatus::parse(&call.status).and_then(|s| s.apply_event(&event)) {
        voice::store::set_status(conn, &call.id, next, None, now).unwrap();
    }
    if matches!(event, EventType::Transcription)
        && let Some(text) = envelope.data.payload.segment_text()
        && !text.trim().is_empty()
    {
        voice::store::append_transcript(conn, &call.id, None, text, now).unwrap();
    }
    Some(
        voice::store::get_call(conn, &call.id)
            .unwrap()
            .unwrap()
            .status,
    )
}

/// Full handler-order pipeline from raw bytes: freshness → signature
/// → parse → dedupe → apply. `Err(status)` mirrors the handler's
/// HTTP outcome (401/400); `Ok` carries [`deliver`]'s result (2xx).
fn receive(
    conn: &Connection,
    public: &str,
    timestamp: &str,
    body: &[u8],
    signature: &str,
    now: i64,
) -> Result<Option<String>, u16> {
    if !voice::webhook::timestamp_fresh(timestamp, now, WINDOW_SECS)
        || voice::verify_signature(public, timestamp, body, signature).is_err()
    {
        return Err(401);
    }
    let envelope = voice::webhook::parse_envelope(body).map_err(|_| 400u16)?;
    Ok(deliver(conn, &envelope, now))
}

fn count(conn: &Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |row| row.get(0)).unwrap()
}

fn status(conn: &Connection) -> String {
    voice::store::get_call(conn, "c-1").unwrap().unwrap().status
}

struct Dialer {
    key: SigningKey,
    public: String,
    now: i64,
}

impl Dialer {
    fn new(seed: u8, now: i64) -> Self {
        let key = signing_key(seed);
        let public = b64(key.verifying_key().as_bytes());
        Dialer { key, public, now }
    }

    fn send(
        &self,
        conn: &Connection,
        id: &str,
        event: &str,
        payload: serde_json::Value,
    ) -> Result<Option<String>, u16> {
        self.send_at(conn, id, event, payload, &self.now.to_string())
    }

    fn send_at(
        &self,
        conn: &Connection,
        id: &str,
        event: &str,
        payload: serde_json::Value,
        timestamp: &str,
    ) -> Result<Option<String>, u16> {
        let body = event_body(id, event, payload);
        let (_, signature) = signed(&self.key, timestamp, &body);
        receive(conn, &self.public, timestamp, &body, &signature, self.now)
    }
}

fn leg1() -> serde_json::Value {
    serde_json::json!({"call_control_id": "leg-1"})
}

#[test]
fn duplicate_delivery_applies_effect_once() {
    let conn = memory_call();
    let dialer = Dialer::new(21, 1_761_571_200);

    // Same state webhook twice: second delivery acks, moves nothing.
    assert_eq!(
        dialer.send(&conn, "wh-1", "call.answered", leg1()),
        Ok(Some("in_progress".to_string()))
    );
    assert_eq!(
        dialer.send(&conn, "wh-1", "call.answered", leg1()),
        Ok(Some("in_progress".to_string()))
    );
    assert_eq!(status(&conn), "in_progress");
    assert_eq!(
        count(
            &conn,
            "SELECT COUNT(*) FROM voice_webhook WHERE id = 'wh-1'"
        ),
        1
    );

    // Same artifact webhook twice: one transcript segment, not two.
    let segment = serde_json::json!({"call_control_id": "leg-1", "transcript": "the fee is eight"});
    assert_eq!(
        dialer.send(&conn, "wh-2", "call.transcription", segment.clone()),
        Ok(Some("in_progress".to_string()))
    );
    assert_eq!(
        dialer.send(&conn, "wh-2", "call.transcription", segment),
        Ok(Some("in_progress".to_string()))
    );
    let segments = voice::store::list_transcripts(&conn, "c-1").unwrap();
    assert_eq!(segments.len(), 1);
    assert_eq!(segments[0].text, "the fee is eight");
}

#[test]
fn out_of_order_events_keep_forward_only_status() {
    let conn = memory_call();
    let dialer = Dialer::new(22, 1_761_571_200);

    // Answered lands before initiated: forward jump, then the late
    // initiated acks without regressing.
    assert_eq!(
        dialer.send(&conn, "wh-1", "call.answered", leg1()),
        Ok(Some("in_progress".to_string()))
    );
    assert_eq!(
        dialer.send(&conn, "wh-2", "call.initiated", leg1()),
        Ok(Some("in_progress".to_string()))
    );
    // Terminal first, stragglers after: hangup wins, late events ack
    // as no-ops.
    let hangup = serde_json::json!({"call_control_id": "leg-1", "hangup_cause": "normal"});
    assert_eq!(
        dialer.send(&conn, "wh-3", "call.hangup", hangup),
        Ok(Some("completed".to_string()))
    );
    assert_eq!(
        dialer.send(&conn, "wh-4", "call.answered", leg1()),
        Ok(Some("completed".to_string()))
    );
    assert_eq!(
        dialer.send(&conn, "wh-5", "call.initiated", leg1()),
        Ok(Some("completed".to_string()))
    );
    assert_eq!(status(&conn), "completed");
    // Every delivery leaves a receipt, even the no-ops.
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM voice_webhook"), 5);
}

#[test]
fn terminal_calls_keep_artifacts_not_state() {
    let conn = memory_call();
    let dialer = Dialer::new(23, 1_761_571_200);
    let hangup = serde_json::json!({"call_control_id": "leg-1", "hangup_cause": "normal"});
    assert_eq!(
        dialer.send(&conn, "wh-1", "call.hangup", hangup),
        Ok(Some("completed".to_string()))
    );
    // Post-terminal transcript still appends (artifacts attach to
    // terminal calls; state never moves).
    let segment = serde_json::json!({"call_control_id": "leg-1", "transcript": "goodbye"});
    assert_eq!(
        dialer.send(&conn, "wh-2", "call.transcription", segment),
        Ok(Some("completed".to_string()))
    );
    assert_eq!(status(&conn), "completed");
    let segments = voice::store::list_transcripts(&conn, "c-1").unwrap();
    assert_eq!(segments.len(), 1);
}

#[test]
fn expired_and_replayed_timestamps_rejected() {
    let conn = memory_call();
    let dialer = Dialer::new(24, 1_761_571_200);

    // Window edges: exactly ±300 s passes, ±301 s fails.
    for (stamp, fresh) in [
        ("1761570899", false), // now - 301
        ("1761570900", true),  // now - 300
        ("1761571200", true),  // now
        ("1761571500", true),  // now + 300
        ("1761571501", false), // now + 301
    ] {
        assert_eq!(
            voice::webhook::timestamp_fresh(stamp, dialer.now, WINDOW_SECS),
            fresh,
            "{stamp}"
        );
    }
    for bad in [
        "",
        "not-a-time",
        "+1761571200",
        "1761571200.0",
        " 1761571200",
    ] {
        assert!(
            !voice::webhook::timestamp_fresh(bad, dialer.now, WINDOW_SECS),
            "{bad:?}"
        );
    }

    // A correctly signed but day-old delivery (a replay) is refused
    // and leaves no receipt behind.
    assert_eq!(
        dialer.send_at(&conn, "wh-9", "call.answered", leg1(), "1761484800"),
        Err(401)
    );
    let body = event_body("wh-9", "call.answered", leg1());
    let (_, signature) = signed(&dialer.key, "1761484800", &body);
    // The signature itself is valid — the window is what rejects the
    // replay, so both layers matter.
    assert!(voice::verify_signature(&dialer.public, "1761484800", &body, &signature).is_ok());
    assert_eq!(status(&conn), "queued");
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM voice_webhook"), 0);
}

#[test]
fn bad_signatures_rejected_without_state_change() {
    let conn = memory_call();
    let dialer = Dialer::new(25, 1_761_571_200);
    let stamp = dialer.now.to_string();
    let body = event_body("wh-1", "call.answered", leg1());
    let (_, signature) = signed(&dialer.key, &stamp, &body);

    // Wrong key, tampered body, tampered timestamp, and malformed
    // signatures all refuse alike.
    let wrong = b64(signing_key(26).verifying_key().as_bytes());
    let mut tampered = body.clone();
    tampered[10] ^= 0xff;
    for (public, timestamp, body, signature) in [
        (
            wrong.as_str(),
            stamp.as_str(),
            body.as_slice(),
            signature.as_str(),
        ),
        (
            dialer.public.as_str(),
            stamp.as_str(),
            tampered.as_slice(),
            signature.as_str(),
        ),
        (
            dialer.public.as_str(),
            "1761571201",
            body.as_slice(),
            signature.as_str(),
        ),
        (
            dialer.public.as_str(),
            stamp.as_str(),
            body.as_slice(),
            "!!!",
        ),
        (dialer.public.as_str(), stamp.as_str(), body.as_slice(), ""),
        ("!!!", stamp.as_str(), body.as_slice(), signature.as_str()),
    ] {
        assert_eq!(
            receive(&conn, public, timestamp, body, signature, dialer.now),
            Err(401)
        );
    }
    // The signature covers the exact body bytes: a re-serialized
    // twin (same JSON, new whitespace) parses but never verifies.
    let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let reserialized = serde_json::to_string_pretty(&value).unwrap().into_bytes();
    assert_ne!(reserialized, body);
    assert!(voice::webhook::parse_envelope(&reserialized).is_ok());
    assert!(voice::verify_signature(&dialer.public, &stamp, &reserialized, &signature).is_err());
    assert_eq!(status(&conn), "queued");
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM voice_webhook"), 0);
}

#[test]
fn unknown_leg_is_recorded_and_dropped() {
    let conn = memory_call();
    let dialer = Dialer::new(27, 1_761_571_200);

    // Unknown leg: 2xx, receipt kept with no call link, call untouched.
    let foreign = serde_json::json!({"call_control_id": "leg-unknown"});
    assert_eq!(
        dialer.send(&conn, "wh-1", "call.answered", foreign),
        Ok(None)
    );
    // Missing leg entirely: same drop path.
    assert_eq!(
        dialer.send(&conn, "wh-2", "call.answered", serde_json::json!({})),
        Ok(None)
    );
    // Unknown event type on a known leg: recorded, moves nothing.
    assert_eq!(
        dialer.send(&conn, "wh-3", "call.sparkle", leg1()),
        Ok(Some("queued".to_string()))
    );
    assert_eq!(status(&conn), "queued");
    assert_eq!(
        count(
            &conn,
            "SELECT COUNT(*) FROM voice_webhook WHERE call_id IS NULL"
        ),
        2
    );
    assert_eq!(
        count(
            &conn,
            "SELECT COUNT(*) FROM voice_webhook WHERE id = 'wh-3' AND call_id = 'c-1'"
        ),
        1
    );
    assert!(
        voice::store::list_transcripts(&conn, "c-1")
            .unwrap()
            .is_empty()
    );
}

#[test]
fn webhook_receipts_link_calls_for_key_transitions() {
    let conn = memory_call();
    let dialer = Dialer::new(28, 1_761_571_200);

    let hangup = serde_json::json!({"call_control_id": "leg-1", "hangup_cause": "normal"});
    for (id, event, payload) in [
        ("wh-1", "call.initiated", leg1()),
        ("wh-2", "call.answered", leg1()),
        ("wh-3", "call.hangup", hangup),
    ] {
        dialer.send(&conn, id, event, payload).unwrap();
    }
    // Retried deliveries add no rows.
    dialer.send(&conn, "wh-3", "call.hangup", leg1()).unwrap();
    let rows: Vec<(String, Option<String>, String)> = conn
        .prepare("SELECT id, call_id, event_type FROM voice_webhook ORDER BY id")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(
        rows,
        vec![
            (
                "wh-1".to_string(),
                Some("c-1".to_string()),
                "call.initiated".to_string()
            ),
            (
                "wh-2".to_string(),
                Some("c-1".to_string()),
                "call.answered".to_string()
            ),
            (
                "wh-3".to_string(),
                Some("c-1".to_string()),
                "call.hangup".to_string()
            ),
        ]
    );
}
