//! Stripe hosted Checkout + the webhook (the only CSRF-exempt POST:
//! raw body, HMAC via aws_lc_rs, `stripe_event` dedupe, server-side
//! session re-fetch for amount/paid/livemode).
//!
//! Card data never touches this server (PCI SAQ A: hosted Checkout).
//! The webhook body is never trusted for money: a verified event only
//! names the session, and `orders::settle` re-reads it from Stripe.
//!
//! W-3 `stripe.mode = fake` (loopback or staging boots only): [`Keys`]
//! carries the store instead of a key, and the three Stripe calls answer
//! from the `fake_stripe_session` table — no network, no card. Checkout
//! is a local page marked DEMO; its Pay button records the session paid
//! and delivers `checkout.session.completed` to the REAL [`webhook`]
//! handler, signed with a per-process random secret, so signature
//! checking, event dedupe and `orders::settle` (which re-reads the fake
//! session) all run unchanged. The receipt page is marked DEMO. Stripe
//! test keys stay the `sandbox` mode.

use axum::{
    Router,
    body::Bytes,
    extract::{Path, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{Html, IntoResponse, Json, Redirect, Response},
    routing::{get, post},
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
    /// W-3 fake Stripe: the store root its sessions live in (no key, no
    /// network). `None` is the real Stripe.
    pub fake: Option<std::path::PathBuf>,
}

impl std::fmt::Debug for Keys {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("stripe::Keys(<redacted>)")
    }
}

impl Keys {
    /// Live money. A fake Stripe never is, whatever key is sealed.
    pub fn live(&self) -> bool {
        self.fake.is_none()
            && (self.secret.starts_with("sk_live") || self.secret.starts_with("rk_live"))
    }
}

/// Stripe keys from sealed config, when the secret key is set (under
/// `stripe.mode = fake`, the fake Stripe whatever is sealed).
pub fn keys(state: &AppState, conn: &rusqlite::Connection) -> Option<Keys> {
    if state.fakes.is_fake(config::Integration::Stripe) {
        return Some(Keys {
            secret: String::new(),
            webhook: Some(fake_webhook_secret().to_string()),
            fake: Some(state.store_root.clone()),
        });
    }
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
    Some(Keys {
        secret,
        webhook,
        fake: None,
    })
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
    /// Absolute session expiry; with `bucket` it makes every retry of
    /// the same attempt byte-identical (Stripe rejects a reused
    /// Idempotency-Key whose parameters differ).
    pub expires_at: i64,
    pub bucket: i64,
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
        ("expires_at".to_string(), req.expires_at.to_string()),
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
    if let Some(root) = &keys.fake {
        return fake_create(root, req);
    }
    let total: i64 = req.lines.iter().map(|l| l.1).sum();
    let form = checkout_form(req);
    let body = read(
        agent()
            .post(&format!("{API}/checkout/sessions"))
            .header("Authorization", &format!("Bearer {}", keys.secret))
            .header("Stripe-Version", API_VERSION)
            .header(
                "Idempotency-Key",
                &format!("{}-checkout-{total}-{}", req.order_id, req.bucket),
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
    if let Some(root) = &keys.fake {
        return fake_retrieve(root, session_id);
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
    if keys.fake.is_some() {
        // Nothing was charged, so nothing moves back.
        return Ok(format!(
            "re_fake_{}",
            payment_intent.trim_start_matches("pi_fake_")
        ));
    }
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
    // Seen before (processed successfully) → done. An event is only
    // recorded after it settles, so a transient failure answers 500 and
    // Stripe retries it (review #36).
    let seen: Option<i64> = match p2
        .query_row("SELECT 1 FROM stripe_event WHERE id = ?1", [id], |r| {
            r.get(0)
        })
        .optional()
    {
        Ok(v) => v,
        Err(_) => return store_unavailable(),
    };
    if seen.is_some() {
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
            let settled =
                tokio::task::spawn_blocking(move || crate::orders::settle(&st, &keys, &order_id))
                    .await;
            if !matches!(settled, Ok(Ok(_))) {
                tracing::warn!(event = %id, "stripe webhook: settle failed; Stripe will retry");
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({"error": "retry"})),
                )
                    .into_response();
            }
        }
    }
    if p2
        .execute(
            "INSERT OR IGNORE INTO stripe_event(id, type, received_at) VALUES (?1, ?2, ?3)",
            rusqlite::params![id, kind, unix_now()],
        )
        .is_err()
    {
        return store_unavailable();
    }
    Json(json!({"received": true})).into_response()
}

