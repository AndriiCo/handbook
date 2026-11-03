//! Staff order API: list, detail, refund, advance.
//!
//! Owner: lane SA-24 commerce (`docs/lanes/sa-24.md`). First cut landed
//! by the builder (P332). Staff see the `customer_ref`, never an email;
//! refunds go through Stripe idempotently and only for paid orders.

use axum::{
    Router,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Json, Response},
    routing::{get, post},
};
use handbook_store::{db, store::Store};
use rusqlite::OptionalExtension;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::admin::AdminAuth;
use crate::{AppState, store_unavailable, unix_now};

#[derive(Debug, Deserialize)]
pub struct ListQuery {
    pub status: Option<String>,
}

/// `GET /admin/orders?status=` (contracts `admin-orders.json`).
async fn list(
    _auth: AdminAuth,
    State(state): State<AppState>,
    Query(q): Query<ListQuery>,
) -> Response {
    let store = Store::new(&state.store_root);
    let Ok(p2) = db::open_p2(&store) else {
        return store_unavailable();
    };
    let _ = crate::orders::migrate_p2_tables(&p2);
    let status = q.status.unwrap_or_else(|| "all".to_string());
    let rows: Vec<Value> = p2
        .prepare(
            "SELECT id, kind, status, mode, authority_slug, customer_ref, amount_json, created_at FROM customer_order
             WHERE (?1 = 'all' OR status = ?1) ORDER BY created_at DESC LIMIT 200",
        )
        .and_then(|mut s| {
            s.query_map([&status], |r| {
                let amount: Value = serde_json::from_str(&r.get::<_, String>(6)?).unwrap_or(json!({}));
                let st: String = r.get(2)?;
                Ok(json!({"id": r.get::<_, String>(0)?, "kind": r.get::<_, String>(1)?, "status": st,
                          "mode": r.get::<_, String>(3)?, "authority_slug": r.get::<_, String>(4)?,
                          "customer": r.get::<_, String>(5)?, "total_cents": amount["total_cents"].as_i64().unwrap_or(0),
                          "currency": amount["currency"].as_str().unwrap_or("usd"), "created_at": r.get::<_, i64>(7)?,
                          "needs_staff": st == "needs_staff"}))
            })?
            .collect()
        })
        .unwrap_or_default();
    let count = |s: &str| rows.iter().filter(|r| r["status"] == s).count();
    Json(json!({"orders": rows, "counts": {"all": rows.len(), "needs_staff": count("needs_staff"),
                                            "to_pack": count("packing"), "shipped": count("shipped")}}))
    .into_response()
}

/// `GET /admin/orders/{id}` — the order view plus its `customer_ref`.
async fn detail(
    _auth: AdminAuth,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    let store = Store::new(&state.store_root);
    let (Ok(meta), Ok(p2)) = (db::open(&store), db::open_p2(&store)) else {
        return store_unavailable();
    };
    let row: Option<(String, String)> = p2
        .query_row(
            "SELECT customer_ref, authority_slug FROM customer_order WHERE id = ?1",
            [&id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .unwrap_or(None);
    let Some((customer, slug)) = row else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "unknown order"})),
        )
            .into_response();
    };
    let name: String = meta
        .query_row("SELECT name FROM authority WHERE slug = ?1", [&slug], |r| {
            r.get(0)
        })
        .unwrap_or(slug);
    match crate::orders::order_view(&p2, &id, &name) {
        Ok(Some(mut v)) => {
            v["customer_ref"] = json!(customer);
            Json(v).into_response()
        }
        _ => store_unavailable(),
    }
}

