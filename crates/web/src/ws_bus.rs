//! WS bus: authenticated topic subscriptions over one endpoint.
//!
//! `GET /apostille/ws/bus?topic={topic}` validates the session
//! BEFORE the upgrade and binds the subscription to the caller's
//! identity ([`auth::ws::authorize`]). Anonymous callers may read
//! `authority:{slug}` only — and only accepted facts, which the
//! publish path enforces by accepting just the ids-only
//! [`AUTHORITY_KINDS`] for authority channels.

use axum::{
    extract::{
        ConnectInfo, Query, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Json},
};
use handbook_store::db;
use serde_json::json;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use tokio::sync::broadcast;

use crate::{AppState, store_unavailable, unix_now};

/// Inbound frame ceiling (4 KiB): clients never publish over
/// the socket, so anything bigger is abuse, not protocol.
pub const MAX_WS_MESSAGE: usize = 4096;
/// Server heartbeat: a ping this often keeps NATs and browsers
/// from silently dropping idle dossier sockets (browsers pong
/// automatically — no client code needed).
pub const WS_PING_SECS: u64 = 30;
/// Open sockets per client IP, both listeners combined (one
/// process serves both, so one process-wide counter is exact).
pub const MAX_WS_PER_IP: usize = 32;

static WS_CONN_COUNT: std::sync::LazyLock<Mutex<HashMap<std::net::IpAddr, usize>>> =
    std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));

/// One held socket slot, released back to the IP budget on drop
/// (the upgrade future owns it for the connection lifetime).
struct WsConnGuard {
    ip: std::net::IpAddr,
}

impl WsConnGuard {
    fn acquire(ip: std::net::IpAddr) -> Option<Self> {
        let mut counts = WS_CONN_COUNT.lock().expect("ws conn counts poisoned");
        let n = counts.entry(ip).or_insert(0);
        if *n >= MAX_WS_PER_IP {
            tracing::warn!("ws bus upgrade denied: per-IP connection cap");
            return None;
        }
        *n += 1;
        Some(Self { ip })
    }
}

impl Drop for WsConnGuard {
    fn drop(&mut self) {
        let mut counts = WS_CONN_COUNT.lock().expect("ws conn counts poisoned");
        if let Some(n) = counts.get_mut(&self.ip) {
            *n = n.saturating_sub(1);
            if *n == 0 {
                counts.remove(&self.ip);
            }
        }
    }
}

/// Event kinds anonymous `authority:{slug}` subscribers may receive.
/// Each carries ids only (the client re-reads the public API), so no
/// unpublished text ever reaches an anonymous socket. `fact_retired`
/// names a fact that just left the public reads; `evidence_updated`
/// names a staff note published or withdrawn.
pub const AUTHORITY_KINDS: &[&str] = &[
    "fact_accepted",
    "fact_retired",
    "plan_accepted",
    "offices_accepted",
    "evidence_updated",
];

/// Publish one ids-only event on `authority:{slug}`. Kinds outside
/// [`AUTHORITY_KINDS`] are dropped (debug builds panic), so a lane can
/// never fan unpublished drafts out to anonymous subscribers.
#[allow(dead_code)] // B0 seam: SA-22 plan accept + SA-23 offices accept.
pub fn publish_authority_event(bus: &Bus, slug: &str, kind: &str, ids: serde_json::Value, at: i64) {
    debug_assert!(AUTHORITY_KINDS.contains(&kind), "authority kind {kind}");
    if !AUTHORITY_KINDS.contains(&kind) {
        return;
    }
    let topic = format!("authority:{slug}");
    let mut msg = serde_json::json!({"type": kind, "topic": topic, "at": at});
    if let (Some(obj), Some(extra)) = (msg.as_object_mut(), ids.as_object()) {
        for (k, v) in extra {
            obj.entry(k.clone()).or_insert_with(|| v.clone());
        }
    }
    bus.publish(&topic, msg.to_string());
}

/// Publish one event on the staff-only `ops` feed (dashboard activity).
pub fn publish_ops(bus: &Bus, kind: &str, body: serde_json::Value, at: i64) {
    let mut msg = serde_json::json!({"type": kind, "topic": "ops", "at": at});
    if let (Some(obj), Some(extra)) = (msg.as_object_mut(), body.as_object()) {
        for (k, v) in extra {
            obj.entry(k.clone()).or_insert_with(|| v.clone());
        }
    }
    bus.publish("ops", msg.to_string());
}

