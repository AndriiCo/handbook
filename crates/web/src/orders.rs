//! Customer orders (`label_fedex`, `label_usps`, `mail_kit`,
//! `address_label`): `customer_order`, `order_shipment`, `order_event`
//! in p2.sqlite, a backing `form_submission` per order, the state
//! machine, delivery options, and `billing:{customer_ref}` events.
//!
//! Owner: lane SA-24 commerce (`docs/lanes/sa-24.md`). C1 (dark) landed
//! by the builder (P325) while the lane session could not start: orders
//! are created and tracked, the free printable address label works end
//! to end, and every paid tier answers 503 `paid_tiers_paused` until C2
//! (Stripe Checkout) and the carrier ledger (SA-25 C3) land.
//!
//! Delivery options come only from accepted facts: the destination is a
//! US postal address written in an accepted `mailing`, `po_box` or
//! `address` fact (or an accepted office), a PO box (or a cited "USPS
//! only") disables FedEx and the card cites that fact, and with no such
//! fact every mail option is disabled with the reason.

use std::sync::LazyLock;

use axum::{
    Router,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Json, Response},
    routing::get,
};
use handbook_store::{db, store::Store};
use rusqlite::{Connection, OptionalExtension};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{AppState, store_unavailable, unix_now};

/// Order kinds (contracts.md enumerations).
pub const KINDS: &[&str] = &["label_fedex", "label_usps", "mail_kit", "address_label"];
const PAID: &[&str] = &["label_fedex", "label_usps", "mail_kit"];

/// Allowed status transitions (contracts.md). `needs_staff`, `refunded`
/// and `canceled` are side states reachable from any live state.
#[allow(dead_code)] // C2 (Stripe) and SA-25 C3 drive status moves through it.
pub fn can_transition(from: &str, to: &str) -> bool {
    let live = !matches!(from, "delivered" | "refunded" | "canceled");
    match (from, to) {
        (_, "needs_staff" | "canceled") => live,
        (_, "refunded") => live && from != "draft",
        ("draft", "quoted" | "labels_ready") => true,
        ("quoted", "awaiting_payment" | "quoted") => true,
        ("awaiting_payment", "paid") => true,
        ("paid", "label_purchasing") => true,
        ("label_purchasing", "labels_ready") => true,
        ("labels_ready", "packing" | "in_transit") => true,
        ("packing", "shipped") => true,
        ("shipped", "in_transit") => true,
        ("in_transit", "delivered") => true,
        ("needs_staff", "label_purchasing" | "labels_ready" | "packing") => true,
        _ => false,
    }
}

// --------------------------------------------------------------- DDL

/// Boot DDL on the P2 DB (`p2.sqlite`, customer PII lives here):
/// `CREATE TABLE IF NOT EXISTS` only — no ALTER, DROP or RENAME.
pub fn migrate_p2_tables(p2: &Connection) -> rusqlite::Result<()> {
    p2.execute_batch(
        "CREATE TABLE IF NOT EXISTS customer_order (
            id TEXT PRIMARY KEY,
            customer_ref TEXT NOT NULL,
            kind TEXT NOT NULL,
            status TEXT NOT NULL,
            authority_slug TEXT NOT NULL,
            path_id TEXT,
            form_fill_id TEXT,
            submission_id INTEGER NOT NULL,
            return_json TEXT NOT NULL,
            dest_json TEXT NOT NULL,
            amount_json TEXT NOT NULL DEFAULT '{}',
            mode TEXT NOT NULL,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL,
            FOREIGN KEY(submission_id) REFERENCES form_submission(id)
        );
        CREATE INDEX IF NOT EXISTS customer_order_customer ON customer_order(customer_ref, created_at);
        CREATE TABLE IF NOT EXISTS order_shipment (
            order_id TEXT NOT NULL,
            shipment_id INTEGER NOT NULL,
            role TEXT NOT NULL,
            carrier TEXT NOT NULL DEFAULT 'none',
            service TEXT NOT NULL DEFAULT '',
            tracking_number TEXT NOT NULL DEFAULT '',
            PRIMARY KEY(order_id, role),
            FOREIGN KEY(order_id) REFERENCES customer_order(id),
            FOREIGN KEY(shipment_id) REFERENCES shipment(id)
        );
        CREATE TABLE IF NOT EXISTS order_event (
            id INTEGER PRIMARY KEY,
            order_id TEXT NOT NULL,
            at INTEGER NOT NULL,
            kind TEXT NOT NULL,
            text TEXT NOT NULL,
            leg TEXT,
            FOREIGN KEY(order_id) REFERENCES customer_order(id)
        );
        CREATE INDEX IF NOT EXISTS order_event_order ON order_event(order_id, at);",
    )
}

// ----------------------------------------------------- destination

/// A US postal address found in accepted text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsAddress {
    pub street: String,
    pub city: String,
    pub state: String,
    pub zip: String,
}

impl UsAddress {
    pub fn is_po_box(&self) -> bool {
        let s = self.street.to_lowercase().replace('.', "");
        s.starts_with("po box") || s.contains(" po box")
    }
}

fn is_zip(s: &str) -> bool {
    // ASCII only: byte slicing below must never split a UTF-8 char
    // (a customer-typed "1234é" once aborted the process).
    if !s.is_ascii() {
        return false;
    }
    let (five, rest) = s.split_at(s.len().min(5));
    five.len() == 5
        && five.bytes().all(|b| b.is_ascii_digit())
        && (rest.is_empty()
            || (rest.len() == 5
                && rest.starts_with('-')
                && rest[1..].bytes().all(|b| b.is_ascii_digit())))
}

