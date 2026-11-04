//! Plant work queue (P392): WebSocket checkout for plant workers plus
//! the staff work-items API.
//!
//! `GET /apostille/api/v1/worker/ws` (PUBLIC host: plant dials out to
//! andrii.ai, so no admin IP allowlist applies) authenticates
//! `Authorization: Bearer <worker.token>` BEFORE the upgrade, then
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

use std::collections::HashMap;
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
use handbook_store::{db, store::Store, work};
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
/// A configured token shorter than this refuses every worker.
pub const MIN_TOKEN_LEN: usize = 32;
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
                (needs META_API_KEY in worker.env).",
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
    /// `shared` (the `worker.token` itself) or `named` (a token minted
    /// for this worker name); empty until `hello`.
    identity: &'static str,
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
                identity: "",
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
                "identity": s.identity,
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

/// Constant-time comparison over SHA-256 digests (equal-length by
/// construction, so token length never leaks through timing).
fn token_matches(presented: &str, expected: &[u8]) -> bool {
    let a = Sha256::digest(presented.as_bytes());
    let b = Sha256::digest(expected);
    a.iter()
        .zip(b.iter())
        .fold(0u8, |diff, (x, y)| diff | (x ^ y))
        == 0
}

/// HMAC-SHA256 (RFC 2104) over `sha2`.
fn hmac_sha256(key: &[u8], msg: &[u8]) -> [u8; 32] {
    let mut block = [0u8; 64];
    if key.len() > 64 {
        block[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        block[..key.len()].copy_from_slice(key);
    }
    let pad = |byte: u8| block.map(|b| b ^ byte);
    let inner = Sha256::new()
        .chain_update(pad(0x36))
        .chain_update(msg)
        .finalize();
    Sha256::new()
        .chain_update(pad(0x5c))
        .chain_update(inner)
        .finalize()
        .into()
}

/// The token for one named worker: `w1.<name>.<hex HMAC>` keyed by the
/// sealed `worker.token`, so rotating that secret revokes every named
/// token too. Minted on the Work page; admits only `name` at `hello`.
pub(crate) fn named_token(secret: &[u8], name: &str) -> String {
    let mac = hmac_sha256(secret, format!("worker-name:{name}").as_bytes());
    let hex: String = mac.iter().map(|b| format!("{b:02x}")).collect();
    format!("w1.{name}.{hex}")
}

/// Who a presented token speaks for.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Identity {
    /// The shared `worker.token`: any worker name.
    Shared,
    /// A [`named_token`]: this name only.
    Named(String),
}

impl Identity {
    fn label(&self) -> &'static str {
        match self {
            Identity::Shared => "shared",
            Identity::Named(_) => "named",
        }
    }
}

fn identify(presented: &str, secret: &[u8]) -> Option<Identity> {
    if token_matches(presented, secret) {
        return Some(Identity::Shared);
    }
    let name = presented.strip_prefix("w1.")?.split_once('.')?.0;
    (valid_worker_name(name) && token_matches(presented, named_token(secret, name).as_bytes()))
        .then(|| Identity::Named(name.to_string()))
}

/// Why a token check could not admit (a response only at the edge).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Refused {
    /// No usable `worker.token` row (missing, short, or no KEK).
    Unconfigured,
    /// The store did not open.
    Store,
    /// A token was configured but the presented one matched nothing.
    Unauthorized,
}

impl Refused {
    fn response(self) -> Response {
        match self {
            Refused::Unconfigured => json_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "worker token not configured",
            ),
            Refused::Store => store_unavailable(),
            Refused::Unauthorized => json_error(StatusCode::UNAUTHORIZED, "unauthorized"),
        }
    }
}

/// The sealed `worker.token`.
fn worker_secret(state: &AppState) -> Result<Vec<u8>, Refused> {
    let Some(kek) = state.kek.as_deref() else {
        return Err(Refused::Unconfigured);
    };
    let Ok(conn) = db::open(&Store::new(&state.store_root)) else {
        return Err(Refused::Store);
    };
    match config::get_secret_for_job(&conn, kek, config::keys::WORKER_TOKEN) {
        Ok(Some(bytes)) if bytes.len() >= MIN_TOKEN_LEN => Ok(bytes),
        Ok(_) => Err(Refused::Unconfigured),
        Err(e) => {
            tracing::warn!("worker ws: read worker.token: {e}");
            Err(Refused::Unconfigured)
        }
    }
}

