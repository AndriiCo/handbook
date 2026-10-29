//! Webhook replay harness: `cargo xtask voice-replay [--corpus <dir>] [--scenario <name>]`.
//!
//! Spec: `docs/specs/voice-telnyx.md` §11.2 (fixture corpus) and
//! §11.3 (stub-Telnyx contract). The harness dials one synthetic
//! call against a port-0 stub Telnyx server, POSTs each scenario
//! fixture through the webhook handler order with a freshly computed
//! Ed25519 signature, and asserts terminal state + row counts.
//!
//! Dependency direction (deliberate): this module depends on `voice`
//! only — never on `web`. The production route (`telnyx_webhook` in
//! `crates/web/src/admin_voice.rs`) lives in a bin-only crate, so
//! the axum `Router` cannot be imported without restructuring the
//! web crate; instead the harness executes the identical handler
//! order step-for-step — freshness → Ed25519 over
//! `{timestamp}|{raw-body}` → parse → `INSERT OR IGNORE` → load by
//! leg → forward-only apply → 2xx, slow work strictly after the ack
//! — using the same `voice` primitives the route calls. The same
//! order is pinned by `crates/voice/tests/webhook_negative.rs`, so
//! a divergence between route and harness fails tests, not review.
//!
//! Documented deviations from a literal route replay:
//!
//! - The stub mp3 fetch runs over plain HTTP to 127.0.0.1.
//!   Production `TelnyxClient::download` and the route's
//!   `pick_recording_url` require `https`, which a localhost stub
//!   cannot satisfy (and a self-signed stub would fail the client's
//!   webpki roots). Ordering (ack recorded before the first fetch
//!   byte), dedupe (replay ⇒ fetch count stays 1), and blob
//!   addressing (real `Store::write_blob`) are identical; the
//!   https-only refusal itself is pinned by the voice unit test
//!   `download_refuses_plain_http`.
//! - Audit rows (`voice_dial`, `voice_hangup`) live in the web
//!   layer's `audit` table, so the harness asserts voice tables
//!   only (call status, webhook receipts, transcript rows, blob).
//! - `machine-detection.json` and `unknown-event.json` are corpus
//!   members but belong to no scenario (the spec's scenario list is
//!   fixed); every run still parses all 14 fixtures, and the voice
//!   `corpus` test pins their shapes.
//!
//! Fixture tokens, substituted into the raw bytes before signing
//! (every fixture uses them except `unknown-leg.json`, whose leg
//! must stay unknown):
//!
//! - `__CALL_CONTROL_ID__` → dialed leg (control + leg ids)
//! - `__CALL_SESSION_ID__` → dialed session id
//! - `__RECORDING_MP3_URL__` / `__RECORDING_WAV_URL__` → stub audio
//!
//! The Ed25519 keypair is generated per run from the OS RNG and never
//! persisted; `bad-signature` signs with a second random key so the
//! 401 comes from a key mismatch, exactly like a forged delivery.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use ed25519_dalek::{Signer, SigningKey};
use handbook_store::store::Store;
use rusqlite::Connection;

/// Replay window the route enforces (spec §Signature).
const WINDOW_SECS: i64 = 300;
/// Every corpus member (§11.2); each run parses all of them.
const CORPUS: [&str; 14] = [
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
];
const LEG_TOKEN: &str = "__CALL_CONTROL_ID__";
const SESSION_TOKEN: &str = "__CALL_SESSION_ID__";
const MP3_TOKEN: &str = "__RECORDING_MP3_URL__";
const WAV_TOKEN: &str = "__RECORDING_WAV_URL__";

/// One swallowed HTTP exchange the stub recorded (never request
/// bodies — they hold phone numbers).
#[derive(Debug, Clone, PartialEq, Eq)]
enum StubAction {
    Dial {
        to: String,
        from: String,
        connection_id: String,
        command_id: String,
    },
    LegAction {
        leg: String,
        action: String,
    },
    Fetch {
        path: String,
    },
}