/// One broadcast channel per topic, created on first use.
#[derive(Clone, Debug, Default)]
pub struct Bus {
    inner: Arc<Mutex<HashMap<String, broadcast::Sender<String>>>>,
}

impl Bus {
    /// Bound on live channels before idle ones are reaped (M1: every
    /// distinct topic string keys a channel, so an enumeration loop
    /// would otherwise grow the map forever).
    const MAX_CHANNELS: usize = 4096;

    /// Empty bus (no channels until first subscribe/publish).
    pub fn new() -> Self {
        Bus::default()
    }

    /// Subscribe to a canonical channel name.
    pub fn subscribe(&self, channel: &str) -> broadcast::Receiver<String> {
        self.sender(channel).subscribe()
    }

    /// Publish one JSON message to a channel (no subscribers: dropped).
    pub fn publish(&self, channel: &str, msg: String) {
        let _ = self.sender(channel).send(msg);
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.inner.lock().expect("bus lock poisoned").len()
    }

    fn sender(&self, channel: &str) -> broadcast::Sender<String> {
        let mut map = self.inner.lock().expect("bus lock poisoned");
        if let Some(tx) = map.get(channel) {
            return tx.clone();
        }
        if map.len() >= Self::MAX_CHANNELS {
            // Reap idle channels (no receivers left): a subscribe loop
            // that disconnects cannot grow the map. A flood of LIVE
            // channels still grows past the bound, but only with a live
            // subscriber attached to each.
            map.retain(|_, tx| tx.receiver_count() > 0);
        }
        map.entry(channel.to_string())
            .or_insert_with(|| broadcast::channel(64).0)
            .clone()
    }
}

#[derive(serde::Deserialize)]
pub struct TopicParams {
    topic: String,
}

/// Upgrade to a topic subscription after session + topic authz.
///
/// Anonymous callers present no cookie and reach `authority:{slug}`
/// channels only; every other topic needs a session whose role (and,
/// for owned topics, `customer_ref`) passes [`auth::ws::authorize`].
/// Denials return 403 JSON instead of upgrading.
///
/// Security M-A: the PUBLIC branch ignores [`auth::ADM_COOKIE`] — a
/// staff session cookie presented there resolves to anonymous, so
/// staff topics are reachable only on the admin host (which mounts
/// [`ws_bus_upgrade_admin`]). The `__Host-` prefix already binds the
/// cookie to the admin host in real browsers; this closes the seam
/// for non-browser clients that copy the cookie value across hosts.
///
/// The admin branch additionally gates on [`admin_origin_allowed`]:
/// only the admin origin upgrades there, so a staff session cookie
/// ridden from any other page (or a header-less native client)
/// cannot open staff topics.
pub async fn ws_bus_upgrade(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Query(params): Query<TopicParams>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    ws_bus_upgrade_inner(state, addr, params, headers, ws, false).await
}

/// The one origin that upgrades on the admin branch. Browsers always
/// send `Origin` on WS handshakes, so a missing one is a native client,
/// never the staff console.
pub fn admin_ws_origin() -> String {
    format!("https://{}", auth::admin_host())
}

/// Admin-branch origin gate: exactly [`admin_ws_origin`], plus —
/// only under the `admin_mtls_off` dev seam (tests, staging
/// tunnel) — loopback `http` origins, the same seam the admin
/// host guard admits. The public `https` origin never passes,
/// even in dev: a staff cookie copied to a public page must not
/// upgrade staff topics anywhere.
pub(crate) fn admin_origin_allowed(origin: Option<&str>, dev_seam: bool) -> bool {
    let Some(o) = origin else {
        return false;
    };
    if o == admin_ws_origin() {
        return true;
    }
    dev_seam && loopback_http_origin(o)
}

