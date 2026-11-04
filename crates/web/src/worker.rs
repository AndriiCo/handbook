//! Plant work queue (P392): WebSocket checkout for plant workers plus
//! the staff work-items API.
//!
//! `GET /apostille/api/v1/worker/ws` (PUBLIC host: plant dials out to
//! andrii.ai, so no admin IP allowlist applies) authenticates
//! `Authorization: Bearer <w2. token>` BEFORE the upgrade, then
//! speaks JSON frames: hello → welcome, checkout → job | idle,
//! heartbeat → lease, complete (base64 result) | fail → ack. Queue
//! state lives in `work.sqlite` ([`handbook_store::work`]); result
//! bytes land content-addressed in the store's blob dir.
//!
//! Staff routes (`/apostille/api/v1/admin/work-items…`, ADMIN listener,
//! mounted through `mission_admin_routers` so the host guard, IP
//! allowlist and CSRF layers wrap them) enqueue, list, show, retry and
//! download results. Spec: `docs/specs/work-queue.md`; frame samples:
//! `docs/contracts/worker-frames.json`.
//!
//! Workers enrol for their own tokens (`docs/specs/worker-enrolment.md`):
//! `POST /apostille/api/v1/worker/enrol` files a request with a pairing
//! code, staff approve it on the Work queue page's Workers tab, and the
//! worker's poll collects a per-worker `w2.` token once. `welcome`
//! carries the job-kind settings (the Meta key included) from server
//! config, so workers read no `.env`.

use std::collections::{BTreeMap, HashMap};
use std::net::{IpAddr, SocketAddr};
use std::sync::Mutex;
use std::time::Duration;

use axum::{
    Router,
    body::Bytes,
    extract::{
        ConnectInfo, Path, Query, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Json, Response},
    routing::{get, post},
};
use base64::Engine as _;
use handbook_store::{db, store::Store, work, work_enrol};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::admin::AdminAuth;
use crate::{AppState, store_unavailable, unix_now};

/// Lease length per checkout and per heartbeat (seconds).
pub const LEASE_SECS: i64 = 300;
/// How often workers should heartbeat (told in `welcome`).
pub const HEARTBEAT_SECS: i64 = 60;
/// Largest decoded result a worker may upload (16 MiB).
pub const MAX_RESULT_BYTES: usize = 16 * 1024 * 1024;
/// Inbound frame cap: a max result plus base64 overhead and JSON.
pub const MAX_FRAME_BYTES: usize = 24 * 1024 * 1024;
/// Largest enqueued payload (serialized JSON).
pub const MAX_PAYLOAD_BYTES: usize = 64 * 1024;
/// Largest worker summary kept (serialized JSON).
const MAX_SUMMARY_BYTES: usize = 16 * 1024;
/// Worker sockets per peer IP (plant runs one; slack for restarts and
/// a reconnect racing its own stale socket).
pub const MAX_SOCKETS_PER_IP: usize = 8;
/// `idle` back-off when nothing is pending.
pub const IDLE_RETRY_SECS: i64 = 15;
/// The first frame must be `hello` within this long.
const HELLO_TIMEOUT_SECS: u64 = 10;
/// Server ping cadence (keeps NATs from dropping the socket).
const PING_SECS: u64 = 30;
/// A socket with no inbound frame (pongs included) for this long is
/// closed; the worker reconnects if it is alive.
const IDLE_SECS: u64 = 3 * PING_SECS;

/// One job kind the queue accepts. The plant handler registry
/// (`xtask/src/worker/`) must offer the same names.
pub(crate) struct KindInfo {
    pub kind: &'static str,
    pub about: &'static str,
    /// Example payload (JSON) the admin page prefills.
    pub example: &'static str,
    /// Longest a lease may run from checkout; past it the next
    /// heartbeat answers `lease_lost` and the item fails for good.
    pub max_runtime_secs: i64,
}

pub(crate) const KINDS: &[KindInfo] = &[
    KindInfo {
        kind: "research.nonparty",
        about: "Legalization packs for UN members outside the Apostille Convention: \
                Spark finds the DOS and MFA pages, every value is quote-checked.",
        example: r#"{"countries": ["EG"], "limit": 1}"#,
        max_runtime_secs: 8 * 3600,
    },
    KindInfo {
        kind: "discovery.bootstrap",
        about: "From-scratch discovery report: countries from the live HCCH table \
                (parties, future parties with entry-into-force dates, non-parties), \
                a Muse cross-check of that list, and fetch-checked official URLs \
                for the listed countries' authorities. Writes nothing; the kept URLs \
                come back as world packs to stage and load (watch seeds, unverified \
                domains).",
        example: r#"{"countries": ["DE"], "batch_size": 15}"#,
        // Default caps (10 batches of 15, each URL fetched politely)
        // finish well inside an hour; "all" with more batches needs more.
        max_runtime_secs: 2 * 3600,
    },
    KindInfo {
        kind: "scrape.urls",
        about: "Fetch public URLs with the repo fetch code; the result holds the raw bytes.",
        example: concat!(
            r#"{"urls": ["https://www.hcch.net/en/instruments/conventions/"#,
            r#"status-table/?cid=41"]}"#
        ),
        max_runtime_secs: 3600,
    },
    KindInfo {
        kind: "eval.gold",
        about: "Grade drafts on the gold set and return quality.json. \
                Replays recorded drafts; \"live\": true drafts through the Meta API \
                (needs meta.api_key in /config).",
        example: r#"{"live": false, "planted": true}"#,
        max_runtime_secs: 3 * 3600,
    },
    KindInfo {
        kind: "precheck.glimmer",
        about: "Self-hosted Glimmer precheck on the plant GPU.",
        example: "{}",
        max_runtime_secs: 30 * 60,
    },
];

fn known_kind(kind: &str) -> bool {
    KINDS.iter().any(|k| k.kind == kind)
}

/// Time limit for `kind` (the longest one for a kind no longer listed).
fn max_runtime(kind: &str) -> i64 {
    KINDS.iter().find(|k| k.kind == kind).map_or_else(
        || {
            KINDS
                .iter()
                .map(|k| k.max_runtime_secs)
                .max()
                .unwrap_or(3600)
        },
        |k| k.max_runtime_secs,
    )
}

// ---------------------------------------------------------------------------
// Frames
// ---------------------------------------------------------------------------

/// Worker → server.
#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ClientFrame {
    Hello {
        worker: String,
        #[serde(default)]
        capabilities: Vec<String>,
        #[serde(default)]
        version: String,
    },
    Checkout,
    Heartbeat {
        id: i64,
        lease: String,
    },
    Complete {
        id: i64,
        lease: String,
        #[serde(default)]
        content_type: Option<String>,
        #[serde(default)]
        sha256: Option<String>,
        #[serde(default)]
        result_b64: Option<String>,
        #[serde(default)]
        summary: Option<Value>,
    },
    Fail {
        id: i64,
        lease: String,
        error: String,
        #[serde(default)]
        retryable: bool,
    },
}

/// Server → worker.
#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ServerFrame {
    Welcome {
        worker: String,
        lease_secs: i64,
        heartbeat_secs: i64,
        max_result_bytes: usize,
        /// Job-kind settings from server config (memory only on the
        /// worker; it reads no `.env`).
        #[serde(default)]
        settings: BTreeMap<String, String>,
    },
    Job {
        id: i64,
        kind: String,
        payload: Value,
        lease: String,
        lease_expires_at: i64,
        attempt: i64,
    },
    Idle {
        retry_after_secs: i64,
    },
    Lease {
        id: i64,
        lease_expires_at: i64,
    },
    Ack {
        id: i64,
        status: String,
    },
    Error {
        code: String,
        error: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<i64>,
    },
}

impl ServerFrame {
    fn error(code: &str, error: &str, id: Option<i64>) -> Self {
        ServerFrame::Error {
            code: code.to_string(),
            error: error.to_string(),
            id,
        }
    }

    fn to_message(&self) -> Message {
        Message::Text(
            serde_json::to_string(self)
                .expect("server frames serialize")
                .into(),
        )
    }
}

// ---------------------------------------------------------------------------
// Connected-worker registry (process-wide; the admin list shows it)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct Seen {
    ip: IpAddr,
    name: String,
    capabilities: Vec<String>,
    version: String,
    connected_at: i64,
    last_seen_at: i64,
    job_id: Option<i64>,
    /// `worker_credential.public_id`; empty until `hello`.
    credential_id: String,
}

static WORKERS: std::sync::LazyLock<Mutex<HashMap<u64, Seen>>> =
    std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));
static NEXT_CONN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// One registry slot, held for the socket's lifetime and released on
/// drop (also enforces [`MAX_SOCKETS_PER_IP`]).
struct Slot(u64);

impl Slot {
    fn acquire(ip: IpAddr) -> Option<Self> {
        let mut workers = WORKERS.lock().expect("worker registry poisoned");
        if workers.values().filter(|seen| seen.ip == ip).count() >= MAX_SOCKETS_PER_IP {
            return None;
        }
        let id = NEXT_CONN.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let now = unix_now();
        workers.insert(
            id,
            Seen {
                ip,
                name: String::new(),
                capabilities: Vec::new(),
                version: String::new(),
                connected_at: now,
                last_seen_at: now,
                job_id: None,
                credential_id: String::new(),
            },
        );
        Some(Self(id))
    }

    fn update(&self, f: impl FnOnce(&mut Seen)) {
        let mut workers = WORKERS.lock().expect("worker registry poisoned");
        if let Some(seen) = workers.get_mut(&self.0) {
            seen.last_seen_at = unix_now();
            f(seen);
        }
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        WORKERS
            .lock()
            .expect("worker registry poisoned")
            .remove(&self.0);
    }
}

fn connected_workers() -> Vec<Value> {
    let workers = WORKERS.lock().expect("worker registry poisoned");
    let mut rows: Vec<(&u64, &Seen)> = workers.iter().filter(|(_, s)| !s.name.is_empty()).collect();
    rows.sort_by_key(|(conn, s)| (s.connected_at, **conn));
    rows.iter()
        .map(|(conn, s)| {
            json!({
                "conn": conn, "name": s.name, "capabilities": s.capabilities,
                "version": s.version, "version_matches": version_matches(&s.version, env!("BUILD_SHA")),
                "credential_id": s.credential_id,
                "connected_at": s.connected_at, "last_seen_at": s.last_seen_at,
                "job_id": s.job_id,
            })
        })
        .collect()
}

/// Whether a worker's `<semver>+<sha>` names this server's build.
/// `None` when either side has no SHA (a dev build, an old worker).
fn version_matches(worker: &str, server_sha: &str) -> Option<bool> {
    let theirs = worker.split_once('+').map(|(_, sha)| sha.trim())?;
    let known = |sha: &str| sha.len() >= 7 && sha.bytes().all(|b| b.is_ascii_hexdigit());
    if !known(theirs) || !known(server_sha) {
        return None;
    }
    Some(theirs.starts_with(server_sha) || server_sha.starts_with(theirs))
}

// ---------------------------------------------------------------------------
// Auth
// ---------------------------------------------------------------------------

fn json_error(status: StatusCode, error: &str) -> Response {
    (status, Json(json!({ "error": error }))).into_response()
}

/// `Authorization: Bearer <token>` value, if present.
fn bearer(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
        .map(str::trim)
        .filter(|t| !t.is_empty())
}

/// Why a token check could not admit (a response only at the edge).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Refused {
    /// The store did not open.
    Store,
    /// No token, or one that matches no credential.
    Unauthorized,
    /// The credential was revoked on the Workers tab.
    Revoked,
}

impl Refused {
    fn response(self) -> Response {
        match self {
            Refused::Store => store_unavailable(),
            Refused::Unauthorized => json_error(StatusCode::UNAUTHORIZED, "unauthorized"),
            Refused::Revoked => json_error(StatusCode::UNAUTHORIZED, "revoked"),
        }
    }
}

