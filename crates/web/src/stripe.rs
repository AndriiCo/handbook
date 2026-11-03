//! Stripe hosted Checkout + the webhook (the only CSRF-exempt POST:
//! raw body, HMAC via aws_lc_rs, `stripe_event` dedupe, server-side
//! session re-fetch for amount/paid/livemode).
//!
//! Owner: lane SA-24 commerce (`docs/lanes/sa-24.md`). C2 first cut
//! landed by the builder (P332) while the lane session could not start.
//!
//! Card data never touches this server (PCI SAQ A: hosted Checkout).
//! The webhook body is never trusted for money: a verified event only
//! names the session, and `orders::settle` re-reads it from Stripe.

use axum::{
    Router,
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Json, Response},
    routing::post,
};
use handbook_store::{db, store::Store};
use rusqlite::OptionalExtension;
use serde_json::{Value, json};

use crate::{AppState, store_unavailable, unix_now};

pub const API: &str = "https://api.stripe.com/v1";
/// Pinned API version: response shapes never drift under us.
pub const API_VERSION: &str = "2024-06-20";
/// Webhook timestamp tolerance (Stripe's default).
pub const TOLERANCE_SECS: i64 = 300;

/// Keys read from sealed config (never logged; `Debug` redacts).
#[derive(Clone)]
pub struct Keys {
    pub secret: String,
    pub webhook: Option<String>,
}

impl std::fmt::Debug for Keys {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("stripe::Keys(<redacted>)")
    }
}

impl Keys {
    pub fn live(&self) -> bool {
        self.secret.starts_with("sk_live") || self.secret.starts_with("rk_live")
    }
}

/// Stripe keys from sealed config, when the secret key is set.
pub fn keys(state: &AppState, conn: &rusqlite::Connection) -> Option<Keys> {
    let kek = state.kek.as_deref()?;
    let secret = config::get_secret_for_job(conn, kek, config::keys::STRIPE_SECRET_KEY)
        .ok()
        .flatten()?;
    let secret = String::from_utf8(secret).ok().filter(|s| !s.is_empty())?;
    let webhook = config::get_secret_for_job(conn, kek, config::keys::STRIPE_WEBHOOK_SECRET)
        .ok()
        .flatten()
        .and_then(|w| String::from_utf8(w).ok())
        .filter(|w| !w.is_empty());
    Some(Keys { secret, webhook })
}

fn agent() -> ureq::Agent {
    crate::oauth::ensure_provider();
    ureq::Agent::new_with_config(
        ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_global(Some(std::time::Duration::from_secs(30)))
            .build(),
    )
}

fn read(resp: Result<ureq::http::Response<ureq::Body>, ureq::Error>) -> Result<Value, String> {
    let resp = resp.map_err(|e| format!("stripe transport: {e}"))?;
    let status = resp.status().as_u16();
    let body: Value = resp
        .into_body()
        .read_json()
        .map_err(|e| format!("stripe body: {e}"))?;
    if !(200..300).contains(&status) {
        // Stripe error messages name the parameter, never card data.
        let msg = body["error"]["message"]
            .as_str()
            .unwrap_or("error")
            .chars()
            .take(160)
            .collect::<String>();
        return Err(format!("stripe {status}: {msg}"));
    }
    Ok(body)
}

/// One Checkout session request.
#[derive(Debug, Clone)]
pub struct CheckoutRequest {
    pub order_id: String,
    pub currency: String,
    pub lines: Vec<(String, i64)>,
    pub success_url: String,
    pub cancel_url: String,
}

/// Form pairs for `POST /v1/checkout/sessions` (pure, unit-tested).
pub fn checkout_form(req: &CheckoutRequest) -> Vec<(String, String)> {
    let mut f = vec![
        ("mode".to_string(), "payment".to_string()),
        ("success_url".to_string(), req.success_url.clone()),
        ("cancel_url".to_string(), req.cancel_url.clone()),
        ("client_reference_id".to_string(), req.order_id.clone()),
        ("metadata[order_id]".to_string(), req.order_id.clone()),
        (
            "payment_intent_data[metadata][order_id]".to_string(),
            req.order_id.clone(),
        ),
        ("expires_at".to_string(), (unix_now() + 3_600).to_string()),
    ];
    for (i, (label, cents)) in req.lines.iter().enumerate() {
        f.push((format!("line_items[{i}][quantity]"), "1".to_string()));
        f.push((
            format!("line_items[{i}][price_data][currency]"),
            req.currency.clone(),
        ));
        f.push((
            format!("line_items[{i}][price_data][unit_amount]"),
            cents.to_string(),
        ));
        f.push((
            format!("line_items[{i}][price_data][product_data][name]"),
            label.clone(),
        ));
    }
    f
}