/// Port-0 stub Telnyx server (§11.3): dial + leg actions + stub
/// audio, with an in-memory action log. Localhost only, one
/// connection at a time (the harness is sequential).
struct Stub {
    base_url: String,
    log: Arc<Mutex<Vec<StubAction>>>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Stub {
    fn start(mp3: Vec<u8>) -> Result<Self, String> {
        let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| format!("stub bind: {e}"))?;
        let port = listener
            .local_addr()
            .map_err(|e| format!("stub addr: {e}"))?
            .port();
        listener
            .set_nonblocking(true)
            .map_err(|e| format!("stub nonblocking: {e}"))?;
        let log = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let thread = std::thread::spawn({
            let log = log.clone();
            let stop = stop.clone();
            move || {
                let mut dials = 0u64;
                loop {
                    if stop.load(Ordering::Relaxed) {
                        break;
                    }
                    match listener.accept() {
                        Ok((stream, _)) => dials = serve_one(stream, &log, &mp3, dials),
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            std::thread::sleep(Duration::from_millis(5));
                        }
                        // Transient backlog aborts (BSD ECONNABORTED
                        // under rapid localhost churn) must not kill
                        // the stub mid-scenario.
                        Err(_) => std::thread::sleep(Duration::from_millis(5)),
                    }
                }
            }
        });
        Ok(Stub {
            base_url: format!("http://127.0.0.1:{port}"),
            log,
            stop,
            thread: Some(thread),
        })
    }

    fn actions(&self) -> Vec<StubAction> {
        self.log.lock().map(|g| g.clone()).unwrap_or_default()
    }
}

impl Drop for Stub {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Serve one stub connection; returns the updated dial counter.
fn serve_one(stream: TcpStream, log: &Mutex<Vec<StubAction>>, mp3: &[u8], dials: u64) -> u64 {
    // Accepted sockets inherit the listener's nonblocking mode on
    // some platforms; reads below are blocking with a timeout.
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
    let mut reader = BufReader::new(stream.try_clone().unwrap_or(stream));
    let mut request_line = String::new();
    if reader.read_line(&mut request_line).is_err() {
        return dials;
    }
    let mut len = 0usize;
    let mut chunked = false;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).is_err() {
            return dials;
        }
        if line.trim().is_empty() {
            break;
        }
        let lower = line.trim().to_ascii_lowercase();
        if let Some(n) = lower
            .strip_prefix("content-length:")
            .and_then(|v| v.trim().parse().ok())
        {
            len = n;
        }
        if lower == "transfer-encoding: chunked" {
            chunked = true;
        }
    }
    let body = if chunked {
        let mut raw = Vec::new();
        loop {
            let mut size_line = String::new();
            if reader.read_line(&mut size_line).is_err() {
                return dials;
            }
            let Ok(size) = usize::from_str_radix(size_line.trim(), 16) else {
                return dials;
            };
            if size == 0 {
                let _ = reader.read_line(&mut String::new());
                break;
            }
            let mut chunk = vec![0u8; size];
            if reader.read_exact(&mut chunk).is_err() {
                return dials;
            }
            raw.extend_from_slice(&chunk);
            let _ = reader.read_line(&mut String::new());
        }
        raw
    } else {
        let mut body = vec![0u8; len.min(1 << 20)];
        if reader.read_exact(&mut body).is_err() {
            return dials;
        }
        body
    };
    let mut stream = reader.into_inner();
    let mut next_dials = dials;
    let (status, ctype, reply) = route(&request_line, &body, log, mp3, &mut next_dials);
    let head = format!(
        "HTTP/1.1 {status} {}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        if status == 200 { "OK" } else { "ERR" },
        reply.len()
    );
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(&reply);
    next_dials
}