/// Loopback `http` origin with no path (`http://127.0.0.1:18319`,
/// `http://localhost`, …): the dev-seam half of the gate above.
fn loopback_http_origin(origin: &str) -> bool {
    let Some(rest) = origin.strip_prefix("http://") else {
        return false;
    };
    if rest.contains('/') {
        return false;
    }
    let host = rest.split_once(':').map_or(rest, |(h, _)| h);
    if host != "localhost" && host != "127.0.0.1" {
        return false;
    }
    match rest.split_once(':') {
        Some((_, port)) => !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit()),
        None => true,
    }
}

/// Admin-branch upgrade: same topics and authz, but the staff
/// session cookie resolves — mounted on the admin router only,
/// behind its host guard + IP allowlist + log layers, and behind
/// the admin origin gate below.
pub async fn ws_bus_upgrade_admin(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Query(params): Query<TopicParams>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    ws_bus_upgrade_inner(state, addr, params, headers, ws, true).await
}

async fn ws_bus_upgrade_inner(
    state: AppState,
    addr: SocketAddr,
    params: TopicParams,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
    accept_admin_cookie: bool,
) -> impl IntoResponse {
    let Some(topic) = auth::ws::WsTopic::parse(&params.topic) else {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"error": "unknown topic"})),
        )
            .into_response();
    };
    // H4: `authority:{slug}` subscribes anonymously, so a well-formed
    // slug for an authority that does not exist must not open a channel
    // (broadcast enumeration + dangling subs). Other nouns name rows
    // the authorizer already checks.
    if let auth::ws::WsTopic::Authority { slug } = &topic {
        let Ok(conn) = db::open(&handbook_store::store::Store::new(&state.store_root)) else {
            return store_unavailable();
        };
        let known: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM authority WHERE slug = ?1)",
                [slug],
                |row| row.get(0),
            )
            .unwrap_or(false);
        if !known {
            return (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(json!({"error": "unknown topic"})),
            )
                .into_response();
        }
    }
    // Admin origin gate: the staff branch upgrades for its own
    // origin only (plus the loopback dev seam) — checked before any
    // session work, for every topic, since the staff cookie alone
    // must never suffice from elsewhere.
    if accept_admin_cookie
        && !admin_origin_allowed(
            headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()),
            state.admin_mtls_off,
        )
    {
        tracing::warn!(
            topic_noun = topic_noun(&topic),
            "ws bus upgrade denied: admin origin gate"
        );
        return (
            StatusCode::FORBIDDEN,
            Json(json!({"error": "origin forbidden"})),
        )
            .into_response();
    }
    // H1 (CSWSH): browsers always send Origin on WS handshakes, so
    // a present-but-disallowlisted Origin is a cross-site page riding
    // ambient cookies — fail closed before any session work. Checked
    // only for credentialed upgrades and non-public topics: anonymous
    // authority reads carry no cookies to ride (public data, by
    // design). A missing Origin is a native client, not a browser.
    let cookies = headers
        .get(header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let admin_cookie = accept_admin_cookie
        .then(|| auth::cookie_value(cookies, auth::ADM_COOKIE))
        .flatten();
    let credentialed = admin_cookie
        .or_else(|| auth::cookie_value(cookies, auth::PUB_COOKIE))
        .is_some()
        || demo_cookie_value(cookies).is_some();
    if credentialed || !matches!(topic, auth::ws::WsTopic::Authority { .. }) {
        let origin = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok());
        if origin.is_some_and(|o| !auth::csrf::origin_allowed(Some(o), state.admin_mtls_off)) {
            tracing::warn!(
                topic_noun = topic_noun(&topic),
                "ws bus upgrade denied: disallowlisted origin"
            );
            return (
                StatusCode::FORBIDDEN,
                Json(json!({"error": "origin forbidden"})),
            )
                .into_response();
        }
    }
    let (role, customer_ref) = match resolve_caller(&state, &headers, accept_admin_cookie) {
        Ok(caller) => caller,
        Err(()) => return store_unavailable(),
    };
    // S6 seam filled: P2 ownership lookup, so submission/shipment
    // owners (and staff) subscribe; strangers still fail closed.
    let owners = crate::p2::P2Owners::new(&state.store_root);
    if !auth::ws::authorize(role, customer_ref.as_deref(), &owners, &topic) {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({"error": "topic forbidden"})),
        )
            .into_response();
    }
    // Per-IP socket budget, checked after authz (no oracle for
    // capped subnets: strangers fail topic checks first). The guard
    // rides the upgrade future — the slot frees on disconnect.
    let Some(conn_guard) = WsConnGuard::acquire(addr.ip()) else {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            Json(json!({"error": "too many connections"})),
        )
            .into_response();
    };
    let channel = topic.as_channel();
    let rx = state.bus.subscribe(&channel);
    ws.max_message_size(MAX_WS_MESSAGE)
        .on_upgrade(move |socket| serve(socket, rx, channel, conn_guard))
}