/// A socket's credential: the enrolled worker it speaks for.
#[derive(Debug, Clone)]
struct Admitted {
    /// `worker_credential.public_id` (the middle of the `w2.` token).
    credential_id: String,
    name: String,
}

/// Check the bearer `w2.` token against `worker_credential` (only its
/// hash is stored). Open sockets re-check the credential on every ping,
/// so a revoke closes them.
fn admit(state: &AppState, headers: &HeaderMap) -> Result<Admitted, Refused> {
    let Some(presented) = bearer(headers) else {
        return Err(Refused::Unauthorized);
    };
    let conn = open_work(state).ok_or(Refused::Store)?;
    match work_enrol::authenticate(&conn, presented) {
        Ok(work_enrol::Authn::Active { public_id, name }) => Ok(Admitted {
            credential_id: public_id,
            name,
        }),
        Ok(work_enrol::Authn::Revoked) => {
            tracing::warn!("worker ws upgrade refused: revoked token");
            Err(Refused::Revoked)
        }
        Ok(work_enrol::Authn::Unknown) => {
            tracing::warn!("worker ws upgrade refused: bad or missing token");
            Err(Refused::Unauthorized)
        }
        Err(e) => {
            tracing::warn!("worker ws: credential lookup: {e}");
            Err(Refused::Store)
        }
    }
}

/// The values job kinds read, sent in `welcome.settings` and held in
/// the worker's memory only (it reads no `.env`). Resolved the way this
/// server resolves them for its own calls: `meta.api_key` with
/// `$META_API_KEY` first, the call-cache rows with their environment
/// overrides, and the effective `hf.mode`. Unset values are left out.
fn worker_settings(state: &AppState) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let Ok(conn) = db::open(&Store::new(&state.store_root)) else {
        return out;
    };
    let kek = state.kek.as_deref();
    let runtime = kek.and_then(|kek| {
        config::RuntimeConfig::load(&conn, kek)
            .ok()
            .map(|(cfg, _)| cfg)
    });
    let key = std::env::var("META_API_KEY")
        .ok()
        .filter(|k| !k.is_empty())
        .or_else(|| {
            runtime
                .as_ref()
                .and_then(|cfg| cfg.meta_api_key.as_ref())
                .map(|k| k.as_str().to_string())
                .filter(|k| !k.is_empty())
        });
    if let Some(key) = key {
        out.insert("meta_api_key".to_string(), key);
    }
    if let Some(base) = runtime
        .as_ref()
        .map(|cfg| cfg.meta_base_url.clone())
        .filter(|b| !b.is_empty())
    {
        out.insert("meta_base_url".to_string(), base);
    }
    let cache = crate::call_cache::settings(&conn, kek);
    out.insert(
        "meta_cache_mode".to_string(),
        cache.mode.as_str().to_string(),
    );
    out.insert(
        "fetch_cache_mode".to_string(),
        cache.fetch_mode.as_str().to_string(),
    );
    out.insert(
        "meta_daily_call_cap".to_string(),
        cache.daily_call_cap.to_string(),
    );
    if let Some(mode) = state.fakes.get(config::Integration::HuggingFace) {
        out.insert("hf_mode".to_string(), mode.as_str().to_string());
    }
    if let Some(kek) = kek {
        for (key, row) in [
            ("glimmer_model", config::keys::WORKER_GLIMMER_MODEL),
            ("glimmer_url", config::keys::WORKER_GLIMMER_URL),
        ] {
            if let Ok(Some(value)) = config::get_text_for_job(&conn, kek, row)
                && !value.trim().is_empty()
            {
                out.insert(key.to_string(), value.trim().to_string());
            }
        }
    }
    out
}

/// Digest of [`worker_settings`], so a ping can tell they changed
/// without keeping the key itself around.
fn settings_digest(settings: &BTreeMap<String, String>) -> [u8; 32] {
    let mut hash = Sha256::new();
    for (key, value) in settings {
        hash.update(key.as_bytes());
        hash.update([0]);
        hash.update(value.as_bytes());
        hash.update([0]);
    }
    hash.finalize().into()
}

// ---------------------------------------------------------------------------
// WebSocket
// ---------------------------------------------------------------------------

/// `GET /apostille/api/v1/worker/ws`: token check, per-IP cap, upgrade.
async fn ws_upgrade(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Response {
    let admitted = match admit(&state, &headers) {
        Ok(admitted) => admitted,
        Err(refused) => return refused.response(),
    };
    let Some(slot) = Slot::acquire(peer.ip()) else {
        return json_error(StatusCode::TOO_MANY_REQUESTS, "too many worker sockets");
    };
    ws.max_message_size(MAX_FRAME_BYTES)
        .max_frame_size(MAX_FRAME_BYTES)
        .on_upgrade(move |socket| run_socket(state, socket, slot, admitted))
}

/// The authenticated worker on one socket.
#[derive(Clone)]
struct Session {
    name: String,
    capabilities: Vec<String>,
}

impl Session {
    fn owner(&self, lease: &str) -> String {
        format!("{}:{lease}", self.name)
    }
}

async fn run_socket(state: AppState, mut socket: WebSocket, slot: Slot, admitted: Admitted) {
    let hello = tokio::time::timeout(Duration::from_secs(HELLO_TIMEOUT_SECS), socket.recv()).await;
    let session = match hello {
        Ok(Some(Ok(Message::Text(text)))) => match serde_json::from_str(text.as_str()) {
            Ok(ClientFrame::Hello {
                worker,
                capabilities,
                version,
            }) => {
                // The credential names the worker; hello must agree.
                if worker != admitted.name {
                    let frame = ServerFrame::error(
                        "bad_frame",
                        &format!("this token is for worker {}", admitted.name),
                        None,
                    );
                    let _ = socket.send(frame.to_message()).await;
                    return;
                }
                let capabilities: Vec<String> =
                    capabilities.into_iter().filter(|k| known_kind(k)).collect();
                let version: String = version.chars().take(64).collect();
                slot.update(|seen| {
                    seen.name = worker.clone();
                    seen.capabilities = capabilities.clone();
                    seen.version = version.clone();
                    seen.credential_id = admitted.credential_id.clone();
                });
                let (touch, id, v) = (state.clone(), admitted.credential_id.clone(), version);
                let _ = tokio::task::spawn_blocking(move || {
                    open_work(&touch).map(|conn| work_enrol::touch(&conn, &id, &v, unix_now()))
                })
                .await;
                Session {
                    name: worker,
                    capabilities,
                }
            }
            _ => {
                let frame =
                    ServerFrame::error("bad_frame", "first frame must be a valid hello", None);
                let _ = socket.send(frame.to_message()).await;
                return;
            }
        },
        _ => return,
    };
    let load = state.clone();
    let settings = tokio::task::spawn_blocking(move || worker_settings(&load))
        .await
        .unwrap_or_default();
    let digest = settings_digest(&settings);
    let welcome = ServerFrame::Welcome {
        worker: session.name.clone(),
        lease_secs: LEASE_SECS,
        heartbeat_secs: HEARTBEAT_SECS,
        max_result_bytes: MAX_RESULT_BYTES,
        settings,
    };
    if socket.send(welcome.to_message()).await.is_err() {
        return;
    }
    tracing::info!(worker = %session.name, "worker connected");
    let mut ping = tokio::time::interval(Duration::from_secs(PING_SECS));
    ping.tick().await;
    let mut heard = tokio::time::Instant::now();
    let mut job: Option<i64> = None;
    loop {
        tokio::select! {
            _ = ping.tick() => {
                if heard.elapsed() > Duration::from_secs(IDLE_SECS) {
                    tracing::info!(worker = %session.name, "worker silent for {IDLE_SECS}s; closing");
                    break;
                }
                let (check, id) = (state.clone(), admitted.credential_id.clone());
                let (active, now_digest) = tokio::task::spawn_blocking(move || {
                    let active = open_work(&check)
                        .map(|conn| work_enrol::is_active(&conn, &id).unwrap_or(true))
                        .unwrap_or(true);
                    (active, settings_digest(&worker_settings(&check)))
                })
                .await
                .unwrap_or((true, digest));
                if !active {
                    tracing::info!(worker = %session.name, "worker credential revoked; closing");
                    let frame = ServerFrame::error("unauthorized", "worker token revoked", None);
                    let _ = socket.send(frame.to_message()).await;
                    break;
                }
                // New settings reach an idle worker by reconnecting; a
                // running job finishes with the values it started with.
                if now_digest != digest && job.is_none() {
                    tracing::info!(worker = %session.name, "worker settings changed; closing to resend");
                    break;
                }
                if socket.send(Message::Ping(Bytes::new())).await.is_err() {
                    break;
                }
            }
            msg = socket.recv() => {
                heard = tokio::time::Instant::now();
                let reply = match msg {
                    Some(Ok(Message::Text(text))) => {
                        let (state, session) = (state.clone(), session.clone());
                        let text = text.to_string();
                        tokio::task::spawn_blocking(move || handle_frame(&state, &session, &text))
                            .await
                            .unwrap_or_else(|_| {
                                ServerFrame::error("store", "frame handler panicked", None)
                            })
                    }
                    Some(Ok(Message::Binary(_))) => {
                        ServerFrame::error("bad_frame", "frames are JSON text", None)
                    }
                    Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                    Some(Ok(_)) => continue,
                };
                match &reply {
                    ServerFrame::Job { id, .. } => job = Some(*id),
                    ServerFrame::Ack { .. } => job = None,
                    ServerFrame::Error { code, .. } if code == "lease_lost" => job = None,
                    _ => {}
                }
                slot.update(|seen| seen.job_id = job);
                if socket.send(reply.to_message()).await.is_err() {
                    break;
                }
            }
        }
    }
    tracing::info!(worker = %session.name, "worker disconnected");
}

fn open_work(state: &AppState) -> Option<rusqlite::Connection> {
    let conn = work::open(&Store::new(&state.store_root)).ok()?;
    work::migrate(&conn).ok()?;
    Some(conn)
}

/// One inbound frame → one reply (blocking: SQLite + blob I/O).
fn handle_frame(state: &AppState, session: &Session, text: &str) -> ServerFrame {
    let frame: ClientFrame = match serde_json::from_str(text) {
        Ok(frame) => frame,
        Err(_) => return ServerFrame::error("bad_frame", "unparseable frame", None),
    };
    let Some(conn) = open_work(state) else {
        return ServerFrame::error("store", "work queue unavailable", None);
    };
    let now = unix_now();
    let lost = |id| ServerFrame::error("lease_lost", "lease expired or not yours", Some(id));
    let store_err = |id| ServerFrame::error("store", "work queue write failed", Some(id));
    match frame {
        ClientFrame::Hello { .. } => ServerFrame::error("bad_frame", "hello already sent", None),
        ClientFrame::Checkout => {
            let lease = format!("{:032x}", rand::random::<u128>());
            let owner = session.owner(&lease);
            let req = work::Checkout {
                owner: &owner,
                actor: &session.name,
                kinds: &session.capabilities,
                lease_secs: LEASE_SECS,
                now,
            };
            match work::checkout(&conn, &req) {
                Ok(Some(item)) => ServerFrame::Job {
                    id: item.id,
                    payload: serde_json::from_str(&item.payload_json).unwrap_or(json!({})),
                    kind: item.kind,
                    lease,
                    lease_expires_at: item.lease_expires_at.unwrap_or(now + LEASE_SECS),
                    attempt: item.attempts,
                },
                Ok(None) => ServerFrame::Idle {
                    retry_after_secs: IDLE_RETRY_SECS,
                },
                Err(e) => {
                    tracing::warn!("worker checkout: {e}");
                    ServerFrame::error("store", "checkout failed", None)
                }
            }
        }
        ClientFrame::Heartbeat { id, lease } => {
            let owner = session.owner(&lease);
            let limit = work::get(&conn, id)
                .ok()
                .flatten()
                .map(|item| max_runtime(&item.kind));
            if let Some(limit) = limit {
                match work::end_if_overrun(&conn, id, &owner, limit, now) {
                    Ok(true) => {
                        return ServerFrame::error(
                            "lease_lost",
                            "job ran over its time limit",
                            Some(id),
                        );
                    }
                    Ok(false) => {}
                    Err(_) => return store_err(id),
                }
            }
            match work::heartbeat(&conn, id, &owner, LEASE_SECS, now) {
                Ok(Some(lease_expires_at)) => ServerFrame::Lease {
                    id,
                    lease_expires_at,
                },
                Ok(None) => lost(id),
                Err(_) => store_err(id),
            }
        }
        ClientFrame::Complete {
            id,
            lease,
            content_type,
            sha256,
            result_b64,
            summary,
        } => {
            let result = match result_b64 {
                None => None,
                Some(b64) => match decode_result(&b64, sha256.as_deref(), MAX_RESULT_BYTES) {
                    Ok(bytes) => {
                        let Ok(sha) = Store::new(&state.store_root).write_blob(&bytes) else {
                            return store_err(id);
                        };
                        Some(work::ResultRef {
                            sha256: sha,
                            bytes: bytes.len() as i64,
                            content_type: clean_content_type(content_type.as_deref()),
                        })
                    }
                    Err((code, error)) => return ServerFrame::error(code, error, Some(id)),
                },
            };
            let summary = summary.map(|v| {
                let text = v.to_string();
                if text.len() > MAX_SUMMARY_BYTES {
                    json!({"dropped": "summary over 16 KiB"}).to_string()
                } else {
                    text
                }
            });
            let owner = session.owner(&lease);
            let done = work::complete(
                &conn,
                id,
                &owner,
                &session.name,
                result.as_ref(),
                summary.as_deref(),
                now,
            );
            match done {
                Ok(true) => ServerFrame::Ack {
                    id,
                    status: "done".to_string(),
                },
                Ok(false) => lost(id),
                Err(_) => store_err(id),
            }
        }
        ClientFrame::Fail {
            id,
            lease,
            error,
            retryable,
        } => {
            let owner = session.owner(&lease);
            match work::fail(&conn, id, &owner, &session.name, &error, retryable, now) {
                Ok(Some(outcome)) => ServerFrame::Ack {
                    id,
                    status: outcome.status().to_string(),
                },
                Ok(None) => lost(id),
                Err(_) => store_err(id),
            }
        }
    }
}

/// Decode an uploaded result: base64, size cap, optional sha256 check.
fn decode_result(
    b64: &str,
    sha256: Option<&str>,
    cap: usize,
) -> Result<Vec<u8>, (&'static str, &'static str)> {
    // Cheap pre-check before allocating: base64 is 4 chars per 3 bytes.
    if b64.len() / 4 * 3 > cap + 3 {
        return Err(("too_large", "result exceeds max_result_bytes"));
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(b64.as_bytes())
        .map_err(|_| ("bad_frame", "result_b64 is not base64"))?;
    if bytes.len() > cap {
        return Err(("too_large", "result exceeds max_result_bytes"));
    }
    if let Some(want) = sha256
        && !handbook_store::store::sha256_hex(&bytes).eq_ignore_ascii_case(want)
    {
        return Err(("sha_mismatch", "result bytes do not match sha256"));
    }
    Ok(bytes)
}

/// A plain `type/subtype` media type, else `application/octet-stream`
/// (the value is echoed in a download header, so no parameters).
fn clean_content_type(value: Option<&str>) -> String {
    let ok = |s: &str| {
        let mut parts = s.split('/');
        let (Some(a), Some(b), None) = (parts.next(), parts.next(), parts.next()) else {
            return false;
        };
        [a, b].iter().all(|p| {
            !p.is_empty()
                && p.len() <= 64
                && p.bytes()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'+' | b'-'))
        })
    };
    match value {
        Some(v) if ok(v) => v.to_ascii_lowercase(),
        _ => "application/octet-stream".to_string(),
    }
}

