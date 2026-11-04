//! Staff-only live FedEx endpoints (SA-19 builder wiring).
//!
//! Mounted on the admin listener behind the same IP + session +
//! CSRF guards as the other staff routers. Every mutating call runs
//! the blocking FedEx I/O on `spawn_blocking`; the idempotency
//! ledger is hydrated per request from `fedex_live_ledger` rows and
//! every outcome is appended to `fedex_live_audit`. Live money moves
//! only when `labels.live` is on and the caps/allowlist/credentials
//! rows are set — otherwise 503, fail closed.
//!
//! The label PDF FedEx returns is written to the content-addressed
//! blob store and indexed in `fedex_live_label` in the same
//! transaction as the ledger row, so staff can download it again
//! (`GET …/fedex-live/label/{order_id}/{leg}`, staff session only).

use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderValue, StatusCode, header},
    response::IntoResponse,
    routing::{get, post},
};
use handbook_store::{db, store::Store};
use labels::{
    BuyRequest, Direction, ShipmentPayload, UreqTransport,
    live::{
        AuditRow, BuyParams, Leg, LiveError, LiveShipment, MemLedger, StoredLabel, VoidParams,
        idempotency_key, oauth_token_pairs, oauth_token_url, parse_token_response,
    },
    validate_shipment,
};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::{
    AppState,
    admin::{AdminAuth, admin_host_guard, csrf_guard, require_admin_ip},
    unix_now,
};

/// Live-buy state, built once from `labels.*` DB rows at boot.
#[derive(Debug, Clone, Default)]
pub struct FedexLiveState {
    pub enabled: bool,
    pub daily_cap: u32,
    pub max_usd_cents: i64,
    pub ship_to_allow: Vec<String>,
    pub client_id: Option<config::SecretText>,
    pub client_secret: Option<config::SecretText>,
    pub account: Option<String>,
}

impl FedexLiveState {
    fn live_config(&self) -> labels::live::LiveConfig {
        labels::live::LiveConfig {
            enabled: self.enabled,
            daily_cap: self.daily_cap,
            max_usd_cents: self.max_usd_cents,
            ship_to_allow: self.ship_to_allow.clone(),
        }
    }

    /// Credentials present for a live call (enabled is checked
    /// separately so caps reads work before credentials land).
    /// Cloned wrappers: the secret never materializes as a plain
    /// `String` outside [`config::SecretText`].
    fn bearer_pair(&self) -> Option<(config::SecretText, config::SecretText)> {
        match (&self.client_id, &self.client_secret) {
            (Some(id), Some(secret)) => Some((id.clone(), secret.clone())),
            _ => None,
        }
    }
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/apostille/api/v1/fedex-live/caps", get(get_caps))
        .route("/apostille/api/v1/fedex-live/quote", post(post_quote))
        .route("/apostille/api/v1/fedex-live/purchase", post(post_purchase))
        .route("/apostille/api/v1/fedex-live/void", post(post_void))
        .route(
            "/apostille/api/v1/fedex-live/label/{order_id}/{leg}",
            get(get_label),
        )
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            csrf_guard,
        ))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            require_admin_ip,
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

fn open(state: &AppState) -> Option<Connection> {
    db::open(&Store::new(&state.store_root)).ok()
}

fn err(status: StatusCode, message: impl Into<String>) -> axum::response::Response {
    (status, Json(serde_json::json!({ "error": message.into() }))).into_response()
}

/// Map a live-buy refusal to its HTTP answer, tagged with an
/// `error_kind` for the access log (the reason a reported
/// `x-request-id` failed). Upstream FedEx failures log the message
/// too: it names the failing step (oauth, rate, ship), never secrets.
fn live_error(e: LiveError) -> axum::response::Response {
    let (status, kind) = match &e {
        LiveError::NotEnabled => (StatusCode::SERVICE_UNAVAILABLE, "fedex_live_off"),
        LiveError::StaffOnly => (StatusCode::FORBIDDEN, "fedex_staff_only"),
        LiveError::ZipNotAllowed => (StatusCode::FORBIDDEN, "fedex_zip_not_allowed"),
        LiveError::DailyCapReached { .. } => (StatusCode::UNPROCESSABLE_ENTITY, "fedex_daily_cap"),
        LiveError::OverCostCap { .. } => (StatusCode::UNPROCESSABLE_ENTITY, "fedex_cost_cap"),
        // DuplicateLeg surfaces only when the ledger row is missing
        // mid-flight; the caller retries under a new order id.
        _ => {
            tracing::warn!(reason = %e, "fedex live call failed");
            (StatusCode::BAD_GATEWAY, "fedex_upstream")
        }
    };
    crate::tag_error(err(status, e.to_string()), kind)
}