/// `POST /admin/orders/{id}/refund` — paid orders only, idempotent.
async fn refund(
    auth: AdminAuth,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    let store = Store::new(&state.store_root);
    let (Ok(meta), Ok(p2)) = (db::open(&store), db::open_p2(&store)) else {
        return store_unavailable();
    };
    let Some(keys) = crate::stripe::keys(&state, &meta) else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"error": "payments not configured"})),
        )
            .into_response();
    };
    let row: Option<(String, Option<String>, String, String)> = p2
        .query_row(
            "SELECT p.status, p.payment_intent, o.status, o.customer_ref FROM order_payment p JOIN customer_order o ON o.id = p.order_id
             WHERE p.order_id = ?1",
            [&id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()
        .unwrap_or(None);
    let Some((pay_status, Some(intent), order_status, customer)) = row.filter(|r| r.0 == "paid")
    else {
        return (
            StatusCode::CONFLICT,
            Json(json!({"error": "only paid orders can be refunded"})),
        )
            .into_response();
    };
    if !crate::orders::can_transition(&order_status, "refunded") {
        return (
            StatusCode::CONFLICT,
            Json(json!({"error": format!("a {order_status} order cannot be refunded")})),
        )
            .into_response();
    }
    let (oid, k) = (id.clone(), keys.clone());
    let refund_id =
        match tokio::task::spawn_blocking(move || crate::stripe::refund(&k, &oid, &intent)).await {
            Ok(Ok(r)) => r,
            Ok(Err(e)) => {
                return (StatusCode::BAD_GATEWAY, Json(json!({"error": e}))).into_response();
            }
            Err(_) => return store_unavailable(),
        };
    let now = unix_now();
    let _ = pay_status;
    let _ = p2.execute(
        "UPDATE order_payment SET refund_id = ?1 WHERE order_id = ?2",
        rusqlite::params![refund_id, id],
    );
    let _ = p2.execute(
        "UPDATE customer_order SET status = 'refunded', updated_at = ?1 WHERE id = ?2",
        rusqlite::params![now, id],
    );
    let _ = p2.execute(
        "INSERT INTO order_event(order_id, at, kind, text, leg) VALUES (?1, ?2, 'order.refunded', 'Refunded in full', NULL)",
        rusqlite::params![id, now],
    );
    let _ = meta.execute(
        "INSERT INTO audit(at, actor, action, entity, entity_id, reason) VALUES (?1, ?2, 'order_refund', 'order', 0, ?3)",
        rusqlite::params![now, auth.user.user_id, id],
    );
    let topic = format!("billing:{customer}");
    state.bus.publish(
        &topic,
        json!({"type": "order.refunded", "topic": topic, "order_id": id, "at": now}).to_string(),
    );
    crate::ws_bus::publish_ops(&state.bus, "order.refunded", json!({"order_id": id}), now);
    Json(json!({"id": id, "status": "refunded"})).into_response()
}

#[derive(Debug, Deserialize)]
pub struct AdvanceBody {
    /// Target status (`shipped`, `in_transit`, `delivered`, `labels_ready`).
    pub to: String,
    #[serde(default)]
    pub note: Option<String>,
}

/// `POST /admin/orders/{id}/advance {to}` — staff move along the state
/// machine (kit packed and shipped, labels bought by hand, delivered).
async fn advance(
    auth: AdminAuth,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<AdvanceBody>,
) -> Response {
    let store = Store::new(&state.store_root);
    let (Ok(meta), Ok(p2)) = (db::open(&store), db::open_p2(&store)) else {
        return store_unavailable();
    };
    let row: Option<(String, String)> = p2
        .query_row(
            "SELECT status, customer_ref FROM customer_order WHERE id = ?1",
            [&id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .unwrap_or(None);
    let Some((from, customer)) = row else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "unknown order"})),
        )
            .into_response();
    };
    if !crate::orders::can_transition(&from, &body.to) {
        return (
            StatusCode::CONFLICT,
            Json(json!({"error": format!("{from} → {} is not allowed", body.to)})),
        )
            .into_response();
    }
    let now = unix_now();
    let text = match body.to.as_str() {
        "shipped" => "Kit shipped to you",
        "in_transit" => "On its way",
        "delivered" => "Delivered",
        "labels_ready" => "Labels ready",
        "packing" => "Kit is being packed",
        _ => "Order updated",
    };
    let note: String = body.note.unwrap_or_default().chars().take(200).collect();
    let _ = p2.execute(
        "UPDATE customer_order SET status = ?1, updated_at = ?2 WHERE id = ?3",
        rusqlite::params![body.to, now, id],
    );
    let _ = p2.execute(
        "INSERT INTO order_event(order_id, at, kind, text, leg) VALUES (?1, ?2, ?3, ?4, NULL)",
        rusqlite::params![
            id,
            now,
            format!("order.{}", body.to),
            if note.is_empty() {
                text.to_string()
            } else {
                format!("{text}: {note}")
            }
        ],
    );
    let _ = meta.execute(
        "INSERT INTO audit(at, actor, action, entity, entity_id, reason) VALUES (?1, ?2, 'order_advance', 'order', 0, ?3)",
        rusqlite::params![now, auth.user.user_id, format!("{id} {from}->{}", body.to)],
    );
    let topic = format!("billing:{customer}");
    state.bus.publish(
        &topic,
        json!({"type": format!("order.{}", body.to), "topic": topic, "order_id": id, "at": now})
            .to_string(),
    );
    crate::ws_bus::publish_ops(
        &state.bus,
        "order.advanced",
        json!({"order_id": id, "to": body.to}),
        now,
    );
    Json(json!({"id": id, "status": body.to})).into_response()
}

/// Staff routes merged onto the ADMIN listener only (wrapped in the
/// admin guard stack by the builder; handlers take `AdminAuth`).
pub fn admin_router(state: AppState) -> Router {
    Router::<AppState>::new()
        .route("/apostille/api/v1/admin/orders", get(list))
        .route("/apostille/api/v1/admin/orders/{id}", get(detail))
        .route("/apostille/api/v1/admin/orders/{id}/refund", post(refund))
        .route("/apostille/api/v1/admin/orders/{id}/advance", post(advance))
        .with_state(state)
}