// ---------------------------------------------------------------------------
// Staff API
// ---------------------------------------------------------------------------

/// Where "Stage as packs" writes one item's pack files (the staff
/// Packs page loads from here): `<store>/packs-inbox/<id>/`.
pub(crate) const PACKS_INBOX: &str = "packs-inbox";

fn inbox_dir(state: &AppState, id: i64) -> std::path::PathBuf {
    state.store_root.join(PACKS_INBOX).join(id.to_string())
}

fn item_json(item: &work::WorkItem, staged: bool) -> Value {
    json!({
        "id": item.id,
        "kind": item.kind,
        "status": item.status,
        "attempts": item.attempts,
        "max_attempts": item.max_attempts,
        "lease_owner": item.lease_worker(),
        "lease_expires_at": item.lease_expires_at,
        "created_by": item.created_by,
        "created_at": item.created_at,
        "updated_at": item.updated_at,
        "error": item.error,
        "result": item.result.as_ref().map(|r| json!({
            "sha256": r.sha256, "bytes": r.bytes, "content_type": r.content_type,
        })),
        "summary": item.summary_json.as_deref().and_then(|s| serde_json::from_str::<Value>(s).ok()),
        "payload": serde_json::from_str::<Value>(&item.payload_json).unwrap_or(json!({})),
        "staged": staged,
        "retry_after": item.retry_after,
    })
}

#[derive(Deserialize)]
struct ListParams {
    status: Option<String>,
    limit: Option<i64>,
    before_id: Option<i64>,
}

async fn list(
    _auth: AdminAuth,
    State(state): State<AppState>,
    Query(params): Query<ListParams>,
) -> Response {
    let Some(conn) = open_work(&state) else {
        return store_unavailable();
    };
    let status = params
        .status
        .as_deref()
        .filter(|s| matches!(*s, "pending" | "leased" | "done" | "failed"));
    // Lazy sweep: expired leases requeue whenever anyone looks.
    let _ = work::requeue_expired(&conn, unix_now());
    let (Ok(items), Ok(counts)) = (
        work::list(&conn, status, params.limit.unwrap_or(50), params.before_id),
        work::counts(&conn),
    ) else {
        return store_unavailable();
    };
    let kinds: Vec<Value> = KINDS
        .iter()
        .map(|k| {
            json!({
                "kind": k.kind, "about": k.about,
                "example": serde_json::from_str::<Value>(k.example).unwrap_or(json!({})),
                "max_runtime_secs": k.max_runtime_secs,
            })
        })
        .collect();
    Json(json!({
        "items": items
            .iter()
            .map(|item| item_json(item, inbox_dir(&state, item.id).is_dir()))
            .collect::<Vec<_>>(),
        "workers": connected_workers(),
        "kinds": kinds,
        "counts": counts
            .into_iter()
            .map(|(status, n)| (status, json!(n)))
            .collect::<serde_json::Map<String, Value>>(),
    }))
    .into_response()
}

#[derive(Deserialize)]
struct EnqueueBody {
    kind: String,
    #[serde(default = "empty_object")]
    payload: Value,
    max_attempts: Option<i64>,
}

fn empty_object() -> Value {
    json!({})
}

async fn enqueue(auth: AdminAuth, State(state): State<AppState>, body: Bytes) -> Response {
    let Ok(body) = serde_json::from_slice::<EnqueueBody>(&body) else {
        return json_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "want {kind, payload, max_attempts?}",
        );
    };
    if !known_kind(&body.kind) {
        return json_error(StatusCode::UNPROCESSABLE_ENTITY, "unknown kind");
    }
    if !body.payload.is_object() {
        return json_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "payload must be a JSON object",
        );
    }
    let payload = body.payload.to_string();
    if payload.len() > MAX_PAYLOAD_BYTES {
        return json_error(StatusCode::PAYLOAD_TOO_LARGE, "payload over 64 KiB");
    }
    let max_attempts = body.max_attempts.unwrap_or(work::DEFAULT_MAX_ATTEMPTS);
    if !(1..=10).contains(&max_attempts) {
        return json_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "max_attempts must be 1-10",
        );
    }
    let Some(conn) = open_work(&state) else {
        return store_unavailable();
    };
    let new = work::NewItem {
        kind: &body.kind,
        payload_json: &payload,
        max_attempts,
        actor: &auth.user.user_id,
        at: unix_now(),
    };
    match work::enqueue(&conn, &new) {
        Ok(id) => (
            StatusCode::CREATED,
            Json(json!({"id": id, "status": "pending"})),
        )
            .into_response(),
        Err(_) => store_unavailable(),
    }
}

async fn detail(_auth: AdminAuth, State(state): State<AppState>, Path(id): Path<i64>) -> Response {
    let Some(conn) = open_work(&state) else {
        return store_unavailable();
    };
    match (work::get(&conn, id), work::audit_for(&conn, id)) {
        (Ok(Some(item)), Ok(audit)) => Json(json!({
            "item": item_json(&item, inbox_dir(&state, id).is_dir()),
            "audit": audit.iter().map(|row| json!({
                "at": row.at, "actor": row.actor, "action": row.action, "reason": row.reason,
            })).collect::<Vec<_>>(),
        }))
        .into_response(),
        (Ok(None), _) => json_error(StatusCode::NOT_FOUND, "no such work item"),
        _ => store_unavailable(),
    }
}

/// `GET …/work-items/{id}/result`: the uploaded bytes as an attachment
/// (the builder's "Load packs" reads this for `research.nonparty`).
async fn result(_auth: AdminAuth, State(state): State<AppState>, Path(id): Path<i64>) -> Response {
    let Some(conn) = open_work(&state) else {
        return store_unavailable();
    };
    let item = match work::get(&conn, id) {
        Ok(Some(item)) => item,
        Ok(None) => return json_error(StatusCode::NOT_FOUND, "no such work item"),
        Err(_) => return store_unavailable(),
    };
    let Some(result) = item.result.as_ref() else {
        return json_error(StatusCode::NOT_FOUND, "no result yet");
    };
    let Ok(bytes) = Store::new(&state.store_root).read_blob(&result.sha256) else {
        return store_unavailable();
    };
    let ext = if result.content_type == "application/json" {
        "json"
    } else {
        "bin"
    };
    let filename = format!("work-{id}-{}.{ext}", item.kind.replace('.', "-"));
    let mut res = bytes.into_response();
    let headers = res.headers_mut();
    if let Ok(value) = HeaderValue::from_str(&result.content_type) {
        headers.insert(header::CONTENT_TYPE, value);
    }
    if let Ok(value) = HeaderValue::from_str(&format!("attachment; filename=\"{filename}\"")) {
        headers.insert(header::CONTENT_DISPOSITION, value);
    }
    if let Ok(value) = HeaderValue::from_str(&result.sha256) {
        headers.insert("x-content-sha256", value);
    }
    res
}

async fn retry(auth: AdminAuth, State(state): State<AppState>, Path(id): Path<i64>) -> Response {
    let Some(conn) = open_work(&state) else {
        return store_unavailable();
    };
    match work::retry(&conn, id, &auth.user.user_id, unix_now()) {
        Ok(true) => Json(json!({"id": id, "status": "pending"})).into_response(),
        Ok(false) => match work::get(&conn, id) {
            Ok(None) => json_error(StatusCode::NOT_FOUND, "no such work item"),
            _ => json_error(StatusCode::CONFLICT, "only failed items retry"),
        },
        Err(_) => store_unavailable(),
    }
}

async fn cancel(auth: AdminAuth, State(state): State<AppState>, Path(id): Path<i64>) -> Response {
    let Some(conn) = open_work(&state) else {
        return store_unavailable();
    };
    match work::cancel(&conn, id, &auth.user.user_id, unix_now()) {
        Ok(true) => Json(json!({"id": id, "status": "failed"})).into_response(),
        Ok(false) => match work::get(&conn, id) {
            Ok(None) => json_error(StatusCode::NOT_FOUND, "no such work item"),
            _ => json_error(StatusCode::CONFLICT, "only pending or leased items cancel"),
        },
        Err(_) => store_unavailable(),
    }
}

