//! Customer-paid carrier labels: the label tiers of `/order`.
//!
//! At quote time the carrier prices both legs (outbound to the office,
//! return to the customer) and `orders.fee_label_cents` is added. After
//! payment a worker buys each leg exactly once through the
//! `label_purchase` ledger (p2.sqlite), stores the PDFs for the
//! customer's cabinet and announces them on `billing:{customer_ref}`.
//!
//! Money rules:
//! - `labels.customer_live` (off by default) gates every customer buy,
//!   separate from the staff switch `labels.live`. Caps:
//!   `labels.customer_daily_cap` (label buys per UTC day, default 10)
//!   and `labels.customer_max_usd` (per label, default 60).
//! - Destinations are limited to this order's own two addresses (the
//!   office it was created for and the customer's return address).
//! - A Stripe test key only buys against the carrier's test endpoint
//!   and a live key only against production (mode coherence).
//! - The ledger row is committed `in_flight` before the carrier call.
//!   A carrier refusal of the shipment itself (HTTP 400/422) voids any
//!   leg already bought and refunds the order in full. Anything that
//!   might have bought (timeout, 5xx, an unreadable answer) is
//!   `unknown`: the order goes to staff and is never retried here.

use std::sync::LazyLock;

use handbook_store::{db, store::Store};
use labels::live::LiveConfig;
use labels::payload::{Direction, Parcel, PostalAddress, ServiceClass, ShipmentPayload};
use labels::{
    BuyOutcome, BuyRequest, FedExClient, HttpResponse, HttpTransport, LabelClient, LabelError,
    TransportError, UreqTransport,
};
use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{AppState, unix_now};

/// Default `labels.customer_daily_cap` (label buys per UTC day).
pub const DEFAULT_DAILY_CAP: i64 = 10;
/// Default `labels.customer_max_usd`, in cents.
pub const DEFAULT_MAX_CENTS: i64 = 6_000;

/// Wakes the buyer as soon as an order settles (it also ticks every
/// minute, which covers a restart between settle and buy).
pub(crate) static POKE: LazyLock<tokio::sync::Notify> = LazyLock::new(tokio::sync::Notify::new);

/// Boot DDL on p2.sqlite: `CREATE TABLE IF NOT EXISTS` only.
pub fn migrate_p2_tables(p2: &Connection) -> rusqlite::Result<()> {
    p2.execute_batch(
        "CREATE TABLE IF NOT EXISTS label_purchase (
            id INTEGER PRIMARY KEY,
            order_id TEXT NOT NULL,
            leg TEXT NOT NULL CHECK (leg IN ('outbound', 'return', 'kit_to_customer')),
            carrier TEXT NOT NULL CHECK (carrier IN ('fedex', 'usps')),
            status TEXT NOT NULL
                CHECK (status IN ('in_flight', 'bought', 'failed', 'unknown', 'voided')),
            livemode INTEGER NOT NULL DEFAULT 0,
            tracking_no TEXT NOT NULL DEFAULT '',
            cost_cents INTEGER,
            label_id INTEGER,
            error TEXT NOT NULL DEFAULT '',
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL,
            UNIQUE (order_id, leg)
         );
         CREATE INDEX IF NOT EXISTS idx_label_purchase_day ON label_purchase(created_at);",
    )
}

// ------------------------------------------------------------ config

/// Customer label switch and caps, read through the 30 s config cache.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Caps {
    pub live: bool,
    pub daily_cap: i64,
    pub max_cents: i64,
}

impl Default for Caps {
    fn default() -> Self {
        Self {
            live: false,
            daily_cap: DEFAULT_DAILY_CAP,
            max_cents: DEFAULT_MAX_CENTS,
        }
    }
}

/// Parse the caps; bad values keep the default (0 buys stops all buys).
pub fn parse_caps(live: Option<bool>, daily_cap: Option<&str>, max_usd: Option<&str>) -> Caps {
    Caps {
        live: live.unwrap_or(false),
        daily_cap: daily_cap
            .and_then(|v| v.trim().parse::<i64>().ok())
            .filter(|n| (0..=1_000).contains(n))
            .unwrap_or(DEFAULT_DAILY_CAP),
        max_cents: max_usd
            .and_then(labels::live::decimal_usd_to_cents)
            .filter(|c| (100..=1_000_000).contains(c))
            .unwrap_or(DEFAULT_MAX_CENTS),
    }
}

pub fn caps(state: &AppState, meta: &Connection) -> Caps {
    let Some(kek) = state.kek.as_deref() else {
        return Caps::default();
    };
    let cache = &state.config_cache;
    let text = |key| cache.text(meta, kek, key).ok().flatten();
    parse_caps(
        cache
            .bool(meta, kek, config::keys::LABELS_CUSTOMER_LIVE)
            .ok()
            .flatten(),
        text(config::keys::LABELS_CUSTOMER_DAILY_CAP).as_deref(),
        text(config::keys::LABELS_CUSTOMER_MAX_USD).as_deref(),
    )
}