#[derive(Debug, Serialize)]
struct Caps {
    enabled: bool,
    daily_cap: u32,
    bought_today: u32,
    max_usd: f64,
}

#[derive(Debug, Deserialize)]
struct LegBody {
    order_id: String,
    leg: String,
}

#[derive(Debug, Deserialize)]
struct QuoteBody {
    payload: ShipmentPayload,
}

#[derive(Debug, Deserialize)]
struct PurchaseBody {
    order_id: String,
    leg: String,
    payload: ShipmentPayload,
    quoted_cents: Option<i64>,
}

fn parse_leg(raw: &str) -> Result<Leg, String> {
    match raw.trim() {
        "outbound" => Ok(Leg::Outbound),
        "return" => Ok(Leg::Return),
        other => Err(format!("leg must be outbound|return, got {other:?}")),
    }
}

/// Start of today (UTC midnight) as a unix timestamp: only ledger
/// rows at or after this count toward the daily cap.
fn utc_day_start(now: i64) -> i64 {
    now - now.rem_euclid(86_400)
}

/// Hydrate the idempotency ledger from `fedex_live_ledger` rows.
/// Only rows created today count toward the daily cap; older rows
/// still guard replay (a void never re-opens a leg).
fn load_ledger(conn: &Connection, today_start: i64) -> Result<MemLedger, rusqlite::Error> {
    let mut ledger = MemLedger::new();
    let mut stmt = conn.prepare(
        "SELECT idempotency_key, tracking_no, carrier_service, quoted_cents, created_at
         FROM fedex_live_ledger",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, Option<i64>>(3)?,
            row.get::<_, i64>(4)?,
        ))
    })?;
    for row in rows {
        let (key, tracking_no, carrier_service, quoted_cents, created_at) = row?;
        let label = StoredLabel {
            tracking_no,
            carrier_service,
            quoted_cents,
            label_pdf: Vec::new(),
        };
        if created_at >= today_start {
            ledger.record_buy(&key, label);
        } else {
            ledger.record_history(&key, label);
        }
    }
    Ok(ledger)
}

/// One newly bought leg to persist (`None` on replay/void — the
/// row already exists or nothing was bought).
struct NewLedgerRow<'a> {
    key: &'a str,
    order_id: &'a str,
    leg: &'a str,
    stored: &'a StoredLabel,
    created_at: i64,
    quoted_cents: Option<i64>,
    /// Blob address of the label PDF, when it reached the store.
    label_sha256: Option<&'a str>,
}

/// Ledger row, label index row and audit rows in one transaction:
/// a bought leg is never recorded without its audit trail.
fn persist_outcome(
    conn: &Connection,
    new_ledger: Option<NewLedgerRow<'_>>,
    audits: &[AuditRow],
) -> Result<(), rusqlite::Error> {
    let tx = conn.unchecked_transaction()?;
    persist_rows(&tx, new_ledger, audits)?;
    tx.commit()
}

fn persist_rows(
    conn: &Connection,
    new_ledger: Option<NewLedgerRow<'_>>,
    audits: &[AuditRow],
) -> Result<(), rusqlite::Error> {
    if let Some(row) = new_ledger {
        if let Some(sha) = row.label_sha256 {
            conn.execute(
                "INSERT OR IGNORE INTO fedex_live_label(
                   idempotency_key, tracking_no, label_sha256, content_type, created_at
                 ) VALUES (?1, ?2, ?3, 'application/pdf', ?4)",
                rusqlite::params![row.key, row.stored.tracking_no, sha, row.created_at],
            )?;
        }
        conn.execute(
            "INSERT OR IGNORE INTO fedex_live_ledger(
               idempotency_key, order_id, leg, tracking_no,
               carrier_service, quoted_cents, created_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![
                row.key,
                row.order_id,
                row.leg,
                row.stored.tracking_no,
                row.stored.carrier_service,
                row.quoted_cents,
                row.created_at
            ],
        )?;
    }
    for audit in audits {
        conn.execute(
            "INSERT INTO fedex_live_audit(at, actor, action, idempotency_key, detail)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![
                audit.at,
                audit.actor,
                audit.action,
                audit.idempotency_key,
                audit.detail
            ],
        )?;
    }
    Ok(())
}

