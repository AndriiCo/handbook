//! WS bus: authenticated topic subscriptions over one endpoint.
//!
//! `GET /apostille/ws/bus?topic={topic}` validates the session
//! BEFORE the upgrade and binds the subscription to the caller's
//! identity ([`auth::ws::authorize`]). Anonymous callers may read
//! `authority:{slug}` only — and only accepted facts, which the
//! publish path enforces by accepting just `fact_accepted` events
//! for authority channels.

use axum::{
    extract::{
        Query, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Json},
};
use handbook_store::db;
use serde_json::json;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::sync::broadcast;

use crate::{AppState, store_unavailable, unix_now};

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
pub async fn ws_bus_upgrade(
    State(state): State<AppState>,
    Query(params): Query<TopicParams>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    ws_bus_upgrade_inner(state, params, headers, ws, false).await
}

/// Admin-branch upgrade: same topics and authz, but the staff
/// session cookie resolves — mounted on the admin router only,
/// behind its host guard + IP allowlist + log layers.
pub async fn ws_bus_upgrade_admin(
    State(state): State<AppState>,
    Query(params): Query<TopicParams>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    ws_bus_upgrade_inner(state, params, headers, ws, true).await
}

async fn ws_bus_upgrade_inner(
    state: AppState,
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
    let channel = topic.as_channel();
    let rx = state.bus.subscribe(&channel);
    ws.on_upgrade(move |socket| serve(socket, rx, channel))
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
async fn serve(mut socket: WebSocket, mut rx: broadcast::Receiver<String>, channel: String) {
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
    loop {
        tokio::select! {
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