/// Create a hosted Checkout session → `(session_id, url)`. The
/// Idempotency-Key ties retries of the same order + amount together.
pub fn create_checkout(keys: &Keys, req: &CheckoutRequest) -> Result<(String, String), String> {
    let total: i64 = req.lines.iter().map(|l| l.1).sum();
    let form = checkout_form(req);
    let body = read(
        agent()
            .post(&format!("{API}/checkout/sessions"))
            .header("Authorization", &format!("Bearer {}", keys.secret))
            .header("Stripe-Version", API_VERSION)
            .header(
                "Idempotency-Key",
                &format!("{}-checkout-{total}", req.order_id),
            )
            .send_form(form.iter().map(|(k, v)| (k.as_str(), v.as_str()))),
    )?;
    let id = body["id"]
        .as_str()
        .ok_or("stripe: no session id")?
        .to_string();
    let url = body["url"]
        .as_str()
        .ok_or("stripe: no checkout url")?
        .to_string();
    Ok((id, url))
}

/// Re-read a session from Stripe (the only source of truth for money).
pub fn retrieve_session(keys: &Keys, session_id: &str) -> Result<Value, String> {
    if !session_id.starts_with("cs_")
        || !session_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_')
    {
        return Err("bad session id".to_string());
    }
    read(
        agent()
            .get(&format!("{API}/checkout/sessions/{session_id}"))
            .header("Authorization", &format!("Bearer {}", keys.secret))
            .header("Stripe-Version", API_VERSION)
            .call(),
    )
}

/// Refund a paid order's PaymentIntent (idempotent per order).
pub fn refund(keys: &Keys, order_id: &str, payment_intent: &str) -> Result<String, String> {
    let body = read(
        agent()
            .post(&format!("{API}/refunds"))
            .header("Authorization", &format!("Bearer {}", keys.secret))
            .header("Stripe-Version", API_VERSION)
            .header("Idempotency-Key", &format!("{order_id}-refund"))
            .send_form([
                ("payment_intent", payment_intent),
                ("metadata[order_id]", order_id),
            ]),
    )?;
    Ok(body["id"].as_str().unwrap_or("").to_string())
}

/// `Stripe-Signature: t=…,v1=…[,v1=…]` check: HMAC-SHA256 over
/// `"{t}.{payload}"` with the endpoint secret, constant-time compare,
/// and `|now - t| <= tolerance`.
pub fn verify_signature(
    payload: &[u8],
    header: &str,
    secret: &str,
    now: i64,
    tolerance: i64,
) -> bool {
    use aws_lc_rs::hmac;
    let mut t: Option<i64> = None;
    let mut sigs: Vec<Vec<u8>> = Vec::new();
    for part in header.split(',') {
        match part.trim().split_once('=') {
            Some(("t", v)) => t = v.parse().ok(),
            Some(("v1", v)) => {
                if let Some(bytes) = hex_decode(v) {
                    sigs.push(bytes);
                }
            }
            _ => {}
        }
    }
    let Some(t) = t else { return false };
    // abs_diff: `t` is attacker-supplied; `now - t` overflowed (abort).
    if t.abs_diff(now) > tolerance.unsigned_abs() || sigs.is_empty() {
        return false;
    }
    let key = hmac::Key::new(hmac::HMAC_SHA256, secret.as_bytes());
    let mut signed = t.to_string().into_bytes();
    signed.push(b'.');
    signed.extend_from_slice(payload);
    sigs.iter().any(|s| hmac::verify(&key, &signed, s).is_ok())
}

fn hex_decode(s: &str) -> Option<Vec<u8>> {
    s.len()
        .is_multiple_of(2)
        .then(|| {
            (0..s.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok())
                .collect()
        })
        .flatten()
}

/// `POST /api/v1/stripe/webhook` — Stripe only (signature), the one
/// CSRF-exempt POST. Every event id is processed once.
async fn webhook(State(state): State<AppState>, headers: HeaderMap, body: Bytes) -> Response {
    let store = Store::new(&state.store_root);
    let (Ok(meta), Ok(p2)) = (db::open(&store), db::open_p2(&store)) else {
        return store_unavailable();
    };
    let _ = crate::orders::migrate_p2_tables(&p2);
    let Some(keys) = keys(&state, &meta) else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"error": "payments not configured"})),
        )
            .into_response();
    };
    let Some(secret) = keys.webhook.clone() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"error": "webhook secret not configured"})),
        )
            .into_response();
    };
    let sig = headers
        .get("stripe-signature")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if !verify_signature(&body, sig, &secret, unix_now(), TOLERANCE_SECS) {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "bad signature"})),
        )
            .into_response();
    }
    let Ok(event) = serde_json::from_slice::<Value>(&body) else {
        return (StatusCode::BAD_REQUEST, Json(json!({"error": "bad event"}))).into_response();
    };
    let (Some(id), Some(kind)) = (event["id"].as_str(), event["type"].as_str()) else {
        return (StatusCode::BAD_REQUEST, Json(json!({"error": "bad event"}))).into_response();
    };
    let fresh = p2
        .execute(
            "INSERT OR IGNORE INTO stripe_event(id, type, received_at) VALUES (?1, ?2, ?3)",
            rusqlite::params![id, kind, unix_now()],
        )
        .unwrap_or(0)
        == 1;
    if !fresh {
        return Json(json!({"received": true, "duplicate": true})).into_response();
    }
    if matches!(
        kind,
        "checkout.session.completed" | "checkout.session.async_payment_succeeded"
    ) {
        let session_id = event["data"]["object"]["id"]
            .as_str()
            .unwrap_or("")
            .to_string();
        let order_id: Option<String> = p2
            .query_row(
                "SELECT order_id FROM order_payment WHERE session_id = ?1",
                [&session_id],
                |r| r.get(0),
            )
            .optional()
            .unwrap_or(None);
        if let Some(order_id) = order_id {
            let st = state.clone();
            let _ =
                tokio::task::spawn_blocking(move || crate::orders::settle(&st, &keys, &order_id))
                    .await;
        }
    }
    Json(json!({"received": true})).into_response()
}