/// Write a fresh buy's PDF to the blob store. `None` when there is
/// nothing to keep (a replay) or the write failed; the purchase still
/// records its ledger row either way, because the money is spent.
fn store_label_pdf(store: &Store, stored: &StoredLabel) -> Option<String> {
    if stored.label_pdf.is_empty() {
        return None;
    }
    match store.write_blob(&stored.label_pdf) {
        Ok(sha) => Some(sha),
        Err(e) => {
            tracing::error!(error = %e, "fedex live label pdf not stored");
            None
        }
    }
}

/// Staff download path for one leg's label PDF.
fn label_url(order_id: &str, leg: Leg) -> String {
    let segment: String = order_id
        .bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect();
    format!(
        "/apostille/api/v1/fedex-live/label/{segment}/{}",
        leg.as_str()
    )
}

/// The stored label row for one leg: `(tracking_no, label_sha256)`.
fn find_label(conn: &Connection, key: &str) -> Result<Option<(String, String)>, rusqlite::Error> {
    use rusqlite::OptionalExtension as _;
    conn.query_row(
        "SELECT tracking_no, label_sha256 FROM fedex_live_label WHERE idempotency_key = ?1",
        [key],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .optional()
}

async fn get_caps(State(state): State<AppState>, _auth: AdminAuth) -> impl IntoResponse {
    let live = &state.fedex_live;
    let bought_today = match open(&state) {
        Some(conn) => conn
            .query_row(
                "SELECT COUNT(*) FROM fedex_live_ledger WHERE created_at >= ?1",
                rusqlite::params![utc_day_start(unix_now())],
                |row| row.get::<_, u32>(0),
            )
            .unwrap_or(0),
        None => 0,
    };
    Json(Caps {
        enabled: live.enabled,
        daily_cap: live.daily_cap,
        bought_today,
        max_usd: live.max_usd_cents as f64 / 100.0,
    })
    .into_response()
}

/// FedEx API root from `labels.fedex_base_url`, allowlisted to the two
/// FedEx roots (credentials travel there, so never an arbitrary host);
/// unset or anything else stays on production, as before this key.
pub(crate) fn fedex_base(state: &AppState) -> &'static str {
    let configured = state.kek.as_deref().and_then(|kek| {
        let conn =
            handbook_store::db::open(&handbook_store::store::Store::new(&state.store_root)).ok()?;
        state
            .config_cache
            .text(&conn, kek, config::keys::LABELS_FEDEX_BASE_URL)
            .ok()
            .flatten()
    });
    base_from(configured.as_deref())
}

fn base_from(configured: Option<&str>) -> &'static str {
    match configured.map(|v| v.trim().trim_end_matches('/')) {
        Some(labels::FEDEX_BASE_SANDBOX) => labels::FEDEX_BASE_SANDBOX,
        _ => labels::FEDEX_BASE_PROD,
    }
}

/// Blocking OAuth client-credentials fetch. Secrets stay in this
/// scope; transport errors never include them (labels guarantee).
pub(crate) fn fetch_bearer(
    base: &str,
    client_id: &str,
    client_secret: &str,
) -> Result<String, LiveError> {
    crate::oauth::ensure_provider();
    let config = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(Some(std::time::Duration::from_secs(30)))
        .build();
    let agent = ureq::Agent::new_with_config(config);
    let url = oauth_token_url(base);
    let pairs = oauth_token_pairs(client_id, client_secret);
    let response = agent
        .post(&url)
        .send_form(pairs.iter().copied())
        .map_err(|e| LiveError::OAuth(e.to_string()))?;
    let status = response.status().as_u16();
    let body = response
        .into_body()
        .read_to_vec()
        .map_err(|e| LiveError::OAuth(e.to_string()))?;
    if !(200..300).contains(&status) {
        return Err(LiveError::OAuth(format!("http {status}")));
    }
    parse_token_response(&body)
}