/// `orders.fee_label_cents`: service fee per paid label order (default
/// 0, at most $1,000).
pub fn fee_cents(state: &AppState, meta: &Connection) -> i64 {
    state
        .kek
        .as_deref()
        .and_then(|kek| {
            state
                .config_cache
                .text(meta, kek, config::keys::ORDERS_FEE_LABEL_CENTS)
                .ok()
                .flatten()
        })
        .and_then(|v| v.trim().parse::<i64>().ok())
        .filter(|c| (0..=100_000).contains(c))
        .unwrap_or(0)
}

/// FedEx API credentials, read per call (secrets are never cached).
struct FedexCreds {
    base: &'static str,
    client_id: String,
    client_secret: String,
    account: String,
}

fn fedex_creds(state: &AppState, meta: &Connection) -> Option<FedexCreds> {
    let kek = state.kek.as_deref()?;
    let secret = |key| {
        config::get_secret_for_job(meta, kek, key)
            .ok()
            .flatten()
            .and_then(|b| String::from_utf8(b).ok())
            .filter(|s| !s.trim().is_empty())
    };
    let account = state
        .config_cache
        .text(meta, kek, config::keys::LABELS_FEDEX_ACCOUNT)
        .ok()
        .flatten()
        .map(|a| a.trim().to_string())
        .filter(|a| !a.is_empty())?;
    Some(FedexCreds {
        base: crate::fedex_live::fedex_base(state),
        client_id: secret(config::keys::LABELS_FEDEX_CLIENT_ID)?,
        client_secret: secret(config::keys::LABELS_FEDEX_CLIENT_SECRET)?,
        account,
    })
}

/// Test money only buys test postage, live money only live postage.
pub fn fedex_mode_ok(base: &str, stripe_live: bool) -> bool {
    (base == labels::FEDEX_BASE_PROD) == stripe_live
}

/// True when the customer tier `kind` can be quoted and bought now:
/// switch on, carrier credentials present and the mode coherent.
pub fn ready(state: &AppState, meta: &Connection, kind: &str, stripe_live: bool) -> bool {
    if !caps(state, meta).live {
        return false;
    }
    match kind {
        "label_fedex" => {
            fedex_creds(state, meta).is_some_and(|c| fedex_mode_ok(c.base, stripe_live))
        }
        "label_usps" => crate::usps::customer_ready(state, meta, stripe_live),
        _ => false,
    }
}

/// [`ready`] with its own metadata connection (for `settle`).
pub fn ready_now(state: &AppState, kind: &str, stripe_live: bool) -> bool {
    db::open(&Store::new(&state.store_root))
        .map(|meta| ready(state, &meta, kind, stripe_live))
        .unwrap_or(false)
}

/// Customer label buys counted against today's cap (anything that may
/// have bought; a definite failure does not count).
pub fn buys_today(p2: &Connection, now: i64) -> i64 {
    p2.query_row(
        "SELECT COUNT(*) FROM label_purchase
         WHERE created_at >= ?1 AND status IN ('in_flight', 'bought', 'unknown')",
        [crate::telemetry::utc_day_start(now)],
        |r| r.get(0),
    )
    .unwrap_or(i64::MAX)
}

// ------------------------------------------------------------ carrier link

/// One carrier for one order: quote and buy over one fresh bearer.
pub(crate) trait LabelLink {
    fn carrier(&self) -> &'static str;
    /// Quote one leg in cents (enabled + destination checks inside).
    fn quote(&self, cfg: &LiveConfig, request: &BuyRequest) -> Result<i64, String>;
    fn buy(&self, request: &BuyRequest) -> Result<BuyOutcome, LabelError>;
    fn cancel(&self, tracking_no: &str) -> Result<(), LabelError>;
    /// Server-side money reference (FedEx account, USPS payment token).
    fn payment_ref(&self) -> &str;
}

/// Borrowing adapter: the carrier clients take their transport by value.
pub(crate) struct By<'a>(pub(crate) &'a dyn HttpTransport);

impl HttpTransport for By<'_> {
    fn post_json(
        &self,
        url: &str,
        bearer: &str,
        accept: Option<&str>,
        body: &Value,
    ) -> Result<HttpResponse, TransportError> {
        self.0.post_json(url, bearer, accept, body)
    }

    fn get(&self, url: &str, bearer: &str) -> Result<HttpResponse, TransportError> {
        self.0.get(url, bearer)
    }

    fn delete(&self, url: &str, bearer: &str) -> Result<HttpResponse, TransportError> {
        self.0.delete(url, bearer)
    }
}

pub(crate) struct FedexLink<T: HttpTransport> {
    pub(crate) base: String,
    pub(crate) bearer: String,
    pub(crate) account: String,
    pub(crate) transport: T,
}

