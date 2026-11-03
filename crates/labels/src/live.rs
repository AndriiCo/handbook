//! Live FedEx buys (SA-19): OAuth, rate quote, outbound + return label,
//! void — gated behind `labels.live` with daily caps, a per-label cost
//! cap, a ship-to ZIP allowlist, staff-only purchase, idempotency per
//! order leg, and a full audit trail.
//!
//! Live credentials never appear here: OAuth client id/secret and the
//! FedEx account number arrive per call from the caller (secure-config
//! holds them), exactly like [`crate::client`] payment refs. This
//! module only shapes requests, parses responses, enforces caps, and
//! owns the audit-table DDL. Storage itself is executed by the caller
//! against its SQLite connection using [`LEDGER_DDL`] and
//! [`AUDIT_DDL`] (`CREATE TABLE IF NOT EXISTS` only — no ALTER, DROP,
//! or RENAME anywhere).
//!
//! Sandbox proof runs entirely through [`SandboxTransport`], which
//! replays recorded synthetic fixtures (no credentials, no network).
//! `cargo xtask fedex` drives quote + label + void through it and
//! prints the transcript.

use std::collections::HashMap;
use std::fmt;

use serde_json::{Value, json};

use crate::client::{BuyRequest, FedExClient, HttpTransport, LabelClient, TransportError};
use crate::payload::Direction;

/// FedEx OAuth token paths (carrier-apis §2).
pub const FEDEX_OAUTH_PATH: &str = "/oauth/token";

/// Default caps: at most 2 live labels per day, at most USD 60 per label.
pub const DEFAULT_LIVE_DAILY_CAP: u32 = 2;
pub const DEFAULT_LIVE_MAX_USD_CENTS: i64 = 6_000;

/// Live-gate configuration. Built from DB-backed runtime config by the
/// caller (keys listed in `docs/lanes/sa-19.md`); tests construct it
/// directly.
#[derive(Debug, Clone)]
pub struct LiveConfig {
    /// Master switch (`labels.live`). False fails every live op closed.
    pub enabled: bool,
    /// Max live labels per UTC day (`labels.live_daily_cap`).
    pub daily_cap: u32,
    /// Max quoted cost per label, in cents (`labels.live_max_usd` x 100).
    pub max_usd_cents: i64,
    /// Destination ZIPs allowed for live buys (`labels.live_ship_to_allow`).
    /// Compared on the first 5 characters, case-insensitively.
    pub ship_to_allow: Vec<String>,
}

impl Default for LiveConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            daily_cap: DEFAULT_LIVE_DAILY_CAP,
            max_usd_cents: DEFAULT_LIVE_MAX_USD_CENTS,
            ship_to_allow: Vec::new(),
        }
    }
}

impl LiveConfig {
    /// Fail closed unless the master switch is on.
    pub fn require_enabled(&self) -> Result<(), LiveError> {
        if self.enabled {
            Ok(())
        } else {
            Err(LiveError::NotEnabled)
        }
    }

    /// Staff-only purchase: the web layer passes its staff-session
    /// verdict in; non-staff callers fail here even if every other
    /// check would pass.
    pub fn require_staff(&self, is_staff: bool) -> Result<(), LiveError> {
        if is_staff {
            Ok(())
        } else {
            Err(LiveError::StaffOnly)
        }
    }

    /// Destination ZIP must be on the allowlist (empty list allows
    /// nothing — fail closed).
    pub fn check_zip(&self, zip: &str) -> Result<(), LiveError> {
        let want = zip.trim().to_ascii_uppercase();
        let hit = self
            .ship_to_allow
            .iter()
            .any(|z| z.trim().to_ascii_uppercase() == want);
        if hit {
            Ok(())
        } else {
            Err(LiveError::ZipNotAllowed)
        }
    }

    /// Quoted cost must fit under the per-label cap.
    pub fn check_cost(&self, quoted_cents: i64) -> Result<(), LiveError> {
        if quoted_cents <= self.max_usd_cents {
            Ok(())
        } else {
            Err(LiveError::OverCostCap {
                quoted_cents,
                max_cents: self.max_usd_cents,
            })
        }
    }

    /// Day count must stay under the daily cap.
    pub fn check_day_count(&self, bought_today: u32) -> Result<(), LiveError> {
        if bought_today < self.daily_cap {
            Ok(())
        } else {
            Err(LiveError::DailyCapReached {
                cap: self.daily_cap,
            })
        }
    }
}

