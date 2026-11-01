//! Admin voice lane: staff dial/record/hangup plus the Telnyx webhook.
//!
//! ADMIN ROUTER ONLY (goal-025 stream 2; mounted from `main` inside
//! the ADMIN-VOICE block, never on the public listener). Staff
//! routes take [`AdminAuth`] (staff role) behind the shared host
//! guard + CSRF recipe; the Telnyx webhook takes no session and
//! authenticates by Ed25519 signature in-handler. Nothing here logs
//! phone numbers, transcript text, or key material.

use axum::{
    Router,
    body::Bytes,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Json},
    routing::{get, post},
};
use handbook_store::{db, store::Store};
use serde_json::json;
// Question bounds live in the voice pack rules, so inline lists and
// packs share them (§11.1).
use voice::scripts::{MAX_QUESTION_CHARS, MAX_QUESTIONS};

use crate::{
    AppState,
    admin::{AdminAuth, admin_host_guard, csrf_guard, require_client_cert},
    store_unavailable, unix_now,
};

/// Telnyx signature replay window (seconds).
const WEBHOOK_WINDOW_SECS: i64 = 300;

/// Staff routes: dial, list, detail, hangup, record, questions.
/// Host guard + CSRF layers are applied here (not inherited from
/// `admin::router`), so the `main` merge keeps the recipe whatever
/// the merge-layer semantics are.
pub fn staff_router(state: AppState) -> Router {
    Router::new()
        .route("/apostille/api/v1/calls/dial", post(dial))
        .route("/apostille/api/v1/calls", get(list_calls))
        .route("/apostille/api/v1/calls/{id}", get(get_call))
        .route("/apostille/api/v1/calls/{id}/hangup", post(hangup))
        .route(
            "/apostille/api/v1/calls/{id}/record/start",
            post(record_start),
        )
        .route(
            "/apostille/api/v1/calls/{id}/record/stop",
            post(record_stop),
        )
        .route(
            "/apostille/api/v1/calls/{id}/questions/{idx}/asked",
            post(mark_asked),
        )
        .route("/apostille/api/v1/calls/{id}/quote", post(quote_to_draft))
        .route(
            "/apostille/api/v1/calls/{id}/segments/{seq}/fix",
            post(fix_segment),
        )
        .route(
            "/apostille/api/v1/calls/{id}/segments/{seq}/annotate",
            post(annotate_segment),
        )
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            csrf_guard,
        ))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            require_client_cert,
        ))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            admin_host_guard,
        ))
        .layer(axum::middleware::from_fn_with_state(
            "admin",
            crate::request_id_and_log,
        ))
        .with_state(state)
}

/// Telnyx webhook receiver: host guard only (no staff session —
/// Telnyx authenticates by signature), NO CSRF layer (the
/// server-to-server POST carries no Origin or session cookies),
/// and NO client-cert layer (Telnyx presents no cert).
pub fn webhook_router(state: AppState) -> Router {
    Router::new()
        .route("/apostille/api/v1/calls/webhook", post(telnyx_webhook))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            admin_host_guard,
        ))
        .layer(axum::middleware::from_fn_with_state(
            "admin",
            crate::request_id_and_log,
        ))
        .with_state(state)
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)] // m3: mass-assignment hygiene on mutating DTOs
struct DialBody {
    to: String,
    authority: Option<String>,
    #[serde(default)]
    questions: Vec<String>,
    /// Script-pack id (§11.1); inline `questions` win over it, and
    /// an empty/missing value reads as the `default` pack.
    #[serde(default)]
    script: Option<String>,
    consent_confirmed: bool,
}

/// Dial a number (E.164) from the account number. Requires
/// `consent_confirmed` — staff attests the callee will be told the
/// call is recorded (Washington is all-party consent).
async fn dial(
    auth: AdminAuth,
    State(state): State<AppState>,
    Json(body): Json<DialBody>,
) -> impl IntoResponse {
    let conn = match db::open(&Store::new(&state.store_root)) {
        Ok(conn) => conn,
        Err(_) => return store_unavailable(),
    };
    let now = unix_now();
    if !body.consent_confirmed {
        audit(
            &conn,
            now,
            &auth.user.user_id,
            "voice_dial_refused",
            "",
            "consent missing",
        );
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"error": "consent_confirmed is required"})),
        )
            .into_response();
    }
    if voice::calls::validate_e164(&body.to).is_err() {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"error": "to must be E.164 (+ plus 7-15 digits)"})),
        )
            .into_response();
    }
    // Question resolution (§11.1): inline questions win over any
    // pack; otherwise the named pack (or `default` when none is
    // named) fills in. A missing/broken pack refuses the dial here,
    // before anything leaves the box.
    let (script_id, resolved): (String, Vec<String>) = if !body.questions.is_empty() {
        if body.questions.len() > MAX_QUESTIONS
            || body
                .questions
                .iter()
                .any(|q| q.trim().is_empty() || q.chars().count() > MAX_QUESTION_CHARS)
        {
            return (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(json!({"error": "1-20 questions, each 1-280 chars"})),
            )
                .into_response();
        }
        (
            voice::scripts::INLINE_SCRIPT_ID.to_string(),
            body.questions
                .iter()
                .map(|q| q.trim().to_string())
                .collect(),
        )
    } else {
        let stem = body
            .script
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or(voice::scripts::DEFAULT_PACK_ID);
        match voice::scripts::load_pack(&voice::scripts::resolve_dir(&state.voice_cfg), stem) {
            Ok(pack) => (pack.id, pack.questions),
            Err(_) => {
                audit(
                    &conn,
                    now,
                    &auth.user.user_id,
                    "voice_dial_refused",
                    "",
                    &format!("script pack refused: {stem}"),
                );
                return (
                    StatusCode::UNPROCESSABLE_ENTITY,
                    Json(json!({"error": "unknown or invalid script pack"})),
                )
                    .into_response();
            }
        }
    };
    let client = match voice_client(&state) {
        Some(client) => client,
        None => return voice_unconfigured(),
    };
    let to = body.to.clone();
    let command_id = format!("dial-{now}");
    let outcome = tokio::task::spawn_blocking(move || client.dial(&to, &command_id)).await;
    let outcome = match outcome {
        Ok(Ok(outcome)) => outcome,
        Ok(Err(e)) => {
            tracing::warn!("voice dial failed: {e}");
            return (
                StatusCode::BAD_GATEWAY,
                Json(json!({"error": "telnyx dial failed"})),
            )
                .into_response();
        }
        Err(_) => return store_unavailable(),
    };
    let call_id = new_call_id();
    let questions: Vec<voice::store::Question> = resolved
        .iter()
        .map(|text| voice::store::Question {
            text: text.clone(),
            asked: false,
        })
        .collect();
    let questions_json = serde_json::to_string(&questions).unwrap_or_else(|_| "[]".to_string());
    let call = voice::store::NewCall {
        id: &call_id,
        call_control_id: &outcome.call_control_id,
        call_leg_id: &outcome.call_leg_id,
        call_session_id: &outcome.call_session_id,
        to_hash: &sha256_hex(body.to.as_bytes()),
        authority: body.authority.as_deref(),
        questions_json: &questions_json,
        script_id: &script_id,
        created_by: &auth.user.user_id,
        now,
    };
    if voice::store::insert_call(&conn, &call).is_err() {
        return store_unavailable();
    }
    audit(
        &conn,
        now,
        &auth.user.user_id,
        "voice_dial",
        &call_id,
        status_detail(&body.authority),
    );
    (
        StatusCode::OK,
        Json(json!({"call_id": call_id, "status": "queued"})),
    )
        .into_response()
}

/// Recent calls, newest first (metadata + question counts only).
async fn list_calls(_auth: AdminAuth, State(state): State<AppState>) -> impl IntoResponse {
    let conn = match db::open(&Store::new(&state.store_root)) {
        Ok(conn) => conn,
        Err(_) => return store_unavailable(),
    };
    let calls = match voice::store::list_calls(&conn, 50) {
        Ok(calls) => calls,
        Err(_) => return store_unavailable(),
    };
    let rendered: Vec<serde_json::Value> = calls
        .iter()
        .map(|call| {
            let questions: Vec<voice::store::Question> =
                serde_json::from_str(&call.questions_json).unwrap_or_default();
            json!({
                "call_id": call.id,
                "status": call.status,
                "authority": call.authority,
                "started_at": call.created_at,
                "ended_at": call.ended_at,
                "recording_status": call.recording_status,
                "questions_asked": questions.iter().filter(|q| q.asked).count(),
                "questions_total": questions.len(),
            })
        })
        .collect();
    (StatusCode::OK, Json(json!({"calls": rendered}))).into_response()
}

/// One call: status, questions, transcript segments, recording state.
/// The UI polls this every 2 s while the call is non-terminal.
async fn get_call(
    _auth: AdminAuth,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let conn = match db::open(&Store::new(&state.store_root)) {
        Ok(conn) => conn,
        Err(_) => return store_unavailable(),
    };
    let Some(call) = voice::store::get_call(&conn, &id).unwrap_or(None) else {
        return not_found();
    };
    let segments = match voice::store::list_transcripts(&conn, &id) {
        Ok(segments) => segments,
        Err(_) => return store_unavailable(),
    };
    let questions: Vec<voice::store::Question> =
        serde_json::from_str(&call.questions_json).unwrap_or_default();
    (
        StatusCode::OK,
        Json(json!({
            "call_id": call.id,
            "status": call.status,
            "status_detail": call.status_detail,
            "authority": call.authority,
            "script_id": call.script_id,
            "questions": questions,
            "transcript": segments.iter().map(|s| json!({
                "seq": s.seq, "speaker": s.speaker, "text": s.text, "at": s.at,
            })).collect::<Vec<_>>(),
            "recording_status": call.recording_status,
            "created_at": call.created_at,
            "ended_at": call.ended_at,
        })),
    )
        .into_response()
}