/// The first `street, City, ST 12345` written in `text` (deterministic;
/// the address is quoted, never inferred). The street starts at its
/// first digit or at "PO Box".
pub fn us_address_in(text: &str) -> Option<UsAddress> {
    let parts: Vec<&str> = text.split(',').map(str::trim).collect();
    for i in 2..parts.len() {
        let tail = parts[i].trim_end_matches(['.', ';', ')']);
        let mut words = tail.split_whitespace();
        let (Some(state), Some(zip)) = (words.next(), words.next()) else {
            continue;
        };
        let zip = zip.trim_end_matches(['.', ';', ')']);
        if state.len() != 2 || !state.bytes().all(|b| b.is_ascii_uppercase()) || !is_zip(zip) {
            continue;
        }
        let city = parts[i - 1].rsplit(['.', ':']).next().unwrap_or("").trim();
        let raw = parts[i - 2];
        // ASCII lowercase keeps byte offsets identical to `raw`.
        let lower = raw.to_ascii_lowercase();
        let start = lower
            .find("po box")
            .or_else(|| lower.find("p.o. box"))
            .or_else(|| raw.find(|c: char| c.is_ascii_digit()));
        let Some(start) = start else { continue };
        let street = raw[start..].trim();
        if city.is_empty() || street.is_empty() || city.len() > 40 || street.len() > 80 {
            continue;
        }
        return Some(UsAddress {
            street: street.to_string(),
            city: city.to_string(),
            state: state.to_string(),
            zip: zip.to_string(),
        });
    }
    None
}

/// One accepted fact as options sees it.
#[derive(Debug, Clone)]
pub struct FactText {
    pub id: i64,
    pub topic: String,
    pub text: String,
}

/// Where mail for this authority goes, and why FedEx may not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Destination {
    pub address: UsAddress,
    pub fact_id: i64,
    /// Fact that makes this route USPS-only (a PO box or a cited
    /// "USPS only"), if any.
    pub usps_only_fact: Option<i64>,
}

/// Mailing destination from accepted facts: `mailing` and `po_box`
/// first (that is where requests are mailed), then `address`.
pub fn destination(facts: &[FactText]) -> Option<Destination> {
    let rank = |t: &str| match t {
        "mailing" => 0,
        "po_box" => 1,
        "address" => 2,
        _ => 9,
    };
    let mut candidates: Vec<&FactText> = facts.iter().filter(|f| rank(&f.topic) < 9).collect();
    candidates.sort_by_key(|f| (rank(&f.topic), f.id));
    let (fact, address) = candidates
        .iter()
        .find_map(|f| us_address_in(&f.text).map(|a| (*f, a)))?;
    let usps_only_fact = if address.is_po_box() {
        Some(fact.id)
    } else {
        facts
            .iter()
            .find(|f| {
                let t = f.text.to_lowercase();
                matches!(f.topic.as_str(), "mailing" | "po_box")
                    && (t.contains("usps only")
                        || t.contains("u.s. postal service only")
                        || t.contains("only by usps"))
            })
            .map(|f| f.id)
    };
    Some(Destination {
        address,
        fact_id: fact.id,
        usps_only_fact,
    })
}

/// Runtime switches the options read (cached config + key presence).
#[derive(Debug, Clone, Default)]
pub struct Switches {
    pub orders_enabled: bool,
    pub stripe_key: bool,
    pub stripe_live: bool,
    pub kit_price_cents: Option<i64>,
    pub currency: String,
}

/// The four delivery cards (contracts `order-options.json`).
pub fn options(slug: &str, dest: Option<&Destination>, sw: &Switches) -> Value {
    let paid_enabled = sw.orders_enabled && sw.stripe_key;
    let no_dest = "No staff-approved US mailing address for this office yet.";
    let paused = "Paid options are paused right now. The free address label still works.";
    let card = |kind: &str, title: &str, from: Option<i64>| -> Value {
        let (enabled, reason, fact_id): (bool, Option<&str>, Option<i64>) = match (kind, dest) {
            (_, None) => (false, Some(no_dest), None),
            ("label_fedex", Some(d)) if d.usps_only_fact.is_some() => (
                false,
                Some("This office receives mail at a PO Box, which FedEx cannot deliver to."),
                d.usps_only_fact,
            ),
            ("address_label", Some(_)) => (true, None, None),
            (_, Some(_)) if !paid_enabled => (false, Some(paused), None),
            ("mail_kit", Some(_)) if from.is_none() => (false, Some(paused), None),
            _ => (true, None, None),
        };
        json!({"kind": kind, "enabled": enabled, "title": title, "reason": reason,
               "fact_id": fact_id, "from_cents": if enabled { from } else { None }})
    };
    json!({
        "slug": slug,
        "mode": if sw.stripe_live { "live" } else { "test" },
        "paid_enabled": paid_enabled,
        "currency": if sw.currency.is_empty() { "usd" } else { sw.currency.as_str() },
        "destination": dest.map(|d| json!({
            "city": d.address.city, "region": d.address.state, "postal_code": d.address.zip,
            "po_box": d.address.is_po_box(), "fact_id": d.fact_id,
        })),
        "options": [
            card("label_fedex", "FedEx labels, print at home", None),
            card("label_usps", "USPS labels with postage, print at home", None),
            card("mail_kit", "Mailing kit shipped to you", sw.kit_price_cents),
            card("address_label", "Free address label, no postage", Some(0)),
        ],
    })
}