/// A socket's credentials: who it speaks for, and the digest of the
/// secret it was checked against (a rotation changes it).
#[derive(Debug, Clone)]
struct Admitted {
    identity: Identity,
    secret_digest: [u8; 32],
}

/// Check the bearer token against the sealed `worker.token` row (the
/// shared token, or a name's [`named_token`]). Read per upgrade, so a
/// rotation in `/config` applies at the next connection, and open
/// sockets re-check it on every ping.
fn admit(state: &AppState, headers: &HeaderMap) -> Result<Admitted, Refused> {
    let secret = worker_secret(state)?;
    match bearer(headers).and_then(|presented| identify(presented, &secret)) {
        Some(identity) => Ok(Admitted {
            identity,
            secret_digest: Sha256::digest(&secret).into(),
        }),
        None => {
            tracing::warn!("worker ws upgrade refused: bad or missing token");
            Err(Refused::Unauthorized)
        }
    }
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

/// Worker names: `[a-z0-9-]{1,32}` (they land in audit rows and
/// `lease_owner`, where `:` separates the token).
fn valid_worker_name(name: &str) -> bool {
    (1..=32).contains(&name.len())
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

async fn run_socket(state: AppState, mut socket: WebSocket, slot: Slot, admitted: Admitted) {
    let hello = tokio::time::timeout(Duration::from_secs(HELLO_TIMEOUT_SECS), socket.recv()).await;
    let session = match hello {
        Ok(Some(Ok(Message::Text(text)))) => match serde_json::from_str(text.as_str()) {
            Ok(ClientFrame::Hello {
                worker,
                capabilities,
                version,
            }) if valid_worker_name(&worker) => {
                if let Identity::Named(name) = &admitted.identity
                    && *name != worker
                {
                    let frame = ServerFrame::error(
                        "bad_frame",
                        &format!("this token is for worker {name}"),
                        None,
                    );
                    let _ = socket.send(frame.to_message()).await;
                    return;
                }
                let capabilities: Vec<String> =
                    capabilities.into_iter().filter(|k| known_kind(k)).collect();
                slot.update(|seen| {
                    seen.name = worker.clone();
                    seen.capabilities = capabilities.clone();
                    seen.version = version.chars().take(64).collect();
                    seen.identity = admitted.identity.label();
                });
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
    let welcome = ServerFrame::Welcome {
        worker: session.name.clone(),
        lease_secs: LEASE_SECS,
        heartbeat_secs: HEARTBEAT_SECS,
        max_result_bytes: MAX_RESULT_BYTES,
    };
    if socket.send(welcome.to_message()).await.is_err() {
        return;
    }
    tracing::info!(worker = %session.name, "worker connected");
    let mut ping = tokio::time::interval(Duration::from_secs(PING_SECS));
    ping.tick().await;
    let mut heard = tokio::time::Instant::now();
    loop {
        tokio::select! {
            _ = ping.tick() => {
                if heard.elapsed() > Duration::from_secs(IDLE_SECS) {
                    tracing::info!(worker = %session.name, "worker silent for {IDLE_SECS}s; closing");
                    break;
                }
                let check = state.clone();
                let rotated = tokio::task::spawn_blocking(move || {
                    worker_secret(&check).map(|s| <[u8; 32]>::from(Sha256::digest(&s)))
                })
                .await
                .is_ok_and(|now| now.ok() != Some(admitted.secret_digest));
                if rotated {
                    tracing::info!(worker = %session.name, "worker.token rotated; closing");
                    let frame = ServerFrame::error("unauthorized", "worker token rotated", None);
                    let _ = socket.send(frame.to_message()).await;
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
                slot.update(|seen| match &reply {
                    ServerFrame::Job { id, .. } => seen.job_id = Some(*id),
                    ServerFrame::Ack { .. } => seen.job_id = None,
                    _ => {}
                });
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

#[derive(Deserialize)]
struct MintBody {
    name: String,
}

/// `POST /apostille/api/v1/admin/workers/token {name}`: a token that
/// admits only `name` (shown once; rotate `worker.token` to revoke).
/// Needs a fresh step-up, like other credential changes.
async fn mint_token(
    auth: AdminAuth,
    State(state): State<AppState>,
    Json(body): Json<MintBody>,
) -> Response {
    if !valid_worker_name(&body.name) {
        return json_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "worker names are 1-32 of a-z, 0-9 and -",
        );
    }
    let Ok(conn) = db::open(&Store::new(&state.store_root)) else {
        return store_unavailable();
    };
    let ages = crate::admin::step_up_ages(&conn, &auth.user, unix_now());
    if !ages.fresh {
        return crate::admin::step_up_refusal(&auth.user, &ages);
    }
    let secret = match worker_secret(&state) {
        Ok(secret) => secret,
        Err(refused) => return refused.response(),
    };
    tracing::info!(worker = %body.name, "named worker token minted");
    Json(json!({"name": body.name, "token": named_token(&secret, &body.name)})).into_response()
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

/// Pack files out of a `packs.v1` bundle: `(stem, pretty JSON)` each.
/// Shape checks only (object, safe unique id); the Packs page runs the
/// full pack rules (`xtask packs-load`) before anything loads.
fn bundle_packs(bytes: &[u8]) -> Result<Vec<(String, Vec<u8>)>, &'static str> {
    let bundle: Value = serde_json::from_slice(bytes).map_err(|_| "result is not JSON")?;
    if bundle["format"] != "packs.v1" {
        return Err("result is not a packs.v1 bundle");
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

/// `POST …/work-items/{id}/stage-packs`: a done `research.nonparty`
/// result's packs land in `<store>/packs-inbox/<id>/` for the Packs
/// page to load. Re-staging replaces the folder. Audited `work_stage`.
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
    let result = match (&item.result, item.kind.as_str(), item.status.as_str()) {
        (Some(result), "research.nonparty", "done") => result,
        _ => {
            return json_error(
                StatusCode::CONFLICT,
                "only a done research.nonparty item with a result stages as packs",
            );
        }
    };
    let Ok(bytes) = Store::new(&state.store_root).read_blob(&result.sha256) else {
        return store_unavailable();
    };
    let files = match bundle_packs(&bytes) {
        Ok(files) => files,
        Err(why) => return json_error(StatusCode::UNPROCESSABLE_ENTITY, why),
    };
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

/// Worker socket route, merged into the PUBLIC `/apostille` API.
pub(crate) fn public_routes() -> Router<AppState> {
    Router::new().route("/api/v1/worker/ws", get(ws_upgrade))
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
        .route("/apostille/api/v1/admin/workers/token", post(mint_token))
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

    const TOKEN: &str = "test-worker-token-0123456789abcdef0123456789";
    const STAFF: &str = "staff-sess-1";

    type Ws = tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >;

    struct Fixture {
        _dir: tempfile::TempDir,
        state: AppState,
    }

    impl Fixture {
        fn new(token: Option<&str>) -> Self {
            let dir = tempfile::tempdir().unwrap();
            let store = Store::new(dir.path());
            store.init().unwrap();
            let meta = db::open(&store).unwrap();
            db::migrate(&meta).unwrap();
            auth::migrate_auth_tables(&meta).unwrap();
            config::migrate_config_tables(&meta).unwrap();
            let kek = config::Kek::generate();
            if let Some(token) = token {
                config::set_value(
                    &meta,
                    &kek,
                    &config::SetValue {
                        version: 1,
                        key: config::keys::WORKER_TOKEN,
                        key_type: config::ConfigType::Secret,
                        plaintext: token.as_bytes(),
                        now: unix_now(),
                        actor: "test",
                    },
                )
                .unwrap();
            }
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
        let fx = Fixture::new(Some(TOKEN));
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
        let mut a = connect(addr, Some(TOKEN)).await.unwrap();
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
        let mut b = connect(addr, Some(TOKEN)).await.unwrap();
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
        let fx = Fixture::new(Some(TOKEN));
        let addr = fx.serve_public();
        let uri = "/apostille/api/v1/admin/work-items";
        let body = json!({"kind": "scrape.urls", "payload": {"urls": []}, "max_attempts": 1});
        let (_, created) = call_json(fx.admin(), staff("POST", uri, Some(body))).await;
        let id = created["id"].as_i64().unwrap();
        let mut ws = connect(addr, Some(TOKEN)).await.unwrap();
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
        let fx = Fixture::new(Some(TOKEN));
        let addr = fx.serve_public();
        let uri = "/apostille/api/v1/admin/work-items";
        let body = json!({"kind": "scrape.urls", "payload": {"urls": []}});
        let (_, created) = call_json(fx.admin(), staff("POST", uri, Some(body))).await;
        let id = created["id"].as_i64().unwrap();
        let mut ws = connect(addr, Some(TOKEN)).await.unwrap();
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

    #[tokio::test]
    async fn named_tokens_admit_only_their_worker() {
        let fx = Fixture::new(Some(TOKEN));
        let addr = fx.serve_public();
        let token = named_token(TOKEN.as_bytes(), "plant-b");
        let mut ws = connect(addr, Some(&token)).await.unwrap();
        assert_eq!(hello(&mut ws, "plant-b").await["type"], "welcome");
        let mut ws = connect(addr, Some(&token)).await.unwrap();
        let refused = hello(&mut ws, "plant-c").await;
        assert_eq!(refused["code"], "bad_frame", "{refused}");
        let forged = token.replacen("plant-b", "plant-c", 1);
        assert_eq!(
            refused_with(connect(addr, Some(&forged)).await.unwrap_err()),
            401
        );
        let mint = "/apostille/api/v1/admin/workers/token";
        let (status, minted) = call_json(
            fx.admin(),
            staff("POST", mint, Some(json!({"name": "plant-b"}))),
        )
        .await;
        assert_eq!(
            (status, minted["token"].as_str()),
            (StatusCode::OK, Some(token.as_str()))
        );
        let (status, _) = call_json(
            fx.admin(),
            staff("POST", mint, Some(json!({"name": "Bad Name"}))),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[test]
    fn hmac_and_version_checks() {
        // RFC 4231 test case 2.
        let mac = hmac_sha256(b"Jefe", b"what do ya want for nothing?");
        let hex: String = mac.iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(
            hex,
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
        assert_eq!(version_matches("0.1.0+1bda381c0ffe", "1bda381"), Some(true));
        assert_eq!(version_matches("0.1.0+deadbee", "1bda381"), Some(false));
        assert_eq!(version_matches("0.1.0", "1bda381"), None);
        assert_eq!(version_matches("0.1.0+1bda381", "unknown"), None);
    }

    #[tokio::test]
    async fn unconfigured_or_short_token_refuses_every_worker() {
        let fx = Fixture::new(None);
        let addr = fx.serve_public();
        assert_eq!(
            refused_with(connect(addr, Some(TOKEN)).await.unwrap_err()),
            503
        );
        let short = Fixture::new(Some("too-short"));
        let addr = short.serve_public();
        assert_eq!(
            refused_with(connect(addr, Some("too-short")).await.unwrap_err()),
            503
        );
    }

    #[tokio::test]
    async fn first_frame_must_be_hello() {
        let fx = Fixture::new(Some(TOKEN));
        let addr = fx.serve_public();
        let mut ws = connect(addr, Some(TOKEN)).await.unwrap();
        let reply = send(&mut ws, json!({"type": "checkout"})).await;
        assert_eq!(reply["code"], "bad_frame");
        let mut ws = connect(addr, Some(TOKEN)).await.unwrap();
        let reply = send(&mut ws, json!({"type": "hello", "worker": "Bad:Name"})).await;
        assert_eq!(reply["code"], "bad_frame");
    }

    #[tokio::test]
    async fn staff_routes_need_a_staff_session() {
        let fx = Fixture::new(Some(TOKEN));
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
        assert!(valid_worker_name("plant-1"));
        for bad in ["", "Plant", "a:b", "a b", &"a".repeat(33)] {
            assert!(!valid_worker_name(bad), "{bad:?}");
        }
        assert_eq!(
            clean_content_type(Some("application/json")),
            "application/json"
        );
        assert_eq!(
            clean_content_type(Some("text/html; charset=utf-8")),
            "application/octet-stream"
        );
        assert_eq!(clean_content_type(None), "application/octet-stream");
        assert!(token_matches(TOKEN, TOKEN.as_bytes()));
        assert!(!token_matches("x", TOKEN.as_bytes()));
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
        let fx = Fixture::new(Some(TOKEN));
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
            bundle_packs(dup.to_string().as_bytes()).unwrap_err(),
            "duplicate pack id in bundle"
        );
        assert!(bundle_packs(b"not json").is_err());
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
