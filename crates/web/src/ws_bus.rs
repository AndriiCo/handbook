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

    fn sender(&self, channel: &str) -> broadcast::Sender<String> {
        self.inner
            .lock()
            .expect("bus lock poisoned")
            .entry(channel.to_string())
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
pub async fn ws_bus_upgrade(
    State(state): State<AppState>,
    Query(params): Query<TopicParams>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    let Some(topic) = auth::ws::WsTopic::parse(&params.topic) else {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"error": "unknown topic"})),
        )
            .into_response();
    };
    let (role, customer_ref) = match resolve_caller(&state, &headers) {
        Ok(caller) => caller,
        Err(()) => return store_unavailable(),
    };
    // SEAM: S6 swaps `NoOwners` for the P2 ownership lookup; until
    // then customers fail closed on submission/shipment topics.
    if !auth::ws::authorize(role, customer_ref.as_deref(), &auth::ws::NoOwners, &topic) {
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
fn resolve_caller(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<(auth::Role, Option<String>), ()> {
    let cookies = headers
        .get(header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let token = auth::cookie_value(cookies, auth::ADM_COOKIE)
        .or_else(|| auth::cookie_value(cookies, auth::PUB_COOKIE));
    let Some(token) = token else {
        return Ok((auth::Role::Anonymous, None));
    };
    let Ok(conn) = db::open(&handbook_store::store::Store::new(&state.store_root)) else {
        return Err(());
    };
    let Ok(Some(session)) = auth::get_session(&conn, token) else {
        return Ok((auth::Role::Anonymous, None));
    };
    if !session.valid(unix_now()) {
        return Ok((auth::Role::Anonymous, None));
    }
    let _ = auth::touch_session(&conn, &session.id, unix_now());
    Ok((session.role, session.customer_ref))
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