/// Resolve (role, customer_ref) from the session cookies. No (or an
/// invalid) session is anonymous, not an error — anonymous subscribe
/// is legitimate on authority topics. A broken store is a 500.
/// `accept_admin_cookie` gates [`auth::ADM_COOKIE`]: false on the
/// public branch (M-A), true on the admin branch.
fn resolve_caller(
    state: &AppState,
    headers: &HeaderMap,
    accept_admin_cookie: bool,
) -> Result<(auth::Role, Option<String>), ()> {
    let cookies = headers
        .get(header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let admin_token = accept_admin_cookie
        .then(|| auth::cookie_value(cookies, auth::ADM_COOKIE))
        .flatten();
    let token = admin_token.or_else(|| auth::cookie_value(cookies, auth::PUB_COOKIE));
    let demo = demo_cookie_value(cookies);
    if token.is_none() && demo.is_none() {
        return Ok((auth::Role::Anonymous, None));
    }
    let Ok(conn) = db::open(&handbook_store::store::Store::new(&state.store_root)) else {
        return Err(());
    };
    if let Some(token) = token {
        let Ok(Some(session)) = auth::get_session(&conn, token) else {
            return Ok((auth::Role::Anonymous, None));
        };
        if !auth::session_usable(&conn, &session, unix_now()) {
            return Ok((auth::Role::Anonymous, None));
        }
        let _ = auth::touch_session(&conn, &session.id, unix_now());
        return Ok((session.role, session.customer_ref));
    }
    // DEMO login until real auth lands: a valid demo_session cookie
    // subscribes as its customer (P2 submission/shipment topics).
    if let Some(demo) = demo
        && let Ok(Some((customer_ref, expires_at))) = db::get_demo_session(&conn, demo)
        && unix_now() < expires_at
    {
        return Ok((auth::Role::Customer, Some(customer_ref)));
    }
    Ok((auth::Role::Anonymous, None))
}

/// Topic noun for denial logs (never the full topic: `billing:`
/// and owner topics name customer data — M5).
fn topic_noun(topic: &auth::ws::WsTopic) -> &'static str {
    match topic {
        auth::ws::WsTopic::Authority { .. } => "authority",
        auth::ws::WsTopic::Call { .. } => "call",
        auth::ws::WsTopic::Submission { .. } => "submission",
        auth::ws::WsTopic::Shipment { .. } => "shipment",
        auth::ws::WsTopic::Billing { .. } => "billing",
        auth::ws::WsTopic::Run { .. } => "run",
        auth::ws::WsTopic::Proposals => "proposals",
        auth::ws::WsTopic::Ops => "ops",
    }
}

/// `demo_session` cookie value, if present and non-empty.
fn demo_cookie_value(cookies: &str) -> Option<&str> {
    cookies
        .split(';')
        .map(str::trim)
        .find_map(|pair| pair.strip_prefix("demo_session="))
        .filter(|v| !v.is_empty())
}