/// Public routes, merged into the `/apostille` API router. The fake
/// checkout routes answer 404 unless `stripe.mode = fake`.
pub fn public_routes() -> Router<AppState> {
    Router::new()
        .route("/api/v1/stripe/webhook", post(webhook))
        .route("/api/v1/stripe/fake/checkout/{id}", get(fake_checkout_page))
        .route("/api/v1/stripe/fake/checkout/{id}/pay", post(fake_pay))
        .route("/api/v1/stripe/fake/receipt/{id}", get(fake_receipt_page))
}

// ------------------------------------------------------------ W-3 fake

/// Session ids the fake Stripe mints (`cs_` keeps `retrieve_session`'s
/// shape check).
pub const FAKE_SESSION_PREFIX: &str = "cs_fake_";

/// The fake webhook signing secret: random per process, never stored or
/// logged, so only this process can sign an event its webhook accepts.
fn fake_webhook_secret() -> &'static str {
    static SECRET: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    SECRET.get_or_init(|| {
        use rand::RngCore as _;
        let mut bytes = [0u8; 32];
        rand::rng().fill_bytes(&mut bytes);
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    })
}

/// P2 DDL (`CREATE TABLE IF NOT EXISTS` only; no numbered migration).
pub fn migrate_p2_tables(p2: &rusqlite::Connection) -> rusqlite::Result<()> {
    p2.execute_batch(
        "CREATE TABLE IF NOT EXISTS fake_stripe_session (
            id TEXT PRIMARY KEY,
            order_id TEXT NOT NULL,
            amount_total INTEGER NOT NULL,
            currency TEXT NOT NULL,
            lines_json TEXT NOT NULL,
            success_url TEXT NOT NULL,
            cancel_url TEXT NOT NULL,
            expires_at INTEGER NOT NULL,
            payment_status TEXT NOT NULL DEFAULT 'unpaid',
            paid_at INTEGER,
            created_at INTEGER NOT NULL
        );",
    )
}

fn fake_p2(root: &std::path::Path) -> Result<rusqlite::Connection, String> {
    let p2 = db::open_p2(&Store::new(root)).map_err(|e| format!("fake stripe: {e}"))?;
    migrate_p2_tables(&p2).map_err(|e| format!("fake stripe: {e}"))?;
    Ok(p2)
}

/// The path of an absolute URL (redirects stay on the host the browser
/// is on: a local boot's public origin may not be reachable).
fn local_path(url: &str) -> String {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .map_or(url, |r| r.find('/').map_or("/", |i| &r[i..]));
    if rest.starts_with('/') && !rest.starts_with("//") {
        rest.to_string()
    } else {
        "/".to_string()
    }
}

fn fake_create(root: &std::path::Path, req: &CheckoutRequest) -> Result<(String, String), String> {
    let p2 = fake_p2(root)?;
    let total: i64 = req.lines.iter().map(|l| l.1).sum();
    let id = format!("{FAKE_SESSION_PREFIX}{}", uuid::Uuid::new_v4().simple());
    p2.execute(
        "INSERT INTO fake_stripe_session(id, order_id, amount_total, currency, lines_json,
                                         success_url, cancel_url, expires_at, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        rusqlite::params![
            id,
            req.order_id,
            total,
            req.currency.to_lowercase(),
            serde_json::to_string(&req.lines).unwrap_or_else(|_| "[]".to_string()),
            local_path(&req.success_url),
            local_path(&req.cancel_url),
            req.expires_at,
            unix_now()
        ],
    )
    .map_err(|e| format!("fake stripe: {e}"))?;
    Ok((
        id.clone(),
        format!("/apostille/api/v1/stripe/fake/checkout/{id}"),
    ))
}