/// §11.3 endpoint shapes: dial, leg actions, stub audio.
fn route(
    request_line: &str,
    body: &[u8],
    log: &Mutex<Vec<StubAction>>,
    mp3: &[u8],
    dials: &mut u64,
) -> (u16, &'static str, Vec<u8>) {
    let mut parts = request_line.split_whitespace();
    let (method, path) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
    if method == "POST" && path == "/v2/calls" {
        let value: serde_json::Value = serde_json::from_slice(body).unwrap_or_default();
        let field = |k: &str| value[k].as_str().unwrap_or("").to_string();
        let (to, from, connection_id, command_id) = (
            field("to"),
            field("from"),
            field("connection_id"),
            field("command_id"),
        );
        if voice::calls::validate_e164(&to).is_err() || from.is_empty() {
            return (
                422,
                "application/json",
                br#"{"errors":[{"detail":"bad number"}]}"#.to_vec(),
            );
        }
        if let Ok(mut log) = log.lock() {
            log.push(StubAction::Dial {
                to,
                from,
                connection_id,
                command_id,
            });
        }
        *dials += 1;
        let reply = format!(
            r#"{{"data":{{"call_control_id":"stub-leg-{dials}","call_leg_id":"stub-leg-{dials}","call_session_id":"stub-sess-{dials}"}}}}"#,
            dials = *dials
        );
        return (200, "application/json", reply.into_bytes());
    }
    if method == "POST"
        && let Some(rest) = path.strip_prefix("/v2/calls/")
        && let Some((leg, action)) = rest.split_once("/actions/")
    {
        if let Ok(mut log) = log.lock() {
            log.push(StubAction::LegAction {
                leg: leg.to_string(),
                action: action.to_string(),
            });
        }
        return (200, "application/json", br#"{"data":{"ok":true}}"#.to_vec());
    }
    if method == "GET"
        && (path == "/stub-audio/recording.mp3" || path == "/stub-audio/recording.wav")
    {
        if let Ok(mut log) = log.lock() {
            log.push(StubAction::Fetch {
                path: path.to_string(),
            });
        }
        let ctype = if path.ends_with(".mp3") {
            "audio/mpeg"
        } else {
            "audio/wav"
        };
        return (200, ctype, mp3.to_vec());
    }
    (
        404,
        "application/json",
        br#"{"errors":[{"detail":"no such stub route"}]}"#.to_vec(),
    )
}

/// Plain-HTTP GET against the localhost stub only (the documented
/// https deviation: anything but `http://127.0.0.1:` is refused
/// without a request).
fn stub_get(url: &str) -> Result<Vec<u8>, String> {
    let rest = url
        .strip_prefix("http://127.0.0.1:")
        .ok_or_else(|| "refusing non-stub fetch".to_string())?;
    let (port, path) = rest
        .split_once('/')
        .ok_or_else(|| "bad stub url".to_string())?;
    let mut stream = TcpStream::connect(format!("127.0.0.1:{port}"))
        .map_err(|e| format!("stub fetch connect: {e}"))?;
    stream
        .set_read_timeout(Some(Duration::from_secs(30)))
        .map_err(|e| format!("stub fetch timeout: {e}"))?;
    write!(
        stream,
        "GET /{path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
    )
    .map_err(|e| format!("stub fetch write: {e}"))?;
    let mut raw = Vec::new();
    stream
        .read_to_end(&mut raw)
        .map_err(|e| format!("stub fetch read: {e}"))?;
    let head_end = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or_else(|| "stub fetch: no header end".to_string())?;
    let head = String::from_utf8_lossy(&raw[..head_end]);
    if !head.starts_with("HTTP/1.1 200") {
        let first = head.lines().next().unwrap_or("").to_string();
        return Err(format!("stub fetch status: {first}"));
    }
    Ok(raw[head_end + 4..].to_vec())
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn b64(bytes: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// Sign `{timestamp}|{body}` exactly like the Telnyx SDK.
fn sign(key: &SigningKey, timestamp: &str, body: &[u8]) -> String {
    let mut message = Vec::with_capacity(timestamp.len() + 1 + body.len());
    message.extend_from_slice(timestamp.as_bytes());
    message.push(b'|');
    message.extend_from_slice(body);
    b64(&key.sign(&message).to_bytes())
}

fn random_key() -> SigningKey {
    use rand::RngCore as _;
    let mut seed = [0u8; 32];
    rand::rng().fill_bytes(&mut seed);
    SigningKey::from_bytes(&seed)
}

/// Build the stub client; a base URL pointing at the real API fails
/// loudly instead of dialing (§11.3).
fn stub_client(base_url: &str) -> Result<voice::TelnyxClient, String> {
    if base_url.contains("api.telnyx.com") {
        return Err("refusing to run the harness against the real Telnyx API".to_string());
    }
    Ok(voice::TelnyxClient::new(
        base_url,
        "stub-key-not-secret",
        "conn-synthetic-0000",
        "+15550000000",
    ))
}

/// Slow work the route runs after the 2xx (production: background
/// tasks; harness: executed by the caller once the ack is recorded).
#[derive(Default)]
struct Pending {
    auto_record_leg: Option<String>,
    fetch_recording: Option<(String, String)>,
}

struct Ctx {
    conn: Connection,
    store: Store,
    client: voice::TelnyxClient,
    key: SigningKey,
    public: String,
    wrong_key: SigningKey,
    now: i64,
}

/// One fixture delivery through the route's handler order. Returns
/// the ack code plus the slow work to run after it — the caller
/// records the ack first, so a fetch can never precede its 2xx.
fn deliver(ctx: &Ctx, body: &[u8], wrong_key: bool) -> (u16, Pending) {
    let mut pending = Pending::default();
    let timestamp = now_secs().to_string();
    let signer = if wrong_key { &ctx.wrong_key } else { &ctx.key };
    let signature = sign(signer, &timestamp, body);
    if !voice::webhook::timestamp_fresh(&timestamp, ctx.now, WINDOW_SECS)
        || voice::verify_signature(&ctx.public, &timestamp, body, &signature).is_err()
    {
        return (401, pending);
    }
    let Ok(envelope) = voice::webhook::parse_envelope(body) else {
        return (400, pending);
    };
    let event = voice::EventType::parse(&envelope.data.event_type);
    let leg = envelope
        .data
        .payload
        .call_control_id
        .clone()
        .unwrap_or_default();
    let call = if leg.is_empty() {
        None
    } else {
        voice::store::get_call_by_leg(&ctx.conn, &leg).unwrap_or(None)
    };
    let fresh = voice::store::record_webhook(
        &ctx.conn,
        &envelope.data.id,
        call.as_ref().map(|c| c.id.as_str()),
        event.as_str(),
        ctx.now,
    );
    match (fresh, call) {
        (Ok(false), _) | (Err(_), _) => (200, pending),
        (Ok(true), None) => (200, pending),
        (Ok(true), Some(call)) => {
            apply(&ctx.conn, &call, &event, &envelope, ctx.now, &mut pending);
            (200, pending)
        }
    }
}

/// Fresh-webhook effects, mirroring the route's `apply_event`
/// (state moves, transcript appends; downloads queue as pending).
fn apply(
    conn: &Connection,
    call: &voice::VoiceCall,
    event: &voice::EventType,
    envelope: &voice::WebhookEnvelope,
    now: i64,
    pending: &mut Pending,
) {
    let payload = &envelope.data.payload;
    if let Some(next) = voice::CallStatus::parse(&call.status).and_then(|s| s.apply_event(event)) {
        let detail = match event {
            voice::EventType::Hangup => hangup_detail(payload),
            _ => None,
        };
        let _ = voice::store::set_status(conn, &call.id, next, detail.as_deref(), now);
    }
    match event {
        voice::EventType::Answered => {
            let _ = voice::store::set_recording(conn, &call.id, None, "recording", now);
            pending.auto_record_leg = Some(call.call_control_id.clone());
        }
        voice::EventType::Transcription => {
            if let Some(text) = payload.segment_text()
                && !text.trim().is_empty()
            {
                let _ = voice::store::append_transcript(
                    conn,
                    &call.id,
                    None,
                    &text.chars().take(2000).collect::<String>(),
                    now,
                );
            }
        }
        voice::EventType::RecordingSaved => {
            if let Some(url) = pick_recording_url(payload) {
                pending.fetch_recording = Some((call.id.clone(), url));
            }
        }
        voice::EventType::MachineDetectionEnded => {
            let detail = format!(
                "amd ended: {}",
                payload.state.as_deref().unwrap_or("unknown")
            );
            let _ = voice::store::set_status(
                conn,
                &call.id,
                voice::CallStatus::parse(&call.status).unwrap_or(voice::CallStatus::Ringing),
                Some(detail.as_str()),
                now,
            );
        }
        _ => {}
    }
}

/// Prefer mp3, then wav, then any stub URL the payload offers. No
/// https filter here — the documented deviation: the stub speaks
/// plain HTTP on localhost, and `stub_get` refuses anything else.
fn pick_recording_url(payload: &voice::webhook::EventPayload) -> Option<String> {
    let urls = payload.recording_urls.as_ref()?;
    urls.get("mp3")
        .or_else(|| urls.get("wav"))
        .or_else(|| urls.values().next())
        .filter(|u| u.starts_with("http://127.0.0.1:"))
        .cloned()
}

fn hangup_detail(payload: &voice::webhook::EventPayload) -> Option<String> {
    match (&payload.hangup_cause, &payload.hangup_source) {
        (Some(cause), Some(source)) => Some(format!("{source}: {cause}")),
        (Some(cause), None) => Some(cause.clone()),
        (None, Some(source)) => Some(source.clone()),
        (None, None) => None,
    }
}

/// Post-ack slow work: auto record+transcription start, then the
/// recording fetch into the blob store.
fn run_pending(ctx: &Ctx, pending: Pending) {
    if let Some(leg) = pending.auto_record_leg {
        let command_id = format!("auto-{leg}");
        let ok = ctx
            .client
            .record_start(&leg, &command_id)
            .and_then(|()| ctx.client.transcription_start(&leg, &command_id))
            .is_ok();
        if !ok && let Ok(Some(call)) = voice::store::get_call_by_leg(&ctx.conn, &leg) {
            let _ = voice::store::set_recording(&ctx.conn, &call.id, None, "failed", now_secs());
        }
    }
    if let Some((call_id, url)) = pending.fetch_recording {
        match stub_get(&url).and_then(|bytes| {
            ctx.store
                .write_blob(&bytes)
                .map(|sha| (sha, bytes.len()))
                .map_err(|e| e.to_string())
        }) {
            Ok((sha, _)) if !sha.is_empty() => {
                if voice::store::set_recording(&ctx.conn, &call_id, Some(&sha), "ready", now_secs())
                    .is_err()
                {
                    let _ = voice::store::set_recording(
                        &ctx.conn,
                        &call_id,
                        None,
                        "pending",
                        now_secs(),
                    );
                }
            }
            _ => {
                let _ =
                    voice::store::set_recording(&ctx.conn, &call_id, None, "pending", now_secs());
            }
        }
    }
}

/// One scenario delivery: fixture file plus whether to sign it with
/// the wrong key (the `bad-signature` scenario only).
struct Delivery {
    file: &'static str,
    wrong_key: bool,
}

fn plan(scenario: &str) -> Result<Vec<Delivery>, String> {
    let good = |file: &'static str| Delivery {
        file,
        wrong_key: false,
    };
    match scenario {
        "happy" => Ok([
            "initiated.json",
            "answered.json",
            "transcription-1.json",
            "transcription-2.json",
            "recording-saved.json",
            "hangup.json",
        ]
        .into_iter()
        .map(good)
        .collect()),
        "duplicates" => Ok(
            ["initiated.json", "answered.json", "duplicate-answered.json"]
                .into_iter()
                .map(good)
                .collect(),
        ),
        "out-of-order" => Ok(["out-of-order-hangup.json", "late-answered.json"]
            .into_iter()
            .map(good)
            .collect()),
        "replays" => Ok(["recording-saved.json", "replay-recording-saved.json"]
            .into_iter()
            .map(good)
            .collect()),
        "unknown-leg" => Ok(vec![good("unknown-leg.json")]),
        "bad-signature" => Ok(vec![Delivery {
            file: "bad-signature.json",
            wrong_key: true,
        }]),
        _ => Err(format!(
            "unknown scenario {scenario:?} (want happy|duplicates|out-of-order|replays|unknown-leg|bad-signature)"
        )),
    }
}

fn count(conn: &Connection, sql: &str) -> Result<i64, String> {
    conn.query_row(sql, [], |row| row.get(0))
        .map_err(|e| format!("count: {e}"))
}

fn check<T: PartialEq + std::fmt::Debug>(what: &str, got: T, want: T) -> Result<(), String> {
    if got == want {
        Ok(())
    } else {
        Err(format!("{what}: got {got:?}, want {want:?}"))
    }
}

fn usage() -> String {
    "usage: cargo xtask voice-replay [--corpus <dir>] [--scenario happy|duplicates|out-of-order|replays|unknown-leg|bad-signature]".to_string()
}

/// CLI entry: parse flags, boot the stub, dial, replay, assert.
pub fn run(args: &[String]) {
    match run_inner(args) {
        Ok(()) => {}
        Err(e) => {
            eprintln!("voice-replay: {e}");
            std::process::exit(1);
        }
    }
}

fn run_inner(args: &[String]) -> Result<(), String> {
    let mut corpus = String::from("crates/voice/tests/fixtures/webhooks");
    let mut scenario = String::from("happy");
    let mut rest = args.iter().peekable();
    while let Some(flag) = rest.next() {
        match flag.as_str() {
            "--corpus" => {
                corpus = rest.next().ok_or_else(usage)?.to_string();
            }
            "--scenario" => {
                scenario = rest.next().ok_or_else(usage)?.to_string();
            }
            _ => return Err(usage()),
        }
    }
    let deliveries = plan(&scenario)?;
    let corpus_dir = std::path::PathBuf::from(&corpus);
    for name in CORPUS {
        let body =
            std::fs::read(corpus_dir.join(name)).map_err(|e| format!("corpus {name}: {e}"))?;
        voice::webhook::parse_envelope(&body).map_err(|e| format!("corpus {name}: {e}"))?;
    }
    println!("voice-replay scenario={scenario} corpus={corpus}");
    println!(
        "corpus: {}/{} fixtures parse as §1 envelopes",
        CORPUS.len(),
        CORPUS.len()
    );

    let mp3_path = corpus_dir.join("../stub-recording.mp3");
    let mp3 = std::fs::read(&mp3_path).map_err(|e| format!("stub mp3: {e}"))?;
    let stub = Stub::start(mp3.clone())?;
    if !stub.base_url.starts_with("http://127.0.0.1:") {
        return Err(format!("stub is not localhost: {}", stub.base_url));
    }
    let client = stub_client(&stub.base_url)?;
    let tmp = tempfile::tempdir().map_err(|e| format!("tempdir: {e}"))?;
    let store = Store::new(tmp.path());
    let conn = Connection::open_in_memory().map_err(|e| format!("db: {e}"))?;
    voice::store::migrate(&conn).map_err(|e| format!("migrate: {e}"))?;

    // One synthetic dial (§8 item 1 shape): E.164 validated, then the
    // stub answers a fresh leg. Consent is attested by construction —
    // the harness is the stand-in for the consenting staff dial.
    let to = "+15550000100";
    voice::calls::validate_e164(to).map_err(|e| e.to_string())?;
    let now = now_secs();
    let dial_command = format!("replay-dial-{now}");
    let outcome = client
        .dial(to, &dial_command)
        .map_err(|e| format!("dial: {e}"))?;
    voice::store::insert_call(
        &conn,
        &voice::store::NewCall {
            id: "replay-call-1",
            call_control_id: &outcome.call_control_id,
            call_leg_id: &outcome.call_leg_id,
            call_session_id: &outcome.call_session_id,
            to_hash: "synthetic-no-pii",
            authority: Some("Harborview Permits Office"),
            questions_json: "[]",
            created_by: "voice-replay",
            now,
        },
    )
    .map_err(|e| format!("insert call: {e}"))?;
    println!("dial: {} (command {dial_command})", outcome.call_control_id);

    let key = random_key();
    let public = b64(key.verifying_key().as_bytes());
    let ctx = Ctx {
        conn,
        store,
        client,
        key,
        public,
        wrong_key: random_key(),
        now,
    };
    let mp3_url = format!("{}/stub-audio/recording.mp3", stub.base_url);
    let wav_url = format!("{}/stub-audio/recording.wav", stub.base_url);
    let mut codes: Vec<(&str, u16)> = Vec::new();
    for delivery in &deliveries {
        let raw = std::fs::read(corpus_dir.join(delivery.file))
            .map_err(|e| format!("fixture {}: {e}", delivery.file))?;
        let text = String::from_utf8(raw).map_err(|e| format!("fixture {}: {e}", delivery.file))?;
        let body = text
            .replace(LEG_TOKEN, &outcome.call_control_id)
            .replace(SESSION_TOKEN, &outcome.call_session_id)
            .replace(MP3_TOKEN, &mp3_url)
            .replace(WAV_TOKEN, &wav_url)
            .into_bytes();
        // Ack first, slow work after — the 2xx-before-fetch order is
        // structural here, not a comment.
        let (code, pending) = deliver(&ctx, &body, delivery.wrong_key);
        codes.push((delivery.file, code));
        println!("POST {} -> {code}", delivery.file);
        run_pending(&ctx, pending);
    }

    assert_scenario(&ctx, &scenario, &codes, &stub.actions(), &mp3)?;
    let status = voice::store::get_call(&ctx.conn, "replay-call-1")
        .map_err(|e| format!("get call: {e}"))?
        .map(|c| c.status)
        .unwrap_or_default();
    println!(
        "final: status={status} webhooks={} transcripts={} fetches={}",
        count(&ctx.conn, "SELECT COUNT(*) FROM voice_webhook")?,
        count(&ctx.conn, "SELECT COUNT(*) FROM voice_transcript")?,
        stub.actions()
            .iter()
            .filter(|a| matches!(a, StubAction::Fetch { .. }))
            .count(),
    );
    println!("PASS scenario={scenario}");
    Ok(())
}

/// Terminal state + row counts per scenario (§11.7 items 1–3).
fn assert_scenario(
    ctx: &Ctx,
    scenario: &str,
    codes: &[(&str, u16)],
    actions: &[StubAction],
    mp3: &[u8],
) -> Result<(), String> {
    let status = || {
        voice::store::get_call(&ctx.conn, "replay-call-1")
            .map_err(|e| format!("get call: {e}"))
            .and_then(|c| {
                c.map(|c| c.status)
                    .ok_or_else(|| "call row missing".to_string())
            })
    };
    let webhooks = || count(&ctx.conn, "SELECT COUNT(*) FROM voice_webhook");
    let transcripts = || count(&ctx.conn, "SELECT COUNT(*) FROM voice_transcript");
    let dials = actions
        .iter()
        .filter(|a| matches!(a, StubAction::Dial { .. }))
        .count();
    let leg_action = |name: &str| {
        actions
            .iter()
            .filter(|a| matches!(a, StubAction::LegAction { action, .. } if action == name))
            .count()
    };
    let fetches = actions
        .iter()
        .filter(|a| matches!(a, StubAction::Fetch { .. }))
        .count();
    // Every scenario dials exactly once; the dial is the anchor the
    // fixtures substitute against.
    check("stub dials", dials, 1)?;
    // The blob round-trips: whatever sha the harness stored must read
    // back as the stub mp3 bytes.
    let assert_blob_ready = || -> Result<(), String> {
        let call = voice::store::get_call(&ctx.conn, "replay-call-1")
            .map_err(|e| format!("get call: {e}"))?
            .ok_or_else(|| "call row missing".to_string())?;
        check(
            "recording_status",
            call.recording_status,
            "ready".to_string(),
        )?;
        let sha = call
            .recording_sha
            .ok_or_else(|| "recording sha missing".to_string())?;
        let back = ctx
            .store
            .read_blob(&sha)
            .map_err(|e| format!("read blob: {e}"))?;
        check("blob bytes", back, mp3.to_vec())
    };
    match scenario {
        "happy" => {
            check(
                "ack codes",
                codes.iter().map(|(_, c)| *c).collect::<Vec<_>>(),
                vec![200u16; 6],
            )?;
            check("status", status()?, "completed".to_string())?;
            check("webhooks", webhooks()?, 6)?;
            check("transcripts", transcripts()?, 2)?;
            check("record_start", leg_action("record_start"), 1)?;
            check("transcription_start", leg_action("transcription_start"), 1)?;
            check("mp3 fetches", fetches, 1)?;
            assert_blob_ready()?;
        }
        "duplicates" => {
            check(
                "ack codes",
                codes.iter().map(|(_, c)| *c).collect::<Vec<_>>(),
                vec![200u16; 3],
            )?;
            check("status", status()?, "in_progress".to_string())?;
            // Same `data.id` twice: one receipt, no refire.
            check("webhooks", webhooks()?, 2)?;
            check("transcripts", transcripts()?, 0)?;
            check("record_start", leg_action("record_start"), 1)?;
            check("mp3 fetches", fetches, 0)?;
        }
        "out-of-order" => {
            check(
                "ack codes",
                codes.iter().map(|(_, c)| *c).collect::<Vec<_>>(),
                vec![200u16; 2],
            )?;
            // Hangup first wins; the late answer acks as a no-op and
            // never resurrects the terminal state.
            check("status", status()?, "completed".to_string())?;
            check("webhooks", webhooks()?, 2)?;
            check("transcripts", transcripts()?, 0)?;
        }
        "replays" => {
            check(
                "ack codes",
                codes.iter().map(|(_, c)| *c).collect::<Vec<_>>(),
                vec![200u16; 2],
            )?;
            check("status", status()?, "queued".to_string())?;
            check("webhooks", webhooks()?, 1)?;
            // The replayed delivery re-acks without re-downloading.
            check("mp3 fetches", fetches, 1)?;
            assert_blob_ready()?;
        }
        "unknown-leg" => {
            check(
                "ack codes",
                codes.iter().map(|(_, c)| *c).collect::<Vec<_>>(),
                vec![200u16; 1],
            )?;
            check("status", status()?, "queued".to_string())?;
            check("webhooks", webhooks()?, 1)?;
            check(
                "unlinked receipts",
                count(
                    &ctx.conn,
                    "SELECT COUNT(*) FROM voice_webhook WHERE call_id IS NULL",
                )?,
                1,
            )?;
            check("transcripts", transcripts()?, 0)?;
        }
        "bad-signature" => {
            check(
                "ack codes",
                codes.iter().map(|(_, c)| *c).collect::<Vec<_>>(),
                vec![401u16; 1],
            )?;
            check("status", status()?, "queued".to_string())?;
            check("webhooks", webhooks()?, 0)?;
            check("transcripts", transcripts()?, 0)?;
            check("mp3 fetches", fetches, 0)?;
        }
        _ => return Err(format!("unknown scenario {scenario:?}")),
    }
    Ok(())
}