/// Serve one subscription: a hello (with an ephemeral presence id
/// for anonymous callers — never linkable), then broadcast fan-out.
/// `_conn_guard` holds the per-IP slot for the socket lifetime.
async fn serve(
    mut socket: WebSocket,
    mut rx: broadcast::Receiver<String>,
    channel: String,
    _conn_guard: WsConnGuard,
) {
    let presence: String = (0..8)
        .map(|_| format!("{:02x}", rand::random::<u8>()))
        .collect();
    let hello = json!({"type": "subscribed", "topic": channel, "presence": presence});
    if socket
        .send(Message::Text(hello.to_string().into()))
        .await
        .is_err()
    {
        return;
    }
    let mut heartbeat = tokio::time::interval(std::time::Duration::from_secs(WS_PING_SECS));
    heartbeat.tick().await;
    loop {
        tokio::select! {
            _ = heartbeat.tick() => {
                if socket
                    .send(Message::Ping(bytes::Bytes::from_static(b"hb")))
                    .await
                    .is_err()
                {
                    break;
                }
            }
            msg = rx.recv() => {
                match msg {
                    Ok(text) => {
                        if socket.send(Message::Text(text.into())).await.is_err() {
                            break;
                        }
                    }
                    // Lagged: skip to live, never replay history.
                    Err(broadcast::error::RecvError::Lagged(_)) => {}
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
            incoming = socket.recv() => {
                match incoming {
                    Some(Ok(Message::Ping(p))) => {
                        if socket.send(Message::Pong(p)).await.is_err() {
                            break;
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    // Clients never publish over the socket; ignore.
                    _ => {}
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn admin_origin_gate_matches_configured_admin_host() {
        assert_eq!(
            admin_ws_origin(),
            format!("https://{}", auth::admin_host()),
            "the gate must track the admin host, not a stale literal"
        );
        // Production shape (no dev seam): exactly the admin origin.
        assert!(admin_origin_allowed(Some(&admin_ws_origin()), false));
        for bad in [
            None,
            Some("https://andrii.ai"),
            Some("https://evil.test"),
            Some("http://127.0.0.1:18319"),
            Some("http://localhost:3000"),
            Some("https://admin.andrii.ai.evil.test"),
            Some("https://admin.andrii.ai/"),
            Some("https://admin.andrii.ai:443"),
        ] {
            assert!(!admin_origin_allowed(bad, false), "denied in prod: {bad:?}");
        }
        // Dev seam (tests, staging tunnel): loopback http joins the
        // admin origin — the public origin still never passes.
        for ok in [
            "https://admin.andrii.ai",
            "http://127.0.0.1:18319",
            "http://localhost:3000",
            "http://127.0.0.1",
        ] {
            assert!(admin_origin_allowed(Some(ok), true), "allowed in dev: {ok}");
        }
        for bad in [
            None,
            Some("https://andrii.ai"),
            Some("https://evil.test"),
            Some("https://127.0.0.1:18319"),
            Some("http://127.0.0.1:abc"),
            Some("http://127.0.0.1:18319/path"),
            Some("http://10.0.0.1:18319"),
        ] {
            assert!(!admin_origin_allowed(bad, true), "denied in dev: {bad:?}");
        }
    }

    #[test]
    fn per_ip_cap_holds_and_releases() {
        let ip: std::net::IpAddr = "203.0.113.7".parse().unwrap();
        let mut guards = Vec::new();
        for _ in 0..MAX_WS_PER_IP {
            guards.push(WsConnGuard::acquire(ip).expect("slot under cap"));
        }
        assert!(WsConnGuard::acquire(ip).is_none(), "cap refuses one more");
        // A neighbour IP is unaffected by another's full budget.
        let other: std::net::IpAddr = "203.0.113.8".parse().unwrap();
        let _other_guard = WsConnGuard::acquire(other).expect("other IP has room");
        guards.pop();
        assert!(
            WsConnGuard::acquire(ip).is_some(),
            "drop frees exactly one slot"
        );
    }

    #[test]
    fn channel_map_reaps_idle_past_bound() {
        // M1: an enumeration loop that disconnects cannot grow the map:
        // past MAX_CHANNELS every idle channel is reaped on next use.
        let bus = Bus::new();
        for i in 0..Bus::MAX_CHANNELS {
            bus.publish(&format!("call:{}", i + 1), "x".to_string());
        }
        assert_eq!(bus.len(), Bus::MAX_CHANNELS);
        bus.publish("call:9999999", "x".to_string());
        assert_eq!(bus.len(), 1);
    }

    #[test]
    fn channel_map_keeps_live_subscribers_past_bound() {
        // Channels with live receivers survive the reap.
        let bus = Bus::new();
        let _live = bus.subscribe("call:1");
        for i in 1..Bus::MAX_CHANNELS {
            bus.publish(&format!("call:{}", i + 1), "x".to_string());
        }
        assert_eq!(bus.len(), Bus::MAX_CHANNELS);
        bus.publish("call:9999999", "x".to_string());
        assert_eq!(bus.len(), 2);
    }
}