fn validate_payload(payload: &ShipmentPayload) -> Result<(), String> {
    let defects = validate_shipment(payload);
    if defects.is_empty() {
        return Ok(());
    }
    Err(defects
        .iter()
        .map(|d| d.message.clone())
        .collect::<Vec<_>>()
        .join("; "))
}

/// FedEx Ship requires a contact phone on both ends (10 digits for
/// US/CA after [`labels::client::fedex_phone`] drops a leading `1`).
fn require_phones(payload: &ShipmentPayload) -> Result<(), String> {
    for (side, address) in [("ship-from", &payload.from), ("ship-to", &payload.to)] {
        let raw = address.phone.as_deref().unwrap_or_default();
        let digits = labels::client::fedex_phone(raw, &address.country);
        let nanp = matches!(address.country.trim(), "US" | "CA");
        let ok = if nanp {
            digits.len() == 10
        } else {
            (7..=15).contains(&digits.len())
        };
        if !ok {
            return Err(format!(
                "{side} phone is required: FedEx needs a contact number (10 digits in the US)"
            ));
        }
    }
    Ok(())
}

async fn post_quote(
    State(state): State<AppState>,
    _auth: AdminAuth,
    Json(body): Json<QuoteBody>,
) -> impl IntoResponse {
    if let Err(message) = validate_payload(&body.payload) {
        return err(StatusCode::UNPROCESSABLE_ENTITY, message);
    }
    let live = state.fedex_live.clone();
    // Rate's `accountNumber.value` must be the shipper account: an
    // empty one is refused by FedEx (and was, before P393).
    let account = live.account.clone().unwrap_or_default();
    let Some((cred_id, cred_secret)) = live.bearer_pair().filter(|_| !account.trim().is_empty())
    else {
        return crate::tag_error(
            err(
                StatusCode::SERVICE_UNAVAILABLE,
                "live fedex buys are not configured",
            ),
            "fedex_not_configured",
        );
    };
    let payload = body.payload;
    let cfg = live.live_config();
    let base = fedex_base(&state);
    let out = tokio::task::spawn_blocking(move || {
        let bearer = fetch_bearer(base, cred_id.as_str(), cred_secret.as_str())?;
        let shipment = LiveShipment::new(base, UreqTransport::default());
        let request = BuyRequest {
            payload: &payload,
            direction: Direction::Outbound,
            payment_ref: &account,
        };
        shipment.quote(&cfg, &bearer, &request)
    })
    .await;
    match out {
        Ok(Ok(cents)) => Json(serde_json::json!({ "quoted_cents": cents })).into_response(),
        Ok(Err(e)) => live_error(e),
        Err(_) => err(StatusCode::BAD_GATEWAY, "quote task failed"),
    }
}