// ----------------------------------------------------------- helpers

static ORDER_LIMITS: LazyLock<crate::passkey::RateBuckets> =
    LazyLock::new(crate::passkey::RateBuckets::default);
static READ_LIMITS: LazyLock<crate::passkey::RateBuckets> =
    LazyLock::new(crate::passkey::RateBuckets::default);

fn err(status: StatusCode, error: &str, reason: Option<&str>) -> Response {
    (status, Json(json!({"error": error, "reason": reason}))).into_response()
}

fn paused() -> Response {
    err(
        StatusCode::SERVICE_UNAVAILABLE,
        "paid_tiers_paused",
        Some("Paid options are paused right now. The free address label still works."),
    )
}

/// Unguessable order id: `ord_` + 10 chars of [A-Za-z0-9].
fn new_order_id() -> String {
    use rand::Rng as _;
    const ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz23456789";
    let mut rng = rand::rng();
    let tail: String = (0..10)
        .map(|_| ALPHABET[rng.random_range(0..ALPHABET.len())] as char)
        .collect();
    format!("ord_{tail}")
}

/// Accepted facts of `slug` (statement + quotes), for destinations.
fn fact_texts(state: &AppState, meta: &Connection, slug: &str) -> Vec<FactText> {
    crate::approval::accepted_fact_views(meta, &state.store_root, slug)
        .unwrap_or_default()
        .into_iter()
        .map(|f| {
            let mut text = f.statement.clone();
            for c in &f.citations {
                text.push('\n');
                text.push_str(&c.quote);
            }
            FactText {
                id: f.id,
                topic: f.topic,
                text,
            }
        })
        .collect()
}

fn switches(state: &AppState, meta: &Connection) -> Switches {
    let Some(kek) = state.kek.as_deref() else {
        return Switches::default();
    };
    let cache = &state.config_cache;
    let stripe = config::get_secret_for_job(meta, kek, config::keys::STRIPE_SECRET_KEY)
        .ok()
        .flatten();
    Switches {
        orders_enabled: cache
            .bool(meta, kek, config::keys::ORDERS_ENABLED)
            .ok()
            .flatten()
            .unwrap_or(false),
        stripe_key: stripe.as_ref().is_some_and(|k| !k.is_empty()),
        stripe_live: stripe.as_ref().is_some_and(|k| k.starts_with(b"sk_live")),
        kit_price_cents: cache
            .text(meta, kek, config::keys::ORDERS_KIT_PRICE_CENTS)
            .ok()
            .flatten()
            .and_then(|v| v.trim().parse().ok())
            .filter(|c: &i64| *c > 0),
        currency: cache
            .text(meta, kek, config::keys::ORDERS_CURRENCY)
            .ok()
            .flatten()
            .unwrap_or_default(),
    }
}

/// Customer-entered return address (never in a URL; p2.sqlite only).
#[derive(Debug, Clone, Deserialize, serde::Serialize)]
pub struct ReturnTo {
    pub name: String,
    pub line1: String,
    #[serde(default)]
    pub line2: Option<String>,
    pub city: String,
    pub region: String,
    pub postal_code: String,
    #[serde(default = "us")]
    pub country: String,
}

fn us() -> String {
    "US".to_string()
}

impl ReturnTo {
    fn valid(&self) -> bool {
        let ok = |s: &str, max: usize| !s.trim().is_empty() && s.chars().count() <= max;
        ok(&self.name, 80)
            && ok(&self.line1, 100)
            && self
                .line2
                .as_deref()
                .is_none_or(|l| l.chars().count() <= 100)
            && ok(&self.city, 60)
            && self.region.len() == 2
            && self.region.bytes().all(|b| b.is_ascii_uppercase())
            && is_zip(self.postal_code.trim())
            && self.country == "US"
    }
}

#[derive(Debug, Deserialize)]
pub struct CreateBody {
    pub slug: String,
    pub kind: String,
    #[serde(default)]
    pub path_id: Option<String>,
    #[serde(default)]
    pub form_fill_id: Option<String>,
    pub return_to: ReturnTo,
}

/// Legs per kind: (role, carrier, service).
fn legs_for(kind: &str) -> &'static [(&'static str, &'static str, &'static str)] {
    match kind {
        "label_fedex" => &[
            ("outbound", "fedex", "FEDEX_2_DAY"),
            ("return", "fedex", "FEDEX_2_DAY"),
        ],
        "label_usps" => &[
            ("outbound", "usps", "USPS_PRIORITY"),
            ("return", "usps", "USPS_PRIORITY"),
        ],
        "mail_kit" => &[
            ("kit_to_customer", "usps", "USPS_PRIORITY"),
            ("outbound", "usps", "USPS_PRIORITY"),
            ("return", "usps", "USPS_PRIORITY"),
        ],
        _ => &[("outbound", "none", "")],
    }
}

fn event_text(kind: &str) -> &'static str {
    match kind {
        "order.created" => "Order placed",
        "label.ready" => "Address label ready to print",
        _ => "Order updated",
    }
}