impl<T: HttpTransport> LabelLink for FedexLink<T> {
    fn carrier(&self) -> &'static str {
        "fedex"
    }

    fn quote(&self, cfg: &LiveConfig, request: &BuyRequest) -> Result<i64, String> {
        cfg.require_enabled().map_err(|e| e.to_string())?;
        cfg.check_zip(&request.payload.to.postal)
            .map_err(|e| e.to_string())?;
        let url = format!("{}/rate/v1/rates/quotes", self.base);
        let body = labels::live::rate_quote_body(request);
        let response = self
            .transport
            .post_json(&url, &self.bearer, None, &body)
            .map_err(|e| e.to_string())?;
        if !(200..300).contains(&response.status) {
            return Err(format!("fedex rate quote: http {}", response.status));
        }
        let json: Value = serde_json::from_slice(&response.body).map_err(|e| e.to_string())?;
        labels::live::parse_quote_cents(&json)
            .ok_or_else(|| "fedex rate quote: no usable rate".to_string())
    }

    fn buy(&self, request: &BuyRequest) -> Result<BuyOutcome, LabelError> {
        FedExClient::new(&self.base, By(&self.transport), true).buy(&self.bearer, request)
    }

    fn cancel(&self, tracking_no: &str) -> Result<(), LabelError> {
        FedExClient::new(&self.base, By(&self.transport), true).cancel(&self.bearer, tracking_no)
    }

    fn payment_ref(&self) -> &str {
        &self.account
    }
}

/// The production link for `kind`, with a fresh bearer (blocking).
fn connect(
    state: &AppState,
    meta: &Connection,
    kind: &str,
    stripe_live: bool,
) -> Result<Box<dyn LabelLink>, String> {
    match kind {
        "label_fedex" => {
            let creds = fedex_creds(state, meta).ok_or("fedex credentials are not set")?;
            if !fedex_mode_ok(creds.base, stripe_live) {
                return Err("fedex endpoint does not match the payment mode".to_string());
            }
            let bearer =
                crate::fedex_live::fetch_bearer(creds.base, &creds.client_id, &creds.client_secret)
                    .map_err(|e| e.to_string())?;
            Ok(Box::new(FedexLink {
                base: creds.base.to_string(),
                bearer,
                account: creds.account,
                transport: UreqTransport::new(),
            }))
        }
        "label_usps" => crate::usps::customer_link(state, meta, stripe_live),
        _ => Err("not a carrier label order".to_string()),
    }
}

// ------------------------------------------------------------ legs + quotes

/// One carrier leg of an order, as the carrier sees it.
pub(crate) struct OrderLeg {
    pub(crate) role: String,
    pub(crate) shipment_id: i64,
    pub(crate) payload: ShipmentPayload,
}

impl OrderLeg {
    fn direction(&self) -> Direction {
        if self.role == "return" {
            Direction::Return
        } else {
            Direction::Outbound
        }
    }
}

/// Address JSON as `create_order` stores it; a missing phone takes the
/// fallback (the operator's ship-from phone), which carriers require.
pub(crate) fn party(json: &str, fallback_phone: Option<&str>) -> PostalAddress {
    let v: Value = serde_json::from_str(json).unwrap_or(json!({}));
    let s = |k: &str| v[k].as_str().unwrap_or("").trim().to_string();
    let phone = v["phone"]
        .as_str()
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .or(fallback_phone)
        .map(str::to_string);
    PostalAddress {
        name: s("name"),
        street1: s("street1"),
        street2: v["street2"]
            .as_str()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(str::to_string),
        city: s("city"),
        state_province: s("state_province"),
        postal: s("postal"),
        country: "US".to_string(),
        phone,
    }
}

/// The outbound and return legs of `order_id`, in that order.
pub(crate) fn order_legs(
    p2: &Connection,
    order_id: &str,
    fallback_phone: Option<&str>,
) -> rusqlite::Result<Vec<OrderLeg>> {
    let mut stmt = p2.prepare(
        "SELECT os.role, os.shipment_id, s.from_json, s.to_json
         FROM order_shipment os JOIN shipment s ON s.id = os.shipment_id
         WHERE os.order_id = ?1 AND os.role IN ('outbound', 'return')
         ORDER BY os.role = 'return', os.rowid",
    )?;
    stmt.query_map([order_id], |r| {
        let role: String = r.get(0)?;
        let from: String = r.get(2)?;
        let to: String = r.get(3)?;
        Ok(OrderLeg {
            role,
            shipment_id: r.get(1)?,
            payload: ShipmentPayload {
                from: party(&from, fallback_phone),
                to: party(&to, fallback_phone),
                parcel: Parcel::LetterEnvelope,
                service: ServiceClass::TwoDay,
                pairing_id: order_id.to_string(),
            },
        })
    })?
    .collect()
}

/// One priced leg, stored in the order's `amount_json.carrier_quotes`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegQuote {
    pub leg: String,
    pub carrier: String,
    pub cents: i64,
}