async fn post_purchase(
    State(state): State<AppState>,
    auth: AdminAuth,
    Json(body): Json<PurchaseBody>,
) -> impl IntoResponse {
    let leg = match parse_leg(&body.leg) {
        Ok(leg) => leg,
        Err(message) => return err(StatusCode::UNPROCESSABLE_ENTITY, message),
    };
    if body.order_id.trim().is_empty() {
        return err(StatusCode::UNPROCESSABLE_ENTITY, "order_id is required");
    }
    if let Err(message) = validate_payload(&body.payload) {
        return err(StatusCode::UNPROCESSABLE_ENTITY, message);
    }
    if let Err(message) = require_phones(&body.payload) {
        return err(StatusCode::UNPROCESSABLE_ENTITY, message);
    }
    let live = state.fedex_live.clone();
    let account = live.account.clone().unwrap_or_default();
    if account.trim().is_empty() {
        return err(
            StatusCode::SERVICE_UNAVAILABLE,
            "live fedex buys are not configured",
        );
    }
    let Some((cred_id, cred_secret)) = live.bearer_pair() else {
        return err(
            StatusCode::SERVICE_UNAVAILABLE,
            "live fedex buys are not configured",
        );
    };
    let Some(conn) = open(&state) else {
        return err(
            StatusCode::SERVICE_UNAVAILABLE,
            "metadata store unavailable",
        );
    };
    let now = unix_now();
    let ledger = match load_ledger(&conn, utc_day_start(now)) {
        Ok(ledger) => ledger,
        Err(_) => return err(StatusCode::SERVICE_UNAVAILABLE, "ledger unavailable"),
    };
    let actor = auth.user.user_id.clone();
    let order_id = body.order_id.trim().to_string();
    let leg_name = body.leg.trim().to_string();
    let payload = body.payload;
    let quoted_cents = body.quoted_cents;
    let cfg = live.live_config();
    let order_id_owned = order_id.clone();
    let base = fedex_base(&state);
    let out = tokio::task::spawn_blocking(move || {
        let bearer = fetch_bearer(base, cred_id.as_str(), cred_secret.as_str())?;
        let shipment = LiveShipment::new(base, UreqTransport::default());
        let key = idempotency_key(&order_id_owned, leg);
        let replayed = ledger.find(&key).is_some();
        let request = BuyRequest {
            payload: &payload,
            direction: leg.direction(),
            payment_ref: &account,
        };
        let mut ledger = ledger;
        let bought = shipment.buy_pair(
            BuyParams {
                cfg: &cfg,
                actor: &actor,
                at: now,
                order_id: &order_id_owned,
                leg,
                bearer: &bearer,
                request: &request,
                quoted_cents,
            },
            &mut ledger,
        )?;
        let audits = ledger.audit_trail().to_vec();
        Ok::<_, LiveError>((bought, audits, replayed))
    })
    .await;
    let (bought, audits, replayed) = match out {
        Ok(Ok(v)) => v,
        Ok(Err(e)) => return live_error(e),
        Err(_) => return err(StatusCode::BAD_GATEWAY, "purchase task failed"),
    };
    let Some(stored) = bought.first() else {
        return err(StatusCode::BAD_GATEWAY, "empty buy outcome");
    };
    let key = idempotency_key(&order_id, leg);
    let label_sha = if replayed {
        None
    } else {
        store_label_pdf(&Store::new(&state.store_root), stored)
    };
    let new_row = if replayed {
        None
    } else {
        Some(NewLedgerRow {
            key: &key,
            order_id: &order_id,
            leg: &leg_name,
            stored,
            created_at: now,
            quoted_cents,
            label_sha256: label_sha.as_deref(),
        })
    };
    if persist_outcome(&conn, new_row, &audits).is_err() {
        return err(StatusCode::SERVICE_UNAVAILABLE, "ledger persist failed");
    }
    let has_label = matches!(find_label(&conn, &key), Ok(Some(_)));
    Json(serde_json::json!({
        "tracking_no": stored.tracking_no,
        "replayed": replayed,
        "label_url": has_label.then(|| label_url(&order_id, leg)),
    }))
    .into_response()
}

/// Staff download of one leg's label PDF (`fedex_live_label` row →
/// blob). Same staff session + IP + host guards as every route here.
async fn get_label(
    State(state): State<AppState>,
    _auth: AdminAuth,
    Path((order_id, leg)): Path<(String, String)>,
) -> impl IntoResponse {
    let leg = match parse_leg(&leg) {
        Ok(leg) => leg,
        Err(message) => return err(StatusCode::NOT_FOUND, message),
    };
    let Some(conn) = open(&state) else {
        return err(
            StatusCode::SERVICE_UNAVAILABLE,
            "metadata store unavailable",
        );
    };
    let key = idempotency_key(&order_id, leg);
    let (tracking_no, sha) = match find_label(&conn, &key) {
        Ok(Some(row)) => row,
        Ok(None) => return err(StatusCode::NOT_FOUND, "no label pdf for this leg"),
        Err(_) => return err(StatusCode::SERVICE_UNAVAILABLE, "ledger unavailable"),
    };
    let Ok(bytes) = Store::new(&state.store_root).read_blob(&sha) else {
        return crate::store_unavailable();
    };
    let name: String = tracking_no
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .collect();
    let Ok(disposition) =
        HeaderValue::from_str(&format!("attachment; filename=\"fedex-{name}.pdf\""))
    else {
        return crate::store_unavailable();
    };
    (
        StatusCode::OK,
        [
            (
                header::CONTENT_TYPE,
                HeaderValue::from_static("application/pdf"),
            ),
            (header::CONTENT_DISPOSITION, disposition),
            (header::CACHE_CONTROL, HeaderValue::from_static("no-store")),
        ],
        bytes,
    )
        .into_response()
}