/// Hang up a call (answered or still ringing).
async fn hangup(
    auth: AdminAuth,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let conn = match db::open(&Store::new(&state.store_root)) {
        Ok(conn) => conn,
        Err(_) => return store_unavailable(),
    };
    let Some(call) = voice::store::get_call(&conn, &id).unwrap_or(None) else {
        return not_found();
    };
    let client = match voice_client(&state) {
        Some(client) => client,
        None => return voice_unconfigured(),
    };
    let leg = call.call_control_id.clone();
    let command_id = format!("hup-{leg}");
    let hung = tokio::task::spawn_blocking(move || client.hangup(&leg, &command_id)).await;
    if !matches!(hung, Ok(Ok(()))) {
        tracing::warn!(call_id = %id, "voice hangup command failed");
        return (
            StatusCode::BAD_GATEWAY,
            Json(json!({"error": "telnyx hangup failed"})),
        )
            .into_response();
    }
    let now = unix_now();
    let _ = voice::store::set_status(
        &conn,
        &id,
        voice::CallStatus::Completed,
        Some("staff hangup"),
        now,
    );
    audit(
        &conn,
        now,
        &auth.user.user_id,
        "voice_hangup",
        &id,
        "staff hangup",
    );
    (
        StatusCode::OK,
        Json(json!({"call_id": id, "status": "completed"})),
    )
        .into_response()
}

/// Start recording a live leg (staff-initiated; consent was
/// attested at dial).
async fn record_start(
    auth: AdminAuth,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    record_command(auth, &state, &id, true).await
}

/// Stop recording a live leg.
async fn record_stop(
    auth: AdminAuth,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    record_command(auth, &state, &id, false).await
}

async fn record_command(
    auth: AdminAuth,
    state: &AppState,
    id: &str,
    start: bool,
) -> axum::response::Response {
    let conn = match db::open(&Store::new(&state.store_root)) {
        Ok(conn) => conn,
        Err(_) => return store_unavailable(),
    };
    let Some(call) = voice::store::get_call(&conn, id).unwrap_or(None) else {
        return not_found();
    };
    let client = match voice_client(state) {
        Some(client) => client,
        None => return voice_unconfigured(),
    };
    let leg = call.call_control_id.clone();
    let command_id = format!("rec-{leg}");
    let ran = tokio::task::spawn_blocking(move || {
        if start {
            client.record_start(&leg, &command_id)
        } else {
            client.record_stop(&leg, &command_id)
        }
    })
    .await;
    if !matches!(ran, Ok(Ok(()))) {
        tracing::warn!(call_id = %id, start, "voice record command failed");
        return (
            StatusCode::BAD_GATEWAY,
            Json(json!({"error": "telnyx record command failed"})),
        )
            .into_response();
    }
    let now = unix_now();
    let _ = voice::store::set_recording(
        &conn,
        id,
        call.recording_sha.as_deref(),
        if start { "recording" } else { "stopped" },
        now,
    );
    let action = if start {
        "voice_record_start"
    } else {
        "voice_record_stop"
    };
    audit(&conn, now, &auth.user.user_id, action, id, "");
    (
        StatusCode::OK,
        Json(json!({"call_id": id, "recording": start})),
    )
        .into_response()
}

/// Mark one prepared question asked.
async fn mark_asked(
    _auth: AdminAuth,
    State(state): State<AppState>,
    Path((id, idx)): Path<(String, usize)>,
) -> impl IntoResponse {
    let conn = match db::open(&Store::new(&state.store_root)) {
        Ok(conn) => conn,
        Err(_) => return store_unavailable(),
    };
    match voice::store::mark_question_asked(&conn, &id, idx, unix_now()) {
        Ok(true) => (StatusCode::OK, Json(json!({"call_id": id, "asked": true}))).into_response(),
        Ok(false) => not_found(),
        Err(_) => store_unavailable(),
    }
}

/// Telnyx → us. Verifies the Ed25519 signature, persists the webhook
/// id (retried deliveries are no-ops), applies the event, and
/// answers 2xx fast; the recording download runs afterwards on a
/// background task. Unknown legs → 2xx + drop (log ids only).
async fn telnyx_webhook(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    let unauthorized = || {
        (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error": "unauthorized"})),
        )
            .into_response()
    };
    let public_key = &state.voice_cfg.webhook_public_key;
    if public_key.is_empty() {
        return unauthorized();
    }
    let timestamp = headers
        .get("telnyx-timestamp")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let signature = headers
        .get("telnyx-signature-ed25519")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let now = unix_now();
    if !voice::webhook::timestamp_fresh(timestamp, now, WEBHOOK_WINDOW_SECS)
        || voice::verify_signature(public_key, timestamp, &body, signature).is_err()
    {
        return unauthorized();
    }
    let Ok(envelope) = voice::webhook::parse_envelope(&body) else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "bad webhook body"})),
        )
            .into_response();
    };
    let event = voice::EventType::parse(&envelope.data.event_type);
    let conn = match db::open(&Store::new(&state.store_root)) {
        Ok(conn) => conn,
        Err(_) => return store_unavailable(),
    };
    let leg = envelope
        .data
        .payload
        .call_control_id
        .clone()
        .unwrap_or_default();
    let call = if leg.is_empty() {
        None
    } else {
        voice::store::get_call_by_leg(&conn, &leg).unwrap_or(None)
    };
    let call_id = call.as_ref().map(|c| c.id.as_str());
    let fresh =
        voice::store::record_webhook(&conn, &envelope.data.id, call_id, event.as_str(), now);
    match (fresh, call) {
        // Retried delivery: already applied, stay 2xx.
        (Ok(false), _) | (Err(_), _) => return ack(),
        (Ok(true), None) => {
            tracing::info!(event = event.as_str(), "voice webhook for unknown leg");
            return ack();
        }
        (Ok(true), Some(call)) => apply_event(&conn, &state, &call, &event, &envelope, now),
    }
    ack()
}