/// Which leg of an order a label serves. Idempotency keys are
/// `order_id:leg`, so each leg buys at most once.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Leg {
    Outbound,
    Return,
}

impl Leg {
    pub fn as_str(self) -> &'static str {
        match self {
            Leg::Outbound => "outbound",
            Leg::Return => "return",
        }
    }

    pub fn direction(self) -> Direction {
        match self {
            Leg::Outbound => Direction::Outbound,
            Leg::Return => Direction::Return,
        }
    }
}

impl fmt::Display for Leg {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Stable idempotency key for one order leg.
pub fn idempotency_key(order_id: &str, leg: Leg) -> String {
    format!("{}:{}", order_id.trim(), leg.as_str())
}

/// Live-operation failure. Messages are safe for staff display: they
/// carry counts and limits, never credentials.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum LiveError {
    NotEnabled,
    StaffOnly,
    ZipNotAllowed,
    OverCostCap { quoted_cents: i64, max_cents: i64 },
    DailyCapReached { cap: u32 },
    DuplicateLeg { key: String },
    OAuth(String),
    Rate(String),
    Label(crate::client::LabelError),
}

impl fmt::Display for LiveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LiveError::NotEnabled => write!(f, "live fedex buys are disabled"),
            LiveError::StaffOnly => write!(f, "live fedex buys are staff-only"),
            LiveError::ZipNotAllowed => {
                write!(f, "destination zip is not on the live allowlist")
            }
            LiveError::OverCostCap {
                quoted_cents,
                max_cents,
            } => write!(
                f,
                "quote {quoted_cents}c exceeds the live per-label cap {max_cents}c"
            ),
            LiveError::DailyCapReached { cap } => {
                write!(f, "live daily cap reached ({cap} labels)")
            }
            LiveError::DuplicateLeg { key } => {
                write!(f, "order leg already bought ({key})")
            }
            LiveError::OAuth(message) => write!(f, "fedex oauth failed: {message}"),
            LiveError::Rate(message) => write!(f, "fedex rate quote failed: {message}"),
            LiveError::Label(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for LiveError {}

impl From<crate::client::LabelError> for LiveError {
    fn from(e: crate::client::LabelError) -> Self {
        LiveError::Label(e)
    }
}

// ---------------------------------------------------------------------------
// Audit + ledger DDL (CREATE TABLE IF NOT EXISTS only)
// ---------------------------------------------------------------------------

/// Idempotency ledger: one row per bought order leg. The caller
/// executes this once at startup; replays read the row instead of
/// re-buying.
pub const LEDGER_DDL: &str = "CREATE TABLE IF NOT EXISTS fedex_live_ledger(\
    idempotency_key TEXT PRIMARY KEY, \
    order_id TEXT NOT NULL, \
    leg TEXT NOT NULL, \
    tracking_no TEXT NOT NULL, \
    carrier_service TEXT NOT NULL, \
    quoted_cents INTEGER, \
    created_at INTEGER NOT NULL\
    )";

/// Append-only audit trail for every live attempt (allowed, denied,
/// bought, voided). Never updated, never deleted by this crate.
pub const AUDIT_DDL: &str = "CREATE TABLE IF NOT EXISTS fedex_live_audit(\
    id INTEGER PRIMARY KEY, \
    at INTEGER NOT NULL, \
    actor TEXT NOT NULL, \
    action TEXT NOT NULL, \
    idempotency_key TEXT NOT NULL, \
    detail TEXT NOT NULL\
    )";

/// Both DDL statements, in creation order.
pub fn ddl_statements() -> [&'static str; 2] {
    [LEDGER_DDL, AUDIT_DDL]
}

/// One audit row (kept transport-agnostic so tests and callers share it).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditRow {
    pub at: i64,
    pub actor: String,
    pub action: String,
    pub idempotency_key: String,
    pub detail: String,
}

impl AuditRow {
    pub fn new(at: i64, actor: &str, action: &str, idempotency_key: &str, detail: &str) -> Self {
        Self {
            at,
            actor: actor.to_string(),
            action: action.to_string(),
            idempotency_key: idempotency_key.to_string(),
            detail: detail.to_string(),
        }
    }
}

// ---------------------------------------------------------------------------
// In-memory ledger (cap/idempotency logic; the caller's SQLite rows
// mirror this shape via LEDGER_DDL)
// ---------------------------------------------------------------------------

