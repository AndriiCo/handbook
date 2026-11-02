//! Staff-only live FedEx endpoints (SA-19 builder wiring).
//!
//! Mounted on the admin listener behind the same IP + session +
//! CSRF guards as the other staff routers. Every mutating call runs
//! the blocking FedEx I/O on `spawn_blocking`; the idempotency
//! ledger is hydrated per request from `fedex_live_ledger` rows and
//! every outcome is appended to `fedex_live_audit`. Live money moves
//! only when `labels.live` is on and the caps/allowlist/credentials
//! rows are set — otherwise 503, fail closed.

use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
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

fn live_error(e: LiveError) -> axum::response::Response {
    match &e {
        LiveError::NotEnabled => err(StatusCode::SERVICE_UNAVAILABLE, e.to_string()),
        LiveError::StaffOnly | LiveError::ZipNotAllowed => {
            err(StatusCode::FORBIDDEN, e.to_string())
        }
        LiveError::DailyCapReached { .. } | LiveError::OverCostCap { .. } => {
            err(StatusCode::UNPROCESSABLE_ENTITY, e.to_string())
        }
        // DuplicateLeg surfaces only when the ledger row is missing
        // mid-flight; the caller retries under a new order id.
        _ => err(StatusCode::BAD_GATEWAY, e.to_string()),
    }
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
}

fn persist_outcome(
    conn: &Connection,
    new_ledger: Option<NewLedgerRow<'_>>,
    audits: &[AuditRow],
) -> Result<(), rusqlite::Error> {
    if let Some(row) = new_ledger {
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

/// Blocking OAuth client-credentials fetch. Secrets stay in this
/// scope; transport errors never include them (labels guarantee).
fn fetch_bearer(client_id: &str, client_secret: &str) -> Result<String, LiveError> {
    crate::oauth::ensure_provider();
    let config = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(Some(std::time::Duration::from_secs(30)))
        .build();
    let agent = ureq::Agent::new_with_config(config);
    let url = oauth_token_url(labels::FEDEX_BASE_PROD);
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

async fn post_quote(
    State(state): State<AppState>,
    _auth: AdminAuth,
    Json(body): Json<QuoteBody>,
) -> impl IntoResponse {
    if let Err(message) = validate_payload(&body.payload) {
        return err(StatusCode::UNPROCESSABLE_ENTITY, message);
    }
    let live = state.fedex_live.clone();
    let Some((cred_id, cred_secret)) = live.bearer_pair() else {
        return err(
            StatusCode::SERVICE_UNAVAILABLE,
            "live fedex buys are not configured",
        );
    };
    let payload = body.payload;
    let cfg = live.live_config();
    let out = tokio::task::spawn_blocking(move || {
        let bearer = fetch_bearer(cred_id.as_str(), cred_secret.as_str())?;
        let shipment = LiveShipment::new(labels::FEDEX_BASE_PROD, UreqTransport::default());
        let request = BuyRequest {
            payload: &payload,
            direction: Direction::Outbound,
            payment_ref: "",
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
    let out = tokio::task::spawn_blocking(move || {
        let bearer = fetch_bearer(cred_id.as_str(), cred_secret.as_str())?;
        let shipment = LiveShipment::new(labels::FEDEX_BASE_PROD, UreqTransport::default());
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
        })
    };
    if persist_outcome(&conn, new_row, &audits).is_err() {
        return err(StatusCode::SERVICE_UNAVAILABLE, "ledger persist failed");
    }
    Json(serde_json::json!({ "tracking_no": stored.tracking_no, "replayed": replayed }))
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
    let cfg = live.live_config();
    let out = tokio::task::spawn_blocking(move || {
        let bearer = fetch_bearer(cred_id.as_str(), cred_secret.as_str())?;
        let shipment = LiveShipment::new(labels::FEDEX_BASE_PROD, UreqTransport::default());
        let mut ledger = ledger;
        shipment.void(
            VoidParams {
                cfg: &cfg,
                actor: &actor,
                at: now,
                order_id: &order_id,
                leg,
                bearer: &bearer,
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
        let (status, v) = json(
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