/// Apply one fresh webhook to its call row (state moves, transcript
/// appends, background recording download).
fn apply_event(
    conn: &rusqlite::Connection,
    state: &AppState,
    call: &voice::VoiceCall,
    event: &voice::EventType,
    envelope: &voice::WebhookEnvelope,
    now: i64,
) {
    let payload = &envelope.data.payload;
    if let Some(next) = voice::CallStatus::parse(&call.status).and_then(|s| s.apply_event(event)) {
        let detail = match event {
            voice::EventType::Hangup => hangup_detail(payload),
            _ => None,
        };
        let _ = voice::store::set_status(conn, &call.id, next, detail.as_deref(), now);
        if matches!(event, voice::EventType::Hangup) {
            audit(
                conn,
                now,
                "telnyx",
                "voice_hangup",
                &call.id,
                detail.as_deref().unwrap_or(""),
            );
        }
    }
    match event {
        voice::EventType::Answered => {
            // Consent was attested at dial (dial refuses without
            // it), so recording + transcription start at answer.
            let _ = voice::store::set_recording(conn, &call.id, None, "recording", now);
            spawn_auto_record(state, &call.call_control_id);
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
                spawn_download(state, &call.id, &url);
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
    tracing::info!(call_id = %call.id, event = event.as_str(), "voice webhook applied");
}

/// Fire-and-forget `record_start` + `transcription_start` at answer.
/// Failures land in `recording_status` for staff to retry; the
/// webhook itself already acked.
fn spawn_auto_record(state: &AppState, leg: &str) {
    let Some(client) = voice_client(state) else {
        return;
    };
    let store_root = state.store_root.clone();
    let command_id = format!("auto-{leg}");
    let call_leg = leg.to_string();
    tokio::spawn(async move {
        let outcome = tokio::task::spawn_blocking({
            let leg = call_leg.clone();
            let command_id = command_id.clone();
            move || {
                client
                    .record_start(&leg, &command_id)
                    .and_then(|()| client.transcription_start(&leg, &command_id))
            }
        })
        .await;
        if !matches!(outcome, Ok(Ok(()))) {
            tracing::warn!("voice auto-record failed");
            let store = Store::new(&store_root);
            if let Ok(conn) = db::open(&store)
                && let Ok(Some(call)) = voice::store::get_call_by_leg(&conn, &call_leg)
            {
                let _ = voice::store::set_recording(&conn, &call.id, None, "failed", unix_now());
            }
        }
    });
}

/// Fire-and-forget recording download after `call.recording.saved`.
/// The Telnyx URL is a bearer URL: fetch it now, persist only the
/// content blob sha.
fn spawn_download(state: &AppState, call_id: &str, url: &str) {
    let Some(client) = voice_client(state) else {
        return;
    };
    let url = url.to_string();
    let call_id = call_id.to_string();
    let store_root = state.store_root.clone();
    tokio::spawn(async move {
        let bytes = tokio::task::spawn_blocking(move || client.download(&url)).await;
        let store = Store::new(&store_root);
        let Ok(conn) = db::open(&store) else {
            return;
        };
        match bytes {
            Ok(Ok(bytes)) => {
                let sha = store.write_blob(&bytes).unwrap_or_default();
                let ok = !sha.is_empty()
                    && voice::store::set_recording(
                        &conn,
                        &call_id,
                        Some(&sha),
                        "ready",
                        unix_now(),
                    )
                    .is_ok();
                tracing::info!(
                    call_id = %call_id,
                    bytes = bytes.len(),
                    stored = ok,
                    "voice recording downloaded"
                );
                if !ok {
                    let _ =
                        voice::store::set_recording(&conn, &call_id, None, "pending", unix_now());
                }
            }
            _ => {
                tracing::warn!(call_id = %call_id, "voice recording download failed");
                let _ = voice::store::set_recording(&conn, &call_id, None, "pending", unix_now());
            }
        }
    });
}

/// Prefer mp3, then wav, then any https URL the payload offers.
fn pick_recording_url(payload: &voice::webhook::EventPayload) -> Option<String> {
    let urls = payload.recording_urls.as_ref()?;
    urls.get("mp3")
        .or_else(|| urls.get("wav"))
        .or_else(|| urls.values().next())
        .filter(|u| u.starts_with("https://"))
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

/// Client from state config + sealed key (`None` → 503 fail-closed).
fn voice_client(state: &AppState) -> Option<voice::TelnyxClient> {
    let key = state
        .voice_api_key
        .as_ref()
        .map(config::SecretText::as_str)
        .unwrap_or_default();
    if key.is_empty()
        || state.voice_cfg.connection_id.is_empty()
        || state.voice_cfg.from_number.is_empty()
    {
        return None;
    }
    Some(voice::TelnyxClient::new(
        &state.voice_cfg.base_url,
        key,
        &state.voice_cfg.connection_id,
        &state.voice_cfg.from_number,
    ))
}

/// Voice audit row: ids + labels only, never numbers or transcripts.
fn audit(
    conn: &rusqlite::Connection,
    now: i64,
    actor: &str,
    action: &str,
    call_id: &str,
    reason: &str,
) {
    let _ = conn.execute(
        "INSERT INTO audit(at, actor, action, entity, entity_id, after_ref, reason)
         VALUES (?1, ?2, ?3, 'voice_call', 0, ?4, ?5)",
        rusqlite::params![now, actor, action, call_id, reason],
    );
}

fn status_detail(authority: &Option<String>) -> &str {
    authority.as_deref().unwrap_or("")
}

fn new_call_id() -> String {
    let bytes = rand::random::<[u8; 16]>();
    let mut id = String::with_capacity(32);
    for b in bytes {
        id.push_str(&format!("{b:02x}"));
    }
    id
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::Digest as _;
    let mut hasher = sha2::Sha256::new();
    hasher.update(bytes);
    let mut out = String::with_capacity(64);
    for b in hasher.finalize() {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

fn ack() -> axum::response::Response {
    (StatusCode::OK, Json(json!({"ok": true}))).into_response()
}

fn not_found() -> axum::response::Response {
    (StatusCode::NOT_FOUND, Json(json!({"error": "not found"}))).into_response()
}

fn voice_unconfigured() -> axum::response::Response {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(json!({"error": "voice not configured"})),
    )
        .into_response()
}

/// Max statement length, in chars (contract).
const MAX_STATEMENT_CHARS: usize = 1024;

/// Unattributed-fact capture id: voice has no capture row. Same value as
/// `handbook-agent`'s `UNATTRIBUTED_CAPTURE_ID`; migration 005's sentinel
/// capture row (id 0) backs the FK.
const UNATTRIBUTED_CAPTURE_ID: i64 = 0;

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)] // m3: mass-assignment hygiene on mutating DTOs
struct QuoteBody {
    segment_ids: Vec<i64>,
    topic: String,
    statement: String,
}

/// Quote transcript segments into ONE proposed fact ("quote to draft").
///
/// Staff-only (AdminAuth + host guard + CSRF via `staff_router`).
/// Attribution: `capture_id = 0` (no capture row for voice) +
/// `created_by = staff:<user>`; the audit row carries call+segment refs
/// in its grep-able reason (zero DDL, the `voice_*` audit pattern).
async fn quote_to_draft(
    auth: AdminAuth,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<QuoteBody>,
) -> impl IntoResponse {
    let conn = match db::open(&Store::new(&state.store_root)) {
        Ok(conn) => conn,
        Err(_) => return store_unavailable(),
    };
    let Some(call) = voice::store::get_call(&conn, &id).unwrap_or(None) else {
        return not_found();
    };
    if body.segment_ids.is_empty() {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"error": "segment_ids must name at least one segment"})),
        )
            .into_response();
    }
    let segments = match voice::store::list_transcripts(&conn, &id) {
        Ok(segments) => segments,
        Err(_) => return store_unavailable(),
    };
    let have: std::collections::BTreeSet<i64> = segments.iter().map(|s| s.seq).collect();
    if !body.segment_ids.iter().all(|seq| have.contains(seq)) {
        return not_found();
    }
    let topic = match contracts::topics::Topic::parse(&body.topic) {
        Ok(topic) => topic,
        Err(_) => {
            return (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(json!({"error": "unknown handbook topic"})),
            )
                .into_response();
        }
    };
    let statement = body.statement.trim();
    let chars = statement.chars().count();
    if chars == 0 || chars > MAX_STATEMENT_CHARS {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"error": "statement must be 1-1024 chars"})),
        )
            .into_response();
    }
    // The call's authority slug owns the fact; without one there is no
    // authority to attribute (422, not a guess).
    let authority_id: Option<i64> = call.authority.as_deref().and_then(|slug| {
        conn.query_row("SELECT id FROM authority WHERE slug = ?1", [slug], |row| {
            row.get(0)
        })
        .ok()
    });
    let Some(authority_id) = authority_id else {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"error": "call has no known authority to quote into"})),
        )
            .into_response();
    };
    let now = unix_now();
    let created_by = format!("staff:{}", auth.user.user_id);
    let fact_id = match handbook_store::db::insert_proposed_fact(
        &conn,
        &handbook_store::db::NewProposedFact {
            authority_id,
            capture_id: UNATTRIBUTED_CAPTURE_ID,
            topic: topic.as_str(),
            statement,
            created_by: &created_by,
            created_at: now,
            // Staff-quoted (not analyzer-drafted): uncited, no summary.
            citation: None,
            reasoning: "",
        },
    ) {
        Ok(fact_id) => fact_id,
        Err(_) => return store_unavailable(),
    };
    // Zero-DDL attribution audit: the existing voice audit row, with the
    // fact id + segment seqs + topic in the grep-able reason. Never the
    // transcript text (no PII in audit, per the voice lane rule).
    let seqs: Vec<String> = body.segment_ids.iter().map(|s| s.to_string()).collect();
    audit(
        &conn,
        now,
        &auth.user.user_id,
        "voice_quote",
        &id,
        &format!(
            "fact_id={fact_id} segments=[{}] topic={}",
            seqs.join(","),
            topic.as_str()
        ),
    );
    (
        StatusCode::OK,
        Json(json!({
            "fact_id": fact_id,
            "call_id": id,
            "topic": topic.as_str(),
            "status": "proposed",
        })),
    )
        .into_response()
}

/// Max annotation note length, in chars (spec §11.4 contract).
const MAX_NOTE_CHARS: usize = 1024;

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)] // m3: mass-assignment hygiene on mutating DTOs
struct FixBody {
    text: String,
    author: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)] // m3: mass-assignment hygiene on mutating DTOs
struct AnnotateBody {
    kind: String,
    note: String,
    author: Option<String>,
}

/// Fix one transcript segment's text (spec §11.4).
///
/// Semantics are literally `media::transcript::apply_fix`: the segment
/// keeps its seq/speaker/timing, only `text` changes (trimmed, blank
/// rejected), and a `Correction` annotation quoting before → after is
/// pinned to the segment. The corrected text lands in place in
/// `voice_transcript`; the text rewrite and the annotation commit
/// atomically (the annotation preserves the before-text, so neither
/// may land alone). The audit row carries ids only.
async fn fix_segment(
    auth: AdminAuth,
    State(state): State<AppState>,
    Path((id, seq)): Path<(String, i64)>,
    Json(body): Json<FixBody>,
) -> impl IntoResponse {
    let conn = match db::open(&Store::new(&state.store_root)) {
        Ok(conn) => conn,
        Err(_) => return store_unavailable(),
    };
    if voice::store::get_call(&conn, &id).unwrap_or(None).is_none() {
        return not_found();
    }
    let Some(segment) = voice::store::get_segment(&conn, &id, seq).unwrap_or(None) else {
        return not_found();
    };
    let now = unix_now();
    let author = body
        .author
        .unwrap_or_else(|| format!("staff:{}", auth.user.user_id));
    let mut transcript = media::Transcript {
        turns: vec![media::TranscriptTurn {
            index: usize::try_from(segment.seq).unwrap_or(usize::MAX),
            speaker: segment.speaker.clone(),
            text: segment.text.clone(),
            start_ms: 0,
            end_ms: 0,
        }],
    };
    let mut annotations = Vec::new();
    let turn_index = transcript.turns[0].index;
    if let Err(fix_err) = media::apply_fix(
        &mut transcript,
        &mut annotations,
        turn_index,
        &body.text,
        &author,
        now,
    ) {
        return fix_error(fix_err);
    }
    let corrected = transcript.turns[0].text.clone();
    let correction = annotations[0].clone();
    debug_assert_eq!(correction.kind, media::AnnotationKind::Correction);
    let wrote = db::in_transaction(&conn, || {
        voice::store::set_segment_text(&conn, &id, seq, &corrected)?;
        voice::store::insert_annotation(
            &conn,
            &id,
            seq,
            "correction",
            &correction.note,
            &correction.author,
            now,
        )?;
        Ok(())
    });
    if wrote.is_err() {
        return store_unavailable();
    }
    audit(
        &conn,
        now,
        &auth.user.user_id,
        "voice_segment_fix",
        &id,
        &format!("seq={seq} kind=correction"),
    );
    (StatusCode::OK, Json(json!({"seq": seq, "text": corrected}))).into_response()
}

fn fix_error(err: media::TranscriptError) -> axum::response::Response {
    match err {
        media::TranscriptError::EmptyCorrection => (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "text must not be blank"})),
        )
            .into_response(),
        media::TranscriptError::EmptyAuthor => (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "author must not be blank"})),
        )
            .into_response(),
        // Unreachable: the segment row was just loaded above.
        media::TranscriptError::UnknownTurn { .. } => not_found(),
        _ => store_unavailable(),
    }
}