/// Insert one order with its backing submission, legs and first
/// events, in one p2 transaction. Returns the order id.
pub fn create_order(
    p2: &Connection,
    customer_ref: &str,
    (authority_id, authority_name): (i64, &str),
    body: &CreateBody,
    dest: &Destination,
    mode: &str,
    now: i64,
) -> rusqlite::Result<String> {
    let tx = p2.unchecked_transaction()?;
    tx.execute(
        "INSERT INTO form_submission(form_template_id, authority_id, customer_ref, values_json,
                                     customer_override, status, created_at)
         VALUES (0, ?1, ?2, '{}', 0, 'draft', ?3)",
        rusqlite::params![authority_id, customer_ref, now],
    )?;
    let submission_id = tx.last_insert_rowid();
    let id = new_order_id();
    let free = body.kind == "address_label";
    let status = if free { "labels_ready" } else { "draft" };
    let dest_json = json!({
        "name": authority_name, "street1": dest.address.street, "city": dest.address.city,
        "state_province": dest.address.state, "postal": dest.address.zip, "country": "US",
        "fact_id": dest.fact_id,
    });
    let return_json = serde_json::to_value(&body.return_to).unwrap_or(json!({}));
    tx.execute(
        "INSERT INTO customer_order(id, customer_ref, kind, status, authority_slug, path_id,
                                    form_fill_id, submission_id, return_json, dest_json,
                                    amount_json, mode, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?13)",
        rusqlite::params![
            id,
            customer_ref,
            body.kind,
            status,
            body.slug,
            body.path_id,
            body.form_fill_id,
            submission_id,
            return_json.to_string(),
            dest_json.to_string(),
            json!({"currency": "usd", "total_cents": 0, "lines": []}).to_string(),
            mode,
            now,
        ],
    )?;
    let customer = json!({
        "name": body.return_to.name, "street1": body.return_to.line1, "street2": body.return_to.line2,
        "city": body.return_to.city, "state_province": body.return_to.region,
        "postal": body.return_to.postal_code, "country": body.return_to.country,
    });
    for (role, carrier, service) in legs_for(&body.kind) {
        let (from, to) = match *role {
            "return" => (&dest_json, &customer),
            "kit_to_customer" => (&json!({"name": "Apostille Handbook"}), &customer),
            _ => (&customer, &dest_json),
        };
        tx.execute(
            "INSERT INTO shipment(submission_id, authority_id, status, from_json, to_json,
                                  parcel_json, paid_by, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, '{\"kind\":\"letter_envelope\"}', ?6, ?7, ?7)",
            rusqlite::params![
                submission_id,
                authority_id,
                if free { "ready" } else { "draft" },
                from.to_string(),
                to.to_string(),
                if free { "customer_postage" } else { "order" },
                now,
            ],
        )?;
        let shipment_id = tx.last_insert_rowid();
        tx.execute(
            "INSERT INTO order_shipment(order_id, shipment_id, role, carrier, service)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![id, shipment_id, role, carrier, service],
        )?;
    }
    let mut events = vec![("order.created", None)];
    if free {
        events.push(("label.ready", Some("outbound")));
    }
    for (kind, leg) in events {
        tx.execute(
            "INSERT INTO order_event(order_id, at, kind, text, leg) VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![id, now, kind, event_text(kind), leg],
        )?;
    }
    tx.commit()?;
    Ok(id)
}

/// The full order view (contracts `order.json`) for its owner.
pub fn order_view(
    p2: &Connection,
    id: &str,
    authority_name: &str,
) -> rusqlite::Result<Option<Value>> {
    let row = p2
        .query_row(
            "SELECT id, customer_ref, kind, status, authority_slug, path_id, form_fill_id,
                    return_json, amount_json, mode, created_at, updated_at
             FROM customer_order WHERE id = ?1",
            [id],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, Option<String>>(5)?,
                    r.get::<_, Option<String>>(6)?,
                    r.get::<_, String>(7)?,
                    r.get::<_, String>(8)?,
                    r.get::<_, String>(9)?,
                    r.get::<_, i64>(10)?,
                    r.get::<_, i64>(11)?,
                ))
            },
        )
        .optional()?;
    let Some((
        id,
        customer_ref,
        kind,
        status,
        slug,
        path_id,
        fill,
        ret,
        amount,
        mode,
        created,
        updated,
    )) = row
    else {
        return Ok(None);
    };
    let free = kind == "address_label";
    let mut legs_stmt = p2.prepare(
        "SELECT os.role, os.carrier, os.service, os.shipment_id, os.tracking_number, s.status
         FROM order_shipment os JOIN shipment s ON s.id = os.shipment_id
         WHERE os.order_id = ?1 ORDER BY os.rowid",
    )?;
    let legs: Vec<Value> = legs_stmt
        .query_map([&id], |r| {
            let role: String = r.get(0)?;
            let carrier: String = r.get(1)?;
            let shipment_id: i64 = r.get(3)?;
            let tracking: String = r.get(4)?;
            let ship_status: String = r.get(5)?;
            let label_ready = ship_status == "ready";
            Ok(json!({
                "role": role, "carrier": carrier, "service": r.get::<_, String>(2)?,
                "shipment_id": shipment_id,
                "status": if label_ready { "label_ready" } else { "pending" },
                "tracking_number": (!tracking.is_empty()).then_some(tracking),
                "tracking_url": Value::Null,
                "label_url": label_ready.then(|| format!("/apostille/api/v1/p2/orders/{id}/labels/{role}")),
                "label_kind": if free { "address_only" } else { "carrier" },
                "topic": format!("shipment:{shipment_id}"),
            }))
        })?
        .collect::<rusqlite::Result<_>>()?;
    let mut ev_stmt = p2.prepare(
        "SELECT at, kind, text, leg FROM order_event WHERE order_id = ?1 ORDER BY at, id",
    )?;
    let events: Vec<Value> = ev_stmt
        .query_map([&id], |r| {
            Ok(
                json!({"at": r.get::<_, i64>(0)?, "kind": r.get::<_, String>(1)?,
                      "text": r.get::<_, String>(2)?, "leg": r.get::<_, Option<String>>(3)?}),
            )
        })?
        .collect::<rusqlite::Result<_>>()?;
    let ret: Value = serde_json::from_str(&ret).unwrap_or(json!({}));
    let mut actions = Vec::new();
    if free && status == "labels_ready" {
        actions.push(json!({"id": "print_label", "label": "Print the address label"}));
    }
    Ok(Some(json!({
        "id": id, "kind": kind, "status": status, "mode": mode,
        "authority": {"slug": slug, "name": authority_name},
        "path_id": path_id, "created_at": created, "updated_at": updated,
        "amount": serde_json::from_str::<Value>(&amount).unwrap_or(json!({})),
        "return_to": {"name": ret["name"], "city": ret["city"], "region": ret["region"],
                      "postal_code": ret["postal_code"], "country": ret["country"]},
        "legs": legs, "form_fill_id": fill, "events": events,
        "receipt": Value::Null, "actions": actions,
        "topic": format!("billing:{customer_ref}"),
    })))
}