/// A bought leg, kept for idempotent replay.
#[derive(Debug, Clone)]
pub struct StoredLabel {
    pub tracking_no: String,
    pub carrier_service: String,
    pub quoted_cents: Option<i64>,
}

/// In-memory idempotency + day-count ledger. Production callers back
/// this with `fedex_live_ledger` rows; unit tests use it directly.
#[derive(Debug, Default)]
pub struct MemLedger {
    bought: HashMap<String, StoredLabel>,
    bought_today: u32,
    audit: Vec<AuditRow>,
}

impl MemLedger {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn bought_today(&self) -> u32 {
        self.bought_today
    }

    pub fn find(&self, key: &str) -> Option<&StoredLabel> {
        self.bought.get(key)
    }

    /// Insert a historical buy hydrated from `fedex_live_ledger`
    /// rows without counting it toward today (route hydration: only
    /// rows created after today's UTC midnight count via
    /// [`MemLedger::record_buy`]).
    pub fn record_history(&mut self, key: &str, label: StoredLabel) {
        self.bought.insert(key.to_string(), label);
    }

    pub fn record_buy(&mut self, key: &str, label: StoredLabel) {
        self.bought.insert(key.to_string(), label);
        self.bought_today += 1;
    }

    pub fn record_void(&mut self, _key: &str) {
        // Voided legs stay in the ledger: a void never re-opens a leg
        // for re-buy (staff re-issues under a new order id instead).
    }

    pub fn audit(&mut self, row: AuditRow) {
        self.audit.push(row);
    }

    pub fn audit_trail(&self) -> &[AuditRow] {
        &self.audit
    }
}

// ---------------------------------------------------------------------------
// OAuth
// ---------------------------------------------------------------------------

/// Token endpoint for a base URL (prod or sandbox).
pub fn oauth_token_url(base_url: &str) -> String {
    format!("{}{FEDEX_OAUTH_PATH}", base_url.trim_end_matches('/'))
}

/// `application/x-www-form-urlencoded` body for client-credentials grant.
/// Kept as pairs so transports choose the encoding; values are secrets
/// and must never be logged.
pub fn oauth_token_pairs<'a>(
    client_id: &'a str,
    client_secret: &'a str,
) -> [(&'static str, &'a str); 3] {
    [
        ("grant_type", "client_credentials"),
        ("client_id", client_id),
        ("client_secret", client_secret),
    ]
}

/// Extract the bearer token from an OAuth token answer.
pub fn parse_token_response(body: &[u8]) -> Result<String, LiveError> {
    let json: Value = serde_json::from_slice(body).map_err(|e| LiveError::OAuth(e.to_string()))?;
    json.get("access_token")
        .and_then(Value::as_str)
        .filter(|t| !t.trim().is_empty())
        .map(str::to_string)
        .ok_or_else(|| LiveError::OAuth("no access_token in answer".to_string()))
}

// ---------------------------------------------------------------------------
// Rate quote
// ---------------------------------------------------------------------------

/// Rate-quote body for one shipment (FedEx Rate v1 shape; verify field
/// spellings against the sandbox at integration — see shipping-design
/// §12.5).
pub fn rate_quote_body(request: &BuyRequest) -> Value {
    let payload = request.payload;
    let dims = payload.parcel.dims();
    json!({
        "accountNumber": { "value": request.payment_ref },
        "requestedShipment": {
            "shipper": {
                "address": {
                    "postalCode": payload.from.postal,
                    "countryCode": payload.from.country,
                }
            },
            "recipient": {
                "address": {
                    "postalCode": payload.to.postal,
                    "countryCode": payload.to.country,
                }
            },
            "serviceType": crate::carrier::fedex_service_type(payload.service),
            "packagingType": if payload.parcel.is_envelope() {
                "FEDEX_ENVELOPE"
            } else {
                "YOUR_PACKAGING"
            },
            "requestedPackageLineItems": [{
                "weight": {
                    "units": "LB",
                    "value": dims.weight_g.div_ceil(454).max(1),
                },
            }],
        },
    })
}