/// Public routes, merged into the `/apostille` API router.
pub fn public_routes() -> Router<AppState> {
    Router::new().route("/api/v1/stripe/webhook", post(webhook))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sign(secret: &str, t: i64, payload: &str) -> String {
        use aws_lc_rs::hmac;
        let key = hmac::Key::new(hmac::HMAC_SHA256, secret.as_bytes());
        let tag = hmac::sign(&key, format!("{t}.{payload}").as_bytes());
        tag.as_ref().iter().map(|b| format!("{b:02x}")).collect()
    }

    #[test]
    fn signatures_verify_and_reject_tamper_staleness_and_garbage() {
        let (secret, now, body) = ("whsec_test_synthetic", 1_793_750_400, r#"{"id":"evt_1"}"#);
        let good = format!("t={now},v1={}", sign(secret, now, body));
        assert!(verify_signature(
            body.as_bytes(),
            &good,
            secret,
            now + 10,
            TOLERANCE_SECS
        ));
        assert!(
            !verify_signature(br#"{"id":"evt_2"}"#, &good, secret, now, TOLERANCE_SECS),
            "tampered body"
        );
        assert!(
            !verify_signature(body.as_bytes(), &good, "whsec_other", now, TOLERANCE_SECS),
            "wrong secret"
        );
        assert!(
            !verify_signature(body.as_bytes(), &good, secret, now + 301, TOLERANCE_SECS),
            "stale"
        );
        assert!(!verify_signature(
            body.as_bytes(),
            "t=abc,v1=zz",
            secret,
            now,
            TOLERANCE_SECS
        ));
        assert!(
            !verify_signature(
                body.as_bytes(),
                "t=-9223372036854775808,v1=00",
                secret,
                now,
                TOLERANCE_SECS
            ),
            "extreme t never overflows"
        );
        assert!(!verify_signature(
            body.as_bytes(),
            "t=9223372036854775807,v1=00",
            secret,
            now,
            TOLERANCE_SECS
        ));
        let rotated = format!("t={now},v1=00ff,v1={}", sign(secret, now, body));
        assert!(
            verify_signature(body.as_bytes(), &rotated, secret, now, TOLERANCE_SECS),
            "any v1 may match"
        );
    }

    #[test]
    fn checkout_form_carries_order_metadata_and_lines() {
        let f = checkout_form(&CheckoutRequest {
            order_id: "ord_q7Hm2xR4kP".into(),
            currency: "usd".into(),
            lines: vec![("Mailing kit".into(), 4900)],
            success_url: "https://example.test/apostille/orders/ord_q7Hm2xR4kP?checkout=success"
                .into(),
            cancel_url: "https://example.test/apostille/order?order=ord_q7Hm2xR4kP&step=pay".into(),
        });
        let get = |k: &str| f.iter().find(|(a, _)| a == k).map(|(_, v)| v.as_str());
        assert_eq!(get("mode"), Some("payment"));
        assert_eq!(get("metadata[order_id]"), Some("ord_q7Hm2xR4kP"));
        assert_eq!(get("line_items[0][price_data][unit_amount]"), Some("4900"));
        assert!(
            get("customer_email").is_none(),
            "no PII leaves for Stripe beyond what the buyer types there"
        );
        assert!(
            Keys {
                secret: "sk_live_x".into(),
                webhook: None
            }
            .live()
        );
        assert!(
            !Keys {
                secret: "sk_test_x".into(),
                webhook: None
            }
            .live()
        );
        assert_eq!(
            format!(
                "{:?}",
                Keys {
                    secret: "sk_test_x".into(),
                    webhook: None
                }
            ),
            "stripe::Keys(<redacted>)"
        );
    }
}