fn authority_of(meta: &Connection, slug: &str) -> Option<(i64, String)> {
    meta.query_row(
        "SELECT id, name FROM authority WHERE slug = ?1",
        [slug],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
    .optional()
    .ok()
    .flatten()
}

fn owner_of(p2: &Connection, id: &str) -> Option<(String, String, String)> {
    p2.query_row(
        "SELECT customer_ref, authority_slug, kind FROM customer_order WHERE id = ?1",
        [id],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )
    .optional()
    .ok()
    .flatten()
}

fn open_stores(state: &AppState) -> Option<(Connection, Connection)> {
    let store = Store::new(&state.store_root);
    let meta = db::open(&store).ok()?;
    let p2 = db::open_p2(&store).ok()?;
    db::migrate_p2(&p2).ok()?;
    migrate_p2_tables(&p2).ok()?;
    Some((meta, p2))
}

// ----------------------------------------------------------- handlers

#[derive(Debug, Deserialize)]
pub struct OptionsQuery {
    pub slug: String,
}

/// `GET /orders/options?slug=` — anyone, 60/min/IP.
async fn get_options(
    State(state): State<AppState>,
    Query(q): Query<OptionsQuery>,
    req: axum::extract::Request,
) -> Response {
    let ip =
        crate::admin_ip::peer_ip(&req).map_or_else(|| "unknown".to_string(), |ip| ip.to_string());
    if !READ_LIMITS.check(&format!("oo:{ip}"), unix_now(), 60, 60) {
        return err(StatusCode::TOO_MANY_REQUESTS, "rate limited", None);
    }
    let Ok(meta) = db::open(&Store::new(&state.store_root)) else {
        return store_unavailable();
    };
    let facts = fact_texts(&state, &meta, &q.slug);
    let dest = destination(&facts);
    Json(options(&q.slug, dest.as_ref(), &switches(&state, &meta))).into_response()
}

/// `POST /orders` — signed-in customer, CSRF double-submit, 10/min.
async fn create(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<CreateBody>,
) -> Response {
    if !crate::passkey::origin_ok(&headers, state.admin_mtls_off)
        || !crate::passkey::csrf_echo_ok(&headers)
    {
        return crate::passkey::csrf_fail();
    }
    let Some(customer_ref) = crate::p2::caller_customer_ref(&state, &headers) else {
        return err(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            Some("Sign in to place an order."),
        );
    };
    let now = unix_now();
    if !ORDER_LIMITS.check(&format!("oc:{customer_ref}"), now, 10, 60) {
        return err(StatusCode::TOO_MANY_REQUESTS, "rate limited", None);
    }
    if !KINDS.contains(&body.kind.as_str()) {
        return err(StatusCode::UNPROCESSABLE_ENTITY, "unknown kind", None);
    }
    if !body.return_to.valid() {
        return err(
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid return address",
            Some("Use a US address: name, street, city, two-letter state and ZIP."),
        );
    }
    let Some((meta, p2)) = open_stores(&state) else {
        return store_unavailable();
    };
    let Some((authority_id, authority_name)) = authority_of(&meta, &body.slug) else {
        return err(StatusCode::NOT_FOUND, "unknown authority", None);
    };
    let facts = fact_texts(&state, &meta, &body.slug);
    let Some(dest) = destination(&facts) else {
        return err(
            StatusCode::CONFLICT,
            "option_unavailable",
            Some("No staff-approved US mailing address for this office yet."),
        );
    };
    let sw = switches(&state, &meta);
    let opts = options(&body.slug, Some(&dest), &sw);
    let card = opts["options"]
        .as_array()
        .and_then(|cards| cards.iter().find(|c| c["kind"] == body.kind.as_str()))
        .cloned()
        .unwrap_or(json!({}));
    if card["enabled"] != true {
        if PAID.contains(&body.kind.as_str()) && card["fact_id"].is_null() {
            return paused();
        }
        return err(
            StatusCode::CONFLICT,
            "option_unavailable",
            card["reason"].as_str(),
        );
    }
    let mode = if sw.stripe_live { "live" } else { "test" };
    let id = match create_order(
        &p2,
        &customer_ref,
        (authority_id, &authority_name),
        &body,
        &dest,
        mode,
        now,
    ) {
        Ok(id) => id,
        Err(_) => return store_unavailable(),
    };
    let topic = format!("billing:{customer_ref}");
    state.bus.publish(
        &topic,
        json!({"type": "order.created", "topic": topic, "order_id": id, "at": now}).to_string(),
    );
    if body.kind == "address_label" {
        state.bus.publish(
            &topic,
            json!({"type": "label.ready", "topic": topic, "order_id": id, "leg": "outbound", "at": now}).to_string(),
        );
    }
    crate::ws_bus::publish_ops(
        &state.bus,
        "order.created",
        json!({"order_id": id, "kind": body.kind}),
        now,
    );
    match order_view(&p2, &id, &authority_name) {
        Ok(Some(view)) => (StatusCode::CREATED, Json(view)).into_response(),
        _ => store_unavailable(),
    }
}

/// (id, kind, status, mode, slug, created_at, updated_at, amount_json).
type ListRow = (String, String, String, String, String, i64, i64, String);

/// `GET /orders` — the caller's orders, newest first.
async fn list(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let Some(customer_ref) = crate::p2::caller_customer_ref(&state, &headers) else {
        return err(StatusCode::UNAUTHORIZED, "unauthorized", None);
    };
    let Some((meta, p2)) = open_stores(&state) else {
        return store_unavailable();
    };
    let rows: Vec<ListRow> = p2
        .prepare(
            "SELECT id, kind, status, mode, authority_slug, created_at, updated_at, amount_json
             FROM customer_order WHERE customer_ref = ?1 ORDER BY created_at DESC, id LIMIT 100",
        )
        .and_then(|mut s| {
            s.query_map([&customer_ref], |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                    r.get(7)?,
                ))
            })?
            .collect()
        })
        .unwrap_or_default();
    let orders: Vec<Value> = rows
        .into_iter()
        .map(|(id, kind, status, mode, slug, created, updated, amount)| {
            let name = authority_of(&meta, &slug)
                .map(|(_, n)| n)
                .unwrap_or_else(|| slug.clone());
            let amount: Value = serde_json::from_str(&amount).unwrap_or(json!({}));
            let next = match (kind.as_str(), status.as_str()) {
                ("address_label", "labels_ready") => "Print your address label",
                (_, "draft" | "quoted" | "awaiting_payment") => "Finish payment to buy the labels",
                (_, "labels_ready") => "Print your labels",
                (_, "packing") => "We are packing your kit",
                (_, "shipped" | "in_transit") => "On its way",
                (_, "delivered") => "Delivered",
                (_, "needs_staff") => "Our team is checking this order",
                _ => "",
            };
            json!({"id": id, "kind": kind, "status": status, "mode": mode,
                   "authority": {"slug": slug, "name": name},
                   "created_at": created, "updated_at": updated,
                   "total_cents": amount["total_cents"].as_i64().unwrap_or(0),
                   "currency": amount["currency"].as_str().unwrap_or("usd"), "next": next})
        })
        .collect();
    Json(json!({"orders": orders, "topic": format!("billing:{customer_ref}")})).into_response()
}