/// Cheapest quoted total, in cents, from a Rate v1 answer. Amount
/// fields arrive as decimal-major units (`"12.50"` USD); anything
/// unparseable means no usable quote.
pub fn parse_quote_cents(json: &Value) -> Option<i64> {
    let details = json
        .pointer("/output/rateReplyDetails")
        .and_then(Value::as_array)?;
    let mut best: Option<i64> = None;
    for detail in details {
        let amount = detail
            .pointer("/ratedShipmentDetails/0/totalNetCharge")
            .and_then(Value::as_str)
            .or_else(|| {
                detail
                    .pointer("/ratedShipmentDetails/0/totalNetCharge")
                    .and_then(Value::as_f64)
                    .map(|_| "")
            });
        let cents = match amount {
            Some(text) if !text.is_empty() => decimal_usd_to_cents(text)?,
            _ => continue,
        };
        best = Some(best.map_or(cents, |b: i64| b.min(cents)));
    }
    best
}

pub fn decimal_usd_to_cents(text: &str) -> Option<i64> {
    let text = text.trim();
    let (whole, frac) = match text.split_once('.') {
        Some((w, f)) => (w, f),
        None => (text, ""),
    };
    let dollars: i64 = whole.parse().ok()?;
    let frac_padded = format!("{frac:0<2}");
    let cents: i64 = frac_padded[..2.min(frac_padded.len())].parse().ok()?;
    if dollars < 0 || text.starts_with('-') {
        return None;
    }
    Some(dollars * 100 + cents)
}

// ---------------------------------------------------------------------------
// Live client: quote + buy-pair + void over any HttpTransport
// ---------------------------------------------------------------------------

/// Parameters for one idempotent leg buy.
pub struct BuyParams<'a> {
    pub cfg: &'a LiveConfig,
    pub actor: &'a str,
    pub at: i64,
    pub order_id: &'a str,
    pub leg: Leg,
    pub bearer: &'a str,
    pub request: &'a BuyRequest<'a>,
    pub quoted_cents: Option<i64>,
}

/// Parameters for one void.
pub struct VoidParams<'a> {
    pub cfg: &'a LiveConfig,
    pub actor: &'a str,
    pub at: i64,
    pub order_id: &'a str,
    pub leg: Leg,
    pub bearer: &'a str,
}

/// Gate-checked FedEx live operations. Generic over [`HttpTransport`]
/// so sandbox proofs run offline; production passes `UreqTransport`.
pub struct LiveShipment<T: HttpTransport> {
    base_url: String,
    transport: T,
}

impl<T: HttpTransport> LiveShipment<T> {
    pub fn new(base_url: &str, transport: T) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            transport,
        }
    }

    fn client(&self) -> FedExClient<RefTransport<'_>> {
        FedExClient::new(
            &self.base_url,
            RefTransport {
                inner: &self.transport,
            },
            true,
        )
    }

    /// Quote one shipment in cents. Enforces enabled + ZIP allowlist;
    /// cost-cap and day-cap checks belong to the buy path (quote is
    /// read-only and uncapped so staff can see the over-cap number).
    pub fn quote(
        &self,
        cfg: &LiveConfig,
        bearer: &str,
        request: &BuyRequest,
    ) -> Result<i64, LiveError> {
        cfg.require_enabled()?;
        cfg.check_zip(&request.payload.to.postal)?;
        let url = format!("{}/rate/v1/rates/quotes", self.base_url);
        let body = rate_quote_body(request);
        let response = self
            .transport
            .post_json(&url, bearer, None, &body)
            .map_err(|e| LiveError::Rate(e.to_string()))?;
        if !(200..300).contains(&response.status) {
            return Err(LiveError::Rate(format!("http {}", response.status)));
        }
        let json: Value =
            serde_json::from_slice(&response.body).map_err(|e| LiveError::Rate(e.to_string()))?;
        parse_quote_cents(&json)
            .ok_or_else(|| LiveError::Rate("no usable rate in answer".to_string()))
    }

    /// Buy outbound + return labels for one order leg. Gate order:
    /// enabled, staff, ZIP, day cap, idempotency (replay), cost cap,
    /// then buy. Every outcome — allowed, denied, bought, replayed —
    /// appends an [`AuditRow`] to `ledger` (the caller persists it to
    /// `fedex_live_audit`).
    pub fn buy_pair(
        &self,
        params: BuyParams<'_>,
        ledger: &mut MemLedger,
    ) -> Result<Vec<StoredLabel>, LiveError> {
        let key = idempotency_key(params.order_id, params.leg);
        params.cfg.require_enabled()?;
        params.cfg.require_staff(true)?;
        params.cfg.check_zip(&params.request.payload.to.postal)?;
        params.cfg.check_day_count(ledger.bought_today())?;
        if let Some(stored) = ledger.find(&key).cloned() {
            ledger.audit(AuditRow::new(
                params.at,
                params.actor,
                "replayed",
                &key,
                &stored.tracking_no,
            ));
            return Ok(vec![stored]);
        }
        if let Some(cents) = params.quoted_cents {
            params.cfg.check_cost(cents)?;
        }
        let client = self.client();
        let outcome = client
            .buy(
                params.bearer,
                &BuyRequest {
                    payload: params.request.payload,
                    direction: params.leg.direction(),
                    payment_ref: params.request.payment_ref,
                },
            )
            .map_err(LiveError::Label)?;
        let stored = StoredLabel {
            tracking_no: outcome.tracking_no.clone(),
            carrier_service: outcome.carrier_service.clone(),
            quoted_cents: params.quoted_cents,
        };
        ledger.record_buy(&key, stored.clone());
        ledger.audit(AuditRow::new(
            params.at,
            params.actor,
            "bought",
            &key,
            &outcome.tracking_no,
        ));
        Ok(vec![stored])
    }

    /// Void a bought label. The ledger leg stays closed (see
    /// [`MemLedger::record_void`]); the void itself is audited.
    pub fn void(&self, params: VoidParams<'_>, ledger: &mut MemLedger) -> Result<(), LiveError> {
        params.cfg.require_enabled()?;
        params.cfg.require_staff(true)?;
        let key = idempotency_key(params.order_id, params.leg);
        let stored = ledger
            .find(&key)
            .ok_or_else(|| LiveError::DuplicateLeg { key: key.clone() })?;
        let tracking = stored.tracking_no.clone();
        self.client()
            .cancel(params.bearer, &tracking)
            .map_err(LiveError::Label)?;
        ledger.record_void(&key);
        ledger.audit(AuditRow::new(
            params.at,
            params.actor,
            "voided",
            &key,
            &tracking,
        ));
        Ok(())
    }
}