/// Pin a flag/note annotation to one transcript segment (spec §11.4).
/// The audit row carries ids only — never the note text.
async fn annotate_segment(
    auth: AdminAuth,
    State(state): State<AppState>,
    Path((id, seq)): Path<(String, i64)>,
    Json(body): Json<AnnotateBody>,
) -> impl IntoResponse {
    let conn = match db::open(&Store::new(&state.store_root)) {
        Ok(conn) => conn,
        Err(_) => return store_unavailable(),
    };
    if voice::store::get_call(&conn, &id).unwrap_or(None).is_none() {
        return not_found();
    }
    if voice::store::get_segment(&conn, &id, seq)
        .unwrap_or(None)
        .is_none()
    {
        return not_found();
    }
    let kind = body.kind.as_str();
    if !matches!(kind, "flag" | "note") {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "kind must be flag or note"})),
        )
            .into_response();
    }
    let note = body.note.trim();
    let chars = note.chars().count();
    if chars == 0 || chars > MAX_NOTE_CHARS {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "note must be 1-1024 chars"})),
        )
            .into_response();
    }
    let author = body
        .author
        .unwrap_or_else(|| format!("staff:{}", auth.user.user_id));
    let author = author.trim();
    if author.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "author must not be blank"})),
        )
            .into_response();
    }
    let now = unix_now();
    if voice::store::insert_annotation(&conn, &id, seq, kind, note, author, now).is_err() {
        return store_unavailable();
    }
    audit(
        &conn,
        now,
        &auth.user.user_id,
        "voice_segment_annotate",
        &id,
        &format!("seq={seq} kind={kind}"),
    );
    (StatusCode::OK, Json(json!({"seq": seq, "kind": kind}))).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use std::path::PathBuf;
    use tower::ServiceExt;

    struct Fixture {
        _tmp: tempfile::TempDir,
        state: AppState,
    }

    fn fixture(voice_cfg: voice::VoiceConfig) -> Fixture {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::new(tmp.path());
        store.init().unwrap();
        // Script packs resolve to this temp dir (empty unless a test
        // writes fixture packs into it).
        let scripts_dir = tmp.path().join("scripts");
        std::fs::create_dir_all(&scripts_dir).unwrap();
        let mut voice_cfg = voice_cfg;
        voice_cfg.scripts_dir = scripts_dir.display().to_string();
        let conn = db::open(&store).unwrap();
        db::migrate(&conn).unwrap();
        auth::migrate_auth_tables(&conn).unwrap();
        voice::store::migrate(&conn).unwrap();
        auth::create_session(
            &conn,
            "staff-1",
            &auth::NewSession {
                user_id: "staff-1",
                role: auth::Role::Staff,
                customer_ref: Some("cust-1"),
                now: unix_now(),
                ip: "127.0.0.1",
                user_agent: "test",
            },
        )
        .unwrap();
        drop(conn);
        let state = AppState {
            spa_index: PathBuf::from("test-spa/index.html"),
            static_dir: PathBuf::from("static"),
            store_root: tmp.path().to_path_buf(),
            bus: crate::ws_bus::Bus::new(),
            kek: None,
            admin_mtls_off: true,
            admin_require_cert: false,
            admin_spa_dir: tmp.path().join("admin-spa"),
            media_terms_read: false,
            label_cfg: crate::p2_labels::LabelCfg::default(),
            voice_cfg,
            voice_api_key: None,
            webauthn: std::sync::Arc::new(crate::passkey::test_webauthn()),
            auth_limits: std::sync::Arc::new(crate::passkey::RateBuckets::default()),
            demo_limits: std::sync::Arc::new(crate::passkey::RateBuckets::default()),
            draft_runs: std::sync::Arc::new(crate::draft::DraftRuns::default()),
            google_cfg: None,
            apple_cfg: None,
            meta_cfg: None,
            mailer: None,
            magic_link_base: "https://example.test/apostille/login".to_string(),
            meta_delete_status_base: "https://example.test/auth/meta/delete-status".to_string(),
        };
        Fixture { _tmp: tmp, state }
    }

    fn staff_request(
        method: &str,
        uri: &str,
        session: bool,
        csrf: bool,
        body: Option<serde_json::Value>,
    ) -> Request<Body> {
        let mut builder = Request::builder()
            .uri(uri)
            .method(method)
            .header("host", "admin.andrii.ai");
        if session {
            builder = builder.header(
                "cookie",
                format!(
                    "{}=staff-1{}",
                    auth::ADM_COOKIE,
                    if csrf {
                        format!("; {}=t", auth::CSRF_COOKIE)
                    } else {
                        String::new()
                    }
                ),
            );
        }
        if csrf {
            builder = builder
                .header(auth::CSRF_HEADER, "t")
                .header("origin", "https://admin.andrii.ai");
        }
        if method == "POST" {
            builder = builder.header("content-type", "application/json");
        }
        builder
            .body(body.map_or_else(Body::empty, |v| Body::from(v.to_string())))
            .unwrap()
    }

    fn dial_body(consent: bool) -> serde_json::Value {
        json!({
            "to": "+13605550100",
            "authority": "usa-wa-sos",
            "questions": ["Current fee?", "Form revision?"],
            "consent_confirmed": consent,
        })
    }

    async fn json_body(res: axum::response::Response) -> serde_json::Value {
        let body = axum::body::to_bytes(res.into_body(), 65536).await.unwrap();
        serde_json::from_slice(&body).unwrap()
    }

    #[tokio::test]
    async fn dial_refuses_without_consent_and_audits() {
        let fx = fixture(voice::VoiceConfig::default());
        let res = staff_router(fx.state.clone())
            .oneshot(staff_request(
                "POST",
                "/apostille/api/v1/calls/dial",
                true,
                true,
                Some(dial_body(false)),
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let store = Store::new(&fx.state.store_root);
        let conn = db::open(&store).unwrap();
        let calls: i64 = conn
            .query_row("SELECT COUNT(*) FROM voice_call", [], |row| row.get(0))
            .unwrap();
        assert_eq!(calls, 0);
        let refused: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM audit WHERE action = 'voice_dial_refused'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(refused, 1);
    }

    #[tokio::test]
    async fn dial_rejects_bad_numbers() {
        let fx = fixture(voice::VoiceConfig::default());
        for to in ["3605550100", "+1-360-555-0100", "", "+1"] {
            let mut body = dial_body(true);
            body["to"] = json!(to);
            let res = staff_router(fx.state.clone())
                .oneshot(staff_request(
                    "POST",
                    "/apostille/api/v1/calls/dial",
                    true,
                    true,
                    Some(body),
                ))
                .await
                .unwrap();
            assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY, "{to}");
        }
    }

    #[tokio::test]
    async fn dial_unconfigured_is_503() {
        let fx = fixture(voice::VoiceConfig::default());
        let res = staff_router(fx.state.clone())
            .oneshot(staff_request(
                "POST",
                "/apostille/api/v1/calls/dial",
                true,
                true,
                Some(dial_body(true)),
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    /// Port-0 stub Telnyx: answers `POST /v2/calls` with a
    /// synthetic leg and counts dials, so refusal tests prove "no
    /// dial" and fallback tests prove "dialed once".
    struct StubTelnyx {
        url: String,
        hits: std::sync::Arc<std::sync::atomic::AtomicUsize>,
        stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
        thread: Option<std::thread::JoinHandle<()>>,
    }

    impl StubTelnyx {
        fn start() -> Self {
            use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            listener.set_nonblocking(true).unwrap();
            let url = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
            let hits = std::sync::Arc::new(AtomicUsize::new(0));
            let stop = std::sync::Arc::new(AtomicBool::new(false));
            let thread = std::thread::spawn({
                let hits = hits.clone();
                let stop = stop.clone();
                move || {
                    while !stop.load(Ordering::SeqCst) {
                        let (stream, _) = match listener.accept() {
                            Ok(pair) => pair,
                            Err(_) => {
                                std::thread::sleep(std::time::Duration::from_millis(5));
                                continue;
                            }
                        };
                        Self::answer(stream);
                        hits.fetch_add(1, Ordering::SeqCst);
                    }
                }
            });
            StubTelnyx {
                url,
                hits,
                stop,
                thread: Some(thread),
            }
        }

        fn hits(&self) -> usize {
            self.hits.load(std::sync::atomic::Ordering::SeqCst)
        }

        /// Drain one request, then answer with a synthetic leg.
        fn answer(mut stream: std::net::TcpStream) {
            use std::io::{BufRead, BufReader, Read, Write};
            stream.set_nonblocking(false).ok();
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                .ok();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut content_len = 0usize;
            let mut chunked = false;
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 {
                    break;
                }
                if line.trim_end().is_empty() {
                    break;
                }
                let lower = line.trim_end().to_ascii_lowercase();
                if let Some(n) = lower
                    .strip_prefix("content-length:")
                    .and_then(|v| v.trim().parse().ok())
                {
                    content_len = n;
                }
                if lower == "transfer-encoding: chunked" {
                    chunked = true;
                }
            }
            if chunked {
                loop {
                    let mut size_line = String::new();
                    if reader.read_line(&mut size_line).unwrap_or(0) == 0 {
                        break;
                    }
                    let size = usize::from_str_radix(size_line.trim(), 16).unwrap_or(0);
                    if size == 0 {
                        reader.read_line(&mut String::new()).ok();
                        break;
                    }
                    let mut chunk = vec![0u8; size];
                    if reader.read_exact(&mut chunk).is_err() {
                        break;
                    }
                    reader.read_line(&mut String::new()).ok();
                }
            } else {
                let mut body = vec![0u8; content_len];
                reader.read_exact(&mut body).ok();
            }
            let reply = r#"{"data":{"call_control_id":"stub-leg-1","call_leg_id":"stub-leg-1","call_session_id":"stub-sess-1"}}"#;
            let text = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}",
                reply.len()
            );
            stream.write_all(text.as_bytes()).ok();
        }
    }

    impl Drop for StubTelnyx {
        fn drop(&mut self) {
            self.stop.store(true, std::sync::atomic::Ordering::SeqCst);
            if let Some(thread) = self.thread.take() {
                thread.join().ok();
            }
        }
    }

    /// Fixture with voice configured against the stub (synthetic
    /// `+1555…` numbers only).
    fn configured_fixture(stub: &StubTelnyx) -> Fixture {
        let mut fx = fixture(voice::VoiceConfig {
            base_url: stub.url.clone(),
            from_number: "+15550000000".to_string(),
            connection_id: "conn-test".to_string(),
            ..voice::VoiceConfig::default()
        });
        fx.state.voice_api_key = Some(config::SecretText::from("test-key".to_string()));
        fx
    }

    fn write_pack(fx: &Fixture, stem: &str, body: &str) {
        std::fs::write(
            fx.state
                .store_root
                .join("scripts")
                .join(format!("{stem}.toml")),
            body,
        )
        .unwrap();
    }

    fn audit_count(fx: &Fixture, action: &str) -> i64 {
        let store = Store::new(&fx.state.store_root);
        let conn = db::open(&store).unwrap();
        conn.query_row(
            "SELECT COUNT(*) FROM audit WHERE action = ?1",
            [action],
            |row| row.get(0),
        )
        .unwrap()
    }

    fn call_count(fx: &Fixture) -> i64 {
        let store = Store::new(&fx.state.store_root);
        let conn = db::open(&store).unwrap();
        conn.query_row("SELECT COUNT(*) FROM voice_call", [], |row| row.get(0))
            .unwrap()
    }

    #[tokio::test]
    async fn dial_missing_pack_is_refused_with_no_dial() {
        let stub = StubTelnyx::start();
        let fx = configured_fixture(&stub);
        let res = staff_router(fx.state.clone())
            .oneshot(staff_request(
                "POST",
                "/apostille/api/v1/calls/dial",
                true,
                true,
                Some(json!({
                    "to": "+15550001111",
                    "authority": "usa-wa-sos",
                    "script": "no-such-pack",
                    "consent_confirmed": true,
                })),
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let body = json_body(res).await;
        assert_eq!(body["error"], "unknown or invalid script pack");
        assert_eq!(audit_count(&fx, "voice_dial_refused"), 1);
        assert_eq!(call_count(&fx), 0);
        assert_eq!(stub.hits(), 0);
    }

    #[tokio::test]
    async fn dial_broken_pack_is_refused_with_no_dial() {
        let stub = StubTelnyx::start();
        let fx = configured_fixture(&stub);
        write_pack(
            &fx,
            "broken-id",
            "id = \"other\"\nauthority = \"O\"\ntopics = [\"hours\"]\nquestions = [\"Q?\"]\n",
        );
        let many = (0..21)
            .map(|i| format!("\"Q{i}?\""))
            .collect::<Vec<_>>()
            .join(", ");
        write_pack(
            &fx,
            "too-many",
            &format!(
                "id = \"too-many\"\nauthority = \"O\"\ntopics = [\"hours\"]\nquestions = [{many}]\n"
            ),
        );
        let long = "Q".repeat(281);
        write_pack(
            &fx,
            "too-long",
            &format!(
                "id = \"too-long\"\nauthority = \"O\"\ntopics = [\"hours\"]\nquestions = [\"{long}\"]\n"
            ),
        );
        for stem in ["broken-id", "too-many", "too-long"] {
            let res = staff_router(fx.state.clone())
                .oneshot(staff_request(
                    "POST",
                    "/apostille/api/v1/calls/dial",
                    true,
                    true,
                    Some(json!({
                        "to": "+15550001111",
                        "script": stem,
                        "consent_confirmed": true,
                    })),
                ))
                .await
                .unwrap();
            assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY, "{stem}");
        }
        assert_eq!(audit_count(&fx, "voice_dial_refused"), 3);
        assert_eq!(call_count(&fx), 0);
        assert_eq!(stub.hits(), 0);
    }

    #[tokio::test]
    async fn dial_falls_back_to_default_pack() {
        let stub = StubTelnyx::start();
        let fx = configured_fixture(&stub);
        write_pack(
            &fx,
            "default",
            "id = \"default\"\nauthority = \"Fictional Harbor Records Office\"\n\
             topics = [\"hours\", \"fees\"]\n\
             questions = [\"Walk-in hours?\", \"Per-document fee?\", \"Payment methods?\"]\n",
        );
        let res = staff_router(fx.state.clone())
            .oneshot(staff_request(
                "POST",
                "/apostille/api/v1/calls/dial",
                true,
                true,
                Some(json!({
                    "to": "+15550001111",
                    "authority": "usa-wa-sos",
                    "consent_confirmed": true,
                })),
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(stub.hits(), 1);
        let dialed = json_body(res).await;
        let call_id = dialed["call_id"].as_str().unwrap().to_string();

        let res = staff_router(fx.state.clone())
            .oneshot(staff_request(
                "GET",
                &format!("/apostille/api/v1/calls/{call_id}"),
                true,
                false,
                None,
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let detail = json_body(res).await;
        assert_eq!(detail["script_id"], "default");
        assert_eq!(detail["questions"].as_array().unwrap().len(), 3);
        assert_eq!(detail["questions"][0]["text"], "Walk-in hours?");
        assert_eq!(detail["questions"][0]["asked"], false);
    }

    #[tokio::test]
    async fn dial_inline_questions_win_over_pack() {
        let stub = StubTelnyx::start();
        let fx = configured_fixture(&stub);
        // No packs on disk and a bogus id: the inline list still wins.
        let res = staff_router(fx.state.clone())
            .oneshot(staff_request(
                "POST",
                "/apostille/api/v1/calls/dial",
                true,
                true,
                Some(json!({
                    "to": "+15550001111",
                    "questions": ["Inline?"],
                    "script": "no-such-pack",
                    "consent_confirmed": true,
                })),
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(stub.hits(), 1);
        let dialed = json_body(res).await;
        let call_id = dialed["call_id"].as_str().unwrap().to_string();

        let res = staff_router(fx.state.clone())
            .oneshot(staff_request(
                "GET",
                &format!("/apostille/api/v1/calls/{call_id}"),
                true,
                false,
                None,
            ))
            .await
            .unwrap();
        let detail = json_body(res).await;
        assert_eq!(detail["script_id"], "inline");
        assert_eq!(detail["questions"].as_array().unwrap().len(), 1);
        assert_eq!(detail["questions"][0]["text"], "Inline?");
    }

    #[tokio::test]
    async fn staff_gates_hold() {
        let fx = fixture(voice::VoiceConfig::default());
        // No session → 401.
        let res = staff_router(fx.state.clone())
            .oneshot(staff_request(
                "GET",
                "/apostille/api/v1/calls",
                false,
                false,
                None,
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
        // Mutation without CSRF/Origin → 403.
        let res = staff_router(fx.state.clone())
            .oneshot(staff_request(
                "POST",
                "/apostille/api/v1/calls/dial",
                true,
                false,
                Some(dial_body(true)),
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::FORBIDDEN);
        // Wrong host → 403.
        let req = Request::builder()
            .uri("/apostille/api/v1/calls")
            .header("host", "evil.test")
            .header("cookie", format!("{}=staff-1", auth::ADM_COOKIE))
            .body(Body::empty())
            .unwrap();
        let res = staff_router(fx.state.clone()).oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn list_detail_and_questions_round_trip() {
        let fx = fixture(voice::VoiceConfig::default());
        let store = Store::new(&fx.state.store_root);
        let conn = db::open(&store).unwrap();
        voice::store::insert_call(
            &conn,
            &voice::store::NewCall {
                id: "c-1",
                call_control_id: "leg-1",
                call_leg_id: "leg-1",
                call_session_id: "sess-1",
                to_hash: "abc",
                authority: Some("usa-wa-sos"),
                questions_json: r#"[{"text":"fee?","asked":false}]"#,
                script_id: "inline",
                created_by: "staff-1",
                now: 1,
            },
        )
        .unwrap();
        voice::store::append_transcript(&conn, "c-1", None, "hello", 2).unwrap();
        drop(conn);

        let res = staff_router(fx.state.clone())
            .oneshot(staff_request(
                "GET",
                "/apostille/api/v1/calls",
                true,
                false,
                None,
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let list = json_body(res).await;
        assert_eq!(list["calls"][0]["call_id"], "c-1");
        assert_eq!(list["calls"][0]["questions_total"], 1);

        let res = staff_router(fx.state.clone())
            .oneshot(staff_request(
                "POST",
                "/apostille/api/v1/calls/c-1/questions/0/asked",
                true,
                true,
                None,
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);

        let res = staff_router(fx.state.clone())
            .oneshot(staff_request(
                "GET",
                "/apostille/api/v1/calls/c-1",
                true,
                false,
                None,
            ))
            .await
            .unwrap();
        let detail = json_body(res).await;
        assert_eq!(detail["questions"][0]["asked"], true);
        assert_eq!(detail["transcript"][0]["text"], "hello");

        // Bad index and unknown call stay 404.
        let res = staff_router(fx.state.clone())
            .oneshot(staff_request(
                "POST",
                "/apostille/api/v1/calls/c-1/questions/9/asked",
                true,
                true,
                None,
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::NOT_FOUND);
    }

    /// Fixed test signing key (synthetic fixture — never a real key).
    fn test_signing_key() -> ed25519_dalek::SigningKey {
        ed25519_dalek::SigningKey::from_bytes(&[11u8; 32])
    }

    fn webhook_request(
        key: &ed25519_dalek::SigningKey,
        timestamp: &str,
        body: &str,
    ) -> Request<Body> {
        use base64::Engine as _;
        use ed25519_dalek::Signer as _;
        let mut message = Vec::new();
        message.extend_from_slice(timestamp.as_bytes());
        message.push(b'|');
        message.extend_from_slice(body.as_bytes());
        let signature: String =
            base64::engine::general_purpose::STANDARD.encode(key.sign(&message).to_bytes());
        Request::builder()
            .uri("/apostille/api/v1/calls/webhook")
            .method("POST")
            .header("host", "admin.andrii.ai")
            .header("content-type", "application/json")
            .header("telnyx-timestamp", timestamp)
            .header("telnyx-signature-ed25519", signature)
            .body(Body::from(body.to_string()))
            .unwrap()
    }

    fn webhook_config() -> voice::VoiceConfig {
        use base64::Engine as _;
        let public: String = base64::engine::general_purpose::STANDARD
            .encode(test_signing_key().verifying_key().as_bytes());
        voice::VoiceConfig {
            webhook_public_key: public,
            ..voice::VoiceConfig::default()
        }
    }

    fn event_body(id: &str, event: &str, payload: serde_json::Value) -> String {
        json!({
            "data": {"record_type": "event", "event_type": event, "id": id,
                     "occurred_at": "2026-10-27T00:00:00Z", "payload": payload},
            "meta": {"attempt": 1},
        })
        .to_string()
    }

    fn seed_call(fx: &Fixture) {
        let store = Store::new(&fx.state.store_root);
        let conn = db::open(&store).unwrap();
        voice::store::insert_call(
            &conn,
            &voice::store::NewCall {
                id: "c-9",
                call_control_id: "leg-9",
                call_leg_id: "leg-9",
                call_session_id: "sess-9",
                to_hash: "abc",
                authority: None,
                questions_json: "[]",
                script_id: "inline",
                created_by: "staff-1",
                now: 1,
            },
        )
        .unwrap();
    }

    #[tokio::test]
    async fn webhook_rejects_bad_signatures() {
        let fx = fixture(webhook_config());
        seed_call(&fx);
        let body = event_body("wh-1", "call.answered", json!({"call_control_id": "leg-9"}));
        let now = unix_now().to_string();
        // Wrong key signs → 401, no state change.
        let wrong = ed25519_dalek::SigningKey::from_bytes(&[12u8; 32]);
        let res = webhook_router(fx.state.clone())
            .oneshot(webhook_request(&wrong, &now, &body))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
        // Stale timestamp → 401 even with the right key.
        let res = webhook_router(fx.state.clone())
            .oneshot(webhook_request(&test_signing_key(), "1000", &body))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
        let store = Store::new(&fx.state.store_root);
        let conn = db::open(&store).unwrap();
        let call = voice::store::get_call(&conn, "c-9").unwrap().unwrap();
        assert_eq!(call.status, "queued");
    }

    #[tokio::test]
    async fn webhook_has_no_client_cert_layer() {
        let mut fx = fixture(webhook_config());
        fx.state.admin_mtls_off = false;
        fx.state.admin_require_cert = true;
        seed_call(&fx);
        let body = event_body("wh-1", "call.answered", json!({"call_control_id": "leg-9"}));
        // Wrong key signs → 401 from the signature check, not 403
        // from a cert gate (the webhook router carries no such layer).
        let wrong = ed25519_dalek::SigningKey::from_bytes(&[12u8; 32]);
        let res = webhook_router(fx.state.clone())
            .oneshot(webhook_request(&wrong, &unix_now().to_string(), &body))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn webhook_drives_call_lifecycle() {
        let fx = fixture(webhook_config());
        seed_call(&fx);
        let key = test_signing_key();
        let now = unix_now().to_string();
        let leg = json!({"call_control_id": "leg-9"});

        for (id, event, payload, want) in [
            ("wh-1", "call.initiated", leg.clone(), "ringing"),
            ("wh-2", "call.answered", leg.clone(), "in_progress"),
            // Duplicate delivery: same id, still 200, no double move.
            ("wh-2", "call.answered", leg.clone(), "in_progress"),
            (
                "wh-3",
                "call.hangup",
                json!({"call_control_id": "leg-9", "hangup_cause": "normal", "hangup_source": "caller"}),
                "completed",
            ),
        ] {
            let body = event_body(id, event, payload);
            let res = webhook_router(fx.state.clone())
                .oneshot(webhook_request(&key, &now, &body))
                .await
                .unwrap();
            assert_eq!(res.status(), StatusCode::OK, "{event}");
            let store = Store::new(&fx.state.store_root);
            let conn = db::open(&store).unwrap();
            let call = voice::store::get_call(&conn, "c-9").unwrap().unwrap();
            assert_eq!(call.status, want, "{event}");
        }
        // Unknown leg → 2xx drop.
        let body = event_body("wh-9", "call.answered", json!({"call_control_id": "nope"}));
        let res = webhook_router(fx.state.clone())
            .oneshot(webhook_request(&key, &now, &body))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        // Transcript segments append.
        let body = event_body(
            "wh-10",
            "call.transcription",
            json!({"call_control_id": "leg-9", "transcript": "the fee is eight dollars"}),
        );
        let res = webhook_router(fx.state.clone())
            .oneshot(webhook_request(&key, &now, &body))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let store = Store::new(&fx.state.store_root);
        let conn = db::open(&store).unwrap();
        let segments = voice::store::list_transcripts(&conn, "c-9").unwrap();
        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].text, "the fee is eight dollars");
    }

    #[tokio::test]
    async fn webhook_out_of_order_and_hangup_audit() {
        let fx = fixture(webhook_config());
        seed_call(&fx);
        let key = test_signing_key();
        let now = unix_now().to_string();
        let leg = json!({"call_control_id": "leg-9"});

        // Answered lands first; the late initiated acks as a no-op.
        for (id, event, want) in [
            ("wh-o1", "call.answered", "in_progress"),
            ("wh-o2", "call.initiated", "in_progress"),
        ] {
            let body = event_body(id, event, leg.clone());
            let res = webhook_router(fx.state.clone())
                .oneshot(webhook_request(&key, &now, &body))
                .await
                .unwrap();
            assert_eq!(res.status(), StatusCode::OK, "{event}");
            let store = Store::new(&fx.state.store_root);
            let conn = db::open(&store).unwrap();
            let call = voice::store::get_call(&conn, "c-9").unwrap().unwrap();
            assert_eq!(call.status, want, "{event}");
        }
        // Hangup completes and audits once; a retried delivery adds
        // no second audit row.
        let hangup = json!({
            "call_control_id": "leg-9",
            "hangup_cause": "normal",
            "hangup_source": "caller",
        });
        for id in ["wh-o3", "wh-o3"] {
            let body = event_body(id, "call.hangup", hangup.clone());
            let res = webhook_router(fx.state.clone())
                .oneshot(webhook_request(&key, &now, &body))
                .await
                .unwrap();
            assert_eq!(res.status(), StatusCode::OK);
        }
        let store = Store::new(&fx.state.store_root);
        let conn = db::open(&store).unwrap();
        let call = voice::store::get_call(&conn, "c-9").unwrap().unwrap();
        assert_eq!(call.status, "completed");
        let audits: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM audit
                 WHERE action = 'voice_hangup' AND after_ref = 'c-9'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(audits, 1);
        let reason: String = conn
            .query_row(
                "SELECT reason FROM audit
                 WHERE action = 'voice_hangup' AND after_ref = 'c-9'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(reason.contains("normal"), "{reason}");
    }

    fn voice_cfg_with_stub(base_url: &str) -> voice::VoiceConfig {
        voice::VoiceConfig {
            base_url: base_url.to_string(),
            from_number: "+13605550000".to_string(),
            connection_id: "conn-1".to_string(),
            ..webhook_config()
        }
    }

    /// Port-0 Telnyx stub for the dial tests: counts connections in
    /// two windows (refusal → 0 hits, consent → 1 hit) and answers
    /// every `POST /v2/calls` with a canned dial outcome. Hermetic:
    /// binds 127.0.0.1:0, no network leaves the box.
    fn dial_stub() -> (
        String,
        std::sync::mpsc::Receiver<usize>,
        std::thread::JoinHandle<()>,
    ) {
        use std::io::{BufRead, BufReader, Read, Write};
        use std::net::TcpListener;
        use std::time::{Duration, Instant};

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
        let (tx, rx) = std::sync::mpsc::channel();
        let handle = std::thread::spawn(move || {
            let serve_until = |mut deadline: Instant,
                               tx: &std::sync::mpsc::Sender<usize>,
                               settle: Option<Duration>| {
                let mut hits = 0usize;
                let mut settled = false;
                while Instant::now() < deadline {
                    match listener.accept() {
                        Ok((stream, _)) => {
                            hits += 1;
                            // First hit starts the settle window: any
                            // dial-caused connection completes before
                            // the handler answers, so strays after the
                            // settle still count as failures.
                            if !settled && let Some(window) = settle {
                                deadline = Instant::now() + window;
                                settled = true;
                            }
                            // Drain one request, answer canned dial-OK.
                            let mut reader = BufReader::new(stream.try_clone().unwrap());
                            let mut head = String::new();
                            reader.read_line(&mut head).unwrap();
                            let mut len = 0usize;
                            let mut chunked = false;
                            loop {
                                let mut line = String::new();
                                reader.read_line(&mut line).unwrap();
                                if line.trim_end().is_empty() {
                                    break;
                                }
                                let lower = line.trim_end().to_ascii_lowercase();
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
                            if chunked {
                                loop {
                                    let mut size_line = String::new();
                                    reader.read_line(&mut size_line).unwrap();
                                    let size = usize::from_str_radix(size_line.trim(), 16).unwrap();
                                    if size == 0 {
                                        reader.read_line(&mut String::new()).unwrap();
                                        break;
                                    }
                                    let mut chunk = vec![0u8; size];
                                    reader.read_exact(&mut chunk).unwrap();
                                    reader.read_line(&mut String::new()).unwrap();
                                }
                            } else {
                                let mut body = vec![0u8; len];
                                reader.read_exact(&mut body).unwrap();
                            }
                            let reply = r#"{"data":{"call_control_id":"leg-7","call_leg_id":"leg-7","call_session_id":"sess-7"}}"#;
                            let mut stream = reader.into_inner();
                            let text = format!(
                                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}",
                                reply.len()
                            );
                            stream.write_all(text.as_bytes()).unwrap();
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            std::thread::sleep(Duration::from_millis(10));
                        }
                        Err(_) => break,
                    }
                }
                tx.send(hits).unwrap();
            };
            // Phase 1: the refused dial must make no request.
            serve_until(Instant::now() + Duration::from_millis(300), &tx, None);
            // Phase 2: the consented dial must make exactly one.
            serve_until(
                Instant::now() + Duration::from_secs(10),
                &tx,
                Some(Duration::from_millis(500)),
            );
        });
        (base, rx, handle)
    }

    /// Refusal makes no Telnyx call (spec §8.3): the stub sees zero
    /// hits for the unconsented dial and exactly one for the
    /// consented control, which also queues the row and audits.
    #[tokio::test]
    async fn dial_refusal_makes_no_telnyx_call() {
        let (base, hits, handle) = dial_stub();
        let mut fx = fixture(voice_cfg_with_stub(&base));
        fx.state.voice_api_key = Some("stub-key".to_string().into());

        let res = staff_router(fx.state.clone())
            .oneshot(staff_request(
                "POST",
                "/apostille/api/v1/calls/dial",
                true,
                true,
                Some(dial_body(false)),
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(
            hits.recv_timeout(std::time::Duration::from_secs(5))
                .unwrap(),
            0
        );

        // Positive control: consent dials once against the same stub.
        let res = staff_router(fx.state.clone())
            .oneshot(staff_request(
                "POST",
                "/apostille/api/v1/calls/dial",
                true,
                true,
                Some(dial_body(true)),
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let dialed = json_body(res).await;
        assert_eq!(dialed["status"], "queued");
        assert_eq!(
            hits.recv_timeout(std::time::Duration::from_secs(15))
                .unwrap(),
            1
        );
        handle.join().unwrap();

        let store = Store::new(&fx.state.store_root);
        let conn = db::open(&store).unwrap();
        let call_id = dialed["call_id"].as_str().unwrap();
        let (status, leg): (String, String) = conn
            .query_row(
                "SELECT status, call_control_id FROM voice_call WHERE id = ?1",
                [call_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(status, "queued");
        assert_eq!(leg, "leg-7");
        let reason: String = conn
            .query_row(
                "SELECT reason FROM audit WHERE action = 'voice_dial' AND after_ref = ?1",
                [call_id],
                |row| row.get(0),
            )
            .unwrap();
        // The dialed number never lands in the audit row.
        assert!(!reason.contains("3605550100"), "{reason}");
    }

    fn seed_call_with_transcripts(
        conn: &rusqlite::Connection,
        call_id: &str,
        authority: Option<&str>,
    ) {
        conn.execute(
            "INSERT OR IGNORE INTO authority(kind, country, region, name, slug)
             VALUES ('state', 'US', 'WA', 'Washington SOS', 'usa-wa-sos')",
            [],
        )
        .unwrap();
        let leg = format!("leg-{call_id}");
        let sess = format!("sess-{call_id}");
        voice::store::insert_call(
            conn,
            &voice::store::NewCall {
                id: call_id,
                call_control_id: &leg,
                call_leg_id: &leg,
                call_session_id: &sess,
                to_hash: "abc",
                authority,
                questions_json: r#"[{"text":"fee?","asked":true}]"#,
                script_id: "inline",
                created_by: "staff-1",
                now: 1,
            },
        )
        .unwrap();
        voice::store::append_transcript(conn, call_id, Some("agent"), "The fee?", 2).unwrap();
        voice::store::append_transcript(conn, call_id, Some("them"), "Eight dollars.", 3).unwrap();
    }

    fn quote_body(segment_ids: Vec<i64>, topic: &str, statement: &str) -> serde_json::Value {
        json!({
            "segment_ids": segment_ids,
            "topic": topic,
            "statement": statement,
        })
    }

    /// Happy path: segments 0-1 quote into ONE proposed fact with
    /// unattributed capture id, staff created_by, and a grep-able audit row
    /// that carries NO transcript text.
    #[tokio::test]
    async fn quote_to_draft_creates_one_proposed_fact() {
        let fx = fixture(voice::VoiceConfig::default());
        let store = Store::new(&fx.state.store_root);
        let conn = db::open(&store).unwrap();
        seed_call_with_transcripts(&conn, "c-q", Some("usa-wa-sos"));
        drop(conn);

        let res = staff_router(fx.state.clone())
            .oneshot(staff_request(
                "POST",
                "/apostille/api/v1/calls/c-q/quote",
                true,
                true,
                Some(quote_body(vec![0, 1], "fees", "Counter fee is $8.")),
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let quote = json_body(res).await;
        assert_eq!(quote["call_id"], "c-q");
        assert_eq!(quote["topic"], "fees");
        assert_eq!(quote["status"], "proposed");
        let fact_id = quote["fact_id"].as_i64().unwrap();
        assert!(fact_id > 0);

        let conn = db::open(&store).unwrap();
        let (status, created_by, capture_id, topic, statement): (
            String,
            String,
            i64,
            String,
            String,
        ) = conn
            .query_row(
                "SELECT status, created_by, capture_id, topic, statement
                 FROM fact WHERE id = ?1",
                [fact_id],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(status, "proposed");
        assert_eq!(created_by, "staff:staff-1");
        assert_eq!(capture_id, 0);
        assert_eq!(topic, "fees");
        assert_eq!(statement, "Counter fee is $8.");
        // Attribution audit: refs in the reason, never the transcript text.
        let reason: String = conn
            .query_row(
                "SELECT reason FROM audit
                 WHERE action = 'voice_quote' AND after_ref = 'c-q'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(reason.contains(&format!("fact_id={fact_id}")), "{reason}");
        assert!(reason.contains("segments=[0,1]"), "{reason}");
        assert!(reason.contains("topic=fees"), "{reason}");
        assert!(!reason.contains("Eight dollars"), "{reason}");
    }

    /// Every invalid shape is a 422 with a non-empty error; nothing lands.
    #[tokio::test]
    async fn quote_to_draft_validation_matrix() {
        let fx = fixture(voice::VoiceConfig::default());
        let store = Store::new(&fx.state.store_root);
        let conn = db::open(&store).unwrap();
        seed_call_with_transcripts(&conn, "c-q", Some("usa-wa-sos"));
        seed_call_with_transcripts(&conn, "c-noauth", None);
        drop(conn);

        let overlong = "x".repeat(1025);
        let cases: Vec<(&str, serde_json::Value)> = vec![
            ("c-q", quote_body(vec![], "fees", "Counter fee is $8.")),
            (
                "c-q",
                quote_body(vec![0], "not-a-topic", "Counter fee is $8."),
            ),
            ("c-q", quote_body(vec![0], "fees", "   ")),
            ("c-q", quote_body(vec![0], "fees", &overlong)),
            (
                "c-noauth",
                quote_body(vec![0], "fees", "Counter fee is $8."),
            ),
        ];
        for (call_id, body) in cases {
            let res = staff_router(fx.state.clone())
                .oneshot(staff_request(
                    "POST",
                    &format!("/apostille/api/v1/calls/{call_id}/quote"),
                    true,
                    true,
                    Some(body.clone()),
                ))
                .await
                .unwrap();
            assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY, "{body}");
            let err = json_body(res).await;
            assert!(!err["error"].as_str().unwrap().is_empty());
        }
        // Foreign segment → 404 (bad sub-resource index, the mark_asked
        // convention), not 422.
        let res = staff_router(fx.state.clone())
            .oneshot(staff_request(
                "POST",
                "/apostille/api/v1/calls/c-q/quote",
                true,
                true,
                Some(quote_body(vec![0, 9], "fees", "Counter fee is $8.")),
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::NOT_FOUND);
        let conn = db::open(&store).unwrap();
        let facts: i64 = conn
            .query_row("SELECT COUNT(*) FROM fact", [], |row| row.get(0))
            .unwrap();
        assert_eq!(facts, 0);
    }

    /// Guards: unknown call 404s; mutations without CSRF/Origin 403
    /// (the CSRF layer runs before auth, so session-less POSTs 403
    /// too — 401-on-GET is covered by `staff_gates_hold`). All before
    /// any write.
    #[tokio::test]
    async fn quote_to_draft_guards() {
        let fx = fixture(voice::VoiceConfig::default());
        let body = quote_body(vec![0], "fees", "Counter fee is $8.");
        // Unknown call → 404.
        let res = staff_router(fx.state.clone())
            .oneshot(staff_request(
                "POST",
                "/apostille/api/v1/calls/nope/quote",
                true,
                true,
                Some(body.clone()),
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::NOT_FOUND);
        // No session and no CSRF/Origin → 403 (CSRF layer first).
        let res = staff_router(fx.state.clone())
            .oneshot(staff_request(
                "POST",
                "/apostille/api/v1/calls/nope/quote",
                false,
                false,
                Some(body.clone()),
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::FORBIDDEN);
        // Session but no CSRF/Origin → 403.
        let res = staff_router(fx.state.clone())
            .oneshot(staff_request(
                "POST",
                "/apostille/api/v1/calls/nope/quote",
                true,
                false,
                Some(body),
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::FORBIDDEN);
        let store = Store::new(&fx.state.store_root);
        let conn = db::open(&store).unwrap();
        let facts: i64 = conn
            .query_row("SELECT COUNT(*) FROM fact", [], |row| row.get(0))
            .unwrap();
        assert_eq!(facts, 0);
    }

    /// Fix happy path: text rewrites in place (speaker/timing kept), a
    /// `Correction` quoting before → after lands in `voice_annotation`,
    /// and the audit row carries ids only.
    #[tokio::test]
    async fn fix_rewrites_text_and_appends_correction() {
        let fx = fixture(voice::VoiceConfig::default());
        let store = Store::new(&fx.state.store_root);
        let conn = db::open(&store).unwrap();
        seed_call_with_transcripts(&conn, "c-f", Some("usa-wa-sos"));
        drop(conn);

        let res = staff_router(fx.state.clone())
            .oneshot(staff_request(
                "POST",
                "/apostille/api/v1/calls/c-f/segments/1/fix",
                true,
                true,
                Some(json!({"text": "  Eight dollars flat.  "})),
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let fixed = json_body(res).await;
        assert_eq!(fixed["seq"], 1);
        assert_eq!(fixed["text"], "Eight dollars flat.");

        let conn = db::open(&store).unwrap();
        let segment = voice::store::get_segment(&conn, "c-f", 1).unwrap().unwrap();
        assert_eq!(segment.text, "Eight dollars flat.");
        assert_eq!(segment.speaker.as_deref(), Some("them"));
        assert_eq!(segment.at, 3);
        let annotations = voice::store::list_annotations(&conn, "c-f").unwrap();
        assert_eq!(annotations.len(), 1);
        assert_eq!(annotations[0].seq, 1);
        assert_eq!(annotations[0].kind, "correction");
        assert_eq!(annotations[0].author, "staff:staff-1");
        assert!(
            annotations[0].note.contains("Eight dollars."),
            "{}",
            annotations[0].note
        );
        assert!(
            annotations[0].note.contains("Eight dollars flat."),
            "{}",
            annotations[0].note
        );
        let reason: String = conn
            .query_row(
                "SELECT reason FROM audit
                 WHERE action = 'voice_segment_fix' AND after_ref = 'c-f'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(reason.contains("seq=1"), "{reason}");
        assert!(reason.contains("kind=correction"), "{reason}");
    }

    /// Annotate happy path: flag/note rows land with the session author
    /// by default; explicit authors are honored.
    #[tokio::test]
    async fn annotate_stores_flag_and_note() {
        let fx = fixture(voice::VoiceConfig::default());
        let store = Store::new(&fx.state.store_root);
        let conn = db::open(&store).unwrap();
        seed_call_with_transcripts(&conn, "c-a", Some("usa-wa-sos"));
        drop(conn);

        for (seq, kind, body) in [
            (
                0,
                "flag",
                json!({"kind": "flag", "note": "jargon — verify"}),
            ),
            (
                1,
                "note",
                json!({"kind": "note", "note": "  called back  ", "author": "staff-anna"}),
            ),
        ] {
            let res = staff_router(fx.state.clone())
                .oneshot(staff_request(
                    "POST",
                    &format!("/apostille/api/v1/calls/c-a/segments/{seq}/annotate"),
                    true,
                    true,
                    Some(body),
                ))
                .await
                .unwrap();
            assert_eq!(res.status(), StatusCode::OK, "{kind}");
            let annotated = json_body(res).await;
            assert_eq!(annotated["seq"], seq);
            assert_eq!(annotated["kind"], kind);
        }

        let conn = db::open(&store).unwrap();
        let annotations = voice::store::list_annotations(&conn, "c-a").unwrap();
        assert_eq!(annotations.len(), 2);
        assert_eq!(annotations[0].kind, "flag");
        assert_eq!(annotations[0].note, "jargon — verify");
        assert_eq!(annotations[0].author, "staff:staff-1");
        assert_eq!(annotations[1].kind, "note");
        assert_eq!(annotations[1].note, "called back");
        assert_eq!(annotations[1].author, "staff-anna");
    }

    /// The §11.4 code matrix: blank inputs are 400, unknown
    /// call/segment is 404, and nothing lands on any error.
    #[tokio::test]
    async fn fix_and_annotate_validation_matrix() {
        let fx = fixture(voice::VoiceConfig::default());
        let store = Store::new(&fx.state.store_root);
        let conn = db::open(&store).unwrap();
        seed_call_with_transcripts(&conn, "c-v", Some("usa-wa-sos"));
        drop(conn);

        let overlong = "x".repeat(1025);
        let bad_requests: Vec<(&str, serde_json::Value)> = vec![
            (
                "/apostille/api/v1/calls/c-v/segments/0/fix",
                json!({"text": "   "}),
            ),
            (
                "/apostille/api/v1/calls/c-v/segments/0/fix",
                json!({"text": "fine", "author": "  "}),
            ),
            (
                "/apostille/api/v1/calls/c-v/segments/0/annotate",
                json!({"kind": "shout", "note": "nope"}),
            ),
            (
                "/apostille/api/v1/calls/c-v/segments/0/annotate",
                json!({"kind": "flag", "note": "  "}),
            ),
            (
                "/apostille/api/v1/calls/c-v/segments/0/annotate",
                json!({"kind": "note", "note": overlong}),
            ),
            (
                "/apostille/api/v1/calls/c-v/segments/0/annotate",
                json!({"kind": "note", "note": "fine", "author": ""}),
            ),
        ];
        for (uri, body) in bad_requests {
            let res = staff_router(fx.state.clone())
                .oneshot(staff_request("POST", uri, true, true, Some(body.clone())))
                .await
                .unwrap();
            assert_eq!(res.status(), StatusCode::BAD_REQUEST, "{uri} {body}");
            let err = json_body(res).await;
            assert!(!err["error"].as_str().unwrap().is_empty());
        }
        let not_founds: Vec<(&str, serde_json::Value)> = vec![
            (
                "/apostille/api/v1/calls/c-v/segments/9/fix",
                json!({"text": "fine"}),
            ),
            (
                "/apostille/api/v1/calls/nope/segments/0/fix",
                json!({"text": "fine"}),
            ),
            (
                "/apostille/api/v1/calls/c-v/segments/9/annotate",
                json!({"kind": "flag", "note": "fine"}),
            ),
            (
                "/apostille/api/v1/calls/nope/segments/0/annotate",
                json!({"kind": "flag", "note": "fine"}),
            ),
        ];
        for (uri, body) in not_founds {
            let res = staff_router(fx.state.clone())
                .oneshot(staff_request("POST", uri, true, true, Some(body.clone())))
                .await
                .unwrap();
            assert_eq!(res.status(), StatusCode::NOT_FOUND, "{uri} {body}");
        }
        // Error paths write nothing: original text intact, no
        // annotations, no segment audit rows.
        let conn = db::open(&store).unwrap();
        let segment = voice::store::get_segment(&conn, "c-v", 0).unwrap().unwrap();
        assert_eq!(segment.text, "The fee?");
        assert!(
            voice::store::list_annotations(&conn, "c-v")
                .unwrap()
                .is_empty()
        );
        let audits: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM audit WHERE after_ref = 'c-v'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(audits, 0);
    }

    /// Grep gate: no audit row for a fixed/annotated call may contain
    /// the before-text, the after-text, or the note text (ids only).
    #[tokio::test]
    async fn segment_audit_rows_carry_no_transcript_text() {
        let fx = fixture(voice::VoiceConfig::default());
        let store = Store::new(&fx.state.store_root);
        let conn = db::open(&store).unwrap();
        seed_call_with_transcripts(&conn, "c-g", Some("usa-wa-sos"));
        drop(conn);

        let res = staff_router(fx.state.clone())
            .oneshot(staff_request(
                "POST",
                "/apostille/api/v1/calls/c-g/segments/0/fix",
                true,
                true,
                Some(json!({"text": "What is the walk-in fee?"})),
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let res = staff_router(fx.state.clone())
            .oneshot(staff_request(
                "POST",
                "/apostille/api/v1/calls/c-g/segments/0/annotate",
                true,
                true,
                Some(json!({"kind": "flag", "note": "verify against the fee schedule"})),
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);

        let conn = db::open(&store).unwrap();
        let mut stmt = conn
            .prepare("SELECT action, actor, after_ref, reason FROM audit WHERE after_ref = 'c-g'")
            .unwrap();
        let rows: Vec<(String, String, String, String)> = stmt
            .query_map([], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            })
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(rows.len(), 2);
        for secret in [
            "The fee?",
            "What is the walk-in fee?",
            "verify against the fee schedule",
        ] {
            for (action, actor, after_ref, reason) in &rows {
                for field in [action, actor, after_ref, reason] {
                    assert!(!field.contains(secret), "{action}: {field}");
                }
            }
        }
        let actions: Vec<&str> = rows.iter().map(|r| r.0.as_str()).collect();
        assert!(actions.contains(&"voice_segment_fix"), "{actions:?}");
        assert!(actions.contains(&"voice_segment_annotate"), "{actions:?}");
    }

    /// Both routes sit behind the staff-router guard stack: mutations
    /// without CSRF/Origin 403 before any write.
    #[tokio::test]
    async fn fix_and_annotate_require_csrf() {
        let fx = fixture(voice::VoiceConfig::default());
        let store = Store::new(&fx.state.store_root);
        let conn = db::open(&store).unwrap();
        seed_call_with_transcripts(&conn, "c-csrf", Some("usa-wa-sos"));
        drop(conn);

        for (uri, body) in [
            (
                "/apostille/api/v1/calls/c-csrf/segments/0/fix",
                json!({"text": "fine"}),
            ),
            (
                "/apostille/api/v1/calls/c-csrf/segments/0/annotate",
                json!({"kind": "note", "note": "fine"}),
            ),
        ] {
            let res = staff_router(fx.state.clone())
                .oneshot(staff_request("POST", uri, true, false, Some(body)))
                .await
                .unwrap();
            assert_eq!(res.status(), StatusCode::FORBIDDEN, "{uri}");
        }
        let conn = db::open(&store).unwrap();
        let segment = voice::store::get_segment(&conn, "c-csrf", 0)
            .unwrap()
            .unwrap();
        assert_eq!(segment.text, "The fee?");
    }
}