async fn post_void(
    State(state): State<AppState>,
    auth: AdminAuth,
    Json(body): Json<LegBody>,
) -> impl IntoResponse {
    let leg = match parse_leg(&body.leg) {
        Ok(leg) => leg,
        Err(message) => return err(StatusCode::UNPROCESSABLE_ENTITY, message),
    };
    if body.order_id.trim().is_empty() {
        return err(StatusCode::UNPROCESSABLE_ENTITY, "order_id is required");
    }
    let live = state.fedex_live.clone();
    // FedEx's cancel body names the shipper account.
    let account = live.account.clone().unwrap_or_default();
    let Some((cred_id, cred_secret)) = live.bearer_pair().filter(|_| !account.trim().is_empty())
    else {
        return err(
            StatusCode::SERVICE_UNAVAILABLE,
            "live fedex buys are not configured",
        );
    };
    let Some(conn) = open(&state) else {
        return err(
            StatusCode::SERVICE_UNAVAILABLE,
            "metadata store unavailable",
        );
    };
    let now = unix_now();
    let ledger = match load_ledger(&conn, utc_day_start(now)) {
        Ok(ledger) => ledger,
        Err(_) => return err(StatusCode::SERVICE_UNAVAILABLE, "ledger unavailable"),
    };
    let actor = auth.user.user_id.clone();
    let order_id = body.order_id.trim().to_string();
    let cfg = live.live_config();
    let base = fedex_base(&state);
    let out = tokio::task::spawn_blocking(move || {
        let bearer = fetch_bearer(base, cred_id.as_str(), cred_secret.as_str())?;
        let shipment = LiveShipment::new(base, UreqTransport::default());
        let mut ledger = ledger;
        shipment.void(
            VoidParams {
                cfg: &cfg,
                actor: &actor,
                at: now,
                order_id: &order_id,
                leg,
                bearer: &bearer,
                account: &account,
            },
            &mut ledger,
        )?;
        Ok::<_, LiveError>(ledger.audit_trail().to_vec())
    })
    .await;
    let audits = match out {
        Ok(Ok(v)) => v,
        Ok(Err(e)) => return live_error(e),
        Err(_) => return err(StatusCode::BAD_GATEWAY, "void task failed"),
    };
    if persist_outcome(&conn, None, &audits).is_err() {
        return err(StatusCode::SERVICE_UNAVAILABLE, "ledger persist failed");
    }
    Json(serde_json::json!({ "voided": true })).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fedex_base_is_allowlisted() {
        assert_eq!(base_from(None), labels::FEDEX_BASE_PROD);
        assert_eq!(
            base_from(Some("https://apis-sandbox.fedex.com/")),
            labels::FEDEX_BASE_SANDBOX
        );
        assert_eq!(
            base_from(Some("https://evil.test")),
            labels::FEDEX_BASE_PROD,
            "never an arbitrary host"
        );
        assert_eq!(
            base_from(Some("https://apis.fedex.com")),
            labels::FEDEX_BASE_PROD
        );
    }
    use axum::{body::Body, http::Request};
    use tower::ServiceExt as _;

    const CSRF: &str = "t";

    fn state() -> (tempfile::TempDir, AppState) {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::new(tmp.path());
        store.init().unwrap();
        let conn = db::open(&store).unwrap();
        db::migrate(&conn).unwrap();
        auth::migrate_auth_tables(&conn).unwrap();
        for ddl in labels::live::ddl_statements() {
            conn.execute(ddl, []).unwrap();
        }
        auth::create_session(
            &conn,
            "staff-1",
            &auth::NewSession {
                user_id: "staff-1",
                role: auth::Role::Staff,
                customer_ref: Some("cust-1"),
                now: crate::unix_now(),
                ip: "127.0.0.1",
                user_agent: "test",
            },
        )
        .unwrap();
        drop(conn);
        let app_state = AppState {
            spa_index: std::path::PathBuf::from("test-spa/index.html"),
            static_dir: std::path::PathBuf::from("static"),
            store_root: tmp.path().to_path_buf(),
            bus: crate::ws_bus::Bus::new(),
            kek: None,
            admin_mtls_off: true,
            admin_allow_ips: config::loopback_allow_ips(),
            admin_spa_dir: tmp.path().join("admin-spa"),
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
            fedex_live: FedexLiveState::default(),
        };
        (tmp, app_state)
    }

    fn request(method: &str, uri: &str, body: Option<serde_json::Value>) -> Request<Body> {
        let mut builder = Request::builder()
            .uri(uri)
            .method(method)
            .header("host", "admin.andrii.ai")
            .header(
                "cookie",
                format!("{}=staff-1; {}={CSRF}", auth::ADM_COOKIE, auth::CSRF_COOKIE),
            )
            .header(auth::CSRF_HEADER, CSRF)
            .header("origin", "https://admin.andrii.ai");
        if method == "POST" {
            builder = builder.header("content-type", "application/json");
        }
        builder = builder.extension(axum::extract::ConnectInfo(
            "127.0.0.1:443".parse::<std::net::SocketAddr>().unwrap(),
        ));
        builder
            .body(Body::from(body.map(|v| v.to_string()).unwrap_or_default()))
            .unwrap()
    }

    async fn json(res: axum::response::Response) -> (StatusCode, serde_json::Value) {
        let status = res.status();
        let bytes = axum::body::to_bytes(res.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        (status, value)
    }

    fn payload() -> serde_json::Value {
        let address = serde_json::json!({
            "name": "Test Office",
            "street1": "1 Main St",
            "city": "Tumwater",
            "state_province": "WA",
            "postal": "98501",
            "country": "US",
        });
        serde_json::json!({
            "from": address,
            "to": address,
            "parcel": { "preset": "letter_envelope" },
            "service": "ground",
            "pairing_id": "test-1",
        })
    }

    #[tokio::test]
    async fn caps_reports_disabled_by_default() {
        let (_tmp, state) = state();
        let (status, v) = json(
            router(state)
                .oneshot(request("GET", "/apostille/api/v1/fedex-live/caps", None))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(v["enabled"], false);
        assert_eq!(v["bought_today"], 0);
    }

    #[tokio::test]
    async fn quote_is_503_without_credentials() {
        let (_tmp, state) = state();
        let (status, v) = json(
            router(state)
                .oneshot(request(
                    "POST",
                    "/apostille/api/v1/fedex-live/quote",
                    Some(serde_json::json!({ "payload": payload() })),
                ))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(v["error"], "live fedex buys are not configured");
    }

    #[tokio::test]
    async fn purchase_rejects_bad_leg_and_bad_payload() {
        let (_tmp, state) = state();
        let (status, _v) = json(
            router(state.clone())
                .oneshot(request(
                    "POST",
                    "/apostille/api/v1/fedex-live/purchase",
                    Some(serde_json::json!({
                        "order_id": "o-1",
                        "leg": "sideways",
                        "payload": payload(),
                    })),
                ))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);

        let (status, v) = json(
            router(state)
                .oneshot(request(
                    "POST",
                    "/apostille/api/v1/fedex-live/purchase",
                    Some(serde_json::json!({
                        "order_id": "   ",
                        "leg": "outbound",
                        "payload": payload(),
                    })),
                ))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(v["error"], "order_id is required");
    }

    #[tokio::test]
    async fn purchase_requires_both_contact_phones() {
        let (_tmp, state) = state();
        let (status, v) = json(
            router(state.clone())
                .oneshot(request(
                    "POST",
                    "/apostille/api/v1/fedex-live/purchase",
                    Some(serde_json::json!({
                        "order_id": "o-1",
                        "leg": "outbound",
                        "payload": payload(),
                    })),
                ))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(
            v["error"].as_str().unwrap().starts_with("ship-from phone"),
            "{v}"
        );
        // Phones present (with a +1 prefix): the phone gate passes and
        // the missing credentials answer instead.
        let mut with_phones = payload();
        with_phones["from"]["phone"] = serde_json::json!("+1 (360) 555-0100");
        with_phones["to"]["phone"] = serde_json::json!("360-555-0199");
        let (status, _v) = json(
            router(state)
                .oneshot(request(
                    "POST",
                    "/apostille/api/v1/fedex-live/purchase",
                    Some(serde_json::json!({
                        "order_id": "o-1",
                        "leg": "outbound",
                        "payload": with_phones,
                    })),
                ))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn bought_label_pdf_is_stored_and_staff_download_it() {
        let (_tmp, state) = state();
        let conn = open(&state).unwrap();
        let store = Store::new(&state.store_root);
        let pdf = b"%PDF-1.4 fedex documented-shape label".to_vec();
        let stored = StoredLabel {
            tracking_no: "794953535000".to_string(),
            carrier_service: "FEDEX_2_DAY".to_string(),
            quoted_cents: Some(1_250),
            label_pdf: pdf.clone(),
        };
        let sha = store_label_pdf(&store, &stored).expect("blob written");
        let order_id = "ord 7/a";
        let key = idempotency_key(order_id, Leg::Return);
        persist_outcome(
            &conn,
            Some(NewLedgerRow {
                key: &key,
                order_id,
                leg: "return",
                stored: &stored,
                created_at: 1_900_000_000,
                quoted_cents: Some(1_250),
                label_sha256: Some(&sha),
            }),
            &[AuditRow::new(
                1_900_000_000,
                "staff-1",
                "bought",
                &key,
                "794953535000",
            )],
        )
        .unwrap();
        // Ledger, label index and audit landed together.
        let count = |table: &str| -> i64 {
            conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
                .unwrap()
        };
        assert_eq!(
            (
                count("fedex_live_ledger"),
                count("fedex_live_label"),
                count("fedex_live_audit")
            ),
            (1, 1, 1)
        );
        // A replay hydrates the leg without bytes: nothing to re-store.
        let ledger = load_ledger(&conn, 0).unwrap();
        let replay = ledger.find(&key).unwrap();
        assert!(replay.label_pdf.is_empty());
        assert_eq!(store_label_pdf(&store, replay), None);

        let url = label_url(order_id, Leg::Return);
        assert_eq!(url, "/apostille/api/v1/fedex-live/label/ord%207%2Fa/return");
        let res = router(state.clone())
            .oneshot(request("GET", &url, None))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(res.headers()[header::CONTENT_TYPE], "application/pdf");
        assert_eq!(
            res.headers()[header::CONTENT_DISPOSITION],
            "attachment; filename=\"fedex-794953535000.pdf\""
        );
        assert_eq!(res.headers()[header::CACHE_CONTROL], "no-store");
        let body = axum::body::to_bytes(res.into_body(), 1024 * 1024)
            .await
            .unwrap();
        assert_eq!(body.as_ref(), pdf.as_slice());

        // The other leg has no label; an unknown leg name is a 404 too.
        for missing in [
            label_url(order_id, Leg::Outbound),
            "/apostille/api/v1/fedex-live/label/ord-1/sideways".to_string(),
        ] {
            let (status, _v) = json(
                router(state.clone())
                    .oneshot(request("GET", &missing, None))
                    .await
                    .unwrap(),
            )
            .await;
            assert_eq!(status, StatusCode::NOT_FOUND, "{missing}");
        }

        // No staff session: refused before any bytes.
        let anonymous = Request::builder()
            .uri(&url)
            .header("host", "admin.andrii.ai")
            .extension(axum::extract::ConnectInfo(
                "127.0.0.1:443".parse::<std::net::SocketAddr>().unwrap(),
            ))
            .body(Body::empty())
            .unwrap();
        let res = router(state).oneshot(anonymous).await.unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn void_is_503_without_credentials() {
        let (_tmp, state) = state();
        let (status, v) = json(
            router(state)
                .oneshot(request(
                    "POST",
                    "/apostille/api/v1/fedex-live/void",
                    Some(serde_json::json!({ "order_id": "o-1", "leg": "outbound" })),
                ))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(v["error"], "live fedex buys are not configured");
    }
}