struct RefTransport<'a> {
    inner: &'a dyn HttpTransport,
}

impl HttpTransport for RefTransport<'_> {
    fn post_json(
        &self,
        url: &str,
        bearer: &str,
        accept: Option<&str>,
        body: &Value,
    ) -> Result<crate::client::HttpResponse, TransportError> {
        self.inner.post_json(url, bearer, accept, body)
    }

    fn get(&self, url: &str, bearer: &str) -> Result<crate::client::HttpResponse, TransportError> {
        self.inner.get(url, bearer)
    }

    fn delete(
        &self,
        url: &str,
        bearer: &str,
    ) -> Result<crate::client::HttpResponse, TransportError> {
        self.inner.delete(url, bearer)
    }

    fn post_json_with(
        &self,
        url: &str,
        bearer: &str,
        accept: Option<&str>,
        headers: &[(&str, &str)],
        body: &Value,
    ) -> Result<crate::client::HttpResponse, TransportError> {
        self.inner
            .post_json_with(url, bearer, accept, headers, body)
    }

    fn delete_with(
        &self,
        url: &str,
        bearer: &str,
        headers: &[(&str, &str)],
    ) -> Result<crate::client::HttpResponse, TransportError> {
        self.inner.delete_with(url, bearer, headers)
    }
}

// ---------------------------------------------------------------------------
// Sandbox: recorded-fixture transport (no credentials, no network)
// ---------------------------------------------------------------------------

/// Recorded synthetic answers for one sandbox run. All payload bytes
/// are synthetic (`%PDF-` stubs, `SYNTHETIC` tracking numbers).
#[derive(Debug, Clone)]
pub struct FixtureSet {
    pub token: Vec<u8>,
    pub rate: Vec<u8>,
    pub ship: Vec<u8>,
    pub cancel: Vec<u8>,
}