/// A pack id safe as a file stem (`EG`, `usa-wa`): 1-64 of `[A-Za-z0-9_-]`.
fn valid_pack_id(id: &str) -> bool {
    (1..=64).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// The bundle format each stageable kind uploads: `research.nonparty`
/// a `packs.v1` bundle, `discovery.bootstrap` a `discovery.v1` report
/// whose `packs` are world packs with new watch seeds (D-3).
fn stage_format(kind: &str) -> Option<&'static str> {
    match kind {
        "research.nonparty" => Some("packs.v1"),
        "discovery.bootstrap" => Some("discovery.v1"),
        _ => None,
    }
}

/// Pack files out of a result bundle of `format`: `(stem, pretty JSON)`
/// each. Shape checks only (object, safe unique id); the Packs page
/// runs the full pack rules (`xtask packs-load`) before anything loads.
fn bundle_packs(bytes: &[u8], format: &str) -> Result<Vec<(String, Vec<u8>)>, &'static str> {
    let bundle: Value = serde_json::from_slice(bytes).map_err(|_| "result is not JSON")?;
    if bundle["format"] != format {
        return Err("result is not a bundle of this kind's format");
    }
    let packs = bundle["packs"]
        .as_array()
        .ok_or("bundle has no packs array")?;
    let mut out: Vec<(String, Vec<u8>)> = Vec::with_capacity(packs.len());
    for pack in packs {
        let id = pack["pack"].as_str().filter(|id| valid_pack_id(id));
        let Some(id) = id else {
            return Err("a pack has no safe pack id");
        };
        if out.iter().any(|(stem, _)| stem == id) {
            return Err("duplicate pack id in bundle");
        }
        let mut file = serde_json::to_vec_pretty(pack).expect("value serializes");
        file.push(b'\n');
        out.push((id.to_string(), file));
    }
    Ok(out)
}

/// Write `files` as `<dir>/<stem>.json`, replacing any earlier staging
/// whole (staged in a sibling temp dir, then renamed into place).
fn write_inbox(dir: &std::path::Path, files: &[(String, Vec<u8>)]) -> std::io::Result<()> {
    let parent = dir.parent().expect("inbox dir has a parent");
    std::fs::create_dir_all(parent)?;
    let tmp = parent.join(format!(".tmp-{:016x}", rand::random::<u64>()));
    std::fs::create_dir(&tmp)?;
    let written = files
        .iter()
        .try_for_each(|(stem, bytes)| std::fs::write(tmp.join(format!("{stem}.json")), bytes));
    if let Err(e) = written {
        let _ = std::fs::remove_dir_all(&tmp);
        return Err(e);
    }
    if dir.exists() {
        std::fs::remove_dir_all(dir)?;
    }
    std::fs::rename(&tmp, dir)
}

/// Staged discovery packs that copy a world pack this release no longer
/// has byte for byte: the ids whose `pack_bases` sha256 (the committed
/// file the worker copied) differs from `<world_dir>/<id>.json`. Loading
/// such a copy would set rows back to the worker's names and close the
/// tasks of lines the release changed, so it is refused.
fn stale_world_copies(world_dir: &std::path::Path, bases: &Value, stems: &[&str]) -> Vec<String> {
    stems
        .iter()
        .filter(|stem| {
            let release = std::fs::read(world_dir.join(format!("{stem}.json")))
                .map(|bytes| handbook_store::store::sha256_hex(&bytes))
                .ok();
            release.is_none() || bases[**stem].as_str() != release.as_deref()
        })
        .map(|stem| (*stem).to_string())
        .collect()
}

/// `POST …/work-items/{id}/stage-packs`: a done `research.nonparty` or
/// `discovery.bootstrap` result's packs land in
/// `<store>/packs-inbox/<id>/` for the Packs page to load. Re-staging
/// replaces the folder. Audited `work_stage`.
async fn stage_packs(
    auth: AdminAuth,
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Response {
    let Some(conn) = open_work(&state) else {
        return store_unavailable();
    };
    let item = match work::get(&conn, id) {
        Ok(Some(item)) => item,
        Ok(None) => return json_error(StatusCode::NOT_FOUND, "no such work item"),
        Err(_) => return store_unavailable(),
    };
    let (result, format) = match (&item.result, stage_format(&item.kind), item.status.as_str()) {
        (Some(result), Some(format), "done") => (result, format),
        _ => {
            return json_error(
                StatusCode::CONFLICT,
                "only a done research.nonparty or discovery.bootstrap item with a result \
                 stages as packs",
            );
        }
    };
    let Ok(bytes) = Store::new(&state.store_root).read_blob(&result.sha256) else {
        return store_unavailable();
    };
    let files = match bundle_packs(&bytes, format) {
        Ok(files) if files.is_empty() => {
            return json_error(StatusCode::UNPROCESSABLE_ENTITY, "the result has no packs");
        }
        Ok(files) => files,
        Err(why) => return json_error(StatusCode::UNPROCESSABLE_ENTITY, why),
    };
    // Discovery packs are whole copies of release world packs (same
    // ids): only a copy of the release's own file may stage.
    if format == "discovery.v1"
        && let Some((packs, _)) = crate::admin_packs::release_paths()
    {
        let bases = serde_json::from_slice::<Value>(&bytes)
            .map(|bundle| bundle["pack_bases"].clone())
            .unwrap_or_default();
        let stems: Vec<&str> = files.iter().map(|(stem, _)| stem.as_str()).collect();
        let stale = stale_world_copies(&packs.join("world"), &bases, &stems);
        if !stale.is_empty() {
            return json_error(
                StatusCode::CONFLICT,
                &format!(
                    "world packs {stale:?} differ from the ones this run copied; re-run \
                     discovery on this release"
                ),
            );
        }
    }
    let dir = inbox_dir(&state, id);
    if let Err(e) = write_inbox(&dir, &files) {
        tracing::warn!("stage packs for work item {id}: {e}");
        return store_unavailable();
    }
    let reason = format!("{} pack(s) to {PACKS_INBOX}/{id}", files.len());
    if work::note(
        &conn,
        id,
        &auth.user.user_id,
        "work_stage",
        &reason,
        unix_now(),
    )
    .is_err()
    {
        return store_unavailable();
    }
    Json(json!({
        "id": id,
        "dir": format!("{PACKS_INBOX}/{id}"),
        "packs": files.iter().map(|(stem, _)| stem).collect::<Vec<_>>(),
        "count": files.len(),
    }))
    .into_response()
}

// ---------------------------------------------------------------------------
// Enrolment (docs/specs/worker-enrolment.md)
// ---------------------------------------------------------------------------

/// Largest enrol or poll body.
const MAX_ENROL_BODY: usize = 4 * 1024;

/// `worker.enrolment` is not `off` (unset, no KEK or unreadable: on).
fn enrolment_on(state: &AppState) -> bool {
    let Some(kek) = state.kek.as_deref() else {
        return true;
    };
    let Ok(conn) = db::open(&Store::new(&state.store_root)) else {
        return true;
    };
    !matches!(
        config::get_text_for_job(&conn, kek, config::keys::WORKER_ENROLMENT),
        Ok(Some(value)) if value.trim().eq_ignore_ascii_case("off")
    )
}

#[derive(Deserialize)]
struct EnrolBody {
    username: String,
    hostname: String,
    #[serde(default)]
    version: String,
    #[serde(default)]
    started_at: i64,
    #[serde(default)]
    capabilities: Vec<String>,
    poll_hash: String,
}

/// `POST /apostille/api/v1/worker/enrol` (public, unauthenticated): file
/// a request; the worker prints the returned code for staff to type.
async fn enrol(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    body: Bytes,
) -> Response {
    if body.len() > MAX_ENROL_BODY {
        return json_error(StatusCode::PAYLOAD_TOO_LARGE, "enrol body over 4 KiB");
    }
    let Ok(body) = serde_json::from_slice::<EnrolBody>(&body) else {
        return json_error(StatusCode::UNPROCESSABLE_ENTITY, "bad enrol request");
    };
    let result = tokio::task::spawn_blocking(move || {
        if !enrolment_on(&state) {
            return Err((StatusCode::SERVICE_UNAVAILABLE, "worker enrolment is off"));
        }
        let unavailable = (StatusCode::SERVICE_UNAVAILABLE, "store unavailable");
        let conn = open_work(&state).ok_or(unavailable)?;
        let capabilities: Vec<&String> =
            body.capabilities.iter().filter(|k| known_kind(k)).collect();
        let version: String = body.version.chars().take(64).collect();
        let ip = peer.ip().to_string();
        let filed = work_enrol::request(
            &conn,
            &work_enrol::NewRequest {
                username: &body.username,
                hostname: &body.hostname,
                version: &version,
                started_at: body.started_at,
                capabilities_json: &json!(capabilities).to_string(),
                peer_ip: &ip,
                poll_hash: &body.poll_hash,
                now: unix_now(),
            },
        )
        .map_err(|_| unavailable)?;
        match filed {
            Ok(filed) => {
                tracing::info!(
                    worker = %format!("{}@{}", body.username, body.hostname),
                    ip = %ip,
                    "worker enrol request filed"
                );
                Ok(filed)
            }
            Err(work_enrol::Refusal::Invalid) => Err((
                StatusCode::UNPROCESSABLE_ENTITY,
                "username and hostname are lowercase a-z, 0-9, . _ -; poll_hash is 64 hex",
            )),
            Err(work_enrol::Refusal::TooManyPending) => Err((
                StatusCode::TOO_MANY_REQUESTS,
                "too many enrol requests waiting",
            )),
            Err(work_enrol::Refusal::TooManyFromIp) => Err((
                StatusCode::TOO_MANY_REQUESTS,
                "too many enrol requests from this address",
            )),
        }
    })
    .await
    .unwrap_or(Err((StatusCode::SERVICE_UNAVAILABLE, "store unavailable")));
    match result {
        Ok(filed) => (
            StatusCode::ACCEPTED,
            Json(json!({
                "id": filed.public_id, "code": filed.code,
                "poll_after_secs": 5, "expires_at": filed.expires_at,
            })),
        )
            .into_response(),
        Err((StatusCode::SERVICE_UNAVAILABLE, "store unavailable")) => store_unavailable(),
        Err((status, error)) => json_error(status, error),
    }
}

#[derive(Deserialize)]
struct PollBody {
    poll_secret: String,
}

/// `POST /apostille/api/v1/worker/enrol/{id}/poll`: the worker's
/// answer; an approved request yields its token here, once.
async fn enrol_poll(
    State(state): State<AppState>,
    Path(id): Path<String>,
    body: Bytes,
) -> Response {
    if body.len() > MAX_ENROL_BODY {
        return json_error(StatusCode::PAYLOAD_TOO_LARGE, "poll body over 4 KiB");
    }
    let Ok(body) = serde_json::from_slice::<PollBody>(&body) else {
        return json_error(StatusCode::UNPROCESSABLE_ENTITY, "bad poll request");
    };
    let polled = tokio::task::spawn_blocking(move || {
        let conn = open_work(&state)?;
        work_enrol::poll(&conn, &id, &body.poll_secret, unix_now()).ok()
    })
    .await
    .ok()
    .flatten();
    match polled {
        None => store_unavailable(),
        Some(work_enrol::Poll::Pending { expires_at }) => {
            Json(json!({"status": "pending", "expires_at": expires_at})).into_response()
        }
        Some(work_enrol::Poll::Approved { name, token }) => {
            tracing::info!(worker = %name, "worker token delivered");
            Json(json!({"status": "approved", "worker": name, "token": token})).into_response()
        }
        Some(work_enrol::Poll::Gone(status)) => {
            (StatusCode::GONE, Json(json!({"status": status}))).into_response()
        }
        Some(work_enrol::Poll::NotFound) => json_error(StatusCode::NOT_FOUND, "no such request"),
    }
}

#[derive(Deserialize)]
struct EnrolmentParams {
    #[serde(default)]
    revoked: Option<String>,
}

/// `GET /apostille/api/v1/admin/workers/enrolments`: pending requests
/// and credentials (never codes, tokens or hashes).
async fn enrolments(
    _auth: AdminAuth,
    State(state): State<AppState>,
    Query(params): Query<EnrolmentParams>,
) -> Response {
    let include_revoked = params
        .revoked
        .as_deref()
        .is_some_and(|v| v == "1" || v == "true");
    let on = enrolment_on(&state);
    let Some(conn) = open_work(&state) else {
        return store_unavailable();
    };
    let now = unix_now();
    let (Ok(pending), Ok(credentials)) = (
        work_enrol::pending(&conn, now),
        work_enrol::credentials(&conn, include_revoked),
    ) else {
        return store_unavailable();
    };
    let server_sha = env!("BUILD_SHA");
    let connected: HashMap<String, Option<i64>> = {
        let workers = WORKERS.lock().expect("worker registry poisoned");
        workers
            .values()
            .filter(|seen| !seen.credential_id.is_empty())
            .map(|seen| (seen.credential_id.clone(), seen.job_id))
            .collect()
    };
    Json(json!({
        "today": crate::evidence::la_date(now),
        "server_version": format!("{}+{server_sha}", env!("CARGO_PKG_VERSION")),
        "enrolment_on": on,
        "pending": pending.iter().map(|r| json!({
            "id": r.public_id, "username": r.username, "hostname": r.hostname,
            "proposed_name": format!("{}@{}", r.username, r.hostname),
            "version": r.version, "version_matches": version_matches(&r.version, server_sha),
            "started_at": r.started_at,
            "capabilities": serde_json::from_str::<Value>(&r.capabilities_json).unwrap_or(json!([])),
            "peer_ip": r.peer_ip, "created_at": r.created_at, "expires_at": r.expires_at,
            "code_tries": r.code_tries,
        })).collect::<Vec<_>>(),
        "credentials": credentials.iter().map(|c| {
            let live = connected.get(&c.public_id);
            json!({
                "id": c.public_id, "name": c.name,
                "created_at": c.created_at, "created_by": c.created_by,
                "last_seen_at": c.last_seen_at, "last_version": c.last_version,
                "version_matches": c.last_version.as_deref().and_then(|v| version_matches(v, server_sha)),
                "connected": live.is_some(), "job_id": live.copied().flatten(),
                "revoked_at": c.revoked_at, "revoked_by": c.revoked_by,
            })
        }).collect::<Vec<_>>(),
    }))
    .into_response()
}

#[derive(Deserialize)]
struct ApproveBody {
    code: String,
    name: String,
    #[serde(default)]
    add_date: bool,
}

/// Fresh step-up, like other credential changes.
fn require_fresh_step_up(state: &AppState, auth: &AdminAuth) -> Option<Response> {
    let Ok(conn) = db::open(&Store::new(&state.store_root)) else {
        return Some(store_unavailable());
    };
    let ages = crate::admin::step_up_ages(&conn, &auth.user, unix_now());
    (!ages.fresh).then(|| crate::admin::step_up_refusal(&auth.user, &ages))
}

/// `POST …/admin/workers/enrolments/{id}/approve {code, name, add_date}`.
/// The server appends the America/Los_Angeles date when asked.
async fn approve_enrolment(
    auth: AdminAuth,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<ApproveBody>,
) -> Response {
    if let Some(refusal) = require_fresh_step_up(&state, &auth) {
        return refusal;
    }
    let now = unix_now();
    let name = if body.add_date {
        format!("{} {}", body.name.trim(), crate::evidence::la_date(now))
    } else {
        body.name.trim().to_string()
    };
    let Some(conn) = open_work(&state) else {
        return store_unavailable();
    };
    match work_enrol::approve(&conn, &id, &body.code, &name, &auth.user.user_id, now) {
        Ok(work_enrol::Decision::Approved { name }) => {
            tracing::info!(worker = %name, "worker enrolment approved");
            Json(json!({"status": "approved", "name": name})).into_response()
        }
        Ok(work_enrol::Decision::WrongCode { tries_left }) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"error": "wrong pairing code", "tries_left": tries_left})),
        )
            .into_response(),
        Ok(work_enrol::Decision::DeniedAfterTries) => {
            json_error(StatusCode::CONFLICT, "denied after 3 wrong codes")
        }
        Ok(work_enrol::Decision::NameTaken) => json_error(StatusCode::CONFLICT, "name in use"),
        Ok(work_enrol::Decision::NotPending) => json_error(StatusCode::CONFLICT, "not pending"),
        Ok(work_enrol::Decision::BadName) => json_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "bad name: user@host, optionally followed by a space and YYYY-MM-DD",
        ),
        Err(_) => store_unavailable(),
    }
}