/// Why a label tier cannot be priced now; the text is customer-facing.
#[derive(Debug, PartialEq, Eq)]
pub enum QuoteError {
    Paused(&'static str),
    Carrier(String),
}

pub const PAUSED: &str = "Paid options are paused right now. The free address label still works.";
const DAY_FULL: &str =
    "Label buying reached today's limit. Try again tomorrow or choose another option.";
const OVER_MAX: &str =
    "A label for this route costs more than our per-label limit. Choose another option.";

/// Price both legs at the carrier with this order's caps. Destinations
/// are limited to the order's own two addresses.
pub(crate) fn quote_with(
    link: &dyn LabelLink,
    legs: &[OrderLeg],
    caps: Caps,
) -> Result<Vec<LegQuote>, QuoteError> {
    if legs.len() != 2 {
        return Err(QuoteError::Paused(PAUSED));
    }
    let cfg = LiveConfig {
        enabled: caps.live,
        daily_cap: u32::MAX,
        max_usd_cents: caps.max_cents,
        ship_to_allow: legs.iter().map(|l| l.payload.to.postal.clone()).collect(),
    };
    let mut out = Vec::with_capacity(legs.len());
    for leg in legs {
        let request = BuyRequest {
            payload: &leg.payload,
            direction: leg.direction(),
            payment_ref: link.payment_ref(),
        };
        let cents = link.quote(&cfg, &request).map_err(QuoteError::Carrier)?;
        if cents <= 0 {
            return Err(QuoteError::Carrier("carrier quoted no price".to_string()));
        }
        if cents > caps.max_cents {
            return Err(QuoteError::Paused(OVER_MAX));
        }
        out.push(LegQuote {
            leg: leg.role.clone(),
            carrier: link.carrier().to_string(),
            cents,
        });
    }
    Ok(out)
}

/// Quote a label order at its carrier (blocking: network).
pub fn quote_order(
    state: &AppState,
    order_id: &str,
    kind: &str,
    stripe_live: bool,
) -> Result<Vec<LegQuote>, QuoteError> {
    let store = Store::new(&state.store_root);
    let (Ok(meta), Ok(p2)) = (db::open(&store), db::open_p2(&store)) else {
        return Err(QuoteError::Carrier("store unavailable".to_string()));
    };
    let caps = caps(state, &meta);
    if !ready(state, &meta, kind, stripe_live) {
        return Err(QuoteError::Paused(PAUSED));
    }
    if buys_today(&p2, unix_now()).saturating_add(2) > caps.daily_cap {
        return Err(QuoteError::Paused(DAY_FULL));
    }
    let phone = ship_from_phone(state, &meta);
    let legs = order_legs(&p2, order_id, phone.as_deref())
        .map_err(|e| QuoteError::Carrier(e.to_string()))?;
    let link = connect(state, &meta, kind, stripe_live).map_err(QuoteError::Carrier)?;
    quote_with(link.as_ref(), &legs, caps)
}

/// Price lines for a label order: each leg as quoted, plus the service
/// fee when it is set.
pub fn label_lines(quotes: &[LegQuote], fee_cents: i64) -> Vec<(String, i64)> {
    let mut lines: Vec<(String, i64)> = quotes
        .iter()
        .map(|q| {
            let label = match (q.carrier.as_str(), q.leg.as_str()) {
                ("fedex", "return") => "FedEx 2Day return label to you",
                ("fedex", _) => "FedEx 2Day label to the office",
                ("usps", "return") => "USPS Priority Mail return label to you",
                (_, _) => "USPS Priority Mail label to the office",
            };
            (label.to_string(), q.cents)
        })
        .collect();
    if fee_cents > 0 {
        lines.push(("Service fee".to_string(), fee_cents));
    }
    lines
}

/// The operator's ship-from phone (`orders.ship_from`), the contact
/// number carriers get when an address has none.
fn ship_from_phone(state: &AppState, meta: &Connection) -> Option<String> {
    crate::orders::ship_from(state, meta).and_then(|s| s.phone)
}

// ------------------------------------------------------------ the buyer

/// Start the buyer: on every settle poke and once a minute.
pub fn spawn(state: &AppState) {
    let state = state.clone();
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = POKE.notified() => {}
                _ = tokio::time::sleep(std::time::Duration::from_secs(60)) => {}
            }
            let job = state.clone();
            let _ = tokio::task::spawn_blocking(move || tick(&job)).await;
        }
    });
}

fn tick(state: &AppState) {
    let Ok(p2) = db::open_p2(&Store::new(&state.store_root)) else {
        return;
    };
    let ids: Vec<String> = p2
        .prepare(
            "SELECT id FROM customer_order
             WHERE status = 'label_purchasing' AND kind IN ('label_fedex', 'label_usps')
             ORDER BY updated_at LIMIT 5",
        )
        .and_then(|mut stmt| {
            stmt.query_map([], |r| r.get(0))?
                .collect::<rusqlite::Result<Vec<String>>>()
        })
        .unwrap_or_default();
    drop(p2);
    for id in ids {
        let _ = buy_order(state, &id, &|state, meta, kind, live| {
            connect(state, meta, kind, live)
        });
    }
}

