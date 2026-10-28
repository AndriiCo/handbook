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

use crate::{
    AppState,
    admin::{AdminAuth, admin_host_guard, csrf_guard},
    store_unavailable, unix_now,
};

/// Telnyx signature replay window (seconds).
const WEBHOOK_WINDOW_SECS: i64 = 300;

/// Cap on prepared questions per call.
const MAX_QUESTIONS: usize = 20;

/// Cap on one question's length (chars).
const MAX_QUESTION_CHARS: usize = 280;

/// Staff routes: dial, list, detail, hangup, record, questions.
/// Host guard + CSRF layers are applied here (not inherited from
/// `admin::router`), so the `main` merge keeps the recipe whatever
/// the merge-layer semantics are.
pub fn staff_router(state: AppState) -> Router {
    Router::new()
        .route("/apostille/api/v1/admin/calls/dial", post(dial))
        .route("/apostille/api/v1/admin/calls", get(list_calls))
        .route("/apostille/api/v1/admin/calls/{id}", get(get_call))
        .route("/apostille/api/v1/admin/calls/{id}/hangup", post(hangup))
        .route(
            "/apostille/api/v1/admin/calls/{id}/record/start",
            post(record_start),
        )
        .route(
            "/apostille/api/v1/admin/calls/{id}/record/stop",
            post(record_stop),
        )
        .route(
            "/apostille/api/v1/admin/calls/{id}/questions/{idx}/asked",
            post(mark_asked),
        )
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            csrf_guard,
        ))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            admin_host_guard,
        ))
        .layer(axum::middleware::from_fn(crate::request_id_and_log))
        .with_state(state)
}

/// Telnyx webhook receiver: host guard only (no staff session —
/// Telnyx authenticates by signature) and NO CSRF layer (the
/// server-to-server POST carries no Origin or session cookies).
pub fn webhook_router(state: AppState) -> Router {
    Router::new()
        .route(
            "/apostille/api/v1/admin/calls/webhook",
            post(telnyx_webhook),
        )
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            admin_host_guard,
        ))
        .layer(axum::middleware::from_fn(crate::request_id_and_log))
        .with_state(state)
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)] // m3: mass-assignment hygiene on mutating DTOs
struct DialBody {
    to: String,
    authority: Option<String>,
    #[serde(default)]
    questions: Vec<String>,
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
    let questions: Vec<voice::store::Question> = body
        .questions
        .iter()
        .map(|text| voice::store::Question {
            text: text.trim().to_string(),
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
            admin_spa_dir: tmp.path().join("admin-spa"),
            media_terms_read: false,
            label_cfg: crate::p2_labels::LabelCfg::default(),
            voice_cfg,
            voice_api_key: None,
            webauthn: std::sync::Arc::new(crate::passkey::test_webauthn()),
            auth_limits: std::sync::Arc::new(crate::passkey::RateBuckets::default()),
            google_cfg: None,
            apple_cfg: None,
            meta_cfg: None,
            mailer: None,
            magic_link_base: "https://example.test/apostille/login".to_string(),
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
                "/apostille/api/v1/admin/calls/dial",
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
                    "/apostille/api/v1/admin/calls/dial",
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
                "/apostille/api/v1/admin/calls/dial",
                true,
                true,
                Some(dial_body(true)),
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn staff_gates_hold() {
        let fx = fixture(voice::VoiceConfig::default());
        // No session → 401.
        let res = staff_router(fx.state.clone())
            .oneshot(staff_request(
                "GET",
                "/apostille/api/v1/admin/calls",
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
                "/apostille/api/v1/admin/calls/dial",
                true,
                false,
                Some(dial_body(true)),
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::FORBIDDEN);
        // Wrong host → 403.
        let req = Request::builder()
            .uri("/apostille/api/v1/admin/calls")
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
                "/apostille/api/v1/admin/calls",
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
                "/apostille/api/v1/admin/calls/c-1/questions/0/asked",
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
                "/apostille/api/v1/admin/calls/c-1",
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
                "/apostille/api/v1/admin/calls/c-1/questions/9/asked",
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
            .uri("/apostille/api/v1/admin/calls/webhook")
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
}