/// One fake session in Stripe's Checkout Session shape (the fields
/// `orders::judge_session` reads). Never `livemode`.
fn fake_retrieve(root: &std::path::Path, session_id: &str) -> Result<Value, String> {
    let p2 = fake_p2(root)?;
    let row: Option<(String, i64, String, i64, String)> = p2
        .query_row(
            "SELECT order_id, amount_total, currency, expires_at, payment_status
             FROM fake_stripe_session WHERE id = ?1",
            [session_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .optional()
        .map_err(|e| format!("fake stripe: {e}"))?;
    let (order_id, amount, currency, expires_at, payment_status) =
        row.ok_or("stripe 404: no such checkout.session")?;
    let paid = payment_status == "paid";
    let status = if paid {
        "complete"
    } else if expires_at <= unix_now() {
        "expired"
    } else {
        "open"
    };
    Ok(json!({
        "id": session_id,
        "object": "checkout.session",
        "client_reference_id": order_id,
        "metadata": {"order_id": order_id},
        "amount_total": amount,
        "currency": currency,
        "livemode": false,
        "status": status,
        "payment_status": payment_status,
        "payment_intent": paid.then(|| format!("pi_fake_{}", &session_id[FAKE_SESSION_PREFIX.len()..])),
    }))
}

fn not_found() -> Response {
    (StatusCode::NOT_FOUND, Json(json!({"error": "not found"}))).into_response()
}

fn html_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// The fake session's display row: lines, total, currency, cancel path,
/// payment status, paid_at.
type FakeView = (Vec<(String, i64)>, i64, String, String, String, Option<i64>);

fn fake_view(state: &AppState, id: &str) -> Option<FakeView> {
    if !state.fakes.is_fake(config::Integration::Stripe) || !id.starts_with(FAKE_SESSION_PREFIX) {
        return None;
    }
    let p2 = fake_p2(&state.store_root).ok()?;
    p2.query_row(
        "SELECT lines_json, amount_total, currency, cancel_url, payment_status, paid_at
         FROM fake_stripe_session WHERE id = ?1",
        [id],
        |r| {
            Ok((
                serde_json::from_str(&r.get::<_, String>(0)?).unwrap_or_default(),
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
            ))
        },
    )
    .optional()
    .ok()
    .flatten()
}

fn money(cents: i64, currency: &str) -> String {
    format!(
        "{}.{:02} {}",
        cents / 100,
        cents % 100,
        currency.to_uppercase()
    )
}

/// A small no-JS page with the DEMO banner on top.
fn demo_page(title: &str, body: &str) -> Response {
    let page = format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\
         <meta name=\"robots\" content=\"noindex\"><title>{title}</title>\
         <style>body{{font-family:system-ui,sans-serif;max-width:32rem;margin:2rem auto;padding:0 1rem;color:#111;background:#fff}}\
         .demo{{border:3px solid #b00;color:#b00;font-weight:700;padding:.75rem;text-align:center}}\
         table{{width:100%;border-collapse:collapse}}td{{padding:.4rem 0;border-bottom:1px solid #ddd}}\
         td:last-child{{text-align:right}}button{{font-size:1rem;padding:.75rem 1.25rem;min-height:44px}}</style>\
         </head><body><p class=\"demo\">DEMO — no card is charged. This is the fake Stripe of a demo boot.</p>{body}</body></html>"
    );
    (
        [(header::CACHE_CONTROL, HeaderValue::from_static("no-store"))],
        Html(page),
    )
        .into_response()
}

fn lines_table(lines: &[(String, i64)], total: i64, currency: &str) -> String {
    let mut rows: String = lines
        .iter()
        .map(|(label, cents)| {
            format!(
                "<tr><td>{}</td><td>{}</td></tr>",
                html_escape(label),
                money(*cents, currency)
            )
        })
        .collect();
    rows.push_str(&format!(
        "<tr><td><strong>Total</strong></td><td><strong>{}</strong></td></tr>",
        money(total, currency)
    ));
    format!("<table>{rows}</table>")
}

/// `GET /api/v1/stripe/fake/checkout/{id}`: the DEMO checkout page.
async fn fake_checkout_page(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let Some((lines, total, currency, cancel, payment_status, _)) = fake_view(&state, &id) else {
        return not_found();
    };
    let action = if payment_status == "paid" {
        "<p>This checkout is already paid.</p>".to_string()
    } else {
        format!(
            "<form method=\"post\" action=\"/apostille/api/v1/stripe/fake/checkout/{id}/pay\">\
             <button type=\"submit\">Pay {} (demo)</button></form>\
             <p><a href=\"{}\">Cancel and go back</a></p>",
            money(total, &currency),
            html_escape(&cancel)
        )
    };
    demo_page(
        "DEMO checkout",
        &format!(
            "<h1>DEMO checkout</h1>{}{action}",
            lines_table(&lines, total, &currency)
        ),
    )
}

/// `POST /api/v1/stripe/fake/checkout/{id}/pay`: record the fake session
/// paid, deliver a signed `checkout.session.completed` to the real
/// webhook handler, then return to the order (303). Same-origin form
/// posts only.
async fn fake_pay(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    if !crate::passkey::origin_ok(&headers, state.admin_mtls_off) {
        return crate::passkey::csrf_fail();
    }
    if fake_view(&state, &id).is_none() {
        return not_found();
    }
    let Ok(p2) = fake_p2(&state.store_root) else {
        return store_unavailable();
    };
    let now = unix_now();
    let success: String = match p2.query_row(
        "UPDATE fake_stripe_session SET payment_status = 'paid', paid_at = COALESCE(paid_at, ?1)
         WHERE id = ?2 RETURNING success_url",
        rusqlite::params![now, id],
        |r| r.get(0),
    ) {
        Ok(url) => url,
        Err(_) => return store_unavailable(),
    };
    drop(p2);
    let event = json!({
        "id": format!("evt_fake_{}", uuid::Uuid::new_v4().simple()),
        "object": "event",
        "type": "checkout.session.completed",
        "livemode": false,
        "created": now,
        "data": {"object": {"id": id, "object": "checkout.session"}},
    })
    .to_string();
    let mut signed = HeaderMap::new();
    let Ok(signature) = HeaderValue::from_str(&sign(event.as_bytes(), fake_webhook_secret(), now))
    else {
        return store_unavailable();
    };
    signed.insert("stripe-signature", signature);
    let delivered = webhook(State(state), signed, Bytes::from(event)).await;
    if !delivered.status().is_success() {
        tracing::warn!(session = %id, status = %delivered.status(), "fake stripe: webhook refused");
    }
    Redirect::to(&success).into_response()
}

/// `Stripe-Signature` header for `payload` (the fake Stripe's sender;
/// [`verify_signature`] is the receiver).
fn sign(payload: &[u8], secret: &str, t: i64) -> String {
    use aws_lc_rs::hmac;
    let key = hmac::Key::new(hmac::HMAC_SHA256, secret.as_bytes());
    let mut signed = t.to_string().into_bytes();
    signed.push(b'.');
    signed.extend_from_slice(payload);
    let tag = hmac::sign(&key, &signed);
    let hex: String = tag.as_ref().iter().map(|b| format!("{b:02x}")).collect();
    format!("t={t},v1={hex}")
}

/// `GET /api/v1/stripe/fake/receipt/{id}`: the DEMO receipt.
async fn fake_receipt_page(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let Some((lines, total, currency, _, payment_status, paid_at)) = fake_view(&state, &id) else {
        return not_found();
    };
    if payment_status != "paid" {
        return not_found();
    }
    demo_page(
        "DEMO receipt",
        &format!(
            "<h1>DEMO receipt</h1><p>Receipt {} · paid (demo) at unix time {} · no card was charged.</p>{}",
            html_escape(&id),
            paid_at.unwrap_or_default(),
            lines_table(&lines, total, &currency)
        ),
    )
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
            expires_at: 1_793_754_000,
            bucket: 2_989_584,
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
                webhook: None,
                fake: None
            }
            .live()
        );
        assert!(
            !Keys {
                secret: "sk_test_x".into(),
                webhook: None,
                fake: None
            }
            .live()
        );
        assert_eq!(
            format!(
                "{:?}",
                Keys {
                    secret: "sk_test_x".into(),
                    webhook: None,
                    fake: None
                }
            ),
            "stripe::Keys(<redacted>)"
        );
    }
}