/// How one pass over an order ended (for tests and logs).
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum BuyEnd {
    Ready,
    Staff(&'static str),
    Refunded,
    Skipped,
}

type Connector<'a> =
    dyn Fn(&AppState, &Connection, &str, bool) -> Result<Box<dyn LabelLink>, String> + 'a;

/// Buy every unbought leg of one paid order (blocking).
pub(crate) fn buy_order(state: &AppState, order_id: &str, connector: &Connector) -> BuyEnd {
    let store = Store::new(&state.store_root);
    let (Ok(meta), Ok(p2)) = (db::open(&store), db::open_p2(&store)) else {
        return BuyEnd::Skipped;
    };
    let row: Option<(String, String, String, String, String)> = p2
        .query_row(
            "SELECT kind, status, customer_ref, mode, amount_json FROM customer_order WHERE id = ?1",
            [order_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .optional()
        .unwrap_or(None);
    let Some((kind, status, customer_ref, mode, amount)) = row else {
        return BuyEnd::Skipped;
    };
    if status != "label_purchasing" {
        return BuyEnd::Skipped;
    }
    let live = mode == "live";
    let ctx = Ctx {
        state,
        p2: &p2,
        order_id,
        customer_ref: &customer_ref,
    };
    let caps = caps(state, &meta);
    if !caps.live {
        return ctx.to_staff("Label buying is paused right now; our team will buy your labels.");
    }
    let phone = ship_from_phone(state, &meta);
    let Ok(legs) = order_legs(&p2, order_id, phone.as_deref()) else {
        return BuyEnd::Skipped;
    };
    let quotes: Vec<LegQuote> = serde_json::from_value(
        serde_json::from_str::<Value>(&amount).unwrap_or(json!({}))["carrier_quotes"].clone(),
    )
    .unwrap_or_default();
    let mut link: Option<Box<dyn LabelLink>> = None;
    for leg in &legs {
        let now = unix_now();
        let existing: Option<String> = p2
            .query_row(
                "SELECT status FROM label_purchase WHERE order_id = ?1 AND leg = ?2",
                rusqlite::params![order_id, leg.role],
                |r| r.get(0),
            )
            .optional()
            .unwrap_or(None);
        match existing.as_deref() {
            Some("bought") => continue,
            Some("failed") => {
                // Definitely not bought: retry, counted against today.
                if buys_today(&p2, now) >= caps.daily_cap {
                    return ctx.to_staff(
                        "Today's label limit is reached; our team will buy your labels.",
                    );
                }
                let _ = p2.execute(
                    "UPDATE label_purchase SET status = 'in_flight', error = '', created_at = ?1,
                            updated_at = ?1
                     WHERE order_id = ?2 AND leg = ?3 AND status = 'failed'",
                    rusqlite::params![now, order_id, leg.role],
                );
            }
            Some(_) => {
                return ctx.to_staff("A label purchase needs a check by our team.");
            }
            None => {
                if buys_today(&p2, now) >= caps.daily_cap {
                    return ctx.to_staff(
                        "Today's label limit is reached; our team will buy your labels.",
                    );
                }
                let carrier = if kind == "label_usps" {
                    "usps"
                } else {
                    "fedex"
                };
                // Committed before the carrier call: the row is the claim.
                if p2
                    .execute(
                        "INSERT INTO label_purchase(order_id, leg, carrier, status, livemode,
                                                    created_at, updated_at)
                         VALUES (?1, ?2, ?3, 'in_flight', ?4, ?5, ?5)",
                        rusqlite::params![order_id, leg.role, carrier, i64::from(live), now],
                    )
                    .is_err()
                {
                    return BuyEnd::Skipped; // another buyer holds this leg
                }
            }
        }
        let quoted = quotes.iter().find(|q| q.leg == leg.role).map(|q| q.cents);
        if quoted.is_none_or(|c| c > caps.max_cents) {
            ctx.mark(leg, "failed", "quote missing or over the per-label limit");
            return ctx.to_staff("Your label price needs a check by our team.");
        }
        if link.is_none() {
            match connector(state, &meta, &kind, live) {
                Ok(l) => link = Some(l),
                Err(e) => {
                    ctx.mark(leg, "failed", &e);
                    return ctx.to_staff(
                        "We could not reach the carrier; our team will buy your labels.",
                    );
                }
            }
        }
        let carrier = link.as_deref().expect("link is set above");
        let request = BuyRequest {
            payload: &leg.payload,
            direction: leg.direction(),
            payment_ref: carrier.payment_ref(),
        };
        match carrier.buy(&request) {
            Ok(outcome) => {
                if ctx
                    .record_bought(&store, leg, carrier.carrier(), &outcome)
                    .is_err()
                {
                    ctx.mark(leg, "unknown", "bought, but the label could not be stored");
                    return ctx.to_staff("Your label needs a check by our team.");
                }
            }
            Err(LabelError::Api { status, message }) if status == 400 || status == 422 => {
                ctx.mark(leg, "failed", &format!("http {status}: {message}"));
                return ctx.refund(&meta, carrier);
            }
            Err(
                e @ (LabelError::Api {
                    status: 401..=499, ..
                }
                | LabelError::NotReady
                | LabelError::EmptyPaymentRef),
            ) => {
                ctx.mark(leg, "failed", &e.to_string());
                return ctx.to_staff(
                    "The carrier did not accept our request; our team will buy your labels.",
                );
            }
            Err(e) => {
                ctx.mark(leg, "unknown", &e.to_string());
                return ctx.to_staff("A label purchase needs a check by our team.");
            }
        }
    }
    let now = unix_now();
    if !crate::orders::set_status(&p2, order_id, "label_purchasing", "labels_ready", now) {
        return BuyEnd::Skipped;
    }
    crate::orders::event(
        &p2,
        order_id,
        "order.labels_ready",
        "Your labels are ready to print",
        now,
    );
    ctx.publish("order.labels_ready", json!({}));
    crate::ws_bus::publish_ops(
        &state.bus,
        "order.labels_ready",
        json!({"order_id": order_id, "kind": kind}),
        now,
    );
    BuyEnd::Ready
}

struct Ctx<'a> {
    state: &'a AppState,
    p2: &'a Connection,
    order_id: &'a str,
    customer_ref: &'a str,
}

impl Ctx<'_> {
    fn publish(&self, kind: &str, extra: Value) {
        let topic = format!("billing:{}", self.customer_ref);
        let mut msg =
            json!({"type": kind, "topic": topic, "order_id": self.order_id, "at": unix_now()});
        if let (Some(map), Some(more)) = (msg.as_object_mut(), extra.as_object()) {
            for (k, v) in more {
                map.insert(k.clone(), v.clone());
            }
        }
        self.state.bus.publish(&topic, msg.to_string());
    }

    fn mark(&self, leg: &OrderLeg, status: &str, error: &str) {
        let clipped: String = error.chars().take(300).collect();
        let _ = self.p2.execute(
            "UPDATE label_purchase SET status = ?1, error = ?2, updated_at = ?3
             WHERE order_id = ?4 AND leg = ?5",
            rusqlite::params![status, clipped, unix_now(), self.order_id, leg.role],
        );
    }

    fn to_staff(&self, text: &'static str) -> BuyEnd {
        let now = unix_now();
        if crate::orders::set_status(
            self.p2,
            self.order_id,
            "label_purchasing",
            "needs_staff",
            now,
        ) {
            crate::orders::event(self.p2, self.order_id, "order.needs_staff", text, now);
            self.publish("order.needs_staff", json!({}));
            crate::ws_bus::publish_ops(
                &self.state.bus,
                "order.needs_staff",
                json!({"order_id": self.order_id, "reason": text}),
                now,
            );
        }
        BuyEnd::Staff(text)
    }

    /// Store one bought leg: PDF blob, `shipping_label`, the shipment
    /// and order rows, the ledger; then tell the cabinet.
    fn record_bought(
        &self,
        store: &Store,
        leg: &OrderLeg,
        carrier: &str,
        outcome: &BuyOutcome,
    ) -> Result<(), String> {
        let now = unix_now();
        let sha = store
            .write_blob(&outcome.label_pdf)
            .map_err(|e| e.to_string())?;
        let tx = self.p2.unchecked_transaction().map_err(|e| e.to_string())?;
        tx.execute(
            "INSERT INTO shipping_label(shipment_id, direction, carrier, service, tracking_no,
                                       payload_json, status, created_at, issued_at, purchase_option,
                                       label_format, label_sha256, content_type)
             VALUES (?1, ?2, ?3, ?4, ?5, '{}', 'issued', ?6, ?6, 'prepaid', 'four_by_six', ?7,
                     'application/pdf')",
            rusqlite::params![
                leg.shipment_id,
                if leg.role == "return" {
                    "return"
                } else {
                    "outbound"
                },
                carrier,
                outcome.carrier_service,
                outcome.tracking_no,
                now,
                sha,
            ],
        )
        .map_err(|e| e.to_string())?;
        let label_id = tx.last_insert_rowid();
        tx.execute(
            "UPDATE shipment SET status = 'ready', postage_cost_cents = ?1, updated_at = ?2
             WHERE id = ?3",
            rusqlite::params![outcome.postage_cost_cents, now, leg.shipment_id],
        )
        .map_err(|e| e.to_string())?;
        tx.execute(
            "UPDATE order_shipment SET tracking_number = ?1 WHERE order_id = ?2 AND role = ?3",
            rusqlite::params![outcome.tracking_no, self.order_id, leg.role],
        )
        .map_err(|e| e.to_string())?;
        tx.execute(
            "UPDATE label_purchase SET status = 'bought', tracking_no = ?1, cost_cents = ?2,
                    label_id = ?3, updated_at = ?4
             WHERE order_id = ?5 AND leg = ?6",
            rusqlite::params![
                outcome.tracking_no,
                outcome.postage_cost_cents,
                label_id,
                now,
                self.order_id,
                leg.role
            ],
        )
        .map_err(|e| e.to_string())?;
        tx.execute(
            "INSERT INTO order_event(order_id, at, kind, text, leg) VALUES (?1, ?2, 'label.ready', ?3, ?4)",
            rusqlite::params![
                self.order_id,
                now,
                if leg.role == "return" {
                    "Return label bought"
                } else {
                    "Outbound label bought"
                },
                leg.role
            ],
        )
        .map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
        self.publish("label.ready", json!({"leg": leg.role}));
        let topic = format!("shipment:{}", leg.shipment_id);
        self.state.bus.publish(
            &topic,
            json!({"type": "label.ready", "topic": topic, "shipment_id": leg.shipment_id,
                   "direction": leg.role, "at": now})
            .to_string(),
        );
        Ok(())
    }

    /// The carrier refused the shipment: void what was bought, refund
    /// the order in full and say so. A refund Stripe refuses goes to
    /// staff instead.
    fn refund(&self, meta: &Connection, carrier: &dyn LabelLink) -> BuyEnd {
        let bought: Vec<(String, String, Option<i64>)> = self
            .p2
            .prepare(
                "SELECT leg, tracking_no, label_id FROM label_purchase
                 WHERE order_id = ?1 AND status = 'bought'",
            )
            .and_then(|mut stmt| {
                stmt.query_map([self.order_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
                    .collect::<rusqlite::Result<Vec<_>>>()
            })
            .unwrap_or_default();
        for (leg, tracking, label_id) in bought {
            if carrier.cancel(&tracking).is_err() {
                return self
                    .to_staff("A bought label needs voiding by our team before your refund.");
            }
            let _ = self.p2.execute(
                "UPDATE label_purchase SET status = 'voided', updated_at = ?1
                 WHERE order_id = ?2 AND leg = ?3",
                rusqlite::params![unix_now(), self.order_id, leg],
            );
            if let Some(id) = label_id {
                let _ = self.p2.execute(
                    "UPDATE shipping_label SET status = 'voided' WHERE id = ?1",
                    [id],
                );
            }
        }
        let intent: Option<String> = self
            .p2
            .query_row(
                "SELECT payment_intent FROM order_payment
                 WHERE order_id = ?1 AND status = 'paid' AND refund_id IS NULL",
                [self.order_id],
                |r| r.get(0),
            )
            .optional()
            .unwrap_or(None)
            .flatten();
        let refunded = match (crate::stripe::keys(self.state, meta), intent) {
            (Some(keys), Some(intent)) => crate::stripe::refund(&keys, self.order_id, &intent).ok(),
            _ => None,
        };
        let Some(refund_id) = refunded else {
            return self
                .to_staff("The carrier could not take this shipment; our team will refund you.");
        };
        let now = unix_now();
        let _ = self.p2.execute(
            "UPDATE order_payment SET refund_id = ?1 WHERE order_id = ?2",
            rusqlite::params![refund_id, self.order_id],
        );
        if crate::orders::set_status(self.p2, self.order_id, "label_purchasing", "refunded", now) {
            crate::orders::event(
                self.p2,
                self.order_id,
                "order.refunded",
                "The carrier could not take this shipment, so we refunded you in full",
                now,
            );
        }
        self.publish("order.refunded", json!({}));
        crate::ws_bus::publish_ops(
            &self.state.bus,
            "order.refunded",
            json!({"order_id": self.order_id, "reason": "carrier refused the shipment"}),
            now,
        );
        BuyEnd::Refunded
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caps_parse_with_safe_defaults() {
        assert_eq!(parse_caps(None, None, None), Caps::default());
        let caps = parse_caps(Some(true), Some(" 4 "), Some("42.50"));
        assert_eq!(
            caps,
            Caps {
                live: true,
                daily_cap: 4,
                max_cents: 4_250
            }
        );
        // Out of range or unparsable: defaults, never "unlimited".
        let caps = parse_caps(Some(true), Some("-1"), Some("99999"));
        assert_eq!(
            (caps.daily_cap, caps.max_cents),
            (DEFAULT_DAILY_CAP, DEFAULT_MAX_CENTS)
        );
        assert_eq!(parse_caps(Some(true), Some("0"), None).daily_cap, 0);
    }

    #[test]
    fn mode_coherence_pairs_test_money_with_the_sandbox() {
        assert!(fedex_mode_ok(labels::FEDEX_BASE_SANDBOX, false));
        assert!(fedex_mode_ok(labels::FEDEX_BASE_PROD, true));
        assert!(!fedex_mode_ok(labels::FEDEX_BASE_PROD, false));
        assert!(!fedex_mode_ok(labels::FEDEX_BASE_SANDBOX, true));
    }

    #[test]
    fn lines_add_the_fee_only_when_set() {
        let quotes = vec![
            LegQuote {
                leg: "outbound".to_string(),
                carrier: "fedex".to_string(),
                cents: 1_250,
            },
            LegQuote {
                leg: "return".to_string(),
                carrier: "fedex".to_string(),
                cents: 1_300,
            },
        ];
        let lines = label_lines(&quotes, 0);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines.iter().map(|l| l.1).sum::<i64>(), 2_550);
        let lines = label_lines(&quotes, 500);
        assert_eq!(lines.last(), Some(&("Service fee".to_string(), 500)));
        assert_eq!(lines.iter().map(|l| l.1).sum::<i64>(), 3_050);
    }

    #[test]
    fn a_missing_phone_takes_the_ship_from_phone() {
        let json = r#"{"name":"A","street1":"1 Main St","city":"Olympia","state_province":"WA","postal":"98504"}"#;
        assert_eq!(
            party(json, Some("+13605550100")).phone.as_deref(),
            Some("+13605550100")
        );
        let own = r#"{"name":"A","street1":"1 Main St","city":"Olympia","state_province":"WA","postal":"98504","phone":"+12065550100"}"#;
        assert_eq!(
            party(own, Some("+13605550100")).phone.as_deref(),
            Some("+12065550100")
        );
        assert_eq!(party(json, None).phone, None);
    }

    struct StubLink {
        cents: i64,
        buy: std::cell::RefCell<Vec<Result<BuyOutcome, LabelError>>>,
        cancels: std::cell::RefCell<Vec<String>>,
    }

    impl LabelLink for StubLink {
        fn carrier(&self) -> &'static str {
            "fedex"
        }
        fn quote(&self, cfg: &LiveConfig, request: &BuyRequest) -> Result<i64, String> {
            cfg.require_enabled().map_err(|e| e.to_string())?;
            cfg.check_zip(&request.payload.to.postal)
                .map_err(|e| e.to_string())?;
            Ok(self.cents)
        }
        fn buy(&self, _request: &BuyRequest) -> Result<BuyOutcome, LabelError> {
            self.buy.borrow_mut().remove(0)
        }
        fn cancel(&self, tracking_no: &str) -> Result<(), LabelError> {
            self.cancels.borrow_mut().push(tracking_no.to_string());
            Ok(())
        }
        fn payment_ref(&self) -> &str {
            "000000000"
        }
    }

    fn legs() -> Vec<OrderLeg> {
        let office = r#"{"name":"Office","street1":"1 Main St","city":"Olympia","state_province":"WA","postal":"98504"}"#;
        let home = r#"{"name":"Pat","street1":"2 Elm St","city":"Seattle","state_province":"WA","postal":"98101"}"#;
        ["outbound", "return"]
            .iter()
            .enumerate()
            .map(|(i, role)| {
                let (from, to) = if *role == "return" {
                    (office, home)
                } else {
                    (home, office)
                };
                OrderLeg {
                    role: role.to_string(),
                    shipment_id: i as i64 + 1,
                    payload: ShipmentPayload {
                        from: party(from, Some("+13605550100")),
                        to: party(to, Some("+13605550100")),
                        parcel: Parcel::LetterEnvelope,
                        service: ServiceClass::TwoDay,
                        pairing_id: "ord_test".to_string(),
                    },
                }
            })
            .collect()
    }

    #[test]
    fn quotes_enforce_the_switch_and_the_per_label_ceiling() {
        let link = StubLink {
            cents: 1_250,
            buy: Default::default(),
            cancels: Default::default(),
        };
        let on = Caps {
            live: true,
            ..Caps::default()
        };
        let quotes = quote_with(&link, &legs(), on).expect("quotes");
        assert_eq!(quotes.len(), 2);
        assert!(
            quotes
                .iter()
                .all(|q| q.cents == 1_250 && q.carrier == "fedex")
        );
        // Off: the gate refuses before any price.
        assert!(matches!(
            quote_with(&link, &legs(), Caps::default()),
            Err(QuoteError::Carrier(_))
        ));
        // Over the ceiling: paused with the reason.
        let tight = Caps {
            live: true,
            max_cents: 1_000,
            ..Caps::default()
        };
        assert_eq!(
            quote_with(&link, &legs(), tight),
            Err(QuoteError::Paused(OVER_MAX))
        );
        // One leg only (a malformed order): paused, never half-priced.
        assert_eq!(
            quote_with(&link, &legs()[..1], on),
            Err(QuoteError::Paused(PAUSED))
        );
    }
}