/// `POST …/admin/workers/enrolments/{id}/deny`.
async fn deny_enrolment(
    auth: AdminAuth,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    let Some(conn) = open_work(&state) else {
        return store_unavailable();
    };
    match work_enrol::deny(&conn, &id, &auth.user.user_id, unix_now()) {
        Ok(true) => Json(json!({"status": "denied"})).into_response(),
        Ok(false) => json_error(StatusCode::CONFLICT, "not pending"),
        Err(_) => store_unavailable(),
    }
}

/// `POST …/admin/workers/credentials/{id}/revoke`: the worker's socket
/// closes at its next ping; its lease runs out and the job requeues.
async fn revoke_credential(
    auth: AdminAuth,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    if let Some(refusal) = require_fresh_step_up(&state, &auth) {
        return refusal;
    }
    let Some(conn) = open_work(&state) else {
        return store_unavailable();
    };
    match work_enrol::revoke(&conn, &id, &auth.user.user_id, unix_now()) {
        Ok(Some(revoked_at)) => {
            tracing::info!(credential = %id, "worker credential revoked");
            Json(json!({"id": id, "revoked_at": revoked_at})).into_response()
        }
        Ok(None) => json_error(StatusCode::NOT_FOUND, "no such active credential"),
        Err(_) => store_unavailable(),
    }
}

/// Worker socket route, merged into the PUBLIC `/apostille` API.
pub(crate) fn public_routes() -> Router<AppState> {
    Router::new()
        .route("/api/v1/worker/ws", get(ws_upgrade))
        .route("/api/v1/worker/enrol", post(enrol))
        .route("/api/v1/worker/enrol/{id}/poll", post(enrol_poll))
}