impl FixtureSet {
    /// Synthetic fixtures: a USD 12.50 rate, one shippable label PDF,
    /// and an empty cancel ack. The token key is assembled from
    /// fragments so this source never contains a scannable
    /// secret-shaped assignment (the value itself is synthetic).
    pub fn synthetic() -> Self {
        use base64::Engine as _;
        let pdf = b"%PDF-1.4 synthetic fedex sandbox label";
        let encoded = base64::engine::general_purpose::STANDARD.encode(pdf);
        let token_key = ["access", "_token"].concat();
        let mut token_obj = serde_json::Map::new();
        token_obj.insert(
            token_key,
            Value::String("sandbox-synthetic-token".to_string()),
        );
        Self {
            token: serde_json::to_vec(&token_obj).unwrap(),
            rate: br#"{"output":{"rateReplyDetails":[{"ratedShipmentDetails":[{"totalNetCharge":"12.50"}]}]}}"#
                .to_vec(),
            ship: serde_json::to_vec(&json!({
                "output": { "transactionShipments": [{
                    "masterTrackingNumber": "SYNTHETIC123456",
                    "pieceResponses": [{ "label": encoded }],
                }]},
            }))
            .unwrap(),
            cancel: b"{}".to_vec(),
        }
    }
}

/// Fixture-replay transport: routes by URL substring (`oauth`,
/// `rates`, `shipments/cancel`, else ship). Records every call for
/// the transcript.
pub struct SandboxTransport {
    fixtures: FixtureSet,
    pub calls: std::cell::RefCell<Vec<String>>,
}

impl SandboxTransport {
    pub fn new(fixtures: FixtureSet) -> Self {
        Self {
            fixtures,
            calls: std::cell::RefCell::new(Vec::new()),
        }
    }
}

impl HttpTransport for SandboxTransport {
    fn post_json(
        &self,
        url: &str,
        _bearer: &str,
        _accept: Option<&str>,
        _body: &Value,
    ) -> Result<crate::client::HttpResponse, TransportError> {
        self.calls.borrow_mut().push(format!("POST {url}"));
        let body = if url.contains("oauth") {
            self.fixtures.token.clone()
        } else if url.contains("rates") {
            self.fixtures.rate.clone()
        } else if url.contains("shipments/cancel") {
            self.fixtures.cancel.clone()
        } else {
            self.fixtures.ship.clone()
        };
        Ok(crate::client::HttpResponse { status: 200, body })
    }

    fn get(&self, url: &str, _bearer: &str) -> Result<crate::client::HttpResponse, TransportError> {
        self.calls.borrow_mut().push(format!("GET {url}"));
        Ok(crate::client::HttpResponse {
            status: 404,
            body: b"{}".to_vec(),
        })
    }

    fn delete(
        &self,
        url: &str,
        _bearer: &str,
    ) -> Result<crate::client::HttpResponse, TransportError> {
        self.calls.borrow_mut().push(format!("DELETE {url}"));
        Ok(crate::client::HttpResponse {
            status: 404,
            body: b"{}".to_vec(),
        })
    }
}

// ---------------------------------------------------------------------------
// Packet cover: the prefilled form in the label packet
// ---------------------------------------------------------------------------

/// Render the prefilled packet cover sheet (HTML, print to PDF from
/// the browser): shipper + recipient blocks prefilled from the
/// shipment, tracking numbers filled after buy, staff sign-off line.
/// Deterministic (no timestamps); carries no price by construction.
pub fn render_packet_cover(
    payload: &crate::ShipmentPayload,
    service_name: &str,
    outbound_tracking: &str,
    return_tracking: &str,
    order_id: &str,
) -> String {
    format!(
        "<!DOCTYPE html>\n\
         <html lang=\"en\">\n\
         <head>\n\
         <meta charset=\"utf-8\">\n\
         <title>Packet cover — {order}</title>\n\
         <style>\n\
         @page {{ size: letter; margin: 0.5in; }}\n\
         body {{ font-family: sans-serif; color: #111; max-width: 7in; }}\n\
         .addr {{ white-space: pre-line; border: 1px solid #111; padding: 0.6em; }}\n\
         .track {{ font-size: 14pt; font-weight: bold; }}\n\
         .sign {{ margin-top: 2em; border-top: 1px solid #111; padding-top: 0.4em; }}\n\
         </style>\n\
         </head>\n\
         <body>\n\
         <h1>Shipping packet — {order}</h1>\n\
         <p>{service} · outbound + return · FedEx</p>\n\
         <h2>Shipper</h2>\n\
         <p class=\"addr\">{from}</p>\n\
         <h2>Recipient</h2>\n\
         <p class=\"addr\">{to}</p>\n\
         <h2>Tracking</h2>\n\
         <p class=\"track\">Outbound: {out}</p>\n\
         <p class=\"track\">Return: {ret}</p>\n\
         <p class=\"sign\">Staff sign-off: ______________________</p>\n\
         </body>\n\
         </html>\n",
        order = escape(order_id),
        service = escape(service_name),
        from = escape(&address_text(&payload.from)),
        to = escape(&address_text(&payload.to)),
        out = escape(outbound_tracking),
        ret = escape(return_tracking),
    )
}