/// `GET /orders/{id}` — owner only (404 otherwise: ids never leak).
async fn get_one(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    let Some(customer_ref) = crate::p2::caller_customer_ref(&state, &headers) else {
        return err(StatusCode::UNAUTHORIZED, "unauthorized", None);
    };
    let Some((meta, p2)) = open_stores(&state) else {
        return store_unavailable();
    };
    match owner_of(&p2, &id) {
        Some((owner, slug, _)) if owner == customer_ref => {
            let name = authority_of(&meta, &slug).map(|(_, n)| n).unwrap_or(slug);
            match order_view(&p2, &id, &name) {
                Ok(Some(view)) => Json(view).into_response(),
                _ => store_unavailable(),
            }
        }
        _ => err(StatusCode::NOT_FOUND, "unknown order", None),
    }
}

/// `GET /orders/{id}/labels/{leg}` — owner only. The free address label
/// is printable HTML marked NOT POSTAGE; carrier labels are SA-25's.
async fn get_label(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((id, leg)): Path<(String, String)>,
) -> Response {
    let Some(customer_ref) = crate::p2::caller_customer_ref(&state, &headers) else {
        return err(StatusCode::UNAUTHORIZED, "unauthorized", None);
    };
    let Some((_, p2)) = open_stores(&state) else {
        return store_unavailable();
    };
    let Some((owner, _, kind)) = owner_of(&p2, &id) else {
        return err(StatusCode::NOT_FOUND, "unknown order", None);
    };
    if owner != customer_ref || kind != "address_label" || leg != "outbound" {
        return err(StatusCode::NOT_FOUND, "no such label", None);
    }
    let row: Option<(String, String)> = p2
        .query_row(
            "SELECT s.from_json, s.to_json FROM order_shipment os JOIN shipment s ON s.id = os.shipment_id
             WHERE os.order_id = ?1 AND os.role = 'outbound'",
            [&id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .ok()
        .flatten();
    let Some((from, to)) = row else {
        return err(StatusCode::NOT_FOUND, "no such label", None);
    };
    let party = |v: &str| -> labels::payload::PostalAddress {
        let v: Value = serde_json::from_str(v).unwrap_or(json!({}));
        let s = |k: &str| v[k].as_str().unwrap_or("").to_string();
        labels::payload::PostalAddress {
            name: s("name"),
            street1: s("street1"),
            street2: v["street2"].as_str().map(str::to_string),
            city: s("city"),
            state_province: s("state_province"),
            postal: s("postal"),
            country: "US".to_string(),
            phone: None,
        }
    };
    let payload = labels::payload::ShipmentPayload {
        from: party(&from),
        to: party(&to),
        parcel: labels::payload::Parcel::LetterEnvelope,
        service: labels::payload::ServiceClass::Ground,
        pairing_id: id.clone(),
    };
    let html = labels::printable::render_printable(
        &payload,
        labels::payload::Direction::Outbound,
        labels::carrier::Carrier::Usps,
        labels::format::PrintFormat::Letter,
        "Address label — add your own postage",
    );
    (
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (header::CACHE_CONTROL, "private, no-store"),
        ],
        html,
    )
        .into_response()
}

/// Quote and Checkout: paused until C2 (Stripe) and the carrier ledger.
async fn quote(Path(_id): Path<String>) -> Response {
    paused()
}

async fn checkout(Path(_id): Path<String>) -> Response {
    paused()
}

/// Customer routes, merged into the `/api/v1/p2` nest (paths are
/// relative: `/orders`, `/orders/{id}`). CSRF double-submit on every
/// POST; owner-or-staff on every read.
pub fn p2_routes() -> Router<AppState> {
    Router::new()
        .route("/orders/options", get(get_options))
        .route("/orders", get(list).post(create))
        .route("/orders/{id}", get(get_one))
        .route("/orders/{id}/labels/{leg}", get(get_label))
        .route("/orders/{id}/quote", axum::routing::post(quote))
        .route("/orders/{id}/checkout", axum::routing::post(checkout))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(id: i64, topic: &str, text: &str) -> FactText {
        FactText {
            id,
            topic: topic.into(),
            text: text.into(),
        }
    }

    #[test]
    fn non_ascii_input_never_panics() {
        assert!(!is_zip("1234é"));
        assert!(!is_zip("é"));
        assert!(!is_zip("98501-é234"));
        assert!(us_address_in("İİİ at PO Box 4000, Olympia, WA 98504").is_some());
        assert!(us_address_in("Hà Nội, Đống Đa, VN 100000").is_none());
        let bad = ReturnTo {
            name: "A".into(),
            line1: "1 A St".into(),
            line2: None,
            city: "B".into(),
            region: "WA".into(),
            postal_code: "1234é".into(),
            country: "US".into(),
        };
        assert!(!bad.valid());
    }

    #[test]
    fn addresses_are_read_from_quoted_text_only() {
        let a = us_address_in("Walk-in service is at 100 Fixture Way, Olympia, WA 98501.").unwrap();
        assert_eq!(
            (
                a.street.as_str(),
                a.city.as_str(),
                a.state.as_str(),
                a.zip.as_str()
            ),
            ("100 Fixture Way", "Olympia", "WA", "98501")
        );
        assert!(!a.is_po_box());
        let b = us_address_in("Mail requests to PO Box 40228, Olympia, WA 98504-0228").unwrap();
        assert_eq!(b.street, "PO Box 40228");
        assert!(b.is_po_box());
        assert!(us_address_in("Open Monday to Friday, 8:00 AM to 4:30 PM.").is_none());
        assert!(us_address_in("Hanoi, Viet Nam").is_none());
    }

    #[test]
    fn po_box_destination_disables_fedex_with_the_cited_fact() {
        let facts = [
            f(
                204,
                "address",
                "Walk-in service is at 100 Fixture Way, Olympia, WA 98501.",
            ),
            f(
                203,
                "po_box",
                "Mailed requests go to PO Box 4000, Olympia, WA 98504.",
            ),
        ];
        let d = destination(&facts).unwrap();
        assert_eq!(
            (d.fact_id, d.usps_only_fact),
            (203, Some(203)),
            "po_box outranks the street address"
        );
        let sw = Switches {
            orders_enabled: true,
            stripe_key: true,
            kit_price_cents: Some(4900),
            ..Default::default()
        };
        let v = options("usa-wa-sos", Some(&d), &sw);
        let card = |k: &str| {
            v["options"]
                .as_array()
                .unwrap()
                .iter()
                .find(|c| c["kind"] == k)
                .unwrap()
                .clone()
        };
        assert_eq!(card("label_fedex")["enabled"], false);
        assert_eq!(card("label_fedex")["fact_id"], 203);
        assert_eq!(card("label_usps")["enabled"], true);
        assert_eq!(card("mail_kit")["from_cents"], 4900);
        assert_eq!(card("address_label")["enabled"], true);
        assert_eq!(v["mode"], "test");
    }

    #[test]
    fn paused_and_addressless_states_are_explained() {
        let d =
            destination(&[f(204, "address", "At 100 Fixture Way, Olympia, WA 98501.")]).unwrap();
        let v = options("x", Some(&d), &Switches::default());
        let cards = v["options"].as_array().unwrap();
        assert!(
            cards
                .iter()
                .filter(|c| c["kind"] != "address_label")
                .all(|c| c["enabled"] == false)
        );
        assert_eq!(
            cards[3]["enabled"], true,
            "free label works while paid tiers pause"
        );
        let none = options("vietnam", None, &Switches::default());
        assert!(
            none["options"]
                .as_array()
                .unwrap()
                .iter()
                .all(|c| c["enabled"] == false && c["reason"].is_string())
        );
        assert!(none["destination"].is_null());
    }

    #[test]
    fn state_machine_allows_only_forward_and_side_moves() {
        assert!(can_transition("draft", "labels_ready"));
        assert!(can_transition("awaiting_payment", "paid"));
        assert!(can_transition("paid", "needs_staff"));
        assert!(!can_transition("draft", "paid"), "no skipping payment");
        assert!(!can_transition("delivered", "canceled"));
        assert!(!can_transition("draft", "refunded"), "nothing to refund");
        assert!(can_transition("labels_ready", "refunded"));
    }

    fn p2_store() -> (tempfile::TempDir, Connection) {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::new(tmp.path());
        store.init().unwrap();
        let p2 = db::open_p2(&store).unwrap();
        db::migrate_p2(&p2).unwrap();
        migrate_p2_tables(&p2).unwrap();
        migrate_p2_tables(&p2).unwrap();
        (tmp, p2)
    }

    fn body(kind: &str) -> CreateBody {
        CreateBody {
            slug: "usa-wa-sos".into(),
            kind: kind.into(),
            path_id: Some("birth-certificate".into()),
            form_fill_id: None,
            return_to: ReturnTo {
                name: "SYNTHETIC Customer".into(),
                line1: "1 Example St".into(),
                line2: None,
                city: "Springfield".into(),
                region: "IL".into(),
                postal_code: "62701".into(),
                country: "US".into(),
            },
        }
    }

    #[test]
    fn every_kind_inserts_its_legs_with_foreign_keys_on() {
        let (_tmp, p2) = p2_store();
        let fk: i64 = p2
            .query_row("PRAGMA foreign_keys", [], |r| r.get(0))
            .unwrap();
        assert_eq!(fk, 1, "the FK the backing submission satisfies is enforced");
        let dest =
            destination(&[f(203, "po_box", "Send to PO Box 4000, Olympia, WA 98504.")]).unwrap();
        for (kind, legs) in [
            ("address_label", 1),
            ("label_fedex", 2),
            ("label_usps", 2),
            ("mail_kit", 3),
        ] {
            let id = create_order(
                &p2,
                "cust-1",
                (7, "Synthetic SOS"),
                &body(kind),
                &dest,
                "test",
                100,
            )
            .unwrap();
            assert!(id.starts_with("ord_") && id.len() == 14);
            let v = order_view(&p2, &id, "Synthetic SOS").unwrap().unwrap();
            assert_eq!(v["legs"].as_array().unwrap().len(), legs, "{kind}");
            assert_eq!(
                v["status"],
                if kind == "address_label" {
                    "labels_ready"
                } else {
                    "draft"
                }
            );
            assert_eq!(v["topic"], "billing:cust-1");
            let owner: String = p2
                .query_row(
                    "SELECT s.customer_ref FROM form_submission s JOIN shipment h ON h.submission_id = s.id
                     JOIN order_shipment os ON os.shipment_id = h.id WHERE os.order_id = ?1 LIMIT 1",
                    [&id],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(
                owner, "cust-1",
                "shipment topic authz resolves to the buyer"
            );
        }
        let free = create_order(
            &p2,
            "cust-1",
            (7, "Synthetic SOS"),
            &body("address_label"),
            &dest,
            "test",
            200,
        )
        .unwrap();
        let v = order_view(&p2, &free, "Synthetic SOS").unwrap().unwrap();
        assert_eq!(
            v["legs"][0]["label_url"],
            format!("/apostille/api/v1/p2/orders/{free}/labels/outbound")
        );
        assert_eq!(v["events"].as_array().unwrap().len(), 2);
        assert!(
            v["return_to"].get("line1").is_none(),
            "street stays out of the view"
        );
    }

    #[test]
    fn contract_seeds_match_the_served_shapes() {
        let seed: Value = serde_json::from_str(include_str!(
            "../../../docs/lanes/contracts/order-options.json"
        ))
        .unwrap();
        let d =
            destination(&[f(203, "po_box", "Send to PO Box 4000, Olympia, WA 98504.")]).unwrap();
        let v = options("usa-wa-sos", Some(&d), &Switches::default());
        for key in seed.as_object().unwrap().keys() {
            assert!(v.get(key).is_some(), "options key {key}");
        }
        for key in seed["options"][0].as_object().unwrap().keys() {
            assert!(v["options"][0].get(key).is_some(), "card key {key}");
        }
        let order: Value =
            serde_json::from_str(include_str!("../../../docs/lanes/contracts/order.json")).unwrap();
        let (_tmp, p2) = p2_store();
        let id = create_order(&p2, "cust-1", (7, "S"), &body("mail_kit"), &d, "test", 1).unwrap();
        let mine = order_view(&p2, &id, "S").unwrap().unwrap();
        for key in order.as_object().unwrap().keys() {
            assert!(mine.get(key).is_some(), "order key {key}");
        }
    }
}