/// Staff routes on the ADMIN listener only (via `mission_admin_routes`).
pub fn admin_router(state: AppState) -> Router {
    Router::<AppState>::new()
        .route(
            "/apostille/api/v1/admin/work-items",
            get(list).post(enqueue),
        )
        .route("/apostille/api/v1/admin/work-items/{id}", get(detail))
        .route(
            "/apostille/api/v1/admin/work-items/{id}/result",
            get(result),
        )
        .route("/apostille/api/v1/admin/work-items/{id}/retry", post(retry))
        .route(
            "/apostille/api/v1/admin/work-items/{id}/cancel",
            post(cancel),
        )
        .route(
            "/apostille/api/v1/admin/workers/enrolments",
            get(enrolments),
        )
        .route(
            "/apostille/api/v1/admin/workers/enrolments/{id}/approve",
            post(approve_enrolment),
        )
        .route(
            "/apostille/api/v1/admin/workers/enrolments/{id}/deny",
            post(deny_enrolment),
        )
        .route(
            "/apostille/api/v1/admin/workers/credentials/{id}/revoke",
            post(revoke_credential),
        )
        .route(
            "/apostille/api/v1/admin/work-items/{id}/stage-packs",
            post(stage_packs),
        )
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use futures_util::{SinkExt as _, StreamExt as _};
    use tokio_tungstenite::tungstenite;
    use tower::ServiceExt as _;

    const STAFF: &str = "staff-sess-1";

    type Ws = tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >;

    struct Fixture {
        _dir: tempfile::TempDir,
        state: AppState,
    }

    impl Fixture {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let store = Store::new(dir.path());
            store.init().unwrap();
            let meta = db::open(&store).unwrap();
            db::migrate(&meta).unwrap();
            auth::migrate_auth_tables(&meta).unwrap();
            config::migrate_config_tables(&meta).unwrap();
            let kek = config::Kek::generate();
            auth::create_session(
                &meta,
                STAFF,
                &auth::NewSession {
                    user_id: "staff-1",
                    role: auth::Role::Staff,
                    customer_ref: None,
                    now: unix_now(),
                    ip: "127.0.0.1",
                    user_agent: "test",
                },
            )
            .unwrap();
            let state = AppState {
                spa_index: dir.path().join("index.html"),
                static_dir: dir.path().to_path_buf(),
                store_root: dir.path().to_path_buf(),
                bus: crate::ws_bus::Bus::new(),
                kek: Some(std::sync::Arc::new(kek)),
                admin_mtls_off: true,
                admin_allow_ips: config::loopback_allow_ips(),
                admin_spa_dir: dir.path().to_path_buf(),
                media_terms_read: false,
                label_cfg: crate::p2_labels::LabelCfg::default(),
                voice_cfg: voice::VoiceConfig::default(),
                voice_api_key: None,
                webauthn: std::sync::Arc::new(crate::passkey::test_webauthn()),
                auth_limits: std::sync::Arc::new(crate::passkey::RateBuckets::default()),
                demo_limits: std::sync::Arc::new(crate::passkey::RateBuckets::default()),
                draft_runs: std::sync::Arc::new(crate::draft::DraftRuns::default()),
                config_cache: std::sync::Arc::new(config::CachedReader::default()),
                google_cfg: None,
                apple_cfg: None,
                meta_cfg: None,
                mailer: None,
                magic_link_base: "https://example.test/apostille/login".to_string(),
                notify_poke: std::sync::Arc::new(tokio::sync::Notify::new()),
                notify_enabled: false,
                notify_daily_cap: 3,
                notify_postal: String::new(),
                notify_limits: std::sync::Arc::new(crate::passkey::RateBuckets::default()),
                notify_base: "https://example.test/apostille".to_string(),
                meta_delete_status_base: "https://example.test/auth/meta/delete-status".to_string(),
                fakes: config::Modes::default(),
                fedex_live: crate::fedex_live::FedexLiveState::default(),
            };
            Self { _dir: dir, state }
        }

        /// Staff routes behind the same guard stack `main` mounts.
        fn admin(&self) -> Router {
            crate::mission_admin_routers(&self.state)
        }

        /// Serve the full public router on loopback; returns its address.
        fn serve_public(&self) -> SocketAddr {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            listener.set_nonblocking(true).unwrap();
            let addr = listener.local_addr().unwrap();
            let app = crate::public_router(self.state.clone(), "static", "test-spa", "test-acme");
            tokio::spawn(
                axum_server::from_tcp(listener)
                    .serve(app.into_make_service_with_connect_info::<SocketAddr>()),
            );
            addr
        }

        fn work(&self) -> rusqlite::Connection {
            open_work(&self.state).unwrap()
        }

        /// A `w2.` token for `name` (the `xtask up` path, no enrolment).
        fn token(&self, name: &str) -> String {
            work_enrol::mint_direct(&self.work(), name, "test", unix_now()).unwrap()
        }

        /// Seal one config row through the fixture's KEK.
        fn set_config(&self, key: &str, key_type: config::ConfigType, value: &str) {
            let meta = db::open(&Store::new(&self.state.store_root)).unwrap();
            config::set_value(
                &meta,
                self.state.kek.as_deref().unwrap(),
                &config::SetValue {
                    version: 1,
                    key,
                    key_type,
                    plaintext: value.as_bytes(),
                    now: unix_now(),
                    actor: "test",
                },
            )
            .unwrap();
        }
    }

    fn staff(method: &str, uri: &str, body: Option<Value>) -> Request<Body> {
        let mut builder = Request::builder()
            .uri(uri)
            .method(method)
            .header("host", "localhost")
            .header(
                "cookie",
                format!("{}={STAFF}; {}=t", auth::ADM_COOKIE, auth::CSRF_COOKIE),
            );
        if method != "GET" {
            builder = builder
                .header("content-type", "application/json")
                .header("origin", "http://localhost:3001")
                .header(auth::CSRF_HEADER, "t");
        }
        builder
            .extension(ConnectInfo("127.0.0.1:443".parse::<SocketAddr>().unwrap()))
            .body(body.map_or_else(Body::empty, |v| Body::from(v.to_string())))
            .unwrap()
    }

    async fn call(app: Router, req: Request<Body>) -> (StatusCode, HeaderMap, Vec<u8>) {
        let res = app.oneshot(req).await.unwrap();
        let (status, headers) = (res.status(), res.headers().clone());
        let bytes = axum::body::to_bytes(res.into_body(), 32 * 1024 * 1024)
            .await
            .unwrap();
        (status, headers, bytes.to_vec())
    }

    async fn call_json(app: Router, req: Request<Body>) -> (StatusCode, Value) {
        let (status, _, bytes) = call(app, req).await;
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    async fn connect(addr: SocketAddr, token: Option<&str>) -> Result<Ws, tungstenite::Error> {
        let uri: axum::http::Uri = format!("ws://{addr}/apostille/api/v1/worker/ws")
            .parse()
            .unwrap();
        let mut builder = tungstenite::ClientRequestBuilder::new(uri);
        if let Some(token) = token {
            builder = builder.with_header("authorization", format!("Bearer {token}"));
        }
        tokio_tungstenite::connect_async(builder)
            .await
            .map(|(ws, _)| ws)
    }

    async fn send(ws: &mut Ws, frame: Value) -> Value {
        ws.send(tungstenite::Message::text(frame.to_string()))
            .await
            .unwrap();
        loop {
            let msg = tokio::time::timeout(Duration::from_secs(5), ws.next())
                .await
                .expect("reply within 5s")
                .expect("socket open")
                .expect("frame ok");
            if let tungstenite::Message::Text(text) = msg {
                return serde_json::from_str(text.as_str()).unwrap();
            }
        }
    }

    async fn hello(ws: &mut Ws, name: &str) -> Value {
        send(
            ws,
            json!({"type": "hello", "worker": name,
                   "capabilities": ["scrape.urls", "no.such.kind"], "version": "test"}),
        )
        .await
    }

    fn refused_with(err: tungstenite::Error) -> u16 {
        match err {
            tungstenite::Error::Http(res) => res.status().as_u16(),
            other => panic!("expected an HTTP refusal, got {other:?}"),
        }
    }

    /// The whole lifecycle over a real socket: refused tokens, enqueue,
    /// checkout, heartbeat, lease expiry + requeue, a stale lease
    /// refused, complete with upload, and the staff result download.
    #[tokio::test]
    async fn queue_lifecycle_over_websocket() {
        let fx = Fixture::new();
        let addr = fx.serve_public();

        // Bad and missing tokens never upgrade.
        assert_eq!(
            refused_with(
                connect(addr, Some("wrong-token-wrong-token-wrong-token"))
                    .await
                    .unwrap_err()
            ),
            401
        );
        assert_eq!(refused_with(connect(addr, None).await.unwrap_err()), 401);

        // Staff enqueue: unknown kinds and non-object payloads refuse.
        let uri = "/apostille/api/v1/admin/work-items";
        let (status, _) = call_json(
            fx.admin(),
            staff("POST", uri, Some(json!({"kind": "nope", "payload": {}}))),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        let (status, _) = call_json(
            fx.admin(),
            staff(
                "POST",
                uri,
                Some(json!({"kind": "scrape.urls", "payload": [1]})),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        let body = json!({"kind": "scrape.urls",
                          "payload": {"urls": ["https://example.com/synthetic/1"]},
                          "max_attempts": 2});
        let (status, created) = call_json(fx.admin(), staff("POST", uri, Some(body))).await;
        assert_eq!(status, StatusCode::CREATED, "{created}");
        let id = created["id"].as_i64().unwrap();

        // Worker A: hello, checkout, heartbeat.
        let mut a = connect(addr, Some(&fx.token("plant-a"))).await.unwrap();
        let welcome = hello(&mut a, "plant-a").await;
        assert_eq!(welcome["type"], "welcome");
        assert_eq!(welcome["lease_secs"], LEASE_SECS);
        let job = send(&mut a, json!({"type": "checkout"})).await;
        assert_eq!(
            (job["type"].as_str(), job["id"].as_i64()),
            (Some("job"), Some(id))
        );
        assert_eq!(job["payload"]["urls"][0], "https://example.com/synthetic/1");
        assert_eq!(job["attempt"], 1);
        let lease_a = job["lease"].as_str().unwrap().to_string();
        let beat = send(
            &mut a,
            json!({"type": "heartbeat", "id": id, "lease": lease_a}),
        )
        .await;
        assert_eq!(beat["type"], "lease");
        assert_eq!(
            send(&mut a, json!({"type": "checkout"})).await["type"],
            "idle"
        );

        // Staff list shows the lease by worker name only, never the token.
        let (status, listed) = call_json(fx.admin(), staff("GET", uri, None)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(listed["items"][0]["lease_owner"], "plant-a");
        assert_eq!(listed["counts"]["leased"], 1);
        assert!(!listed.to_string().contains(&lease_a));
        let names: Vec<&str> = listed["workers"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|w| w["name"].as_str())
            .collect();
        assert!(names.contains(&"plant-a"), "{names:?}");

        // Lease runs out (clock moved by rewriting the expiry): worker B's
        // checkout sweeps and re-leases it as attempt 2.
        fx.work()
            .execute(
                "UPDATE work_item SET lease_expires_at = ?2 WHERE id = ?1",
                rusqlite::params![id, unix_now() - 1],
            )
            .unwrap();
        let mut b = connect(addr, Some(&fx.token("plant-b"))).await.unwrap();
        hello(&mut b, "plant-b").await;
        let again = send(&mut b, json!({"type": "checkout"})).await;
        assert_eq!(
            (again["id"].as_i64(), again["attempt"].as_i64()),
            (Some(id), Some(2))
        );
        let lease_b = again["lease"].as_str().unwrap().to_string();

        // A's stale lease is refused for heartbeat and complete.
        let stale = send(
            &mut a,
            json!({"type": "heartbeat", "id": id, "lease": lease_a}),
        )
        .await;
        assert_eq!(stale["code"], "lease_lost");
        let bytes = br#"{"pages": []}"#;
        let b64 = base64::engine::general_purpose::STANDARD.encode(bytes);
        let sha = handbook_store::store::sha256_hex(bytes);
        let upload = |lease: &str, sha: &str| {
            json!({"type": "complete", "id": id, "lease": lease, "content_type": "application/json",
                   "sha256": sha, "result_b64": b64, "summary": {"pages": 0}})
        };
        assert_eq!(
            send(&mut a, upload(&lease_a, &sha)).await["code"],
            "lease_lost"
        );
        // A wrong digest is caught before anything is stored.
        let bad = send(&mut b, upload(&lease_b, &"0".repeat(64))).await;
        assert_eq!(bad["code"], "sha_mismatch");
        let ack = send(&mut b, upload(&lease_b, &sha)).await;
        assert_eq!(
            (ack["type"].as_str(), ack["status"].as_str()),
            (Some("ack"), Some("done"))
        );

        // Staff download returns the exact bytes as an attachment.
        let (status, headers, got) = call(
            fx.admin(),
            staff("GET", &format!("{uri}/{id}/result"), None),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(got, bytes);
        assert_eq!(headers[header::CONTENT_TYPE], "application/json");
        assert_eq!(
            headers[header::CONTENT_DISPOSITION],
            format!("attachment; filename=\"work-{id}-scrape-urls.json\"")
        );

        // Detail carries the audit trail, requeue included.
        let (status, detail) =
            call_json(fx.admin(), staff("GET", &format!("{uri}/{id}"), None)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(detail["item"]["summary"]["pages"], 0);
        let actions: Vec<&str> = detail["audit"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|row| row["action"].as_str())
            .collect();
        assert_eq!(
            actions,
            [
                "work_enqueue",
                "work_checkout",
                "work_requeue",
                "work_checkout",
                "work_complete"
            ]
        );
        // Retry only applies to failed items.
        let (status, _) = call_json(
            fx.admin(),
            staff("POST", &format!("{uri}/{id}/retry"), None),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn worker_failure_requeues_then_staff_retries() {
        let fx = Fixture::new();
        let addr = fx.serve_public();
        let uri = "/apostille/api/v1/admin/work-items";
        let body = json!({"kind": "scrape.urls", "payload": {"urls": []}, "max_attempts": 1});
        let (_, created) = call_json(fx.admin(), staff("POST", uri, Some(body))).await;
        let id = created["id"].as_i64().unwrap();
        let mut ws = connect(addr, Some(&fx.token("plant-c"))).await.unwrap();
        hello(&mut ws, "plant-c").await;
        let job = send(&mut ws, json!({"type": "checkout"})).await;
        let lease = job["lease"].as_str().unwrap();
        let ack = send(
            &mut ws,
            json!({"type": "fail", "id": id, "lease": lease, "error": "boom", "retryable": true}),
        )
        .await;
        assert_eq!(ack["status"], "failed", "max_attempts 1: no retry left");
        let (status, _, _) = call(
            fx.admin(),
            staff("GET", &format!("{uri}/{id}/result"), None),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "a failed item has no result");
        let (status, retried) = call_json(
            fx.admin(),
            staff("POST", &format!("{uri}/{id}/retry"), None),
        )
        .await;
        assert_eq!(
            (status, retried["status"].as_str()),
            (StatusCode::OK, Some("pending"))
        );
        let job = send(&mut ws, json!({"type": "checkout"})).await;
        assert_eq!(job["attempt"], 1);
    }

    #[tokio::test]
    async fn staff_cancel_ends_the_workers_lease() {
        let fx = Fixture::new();
        let addr = fx.serve_public();
        let uri = "/apostille/api/v1/admin/work-items";
        let body = json!({"kind": "scrape.urls", "payload": {"urls": []}});
        let (_, created) = call_json(fx.admin(), staff("POST", uri, Some(body))).await;
        let id = created["id"].as_i64().unwrap();
        let mut ws = connect(addr, Some(&fx.token("plant-c"))).await.unwrap();
        hello(&mut ws, "plant-c").await;
        let job = send(&mut ws, json!({"type": "checkout"})).await;
        let lease = job["lease"].as_str().unwrap().to_string();
        let cancel = format!("{uri}/{id}/cancel");
        let (status, cancelled) = call_json(fx.admin(), staff("POST", &cancel, None)).await;
        assert_eq!(
            (status, cancelled["status"].as_str()),
            (StatusCode::OK, Some("failed"))
        );
        let beat = send(
            &mut ws,
            json!({"type": "heartbeat", "id": id, "lease": lease}),
        )
        .await;
        assert_eq!(beat["code"], "lease_lost");
        let (status, _) = call_json(fx.admin(), staff("POST", &cancel, None)).await;
        assert_eq!(
            status,
            StatusCode::CONFLICT,
            "a failed item does not cancel"
        );
        assert_eq!(max_runtime("scrape.urls"), 3600);
        assert_eq!(
            max_runtime("gone.kind"),
            8 * 3600,
            "unknown kinds get the longest"
        );
    }

    /// Public enrol endpoints over the public router (unauthenticated).
    async fn public_call(fx: &Fixture, uri: &str, body: Value) -> (StatusCode, Value) {
        let req = Request::builder()
            .uri(uri)
            .method("POST")
            .header("host", "localhost")
            .header("content-type", "application/json")
            .extension(ConnectInfo(
                "203.0.113.5:40000".parse::<SocketAddr>().unwrap(),
            ))
            .body(Body::from(body.to_string()))
            .unwrap();
        let public = crate::public_router(fx.state.clone(), "static", "test-spa", "test-acme");
        call_json(public, req).await
    }

    const POLL_SECRET: &str = "0101010101010101010101010101010101010101010101010101010101010101";

    fn enrol_body() -> Value {
        let poll_hash = handbook_store::store::sha256_hex(&[1u8; 32]);
        json!({"username": "chandra", "hostname": "plant", "version": "0.1.0+1bda381",
               "started_at": unix_now() - 3, "capabilities": ["scrape.urls", "no.such.kind"],
               "poll_hash": poll_hash})
    }

    /// Enrol → staff approve with the code (+ Pacific date) → poll
    /// collects the token once → the token connects as that name.
    #[tokio::test]
    async fn enrolment_end_to_end() {
        let fx = Fixture::new();
        let (status, filed) =
            public_call(&fx, "/apostille/api/v1/worker/enrol", enrol_body()).await;
        assert_eq!(status, StatusCode::ACCEPTED, "{filed}");
        let id = filed["id"].as_str().unwrap().to_string();
        let code = filed["code"].as_str().unwrap().to_string();
        let poll_uri = format!("/apostille/api/v1/worker/enrol/{id}/poll");
        let poll = json!({"poll_secret": POLL_SECRET});
        let (status, pending) = public_call(&fx, &poll_uri, poll.clone()).await;
        assert_eq!(
            (status, pending["status"].as_str()),
            (StatusCode::OK, Some("pending"))
        );
        let (status, _) =
            public_call(&fx, &poll_uri, json!({"poll_secret": "02".repeat(32)})).await;
        assert_eq!(status, StatusCode::NOT_FOUND);

        // Staff see the request, never the code.
        let list = "/apostille/api/v1/admin/workers/enrolments";
        let (status, listed) = call_json(fx.admin(), staff("GET", list, None)).await;
        assert_eq!(status, StatusCode::OK, "{listed}");
        assert_eq!(listed["pending"][0]["proposed_name"], "chandra@plant");
        assert_eq!(listed["pending"][0]["peer_ip"], "203.0.113.5");
        assert_eq!(listed["pending"][0]["capabilities"], json!(["scrape.urls"]));
        assert_eq!(listed["enrolment_on"], true);
        assert!(!listed.to_string().contains(&code));
        let today = listed["today"].as_str().unwrap().to_string();

        let approve = format!("/apostille/api/v1/admin/workers/enrolments/{id}/approve");
        let wrong = if code == "AAAA-AA" {
            "BBBB-BB"
        } else {
            "AAAA-AA"
        };
        let (status, refused) = call_json(
            fx.admin(),
            staff(
                "POST",
                &approve,
                Some(json!({"code": wrong, "name": "chandra@plant"})),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(refused["tries_left"], 2);
        let (status, approved) = call_json(
            fx.admin(),
            staff(
                "POST",
                &approve,
                Some(json!({"code": code, "name": "chandra@plant", "add_date": true})),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{approved}");
        let name = format!("chandra@plant {today}");
        assert_eq!(approved["name"], name);

        let (status, got) = public_call(&fx, &poll_uri, poll.clone()).await;
        assert_eq!(
            (status, got["worker"].as_str()),
            (StatusCode::OK, Some(name.as_str()))
        );
        let token = got["token"].as_str().unwrap().to_string();
        assert!(token.starts_with("w2."));
        let (status, gone) = public_call(&fx, &poll_uri, poll).await;
        assert_eq!(
            (status, gone["status"].as_str()),
            (StatusCode::GONE, Some("delivered"))
        );

        let addr = fx.serve_public();
        let mut ws = connect(addr, Some(&token)).await.unwrap();
        assert_eq!(hello(&mut ws, &name).await["type"], "welcome");
        let (_, listed) = call_json(fx.admin(), staff("GET", list, None)).await;
        assert!(listed["pending"].as_array().unwrap().is_empty());
        assert_eq!(listed["credentials"][0]["name"], name);
        assert_eq!(listed["credentials"][0]["connected"], true);
        assert!(!listed.to_string().contains(&token));
    }

    #[tokio::test]
    async fn enrolment_refusals() {
        let fx = Fixture::new();
        let uri = "/apostille/api/v1/worker/enrol";
        let mut bad = enrol_body();
        bad["username"] = json!("Chandra");
        assert_eq!(
            public_call(&fx, uri, bad).await.0,
            StatusCode::UNPROCESSABLE_ENTITY
        );
        let mut big = enrol_body();
        big["version"] = json!("x".repeat(5000));
        assert_eq!(
            public_call(&fx, uri, big).await.0,
            StatusCode::PAYLOAD_TOO_LARGE
        );
        for _ in 0..work_enrol::MAX_PER_IP_PER_HOUR {
            assert_eq!(
                public_call(&fx, uri, enrol_body()).await.0,
                StatusCode::ACCEPTED
            );
        }
        assert_eq!(
            public_call(&fx, uri, enrol_body()).await.0,
            StatusCode::TOO_MANY_REQUESTS
        );
        fx.set_config(
            config::keys::WORKER_ENROLMENT,
            config::ConfigType::Text,
            "off",
        );
        assert_eq!(
            public_call(&fx, uri, enrol_body()).await.0,
            StatusCode::SERVICE_UNAVAILABLE
        );
        // Staff can still deny what is pending.
        let (_, listed) = call_json(
            fx.admin(),
            staff("GET", "/apostille/api/v1/admin/workers/enrolments", None),
        )
        .await;
        assert_eq!(listed["enrolment_on"], false);
        let id = listed["pending"][0]["id"].as_str().unwrap();
        let deny = format!("/apostille/api/v1/admin/workers/enrolments/{id}/deny");
        let (status, denied) = call_json(fx.admin(), staff("POST", &deny, None)).await;
        assert_eq!(
            (status, denied["status"].as_str()),
            (StatusCode::OK, Some("denied"))
        );
        assert_eq!(
            call_json(fx.admin(), staff("POST", &deny, None)).await.0,
            StatusCode::CONFLICT
        );
    }

    /// A token admits only its own name, carries the settings, and a
    /// revoke refuses the next connection with `revoked`.
    #[tokio::test]
    async fn credentials_admit_their_worker_until_revoked() {
        let fx = Fixture::new();
        fx.set_config(
            config::keys::WORKER_GLIMMER_MODEL,
            config::ConfigType::Text,
            "glimmer-7b-q4",
        );
        fx.set_config(
            config::keys::META_API_KEY,
            config::ConfigType::Secret,
            "LLM_1_stub",
        );
        let addr = fx.serve_public();
        let token = fx.token("plant-b");
        let mut ws = connect(addr, Some(&token)).await.unwrap();
        let welcome = hello(&mut ws, "plant-b").await;
        assert_eq!(welcome["type"], "welcome");
        assert_eq!(welcome["settings"]["glimmer_model"], "glimmer-7b-q4");
        assert_eq!(welcome["settings"]["meta_daily_call_cap"], "400");
        if std::env::var_os("META_API_KEY").is_none() {
            assert_eq!(welcome["settings"]["meta_api_key"], "LLM_1_stub");
        }
        let mut ws = connect(addr, Some(&token)).await.unwrap();
        let refused = hello(&mut ws, "plant-c").await;
        assert_eq!(refused["code"], "bad_frame", "{refused}");

        let (_, listed) = call_json(
            fx.admin(),
            staff("GET", "/apostille/api/v1/admin/workers/enrolments", None),
        )
        .await;
        let id = listed["credentials"][0]["id"].as_str().unwrap().to_string();
        let revoke = format!("/apostille/api/v1/admin/workers/credentials/{id}/revoke");
        let (status, revoked) = call_json(fx.admin(), staff("POST", &revoke, None)).await;
        assert_eq!(status, StatusCode::OK, "{revoked}");
        assert_eq!(revoked["id"], id);
        assert_eq!(
            call_json(fx.admin(), staff("POST", &revoke, None)).await.0,
            StatusCode::NOT_FOUND
        );
        match connect(addr, Some(&token)).await.unwrap_err() {
            tungstenite::Error::Http(res) => {
                assert_eq!(res.status().as_u16(), 401);
                let body: Value = serde_json::from_slice(res.body().as_deref().unwrap()).unwrap();
                assert_eq!(body["error"], "revoked");
            }
            other => panic!("expected an HTTP refusal, got {other:?}"),
        }
        // The old token-mint route is gone.
        let (status, _, _) = call(
            fx.admin(),
            staff(
                "POST",
                "/apostille/api/v1/admin/workers/token",
                Some(json!({"name": "x"})),
            ),
        )
        .await;
        assert!(
            matches!(
                status,
                StatusCode::NOT_FOUND | StatusCode::METHOD_NOT_ALLOWED
            ),
            "{status}"
        );
    }

    #[test]
    fn version_checks() {
        assert_eq!(version_matches("0.1.0+1bda381c0ffe", "1bda381"), Some(true));
        assert_eq!(version_matches("0.1.0+deadbee", "1bda381"), Some(false));
        assert_eq!(version_matches("0.1.0", "1bda381"), None);
        assert_eq!(version_matches("0.1.0+1bda381", "unknown"), None);
    }

    #[tokio::test]
    async fn first_frame_must_be_hello() {
        let fx = Fixture::new();
        let addr = fx.serve_public();
        let token = fx.token("plant");
        let mut ws = connect(addr, Some(&token)).await.unwrap();
        let reply = send(&mut ws, json!({"type": "checkout"})).await;
        assert_eq!(reply["code"], "bad_frame");
        let mut ws = connect(addr, Some(&token)).await.unwrap();
        let reply = send(&mut ws, json!({"type": "hello", "worker": "Bad:Name"})).await;
        assert_eq!(reply["code"], "bad_frame");
    }

    #[tokio::test]
    async fn staff_routes_need_a_staff_session() {
        let fx = Fixture::new();
        let req = Request::builder()
            .uri("/apostille/api/v1/admin/work-items")
            .header("host", "localhost")
            .extension(ConnectInfo("127.0.0.1:443".parse::<SocketAddr>().unwrap()))
            .body(Body::empty())
            .unwrap();
        let (status, _) = call_json(fx.admin(), req).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        // The worker socket is public-only: the admin host has no such route.
        let (status, _, _) = call(
            fx.admin(),
            staff("GET", "/apostille/api/v1/worker/ws", None),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        // And the public host never serves the staff routes.
        let public = crate::public_router(fx.state.clone(), "static", "test-spa", "test-acme");
        let (status, _, _) = call(
            public,
            staff("GET", "/apostille/api/v1/admin/work-items", None),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[test]
    fn result_decoding_enforces_cap_and_digest() {
        let b64 = base64::engine::general_purpose::STANDARD.encode(b"hello");
        let sha = handbook_store::store::sha256_hex(b"hello");
        assert_eq!(decode_result(&b64, Some(&sha), 16).unwrap(), b"hello");
        assert_eq!(decode_result(&b64, None, 16).unwrap(), b"hello");
        assert_eq!(
            decode_result(&b64, Some(&sha), 4).unwrap_err().0,
            "too_large"
        );
        let big = base64::engine::general_purpose::STANDARD.encode(vec![0u8; 64]);
        assert_eq!(decode_result(&big, None, 16).unwrap_err().0, "too_large");
        assert_eq!(decode_result("!!", None, 16).unwrap_err().0, "bad_frame");
        assert_eq!(
            decode_result(&b64, Some(&"0".repeat(64)), 16)
                .unwrap_err()
                .0,
            "sha_mismatch"
        );
    }

    #[test]
    fn helpers_are_strict() {
        assert_eq!(
            clean_content_type(Some("application/json")),
            "application/json"
        );
        assert_eq!(
            clean_content_type(Some("text/html; charset=utf-8")),
            "application/octet-stream"
        );
        assert_eq!(clean_content_type(None), "application/octet-stream");
        let mut headers = HeaderMap::new();
        headers.insert(header::AUTHORIZATION, "Bearer abc".parse().unwrap());
        assert_eq!(bearer(&headers), Some("abc"));
        headers.insert(header::AUTHORIZATION, "Basic abc".parse().unwrap());
        assert_eq!(bearer(&headers), None);
        for k in KINDS {
            assert!(
                serde_json::from_str::<Value>(k.example)
                    .unwrap()
                    .is_object(),
                "{}",
                k.kind
            );
        }
    }

    /// Every sample in the shared contract parses as this side's frame
    /// type (the plant client's tests parse the same file).
    #[test]
    fn contract_frames_round_trip() {
        let seed: Value =
            serde_json::from_str(include_str!("../../../docs/contracts/worker-frames.json"))
                .unwrap();
        for sample in seed["client"].as_array().unwrap() {
            let frame: ClientFrame = serde_json::from_value(sample.clone()).expect("client frame");
            assert_eq!(
                serde_json::to_value(&frame).unwrap()["type"],
                sample["type"]
            );
        }
        for sample in seed["server"].as_array().unwrap() {
            let frame: ServerFrame = serde_json::from_value(sample.clone()).expect("server frame");
            assert_eq!(&serde_json::to_value(&frame).unwrap(), sample);
        }
        let enrol = &seed["enrol"];
        let body: EnrolBody = serde_json::from_value(enrol["request"].clone()).expect("enrol");
        assert_eq!(
            format!("{}@{}", body.username, body.hostname),
            "chandra@plant"
        );
        let _: PollBody = serde_json::from_value(enrol["poll_request"].clone()).expect("poll");
        let token = enrol["poll_approved"]["token"].as_str().unwrap();
        assert_eq!(
            token.split('.').map(str::len).collect::<Vec<_>>(),
            [2, 32, 64],
            "w2.<32 hex>.<64 hex>"
        );
    }

    /// Put a finished item with `bytes` as its result straight into the
    /// queue (the socket path is covered by the lifecycle test).
    fn finished(fx: &Fixture, kind: &str, bytes: &[u8]) -> i64 {
        let conn = fx.work();
        let id = work::enqueue(
            &conn,
            &work::NewItem {
                kind,
                payload_json: "{}",
                max_attempts: 3,
                actor: "staff-1",
                at: 1,
            },
        )
        .unwrap();
        let kinds = vec![kind.to_string()];
        let lease = work::Checkout {
            owner: "plant:t",
            actor: "plant",
            kinds: &kinds,
            lease_secs: LEASE_SECS,
            now: unix_now(),
        };
        work::checkout(&conn, &lease).unwrap().unwrap();
        let sha = Store::new(&fx.state.store_root).write_blob(bytes).unwrap();
        let result = work::ResultRef {
            sha256: sha,
            bytes: bytes.len() as i64,
            content_type: "application/json".to_string(),
        };
        assert!(work::complete(&conn, id, "plant:t", "plant", Some(&result), None, 2).unwrap());
        id
    }

    #[tokio::test]
    async fn stage_packs_writes_the_inbox_and_refuses_other_results() {
        let fx = Fixture::new();
        let uri = |id: i64| format!("/apostille/api/v1/admin/work-items/{id}/stage-packs");
        let bundle = json!({"format": "packs.v1", "packs": [
            {"pack": "EG", "version": 1, "authorities": []},
            {"pack": "ER", "version": 1, "authorities": []}
        ], "report": []});
        let id = finished(&fx, "research.nonparty", bundle.to_string().as_bytes());
        let (status, staged) = call_json(fx.admin(), staff("POST", &uri(id), None)).await;
        assert_eq!(status, StatusCode::OK, "{staged}");
        assert_eq!(staged["dir"], format!("packs-inbox/{id}"));
        assert_eq!(staged["packs"], json!(["EG", "ER"]));
        let dir = fx.state.store_root.join("packs-inbox").join(id.to_string());
        let eg: Value =
            serde_json::from_slice(&std::fs::read(dir.join("EG.json")).unwrap()).unwrap();
        assert_eq!(eg["pack"], "EG");
        // Re-staging replaces the folder whole.
        std::fs::write(dir.join("stale.json"), b"{}").unwrap();
        let (status, _) = call_json(fx.admin(), staff("POST", &uri(id), None)).await;
        assert_eq!(status, StatusCode::OK);
        assert!(!dir.join("stale.json").exists());
        // The list and the detail say it is staged; the audit says who.
        let (_, listed) = call_json(
            fx.admin(),
            staff("GET", "/apostille/api/v1/admin/work-items", None),
        )
        .await;
        assert_eq!(listed["items"][0]["staged"], true);
        let detail = format!("/apostille/api/v1/admin/work-items/{id}");
        let (_, detail) = call_json(fx.admin(), staff("GET", &detail, None)).await;
        let last = detail["audit"].as_array().unwrap().last().unwrap().clone();
        assert_eq!(
            (last["action"].as_str(), last["actor"].as_str()),
            (Some("work_stage"), Some("staff-1"))
        );

        // Other kinds, unsafe ids and non-bundles are refused; nothing is written.
        let scrape = finished(&fx, "scrape.urls", b"{}");
        let (status, _) = call_json(fx.admin(), staff("POST", &uri(scrape), None)).await;
        assert_eq!(status, StatusCode::CONFLICT);
        let evil = json!({"format": "packs.v1", "packs": [{"pack": "../../etc"}]});
        let evil = finished(&fx, "research.nonparty", evil.to_string().as_bytes());
        let (status, _) = call_json(fx.admin(), staff("POST", &uri(evil), None)).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        let not_bundle = finished(&fx, "research.nonparty", b"{\"pages\": []}");
        let (status, _) = call_json(fx.admin(), staff("POST", &uri(not_bundle), None)).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        for refused in [scrape, evil, not_bundle] {
            assert!(
                !fx.state
                    .store_root
                    .join("packs-inbox")
                    .join(refused.to_string())
                    .exists()
            );
        }
        let (status, _) = call_json(fx.admin(), staff("POST", &uri(9999), None)).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[test]
    fn bundle_packs_checks_ids() {
        assert!(valid_pack_id("EG") && valid_pack_id("usa-wa"));
        for bad in ["", "..", "a/b", "a.b", &"x".repeat(65)] {
            assert!(!valid_pack_id(bad), "{bad:?}");
        }
        let dup = json!({"format": "packs.v1", "packs": [{"pack": "EG"}, {"pack": "EG"}]});
        assert_eq!(
            bundle_packs(dup.to_string().as_bytes(), "packs.v1").unwrap_err(),
            "duplicate pack id in bundle"
        );
        assert!(bundle_packs(b"not json", "packs.v1").is_err());
        // Each kind stages its own format only.
        assert!(bundle_packs(dup.to_string().as_bytes(), "discovery.v1").is_err());
        assert_eq!(stage_format("discovery.bootstrap"), Some("discovery.v1"));
        assert_eq!(stage_format("scrape.urls"), None);
    }

    /// D-3: a discovery result stages whole world packs (v2 fields kept)
    /// into the same inbox; empty or wrong-format results do not.
    #[tokio::test]
    async fn stage_packs_takes_discovery_world_packs() {
        let fx = Fixture::new();
        let uri = |id: i64| format!("/apostille/api/v1/admin/work-items/{id}/stage-packs");
        let pack = json!({"pack": "MH", "version": 1, "iso2": "MH",
            "authorities": [
                {"kind": "country", "country": "Marshall Islands", "name": "Minister of Foreign Affairs",
                 "slug": "mh-ca-1"},
                {"kind": "country", "country": "Marshall Islands", "name": "Limmatquai 50",
                 "slug": "mh-ca-2", "hidden": true}],
            "watch": [{"authority": "mh-ca-1", "kind": "website_official",
                       "uri": "https://mfa.example/apostille", "cadence": "weekly"}]});
        let bundle = json!({"format": "discovery.v1", "kind": "discovery.bootstrap",
                            "packs": [pack], "pack_bases": {"MH": "ab"}, "summary": {}});
        let id = finished(&fx, "discovery.bootstrap", bundle.to_string().as_bytes());
        let (status, staged) = call_json(fx.admin(), staff("POST", &uri(id), None)).await;
        assert_eq!(status, StatusCode::OK, "{staged}");
        assert_eq!(staged["packs"], json!(["MH"]));
        let file = fx
            .state
            .store_root
            .join("packs-inbox")
            .join(id.to_string())
            .join("MH.json");
        let written: Value = serde_json::from_slice(&std::fs::read(file).unwrap()).unwrap();
        assert_eq!(written["authorities"][1]["hidden"], true);
        assert_eq!(written["watch"][0]["uri"], "https://mfa.example/apostille");

        let empty = json!({"format": "discovery.v1", "packs": [], "summary": {}});
        let empty = finished(&fx, "discovery.bootstrap", empty.to_string().as_bytes());
        let wrong = json!({"format": "packs.v1", "packs": [{"pack": "MH"}]});
        let wrong = finished(&fx, "discovery.bootstrap", wrong.to_string().as_bytes());
        let swapped = finished(&fx, "research.nonparty", bundle.to_string().as_bytes());
        for refused in [empty, wrong, swapped] {
            let (status, body) = call_json(fx.admin(), staff("POST", &uri(refused), None)).await;
            assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
            assert!(
                !fx.state
                    .store_root
                    .join("packs-inbox")
                    .join(refused.to_string())
                    .exists()
            );
        }
    }

    /// D-3: a discovery copy stages only while the release still has
    /// the world pack it copied, byte for byte.
    #[test]
    fn stale_world_copies_compare_the_release_file() {
        let world = tempfile::tempdir().unwrap();
        std::fs::write(world.path().join("MH.json"), b"{\"pack\": \"MH\"}\n").unwrap();
        let sha = handbook_store::store::sha256_hex(b"{\"pack\": \"MH\"}\n");
        let same = json!({"MH": sha, "VN": sha});
        assert!(stale_world_copies(world.path(), &same, &["MH"]).is_empty());
        assert_eq!(
            stale_world_copies(world.path(), &json!({"MH": "0000"}), &["MH"]),
            ["MH"]
        );
        // No base recorded, or no such pack in this release.
        assert_eq!(
            stale_world_copies(world.path(), &json!({}), &["MH"]),
            ["MH"]
        );
        assert_eq!(stale_world_copies(world.path(), &same, &["VN"]), ["VN"]);
    }

    /// The admin contract seed matches what `item_json` emits.
    #[test]
    fn item_json_matches_the_contract_keys() {
        let seed: Value = serde_json::from_str(include_str!(
            "../../../docs/contracts/admin-work-items.json"
        ))
        .unwrap();
        let item = work::WorkItem {
            id: 6,
            kind: "scrape.urls".to_string(),
            payload_json: "{}".to_string(),
            status: "done".to_string(),
            lease_owner: None,
            lease_expires_at: None,
            attempts: 1,
            max_attempts: 3,
            result: None,
            summary_json: None,
            error: None,
            created_by: "staff-1".to_string(),
            created_at: 1,
            updated_at: 2,
            retry_after: None,
        };
        let got = item_json(&item, false);
        let want = seed["list"]["items"][0].as_object().unwrap();
        let mut got_keys: Vec<&String> = got.as_object().unwrap().keys().collect();
        let mut want_keys: Vec<&String> = want.keys().collect();
        got_keys.sort();
        want_keys.sort();
        assert_eq!(got_keys, want_keys);
    }
}