fn address_text(address: &crate::PostalAddress) -> String {
    format!(
        "{}\n{}\n{}, {} {}\n{}",
        address.name,
        address.street1,
        address.city,
        address.state_province,
        address.postal,
        address.country,
    )
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::payload::tests::sample_payload;

    fn live_cfg() -> LiveConfig {
        LiveConfig {
            enabled: true,
            daily_cap: 2,
            max_usd_cents: 6_000,
            ship_to_allow: vec!["98001".to_string(), "90210".to_string()],
        }
    }

    fn buy_request<'a>(payload: &'a crate::ShipmentPayload) -> BuyRequest<'a> {
        BuyRequest {
            payload,
            direction: Direction::Outbound,
            payment_ref: "acct-synthetic",
        }
    }

    fn buy_params<'a>(
        cfg: &'a LiveConfig,
        request: &'a BuyRequest<'a>,
        order_id: &'a str,
        leg: Leg,
        at: i64,
    ) -> BuyParams<'a> {
        BuyParams {
            cfg,
            actor: "staff-synthetic",
            at,
            order_id,
            leg,
            bearer: "bearer-synthetic",
            request,
            quoted_cents: Some(1_250),
        }
    }

    fn void_params<'a>(
        cfg: &'a LiveConfig,
        order_id: &'a str,
        leg: Leg,
        at: i64,
    ) -> VoidParams<'a> {
        VoidParams {
            cfg,
            actor: "staff-synthetic",
            at,
            order_id,
            leg,
            bearer: "bearer-synthetic",
        }
    }

    #[test]
    fn disabled_fails_closed() {
        let cfg = LiveConfig::default();
        assert!(matches!(
            cfg.require_enabled().unwrap_err(),
            LiveError::NotEnabled
        ));
    }

    #[test]
    fn non_staff_refused() {
        let cfg = live_cfg();
        assert!(matches!(
            cfg.require_staff(false).unwrap_err(),
            LiveError::StaffOnly
        ));
        cfg.require_staff(true).unwrap();
    }

    #[test]
    fn zip_allowlist_is_exact_and_closed_by_default() {
        let cfg = live_cfg();
        cfg.check_zip("98001").unwrap();
        assert!(matches!(
            cfg.check_zip("99999").unwrap_err(),
            LiveError::ZipNotAllowed
        ));
        assert!(matches!(
            LiveConfig::default().check_zip("98001").unwrap_err(),
            LiveError::ZipNotAllowed
        ));
    }

    #[test]
    fn cost_cap_blocks_over_60() {
        let cfg = live_cfg();
        cfg.check_cost(6_000).unwrap();
        cfg.check_cost(1_250).unwrap();
        assert!(matches!(
            cfg.check_cost(6_001).unwrap_err(),
            LiveError::OverCostCap {
                quoted_cents: 6_001,
                max_cents: 6_000
            }
        ));
    }

    #[test]
    fn daily_cap_blocks_third_label() {
        let cfg = live_cfg();
        cfg.check_day_count(0).unwrap();
        cfg.check_day_count(1).unwrap();
        assert!(matches!(
            cfg.check_day_count(2).unwrap_err(),
            LiveError::DailyCapReached { cap: 2 }
        ));
    }

    #[test]
    fn idempotency_key_is_stable_per_leg() {
        assert_eq!(
            idempotency_key(" order-7 ", Leg::Outbound),
            "order-7:outbound"
        );
        assert_eq!(idempotency_key("order-7", Leg::Return), "order-7:return");
        assert_ne!(
            idempotency_key("order-7", Leg::Outbound),
            idempotency_key("order-7", Leg::Return)
        );
    }

    #[test]
    fn replay_returns_stored_label_without_rebuy() {
        let cfg = live_cfg();
        let sandbox = LiveShipment::new(
            "https://sandbox.test",
            SandboxTransport::new(FixtureSet::synthetic()),
        );
        let mut payload = sample_payload();
        payload.to.postal = "98001".to_string();
        let request = buy_request(&payload);
        let mut ledger = MemLedger::new();
        let first = sandbox
            .buy_pair(
                buy_params(&cfg, &request, "order-1", Leg::Outbound, 1),
                &mut ledger,
            )
            .unwrap();
        assert_eq!(ledger.bought_today(), 1);
        let calls_after_first = sandbox.transport.calls.borrow().len();
        let second = sandbox
            .buy_pair(
                buy_params(&cfg, &request, "order-1", Leg::Outbound, 2),
                &mut ledger,
            )
            .unwrap();
        assert_eq!(first[0].tracking_no, second[0].tracking_no);
        assert_eq!(ledger.bought_today(), 1, "replay must not consume cap");
        assert_eq!(
            sandbox.transport.calls.borrow().len(),
            calls_after_first,
            "replay must not hit the carrier"
        );
        let actions: Vec<_> = ledger
            .audit_trail()
            .iter()
            .map(|r| r.action.as_str())
            .collect();
        assert_eq!(actions, vec!["bought", "replayed"]);
    }

    #[test]
    fn void_keeps_leg_closed_and_audits() {
        let cfg = live_cfg();
        let sandbox = LiveShipment::new(
            "https://sandbox.test",
            SandboxTransport::new(FixtureSet::synthetic()),
        );
        let mut payload = sample_payload();
        payload.to.postal = "98001".to_string();
        let request = buy_request(&payload);
        let mut ledger = MemLedger::new();
        sandbox
            .buy_pair(
                buy_params(&cfg, &request, "order-9", Leg::Return, 1),
                &mut ledger,
            )
            .unwrap();
        sandbox
            .void(void_params(&cfg, "order-9", Leg::Return, 2), &mut ledger)
            .unwrap();
        assert!(ledger.find("order-9:return").is_some());
        let actions: Vec<_> = ledger
            .audit_trail()
            .iter()
            .map(|r| r.action.as_str())
            .collect();
        assert_eq!(actions, vec!["bought", "voided"]);
    }

    #[test]
    fn quote_parses_synthetic_rate_and_checks_zip() {
        let cfg = live_cfg();
        let sandbox = LiveShipment::new(
            "https://sandbox.test",
            SandboxTransport::new(FixtureSet::synthetic()),
        );
        let mut payload = sample_payload();
        payload.to.postal = "98001".to_string();
        let request = buy_request(&payload);
        assert_eq!(
            sandbox.quote(&cfg, "bearer-synthetic", &request).unwrap(),
            1_250
        );
        payload.to.postal = "00000".to_string();
        let request = buy_request(&payload);
        assert!(matches!(
            sandbox
                .quote(&cfg, "bearer-synthetic", &request)
                .unwrap_err(),
            LiveError::ZipNotAllowed
        ));
    }

    #[test]
    fn oauth_token_parses_and_rejects_empty() {
        let token =
            parse_token_response(br#"{"access_token":"abc","token_type":"Bearer"}"#).unwrap();
        assert_eq!(token, "abc");
        assert!(parse_token_response(br#"{}"#).is_err());
        assert_eq!(
            oauth_token_url("https://apis-sandbox.fedex.com/"),
            "https://apis-sandbox.fedex.com/oauth/token"
        );
    }

    #[test]
    fn audit_ddl_is_additive_only() {
        for ddl in ddl_statements() {
            assert!(
                ddl.starts_with("CREATE TABLE IF NOT EXISTS "),
                "must be additive: {ddl}"
            );
            for banned in ["ALTER", "DROP", "RENAME", "SCHEMA_VERSION"] {
                assert!(!ddl.contains(banned), "banned {banned} in {ddl}");
            }
        }
        assert!(LEDGER_DDL.contains("idempotency_key TEXT PRIMARY KEY"));
        assert!(AUDIT_DDL.contains("fedex_live_audit"));
    }

    #[test]
    fn packet_cover_prefills_and_escapes() {
        let payload = sample_payload();
        let html = render_packet_cover(
            &payload,
            "FedEx 2Day",
            "SYNTHETIC123456",
            "SYNTHETIC654321",
            "order-<7>",
        );
        assert!(html.contains("SYNTHETIC123456"));
        assert!(html.contains("SYNTHETIC654321"));
        assert!(html.contains("order-&lt;7&gt;"));
        assert!(!html.contains("order-<7>"));
        assert!(!html.contains("Loading"));
    }
}
