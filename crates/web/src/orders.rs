//! Customer orders (`label_fedex`, `label_usps`, `mail_kit`,
//! `address_label`): `customer_order`, `order_shipment`, `order_event`
//! in p2.sqlite, a backing `form_submission` per order, the state
//! machine, delivery options, and `billing:{customer_ref}` events.
//!
//! Orders are created and tracked, the free printable address label works end
//! to end, and every paid tier answers 503 `paid_tiers_paused` until C2
//! (Stripe Checkout) and the carrier ledger (SA-25 C3) land.
//!
//! Delivery options come only from accepted facts: the USPS destination
//! is a US postal address written in an accepted `mailing` or `po_box`
//! fact (an office with neither falls back to its `address` fact), a PO
//! box (or a cited "USPS only") disables FedEx and the card cites that
//! fact, and with no such fact every mail option is disabled with the
//! reason. A PO-box fact the public `/facts` read banners as in conflict
//! (the shared rule, [`db::facts_disagree`]) pauses every USPS kind
//! until the office confirms one box; FedEx reads only a cited courier
//! street, never the walk-in counter or a box.

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
pub fn can_transition(from: &str, to: &str) -> bool {
    let live = !matches!(from, "delivered" | "refunded" | "canceled");
    match (from, to) {
        (_, "needs_staff" | "canceled") => live,
        (_, "refunded") => live && from != "draft",
        ("draft", "quoted" | "labels_ready") => true,
        ("quoted", "awaiting_payment" | "quoted") => true,
        // An expired Checkout session hands the order back for re-quoting.
        ("awaiting_payment", "quoted") => true,
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
        CREATE INDEX IF NOT EXISTS order_event_order ON order_event(order_id, at);
        CREATE TABLE IF NOT EXISTS order_payment (
            order_id TEXT PRIMARY KEY,
            session_id TEXT NOT NULL UNIQUE,
            checkout_url TEXT NOT NULL,
            amount_cents INTEGER NOT NULL,
            currency TEXT NOT NULL,
            livemode INTEGER NOT NULL,
            status TEXT NOT NULL,
            payment_intent TEXT,
            created_at INTEGER NOT NULL,
            paid_at INTEGER,
            refund_id TEXT,
            FOREIGN KEY(order_id) REFERENCES customer_order(id)
        );
        CREATE TABLE IF NOT EXISTS stripe_event (
            id TEXT PRIMARY KEY,
            type TEXT NOT NULL,
            received_at INTEGER NOT NULL
        );",
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
    /// The street is a PO box, however it is spelled ([`PO_BOX`]).
    pub fn is_po_box(&self) -> bool {
        PO_BOX.is_match(&self.street)
    }
}

/// The PO box keyword however it is written: `PO Box`, `PO BOX`,
/// `P.O. Box`, `P.O.Box`, `P O Box`, `POBox`, `Post Office Box`.
static PO_BOX: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"(?i)\b(?:p\s*\.?\s*o\s*\.?\s*|post\s+office\s+)box(?:\b|[0-9])")
        .expect("PO box keyword compiles")
});

/// A PO box and its number: the keyword ([`PO_BOX`]), then an optional
/// `#`, `:`, `No.` or `Number`, then the digits (`PO Box #40234`,
/// `PO Box No. 40220`, `Post Office Box 40220`).
static PO_BOX_NUMBER: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(
        r"(?i)\b(?:p\s*\.?\s*o\s*\.?\s*|post\s+office\s+)box\s*(?:(?:#|:|no\b\.?|num\b\.?|number\b)\s*)?#?\s*([0-9]+)",
    )
    .expect("PO box number compiles")
});

/// The first PO box written in `text` with its number: the byte span
/// from the keyword through the digits, and the digits as written.
pub(crate) fn po_box_in(text: &str) -> Option<(std::ops::Range<usize>, &str)> {
    let c = PO_BOX_NUMBER.captures(text)?;
    Some((c.get(0)?.range(), c.get(1)?.as_str()))
}

/// `text` writes the PO box keyword, with or without a number.
pub(crate) fn names_box_keyword(text: &str) -> bool {
    PO_BOX.is_match(text)
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
/// PO box keyword (any spelling, [`PO_BOX`]: `Post Office Box 40220` is
/// a box, never a street named `40220`), else at its first digit.
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
        let start = PO_BOX
            .find(raw)
            .map(|m| m.start())
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
    /// The statement, then each citation quote on its own line.
    pub text: String,
    /// The public `/facts` read banners this fact: an accepted sibling
    /// disagrees with it under the shared rule ([`db::facts_disagree`],
    /// `FactView::conflict`).
    pub disputed: bool,
}

/// The USPS mail address and the fact it is quoted from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MailAddress {
    pub address: UsAddress,
    pub fact_id: i64,
}

/// Where mail for this authority goes, and why FedEx may not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Destination {
    /// Where USPS kinds ship; `None` when the office's `mailing` and
    /// `po_box` facts name no address the reader can place (USPS kinds
    /// are then off: the walk-in counter never stands in for them).
    pub mail: Option<MailAddress>,
    /// Fact that makes this route USPS-only (a PO box or a cited
    /// "USPS only"), if any.
    pub usps_only_fact: Option<i64>,
    /// Where a FedEx envelope goes: a street address that a `mailing`
    /// fact gives or that a fact cites for couriers (see
    /// [`destination`]); `None` disables FedEx.
    pub fedex: Option<CourierAddress>,
    /// Accepted USPS facts naming a PO box that another one contradicts
    /// (the shared conflict rule, or the address reader's box number or
    /// ZIP), mail facts first. While any is listed, no USPS kind ships:
    /// the mail address may be the wrong box.
    pub disputed: Vec<i64>,
}

impl Destination {
    /// Where an order of `kind` ships, with its building line and the
    /// fact it comes from: FedEx only to the courier address, never the
    /// mail address; a USPS kind only to an undisputed mail address.
    pub fn ship_to(&self, kind: &str) -> Option<(&UsAddress, Option<&str>, i64)> {
        if kind == "label_fedex" {
            return self
                .fedex
                .as_ref()
                .map(|c| (&c.address, c.line2.as_deref(), c.fact_id));
        }
        if !self.disputed.is_empty() {
            return None;
        }
        self.mail.as_ref().map(|m| (&m.address, None, m.fact_id))
    }
}

/// A street address FedEx can deliver to, with its building line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CourierAddress {
    pub address: UsAddress,
    pub line2: Option<String>,
    pub fact_id: i64,
}

/// Words that cite an address as taking courier deliveries.
const COURIER_WORDS: &[&str] = &[
    "courier",
    "fedex",
    "ups",
    "overnight",
    "express",
    "delivery service",
    "street address",
];

fn cites_courier(text: &str) -> bool {
    let t = text.to_lowercase();
    COURIER_WORDS.iter().any(|w| {
        t.match_indices(w).any(|(i, _)| {
            let before = t[..i]
                .chars()
                .next_back()
                .is_none_or(|c| !c.is_alphanumeric());
            let after = t[i + w.len()..]
                .chars()
                .next()
                .is_none_or(|c| !c.is_alphanumeric());
            before && after
        })
    })
}

/// Why every USPS kind (`label_usps`, `mail_kit`, `address_label`) is
/// off while the office's pages disagree on its PO box.
pub const DISPUTED_PO_BOX: &str = "The office's pages list different PO boxes. \
     Labels are paused for this office until it confirms which one is current.";

/// Why FedEx is off when the office's mail is a PO box and no fact
/// names a street for couriers (the card cites the box fact).
pub const PO_BOX_NO_COURIER: &str = "This office receives mail at a PO Box, which FedEx cannot \
     deliver to, and no approved fact names a street address for couriers.";

/// Why FedEx is off when no approved fact names a courier street: a
/// walk-in counter address alone never receives a customer's envelope.
pub const NO_COURIER_STREET: &str = "No approved fact names a street address for couriers at \
     this office.";

/// The PO box a `mailing` or `po_box` fact names, as the address reader
/// parses it ([`crate::label_autofill::address_in`]): (box number, ZIP),
/// the number without its leading zeros (`PO Box #040220` is `40220`),
/// or empty when the reader finds the keyword but no number (such a box
/// never matches another, so the pair reads as disputed). `None` for
/// other topics and for a street address.
fn usps_box(f: &FactText) -> Option<(String, String)> {
    if !db::USPS_TOPICS.contains(&f.topic.as_str()) {
        return None;
    }
    let address = crate::label_autofill::address_in(&f.text)?.address;
    if !address.is_po_box() {
        return None;
    }
    let number = po_box_in(&address.street).map_or("", |(_, digits)| {
        let trimmed = digits.trim_start_matches('0');
        if trimmed.is_empty() { "0" } else { trimmed }
    });
    Some((number.to_string(), address.zip))
}

/// Two parsed PO boxes are the same box: equal (read) numbers, and ZIPs
/// that agree under the shared conflict rule ([`db::zips_agree`]): a
/// five-digit ZIP agrees with the ZIP+4 that extends it (`98504` and
/// `98504-0228`), two different +4 codes do not.
fn same_box(a: &(String, String), b: &(String, String)) -> bool {
    !a.0.is_empty() && a.0 == b.0 && db::zips_agree(&a.1, &b.1)
}

/// A `mailing` or `po_box` fact that names a PO box: its topic is
/// `po_box`, the shared rule reads a `box:` token in its text
/// ([`db::value_tokens`]), or its address is a box. The dossier head's
/// JSON-LD postal check reads the same set (`unfurl::usps_postal`).
pub(crate) fn names_po_box(f: &FactText) -> bool {
    db::USPS_TOPICS.contains(&f.topic.as_str())
        && (f.topic == "po_box"
            || db::value_tokens(&f.text)
                .iter()
                .any(|t| t.starts_with("box:"))
            || crate::label_autofill::address_in(&f.text).is_some_and(|a| a.address.is_po_box()))
}

/// Mailing destination from accepted facts. Each fact is read line by
/// line (statement, then each citation quote), both `street, City, ST
/// ZIP` and `street City, ST ZIP` ([`crate::label_autofill::address_in`]).
///
/// USPS kinds ship to the first `mailing`, then `po_box`, fact that
/// reads as an address (that is where requests are mailed). Only an
/// office with no `mailing` or `po_box` fact at all falls back to its
/// `address` street; when those facts exist but none reads as an
/// address, USPS kinds are off rather than sent to the walk-in counter.
///
/// FedEx goes only to a street address (never a PO box, however it is
/// spelled) that a `mailing` fact gives or that a fact cites for
/// couriers (FedEx, UPS, overnight, express…); an `address` or
/// `dropbox` street that names no courier is a walk-in counter and
/// never receives a customer's envelope. A cited "USPS only" disables
/// FedEx outright.
///
/// The PO box is disputed when a `mailing`/`po_box` fact that names a
/// box is in conflict on the public read ([`FactText::disputed`], the
/// shared rule `/facts` banners with), and, as an extra fail-closed
/// check, when two such facts parse to different box numbers or ZIPs
/// (`PO BOX 40220 Olympia` and `P.O. Box 40220, Olympia` agree; a bare
/// `98504` agrees with `98504-0220`). Then no USPS kind ships, and
/// FedEx still reads only its courier street.
pub fn destination(facts: &[FactText]) -> Option<Destination> {
    let rank = |t: &str| match t {
        "mailing" => 0,
        "po_box" => 1,
        "address" => 2,
        "dropbox" => 3,
        _ => 9,
    };
    let mut candidates: Vec<&FactText> = facts.iter().filter(|f| rank(&f.topic) < 9).collect();
    candidates.sort_by_key(|f| (rank(&f.topic), f.id));
    let read = |f: &FactText| crate::label_autofill::address_in(&f.text);
    let usps = |f: &FactText| db::USPS_TOPICS.contains(&f.topic.as_str());
    let has_usps = candidates.iter().any(|f| usps(f));
    let mail = candidates
        .iter()
        .filter(|f| {
            if has_usps {
                usps(f)
            } else {
                f.topic == "address"
            }
        })
        .find_map(|f| {
            read(f).map(|found| MailAddress {
                address: found.address,
                fact_id: f.id,
            })
        });
    let cited_usps_only = facts
        .iter()
        .find(|f| {
            let t = f.text.to_lowercase();
            usps(f)
                && (t.contains("usps only")
                    || t.contains("u.s. postal service only")
                    || t.contains("only by usps"))
        })
        .map(|f| f.id);
    // The fact that makes this office's mail a box: the mail address
    // itself, a cited "USPS only", or (when no address reads at all) the
    // first USPS fact that names a box.
    let usps_only_fact = match &mail {
        Some(m) if m.address.is_po_box() => Some(m.fact_id),
        Some(_) => cited_usps_only,
        None => {
            cited_usps_only.or_else(|| candidates.iter().find(|f| names_po_box(f)).map(|f| f.id))
        }
    };
    let fedex = if cited_usps_only.is_some() {
        None
    } else {
        candidates.iter().find_map(|f| {
            let a = read(f)?;
            let courier = f.topic == "mailing" || cites_courier(&f.text);
            (!a.address.is_po_box() && courier).then_some(CourierAddress {
                address: a.address,
                line2: a.line2,
                fact_id: f.id,
            })
        })
    };
    let boxes: Vec<(i64, (String, String))> = candidates
        .iter()
        .filter_map(|f| Some((f.id, usps_box(f)?)))
        .collect();
    let parsed_dispute = |id: i64| {
        boxes
            .iter()
            .find(|(box_id, _)| *box_id == id)
            .is_some_and(|(_, a)| boxes.iter().any(|(_, b)| !same_box(a, b)))
    };
    let disputed: Vec<i64> = candidates
        .iter()
        .filter(|f| (f.disputed && names_po_box(f)) || parsed_dispute(f.id))
        .map(|f| f.id)
        .collect();
    if mail.is_none() && fedex.is_none() && usps_only_fact.is_none() && disputed.is_empty() {
        return None;
    }
    Some(Destination {
        mail,
        usps_only_fact: if fedex.is_some() {
            None
        } else {
            usps_only_fact
        },
        fedex,
        disputed,
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
    /// Customer FedEx labels can be quoted and bought now
    /// (`labels.customer_live`, FedEx keys, coherent mode).
    pub fedex_ready: bool,
    /// Customer USPS labels can be quoted and bought now.
    pub usps_ready: bool,
    /// `orders.ship_from` holds a valid fulfilment address (kits ship
    /// from it, so the kit card waits for it).
    pub ship_from_ok: bool,
    /// `orders.fee_label_cents`, added to every paid label order.
    pub label_fee_cents: i64,
}

/// Why a paid card is off while paid tiers are paused (POST answers
/// 503 `paid_tiers_paused` for it; every other refusal is a 409).
const PAID_PAUSED: &str = "Paid options are paused right now. The free address label still works.";

/// The four delivery cards (contracts `order-options.json`). A disputed
/// PO box turns off every USPS card, citing the first disputed fact,
/// and `destination` reads null: no USPS kind has one to use. So does
/// an office whose mailing facts name no address the reader can place.
pub fn options(slug: &str, dest: Option<&Destination>, sw: &Switches) -> Value {
    let paid_enabled = sw.orders_enabled && sw.stripe_key;
    let no_dest = "No staff-approved US mailing address for this office yet.";
    let card = |kind: &str, title: &str, from: Option<i64>| -> Value {
        let (enabled, reason, fact_id): (bool, Option<&str>, Option<i64>) = match (kind, dest) {
            (_, None) => (false, Some(no_dest), None),
            ("label_fedex", Some(d)) if d.fedex.is_none() => match d.usps_only_fact {
                Some(id) => (false, Some(PO_BOX_NO_COURIER), Some(id)),
                None => (false, Some(NO_COURIER_STREET), None),
            },
            ("label_usps" | "mail_kit" | "address_label", Some(d)) if !d.disputed.is_empty() => {
                (false, Some(DISPUTED_PO_BOX), d.disputed.first().copied())
            }
            ("label_usps" | "mail_kit" | "address_label", Some(d)) if d.mail.is_none() => {
                (false, Some(no_dest), None)
            }
            ("address_label", Some(_)) => (true, None, None),
            (_, Some(_)) if !paid_enabled => (false, Some(PAID_PAUSED), None),
            ("mail_kit", Some(_)) if from.is_none() || !sw.ship_from_ok => {
                (false, Some(PAID_PAUSED), None)
            }
            ("label_fedex", Some(_)) if !sw.fedex_ready => (false, Some(PAID_PAUSED), None),
            ("label_usps", Some(_)) if !sw.usps_ready => (false, Some(PAID_PAUSED), None),
            _ => (true, None, None),
        };
        json!({"kind": kind, "enabled": enabled, "title": title, "reason": reason,
               "fact_id": fact_id, "from_cents": if enabled { from } else { None }})
    };
    let mail = dest
        .filter(|d| d.disputed.is_empty())
        .and_then(|d| d.mail.as_ref());
    json!({
        "slug": slug,
        "mode": if sw.stripe_live { "live" } else { "test" },
        "paid_enabled": paid_enabled,
        "currency": if sw.currency.is_empty() { "usd" } else { sw.currency.as_str() },
        "destination": mail.map(|m| json!({
            "city": m.address.city, "region": m.address.state, "postal_code": m.address.zip,
            "po_box": m.address.is_po_box(), "fact_id": m.fact_id,
        })),
        "fedex_destination": dest.and_then(|d| d.fedex.as_ref()).map(|c| json!({
            "city": c.address.city, "region": c.address.state, "postal_code": c.address.zip,
            "fact_id": c.fact_id,
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
/// Network-keyed write limits: demo logins mint customer refs freely,
/// so per-customer buckets alone do not bound one caller (review #15).
static NET_LIMITS: LazyLock<crate::passkey::RateBuckets> =
    LazyLock::new(crate::passkey::RateBuckets::default);
/// Max orders one customer may create per rolling day.
const MAX_ORDERS_PER_DAY: i64 = 30;
/// A quote is honoured at checkout for this long.
const QUOTE_TTL_SECS: i64 = 1_800;
/// Checkout sessions live this long (Stripe minimum is 30 minutes).
const SESSION_TTL_SECS: i64 = 3_600;

fn err(status: StatusCode, error: &str, reason: Option<&str>) -> Response {
    (status, Json(json!({"error": error, "reason": reason}))).into_response()
}

fn paused() -> Response {
    paused_because(crate::customer_labels::PAUSED)
}

fn paused_because(reason: &str) -> Response {
    err(
        StatusCode::SERVICE_UNAVAILABLE,
        "paid_tiers_paused",
        Some(reason),
    )
}

/// The customer-paid carrier label tiers.
fn is_label(kind: &str) -> bool {
    matches!(kind, "label_fedex" | "label_usps")
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

/// Accepted facts of `slug` (statement + quotes), for destinations,
/// each with the conflict flag the public `/facts` read banners it with.
pub(crate) fn fact_texts(state: &AppState, meta: &Connection, slug: &str) -> Vec<FactText> {
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
                disputed: f.conflict,
            }
        })
        .collect()
}

fn switches(state: &AppState, meta: &Connection) -> Switches {
    let Some(kek) = state.kek.as_deref() else {
        return Switches::default();
    };
    let cache = &state.config_cache;
    // W-3: the fake Stripe needs no key and is never live money.
    let stripe = if state.fakes.is_fake(config::Integration::Stripe) {
        Some(b"fake".to_vec())
    } else {
        config::get_secret_for_job(meta, kek, config::keys::STRIPE_SECRET_KEY)
            .ok()
            .flatten()
    };
    Switches {
        orders_enabled: cache
            .bool(meta, kek, config::keys::ORDERS_ENABLED)
            .ok()
            .flatten()
            .unwrap_or(false),
        stripe_key: stripe.as_ref().is_some_and(|k| !k.is_empty()),
        stripe_live: stripe_live(&stripe),
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
        fedex_ready: crate::customer_labels::ready(
            state,
            meta,
            "label_fedex",
            stripe_live(&stripe),
        ),
        usps_ready: crate::customer_labels::ready(state, meta, "label_usps", stripe_live(&stripe)),
        ship_from_ok: ship_from(state, meta).is_some(),
        label_fee_cents: crate::customer_labels::fee_cents(state, meta),
    }
}

fn stripe_live(key: &Option<Vec<u8>>) -> bool {
    key.as_ref()
        .is_some_and(|k| k.starts_with(b"sk_live") || k.starts_with(b"rk_live"))
}

/// `orders.ship_from`: the fulfilment address kits ship from (one JSON
/// object in the [`ReturnTo`] shape, plus an optional `phone` carriers
/// use as the contact number). Invalid or unset reads as `None`.
pub(crate) fn ship_from(state: &AppState, meta: &Connection) -> Option<ReturnTo> {
    let kek = state.kek.as_deref()?;
    let raw = state
        .config_cache
        .text(meta, kek, config::keys::ORDERS_SHIP_FROM)
        .ok()
        .flatten()?;
    parse_ship_from(&raw)
}

pub(crate) fn parse_ship_from(raw: &str) -> Option<ReturnTo> {
    serde_json::from_str::<ReturnTo>(raw.trim())
        .ok()
        .filter(ReturnTo::valid)
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
    /// Optional contact number; carriers print it on labels.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phone: Option<String>,
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
            && self.phone.as_deref().is_none_or(|p| {
                // A US number: 10 digits, or 11 with the country code 1.
                let digits: String = p.chars().filter(char::is_ascii_digit).collect();
                (digits.len() == 10 || (digits.len() == 11 && digits.starts_with('1')))
                    && p.len() <= 24
                    && p.bytes()
                        .all(|b| b.is_ascii_digit() || b" +-().".contains(&b))
            })
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
#[cfg(test)]
pub fn create_order(
    p2: &Connection,
    customer_ref: &str,
    authority: (i64, &str),
    body: &CreateBody,
    dest: &Destination,
    mode: &str,
    now: i64,
) -> rusqlite::Result<String> {
    create_order_from(p2, customer_ref, authority, body, dest, None, mode, now)
}

/// [`create_order`] with the operator's ship-from address, which a
/// mailing kit ships from (`orders.ship_from`).
#[allow(clippy::too_many_arguments)]
pub fn create_order_from(
    p2: &Connection,
    customer_ref: &str,
    (authority_id, authority_name): (i64, &str),
    body: &CreateBody,
    dest: &Destination,
    ship_from: Option<&ReturnTo>,
    mode: &str,
    now: i64,
) -> rusqlite::Result<String> {
    // A FedEx order ships to the courier address, every other kind to
    // the mail address ([`Destination::ship_to`]). The options gate
    // refuses the rest first; this never falls back to the mail address
    // for FedEx, or to a disputed box for USPS.
    let Some((address, line2, fact_id)) = dest.ship_to(&body.kind) else {
        return Err(rusqlite::Error::ToSqlConversionFailure(Box::new(
            std::io::Error::other(format!("no {} destination for this office", body.kind)),
        )));
    };
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
    let mut dest_json = json!({
        "name": authority_name, "street1": address.street, "city": address.city,
        "state_province": address.state, "postal": address.zip, "country": "US",
        "fact_id": fact_id,
    });
    if let Some(line2) = line2 {
        dest_json["street2"] = json!(line2);
    }
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
    let address = |a: &ReturnTo| {
        json!({
            "name": a.name, "street1": a.line1, "street2": a.line2,
            "city": a.city, "state_province": a.region,
            "postal": a.postal_code, "country": a.country, "phone": a.phone,
        })
    };
    let customer = address(&body.return_to);
    let kit_from = ship_from.map_or_else(|| json!({"name": "Apostille Handbook"}), address);
    for (role, carrier, service) in legs_for(&body.kind) {
        let (from, to) = match *role {
            "return" => (&dest_json, &customer),
            "kit_to_customer" => (&kit_from, &customer),
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
                "tracking_url": match carrier.as_str() {
                    "fedex" => labels::tracking_url(labels::Carrier::FedEx, &tracking),
                    "usps" => labels::tracking_url(labels::Carrier::Usps, &tracking),
                    _ => None,
                },
                "tracking_number": (!tracking.is_empty()).then_some(tracking),
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
    if is_label(&kind) && status == "labels_ready" {
        actions.push(json!({"id": "print_labels", "label": "Print your labels"}));
    }
    Ok(Some(json!({
        "id": id, "kind": kind, "status": status, "mode": mode,
        "authority": {"slug": slug, "name": authority_name},
        "path_id": path_id, "created_at": created, "updated_at": updated,
        "amount": serde_json::from_str::<Value>(&amount).unwrap_or(json!({})),
        "return_to": {"name": ret["name"], "city": ret["city"], "region": ret["region"],
                      "postal_code": ret["postal_code"], "country": ret["country"]},
        "legs": legs, "form_fill_id": fill, "events": events,
        // A W-3 fake payment links its DEMO receipt and says so.
        "receipt": p2
            .query_row("SELECT paid_at, refund_id, session_id FROM order_payment WHERE order_id = ?1 AND status = 'paid'", [&id], |r| {
                let session: String = r.get(2)?;
                let demo = session.starts_with(crate::stripe::FAKE_SESSION_PREFIX);
                let receipt_url = demo.then(|| format!("/apostille/api/v1/stripe/fake/receipt/{session}"));
                Ok(json!({"paid_at": r.get::<_, i64>(0)?, "receipt_url": receipt_url, "refunded": r.get::<_, Option<String>>(1)?.is_some(), "demo": demo}))
            })
            .optional()?
            .unwrap_or(Value::Null),
        "actions": actions,
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
    let ip = crate::admin_ip::rate_key(crate::admin_ip::peer_ip(&req));
    if !READ_LIMITS.check_read(&format!("oo:{ip}"), unix_now(), 60, 60) {
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
    crate::admin_ip::PeerIp(peer): crate::admin_ip::PeerIp,
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
    // Network first: a flood of fresh demo refs is refused before it
    // can fill the per-customer map (re-review #9).
    if !NET_LIMITS.check_write(
        &format!("oc:{}", crate::admin_ip::rate_key(peer)),
        now,
        20,
        60,
    ) || !ORDER_LIMITS.check_write(&format!("oc:{customer_ref}"), now, 10, 60)
    {
        return err(StatusCode::TOO_MANY_REQUESTS, "rate limited", None);
    }
    if !KINDS.contains(&body.kind.as_str()) {
        return err(StatusCode::UNPROCESSABLE_ENTITY, "unknown kind", None);
    }
    let path_ok = body.path_id.as_deref().is_none_or(|p| {
        !p.is_empty()
            && p.len() <= 64
            && p.bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
    });
    let fill_ok = body.form_fill_id.as_deref().is_none_or(|f| {
        f.len() == 14
            && f.starts_with("fil_")
            && f[4..]
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
    });
    if !path_ok || !fill_ok {
        return err(
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid path or form fill",
            None,
        );
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
    let recent: i64 = p2
        .query_row(
            "SELECT COUNT(*) FROM customer_order WHERE customer_ref = ?1 AND created_at >= ?2",
            rusqlite::params![customer_ref, now - 86_400],
            |r| r.get(0),
        )
        .unwrap_or(i64::MAX);
    if recent >= MAX_ORDERS_PER_DAY {
        return err(StatusCode::TOO_MANY_REQUESTS, "too many orders today", None);
    }
    if let Some(fill) = body.form_fill_id.as_deref() {
        let mine = p2
            .query_row(
                "SELECT 1 FROM form_fill WHERE id = ?1 AND customer_ref = ?2",
                rusqlite::params![fill, customer_ref],
                |_| Ok(()),
            )
            .optional()
            .unwrap_or(None)
            .is_some();
        if !mine {
            return err(StatusCode::UNPROCESSABLE_ENTITY, "unknown form fill", None);
        }
    }
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
        if PAID.contains(&body.kind.as_str()) && card["reason"] == PAID_PAUSED {
            return paused();
        }
        return err(
            StatusCode::CONFLICT,
            "option_unavailable",
            card["reason"].as_str(),
        );
    }
    let mode = if sw.stripe_live { "live" } else { "test" };
    let kit_from = ship_from(&state, &meta);
    let id = match create_order_from(
        &p2,
        &customer_ref,
        (authority_id, &authority_name),
        &body,
        &dest,
        kit_from.as_ref(),
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

/// One bought carrier label PDF of an order leg (owner checked by the
/// caller).
fn carrier_label_pdf(state: &AppState, p2: &Connection, id: &str, leg: &str) -> Response {
    let sha: Option<String> = p2
        .query_row(
            "SELECT l.label_sha256 FROM order_shipment os
             JOIN shipping_label l ON l.shipment_id = os.shipment_id
             WHERE os.order_id = ?1 AND os.role = ?2 AND l.status = 'issued'
               AND l.label_sha256 != ''
             ORDER BY l.id DESC LIMIT 1",
            rusqlite::params![id, leg],
            |r| r.get(0),
        )
        .optional()
        .unwrap_or(None);
    let Some(bytes) = sha.and_then(|sha| Store::new(&state.store_root).read_blob(&sha).ok()) else {
        return err(StatusCode::NOT_FOUND, "no such label", None);
    };
    (
        [
            (header::CONTENT_TYPE, "application/pdf".to_string()),
            (header::CACHE_CONTROL, "private, no-store".to_string()),
            (
                header::CONTENT_DISPOSITION,
                format!("inline; filename=\"{id}-{leg}-label.pdf\""),
            ),
        ],
        bytes,
    )
        .into_response()
}

/// `GET /orders/{id}/labels/{leg}` — owner only. The free address label
/// is printable HTML marked NOT POSTAGE; carrier labels are the bought
/// PDFs.
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
    if owner == customer_ref && is_label(&kind) && matches!(leg.as_str(), "outbound" | "return") {
        return carrier_label_pdf(&state, &p2, &id, &leg);
    }
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

// ------------------------------------------------------------ money (C2)

/// Price lines for an order kind, from operator config only. Carrier
/// label tiers stay unpriced (paused) until live carrier rates land
/// (SA-25 CR1); the mailing kit is an operator-set all-in price.
pub fn price_lines(kind: &str, sw: &Switches) -> Option<Vec<(String, i64)>> {
    match kind {
        "mail_kit" => sw.kit_price_cents.map(|c| {
            vec![(
                "Mailing kit with prepaid envelopes both ways".to_string(),
                c,
            )]
        }),
        _ => None,
    }
}

/// Owner check + stores, or the error response.
fn owned(
    state: &AppState,
    headers: &HeaderMap,
    id: &str,
) -> Result<(Connection, Connection, String, String, String), Box<Response>> {
    let Some(customer_ref) = crate::p2::caller_customer_ref(state, headers) else {
        return Err(Box::new(err(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            None,
        )));
    };
    let Some((meta, p2)) = open_stores(state) else {
        return Err(Box::new(store_unavailable()));
    };
    match owner_of(&p2, id) {
        Some((owner, slug, kind)) if owner == customer_ref => {
            Ok((meta, p2, customer_ref, slug, kind))
        }
        _ => Err(Box::new(err(StatusCode::NOT_FOUND, "unknown order", None))),
    }
}

pub(crate) fn event(p2: &Connection, id: &str, kind: &str, text: &str, now: i64) {
    let _ = p2.execute(
        "INSERT INTO order_event(order_id, at, kind, text, leg) VALUES (?1, ?2, ?3, ?4, NULL)",
        rusqlite::params![id, now, kind, text],
    );
}

pub(crate) fn set_status(p2: &Connection, id: &str, from: &str, to: &str, now: i64) -> bool {
    can_transition(from, to)
        && p2
            .execute(
                "UPDATE customer_order SET status = ?1, updated_at = ?2 WHERE id = ?3 AND status = ?4",
                rusqlite::params![to, now, id, from],
            )
            .unwrap_or(0)
            == 1
}

/// `POST /orders/{id}/quote` — owner; prices from operator config.
async fn quote(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    if !crate::passkey::origin_ok(&headers, state.admin_mtls_off)
        || !crate::passkey::csrf_echo_ok(&headers)
    {
        return crate::passkey::csrf_fail();
    }
    let (meta, p2, _customer, _slug, kind) = match owned(&state, &headers, &id) {
        Ok(v) => v,
        Err(r) => return *r,
    };
    let sw = switches(&state, &meta);
    if !(sw.orders_enabled && sw.stripe_key) {
        return paused();
    }
    let current: String = p2
        .query_row(
            "SELECT status FROM customer_order WHERE id = ?1",
            [&id],
            |r| r.get(0),
        )
        .unwrap_or_default();
    if !matches!(current.as_str(), "draft" | "quoted") {
        return err(StatusCode::CONFLICT, "order cannot be quoted now", None);
    }
    // Label tiers price at the carrier (both legs) plus the service
    // fee; the kit is an operator-set price.
    let mut carrier_quotes: Vec<crate::customer_labels::LegQuote> = Vec::new();
    let lines = if is_label(&kind) {
        let (st, oid, k, live) = (state.clone(), id.clone(), kind.clone(), sw.stripe_live);
        match tokio::task::spawn_blocking(move || {
            crate::customer_labels::quote_order(&st, &oid, &k, live)
        })
        .await
        {
            Ok(Ok(quotes)) => {
                let lines = crate::customer_labels::label_lines(&quotes, sw.label_fee_cents);
                carrier_quotes = quotes;
                lines
            }
            Ok(Err(crate::customer_labels::QuoteError::Paused(why))) => {
                return paused_because(why);
            }
            Ok(Err(crate::customer_labels::QuoteError::Carrier(e))) => {
                tracing::warn!(order = %id, error = %e, "label quote failed at the carrier");
                return paused_because(
                    "The carrier could not price this shipment right now. Try again in a few minutes.",
                );
            }
            Err(_) => return store_unavailable(),
        }
    } else {
        let Some(lines) = price_lines(&kind, &sw) else {
            return paused();
        };
        lines
    };
    let total: i64 = lines.iter().map(|l| l.1).sum();
    let max = state
        .kek
        .as_deref()
        .and_then(|kek| {
            state
                .config_cache
                .text(&meta, kek, config::keys::ORDERS_MAX_ORDER_CENTS)
                .ok()
                .flatten()
        })
        .and_then(|v| v.trim().parse::<i64>().ok())
        .unwrap_or(15_000);
    if total > max {
        return paused();
    }
    let status: String = p2
        .query_row(
            "SELECT status FROM customer_order WHERE id = ?1",
            [&id],
            |r| r.get(0),
        )
        .unwrap_or_default();
    let now = unix_now();
    let currency = if sw.currency.is_empty() {
        "usd".to_string()
    } else {
        sw.currency.clone()
    };
    let amount = json!({"currency": currency, "total_cents": total, "quoted_at": now,
                        "lines": lines.iter().map(|(l, c)| json!({"label": l, "cents": c})).collect::<Vec<_>>(),
                        "carrier_quotes": carrier_quotes});
    if status == "draft" && !set_status(&p2, &id, "draft", "quoted", now) {
        return err(StatusCode::CONFLICT, "order cannot be quoted now", None);
    }
    if !matches!(status.as_str(), "draft" | "quoted") {
        return err(StatusCode::CONFLICT, "order cannot be quoted now", None);
    }
    if p2
        .execute(
            "UPDATE customer_order SET amount_json = ?1, updated_at = ?2
             WHERE id = ?3 AND status IN ('draft', 'quoted')",
            rusqlite::params![amount.to_string(), now, id],
        )
        .unwrap_or(0)
        != 1
    {
        return err(StatusCode::CONFLICT, "order cannot be quoted now", None);
    }
    Json(json!({"order_id": id, "mode": if sw.stripe_live { "live" } else { "test" }, "currency": amount["currency"],
                "total_cents": total, "expires_at": now + QUOTE_TTL_SECS, "lines": amount["lines"],
                "carrier_quotes": amount["carrier_quotes"]}))
    .into_response()
}

/// `POST /orders/{id}/checkout` — owner; hosted Checkout session.
/// Re-prices from config (a stale or changed quote must be re-quoted),
/// enforces the daily paid-order cap and max order, claims the order
/// before calling Stripe, and replaces a session that has expired.
async fn checkout(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    if !crate::passkey::origin_ok(&headers, state.admin_mtls_off)
        || !crate::passkey::csrf_echo_ok(&headers)
    {
        return crate::passkey::csrf_fail();
    }
    let (meta, p2, customer_ref, _slug, kind) = match owned(&state, &headers, &id) {
        Ok(v) => v,
        Err(r) => return *r,
    };
    let now = unix_now();
    if !ORDER_LIMITS.check_write(&format!("ok:{customer_ref}"), now, 10, 60) {
        return err(StatusCode::TOO_MANY_REQUESTS, "rate limited", None);
    }
    let sw = switches(&state, &meta);
    let Some(keys) = crate::stripe::keys(&state, &meta).filter(|_| sw.orders_enabled) else {
        return paused();
    };
    // The resolved identity decides, not a stray cookie (review #45):
    // demo_login only ever mints `demo-` refs.
    if keys.live() && customer_ref.starts_with("demo-") {
        return err(
            StatusCode::FORBIDDEN,
            "demo sessions never pay in live mode",
            None,
        );
    }
    let (status, amount, mode): (String, String, String) = match p2.query_row(
        "SELECT status, amount_json, mode FROM customer_order WHERE id = ?1",
        [&id],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    ) {
        Ok(v) => v,
        Err(_) => return store_unavailable(),
    };
    if (mode == "live") != keys.live() {
        return paused(); // order created under the other mode
    }
    let mut status = status;
    if status == "awaiting_payment" {
        let open: Option<(String, String, i64)> = p2
            .query_row(
                "SELECT session_id, checkout_url, created_at FROM order_payment
                 WHERE order_id = ?1 AND status = 'open'",
                [&id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()
            .unwrap_or(None);
        match open {
            Some((_, url, created)) if now - created < SESSION_TTL_SECS - 300 => {
                return Json(json!({"checkout_url": url})).into_response();
            }
            _ => {
                // Dead or missing session: let settle decide (it marks an
                // expired session and hands the order back to `quoted`).
                let (st, oid, k) = (state.clone(), id.clone(), keys.clone());
                match tokio::task::spawn_blocking(move || settle(&st, &k, &oid)).await {
                    Ok(Ok(Settled::Expired)) => status = "quoted".to_string(),
                    Ok(Ok(Settled::Paid | Settled::AlreadyPaid)) => {
                        return err(StatusCode::CONFLICT, "this order is already paid", None);
                    }
                    _ => {
                        return err(
                            StatusCode::CONFLICT,
                            "checkout is still open; try again shortly",
                            None,
                        );
                    }
                }
            }
        }
    }
    if status != "quoted" {
        return err(StatusCode::CONFLICT, "quote the order first", None);
    }
    let stored: Value = serde_json::from_str(&amount).unwrap_or(json!({}));
    let fresh = stored["quoted_at"]
        .as_i64()
        .is_some_and(|t| now - t <= QUOTE_TTL_SECS);
    let lines = if is_label(&kind) {
        let ready = if kind == "label_fedex" {
            sw.fedex_ready
        } else {
            sw.usps_ready
        };
        let quotes: Vec<crate::customer_labels::LegQuote> =
            serde_json::from_value(stored["carrier_quotes"].clone()).unwrap_or_default();
        if !ready || quotes.len() != 2 {
            return paused();
        }
        let caps = crate::customer_labels::caps(&state, &meta);
        let usps_full = kind == "label_usps"
            && sw.stripe_live
            && crate::usps::live_buys_today(&p2, now).saturating_add(2)
                > crate::usps::live_caps(&state, &meta).daily_cap;
        if usps_full
            || crate::customer_labels::buys_today(&p2, now).saturating_add(2) > caps.daily_cap
        {
            return paused_because(
                "Label buying reached today's limit. Try again tomorrow or choose another option.",
            );
        }
        // Carrier rates hold for the quote window only: an older quote is
        // priced again at the carrier, never renewed at the old rate.
        if !fresh {
            return err(
                StatusCode::CONFLICT,
                "quote_expired",
                Some("Shipping rates are held for 30 minutes. Review the new total and pay again."),
            );
        }
        crate::customer_labels::label_lines(&quotes, sw.label_fee_cents)
    } else {
        let Some(lines) = price_lines(&kind, &sw) else {
            return paused();
        };
        lines
    };
    let total: i64 = lines.iter().map(|l| l.1).sum();
    let currency = if sw.currency.is_empty() {
        "usd".to_string()
    } else {
        sw.currency.clone()
    };
    // The total is re-priced from config right here, so a stale quote
    // at the same price and currency is simply renewed; only a changed
    // price sends the customer back to review it (re-review #0).
    if stored["total_cents"].as_i64() != Some(total)
        || stored["currency"].as_str() != Some(currency.as_str())
    {
        return err(
            StatusCode::CONFLICT,
            "quote_expired",
            Some("Prices were updated. Review the new total and pay again."),
        );
    }
    if !fresh {
        let _ = p2.execute(
            "UPDATE customer_order SET amount_json = json_set(amount_json, '$.quoted_at', ?1)
             WHERE id = ?2 AND status = 'quoted'",
            rusqlite::params![now, id],
        );
    }
    let max = state
        .kek
        .as_deref()
        .and_then(|kek| {
            state
                .config_cache
                .text(&meta, kek, config::keys::ORDERS_MAX_ORDER_CENTS)
                .ok()
                .flatten()
        })
        .and_then(|v| v.trim().parse::<i64>().ok())
        .unwrap_or(15_000);
    let cap = state
        .kek
        .as_deref()
        .and_then(|kek| {
            state
                .config_cache
                .text(&meta, kek, config::keys::ORDERS_DAILY_CAP)
                .ok()
                .flatten()
        })
        .and_then(|v| v.trim().parse::<i64>().ok())
        .unwrap_or(20);
    // A soft cap: once this many payments were captured today in this
    // mode, no new checkout opens (sessions already open may still
    // complete). Abandoned sessions never consume it (re-review #1).
    let today: i64 = p2
        .query_row(
            "SELECT COUNT(*) FROM order_payment WHERE status = 'paid' AND paid_at >= ?1 AND livemode = ?2",
            rusqlite::params![now - now.rem_euclid(86_400), i64::from(mode == "live")],
            |r| r.get(0),
        )
        .unwrap_or(i64::MAX);
    if total > max || today >= cap {
        return paused();
    }
    // Claim first: only one checkout can move quoted → awaiting_payment,
    // and only while the stored quote is still the one priced above (a
    // re-quote racing this request must not change what Stripe charges).
    let quotes_json = serde_json::to_string(
        &stored["carrier_quotes"]
            .as_array()
            .cloned()
            .unwrap_or_default(),
    )
    .unwrap_or_else(|_| "[]".to_string());
    let claimed = p2
        .execute(
            "UPDATE customer_order SET status = 'awaiting_payment', updated_at = ?1
             WHERE id = ?2 AND status = 'quoted'
               AND json_extract(amount_json, '$.total_cents') = ?3
               AND COALESCE(json_extract(amount_json, '$.carrier_quotes'), '[]') = json(?4)",
            rusqlite::params![now, id, total, quotes_json],
        )
        .unwrap_or(0)
        == 1;
    if !claimed {
        return err(StatusCode::CONFLICT, "order changed; reload it", None);
    }
    let bucket = now / 600;
    let base = state.notify_base.trim_end_matches('/').to_string();
    let req = crate::stripe::CheckoutRequest {
        order_id: id.clone(),
        expires_at: bucket * 600 + SESSION_TTL_SECS,
        bucket,
        currency: currency.clone(),
        lines: lines.clone(),
        success_url: format!("{base}/orders/{id}?checkout=success"),
        cancel_url: format!("{base}/order?order={id}&step=pay&checkout=cancel"),
    };
    let created =
        tokio::task::spawn_blocking(move || crate::stripe::create_checkout(&keys, &req)).await;
    let (session_id, url) = match created {
        Ok(Ok(v)) => v,
        other => {
            if let Ok(Err(e)) = &other {
                tracing::warn!(order = %id, error = %e, "checkout: Stripe refused the session");
            }
            let _ = set_status(&p2, &id, "awaiting_payment", "quoted", now);
            return err(
                StatusCode::BAD_GATEWAY,
                "checkout unavailable",
                Some("Payment is unavailable right now. Please try again shortly."),
            );
        }
    };
    let ok = p2
        .execute(
            "INSERT INTO order_payment(order_id, session_id, checkout_url, amount_cents, currency, livemode, status, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'open', ?7)
             ON CONFLICT(order_id) DO UPDATE SET session_id = excluded.session_id,
                 checkout_url = excluded.checkout_url, amount_cents = excluded.amount_cents,
                 currency = excluded.currency, livemode = excluded.livemode, status = 'open',
                 created_at = excluded.created_at, paid_at = NULL, payment_intent = NULL
             WHERE order_payment.status != 'paid'",
            rusqlite::params![id, session_id, url, total, currency, i64::from(mode == "live"), now],
        )
        .unwrap_or(0)
        == 1;
    if !ok {
        return store_unavailable();
    }
    event(&p2, &id, "checkout.started", "Checkout opened", now);
    Json(json!({"checkout_url": url})).into_response()
}

/// What `settle` did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Settled {
    Paid,
    AlreadyPaid,
    NotPaidYet,
    /// The session expired unpaid; the order is back to `quoted`.
    Expired,
    Mismatch(String),
}

/// Settle one order against Stripe's own record of its session: paid
/// only when Stripe says paid AND order id, amount, currency and
/// livemode match. Idempotent and race-safe (the payment row flips
/// once). A matching payment is never dropped: if staff moved the
/// order meanwhile, the money is recorded and the order goes to
/// needs_staff (review #11). Publishes `order.paid` on `billing:` +
/// `ops`.
pub fn settle(
    state: &AppState,
    keys: &crate::stripe::Keys,
    order_id: &str,
) -> Result<Settled, String> {
    let store = Store::new(&state.store_root);
    let p2 = db::open_p2(&store).map_err(|e| e.to_string())?;
    let (session_id, amount, currency, livemode, pay_status): (String, i64, String, i64, String) = p2
        .query_row(
            "SELECT session_id, amount_cents, currency, livemode, status FROM order_payment WHERE order_id = ?1",
            [order_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .map_err(|_| "no checkout for this order".to_string())?;
    if pay_status == "paid" {
        return Ok(Settled::AlreadyPaid);
    }
    let session = crate::stripe::retrieve_session(keys, &session_id)?;
    let now = unix_now();
    if session["status"].as_str() == Some("expired")
        && session["payment_status"].as_str() != Some("paid")
    {
        let tx = p2.unchecked_transaction().map_err(|e| e.to_string())?;
        tx.execute(
            "UPDATE order_payment SET status = 'expired' WHERE order_id = ?1 AND status = 'open'",
            [order_id],
        )
        .map_err(|e| e.to_string())?;
        if set_status(&tx, order_id, "awaiting_payment", "quoted", now) {
            event(
                &tx,
                order_id,
                "checkout.expired",
                "Checkout timed out; review the total to pay",
                now,
            );
        }
        tx.commit().map_err(|e| e.to_string())?;
        return Ok(Settled::Expired);
    }
    let verdict = judge_session(&session, order_id, amount, &currency, livemode == 1);
    if let Settled::Mismatch(why) = &verdict {
        let _ = p2.execute(
            "UPDATE order_payment SET status = 'mismatch' WHERE order_id = ?1 AND status = 'open'",
            [order_id],
        );
        event(
            &p2,
            order_id,
            "order.needs_staff",
            &format!("Payment check failed: {why}"),
            now,
        );
        return Ok(verdict);
    }
    if verdict != Settled::Paid {
        return Ok(verdict);
    }
    let (customer_ref, kind, status): (String, String, String) = p2
        .query_row(
            "SELECT customer_ref, kind, status FROM customer_order WHERE id = ?1",
            [order_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .map_err(|e| e.to_string())?;
    let tx = p2.unchecked_transaction().map_err(|e| e.to_string())?;
    let flipped = tx
        .execute(
            "UPDATE order_payment SET status = 'paid', paid_at = ?1, payment_intent = ?2
             WHERE order_id = ?3 AND status != 'paid'",
            rusqlite::params![now, session["payment_intent"].as_str(), order_id],
        )
        .map_err(|e| e.to_string())?;
    if flipped == 0 {
        return Ok(Settled::AlreadyPaid); // a concurrent settle won
    }
    event(
        &tx,
        order_id,
        "order.paid",
        &format!(
            "Paid {} ({})",
            dollars(amount),
            if keys.fake.is_some() {
                "DEMO, no card charged"
            } else if livemode == 1 {
                "live"
            } else {
                "test mode"
            }
        ),
        now,
    );
    let mut buying = false;
    if set_status(&tx, order_id, &status, "paid", now) {
        // Kits go to the packing queue; carrier-label orders go to the
        // label buyer when customer buys are on, else to staff.
        buying =
            kind != "mail_kit" && crate::customer_labels::ready_now(state, &kind, livemode == 1);
        let (to, kind_event, text) = if kind == "mail_kit" {
            ("packing", "kit.queued", "Kit queued for packing")
        } else if buying {
            ("label_purchasing", "label.buying", "Buying your labels")
        } else {
            (
                "needs_staff",
                "order.needs_staff",
                "Our team is buying your labels",
            )
        };
        tx.execute(
            "UPDATE customer_order SET status = ?1, updated_at = ?2 WHERE id = ?3 AND status = 'paid'",
            rusqlite::params![to, now, order_id],
        )
        .map_err(|e| e.to_string())?;
        event(&tx, order_id, kind_event, text, now);
    } else {
        // Paid while staff had moved the order: keep the money on record
        // and hand it to staff (refund or fulfil), never drop it.
        let _ = tx.execute(
            "UPDATE customer_order SET status = 'needs_staff', updated_at = ?1 WHERE id = ?2
             AND status NOT IN ('refunded', 'delivered')",
            rusqlite::params![now, order_id],
        );
        event(
            &tx,
            order_id,
            "order.needs_staff",
            &format!("Payment received while the order was {status}"),
            now,
        );
    }
    tx.commit().map_err(|e| e.to_string())?;
    if buying {
        crate::customer_labels::POKE.notify_one();
    }
    let topic = format!("billing:{customer_ref}");
    state.bus.publish(
        &topic,
        json!({"type": "order.paid", "topic": topic, "order_id": order_id, "at": now}).to_string(),
    );
    crate::ws_bus::publish_ops(
        &state.bus,
        "order.paid",
        json!({"order_id": order_id, "kind": kind}),
        now,
    );
    Ok(Settled::Paid)
}

/// The money check, pure (unit-tested).
pub fn judge_session(
    session: &Value,
    order_id: &str,
    amount: i64,
    currency: &str,
    live: bool,
) -> Settled {
    if session["metadata"]["order_id"].as_str() != Some(order_id) {
        return Settled::Mismatch("session belongs to another order".into());
    }
    if session["payment_status"].as_str() != Some("paid") {
        return Settled::NotPaidYet;
    }
    if session["amount_total"].as_i64() != Some(amount) {
        return Settled::Mismatch("amount differs from the quote".into());
    }
    if session["currency"]
        .as_str()
        .map(str::to_lowercase)
        .as_deref()
        != Some(&currency.to_lowercase())
    {
        return Settled::Mismatch("currency differs".into());
    }
    if session["livemode"].as_bool() != Some(live) {
        return Settled::Mismatch("live/test mode differs".into());
    }
    Settled::Paid
}

fn dollars(cents: i64) -> String {
    format!("${}.{:02}", cents / 100, cents % 100)
}

/// `POST /orders/{id}/confirm` — owner; the success redirect lands
/// here so staging (no webhooks) and missed webhooks still settle.
async fn confirm(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    if !crate::passkey::origin_ok(&headers, state.admin_mtls_off)
        || !crate::passkey::csrf_echo_ok(&headers)
    {
        return crate::passkey::csrf_fail();
    }
    let (meta, p2, customer, slug, _kind) = match owned(&state, &headers, &id) {
        Ok(v) => v,
        Err(r) => return *r,
    };
    let open = p2
        .query_row(
            "SELECT 1 FROM order_payment WHERE order_id = ?1 AND status = 'open' AND created_at >= ?2",
            rusqlite::params![id, unix_now() - SESSION_TTL_SECS - 600],
            |_| Ok(()),
        )
        .optional()
        .unwrap_or(None)
        .is_some();
    // One Stripe read per confirm, throttled per customer (review #27).
    if open
        && ORDER_LIMITS.check_write(&format!("of:{customer}"), unix_now(), 10, 60)
        && let Some(keys) = crate::stripe::keys(&state, &meta)
    {
        let (st, oid) = (state.clone(), id.clone());
        let _ = tokio::task::spawn_blocking(move || settle(&st, &keys, &oid)).await;
    }
    let name = authority_of(&meta, &slug).map(|(_, n)| n).unwrap_or(slug);
    match order_view(&p2, &id, &name) {
        Ok(Some(v)) => Json(v).into_response(),
        _ => store_unavailable(),
    }
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
        .route("/orders/{id}/confirm", axum::routing::post(confirm))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(id: i64, topic: &str, text: &str) -> FactText {
        FactText {
            id,
            topic: topic.into(),
            text: text.into(),
            disputed: false,
        }
    }

    #[test]
    fn non_ascii_input_never_panics() {
        assert!(!is_zip("1234é"));
        assert!(!is_zip("é"));
        assert!(!is_zip("98501-é234"));
        // The slice must be the right one, not merely present (re-review
        // #37): Unicode lowercasing used to shift the offsets.
        let a = us_address_in("İİİ at PO Box 4000, Olympia, WA 98504").unwrap();
        assert!(a.street.ends_with("PO Box 4000"), "street {:?}", a.street);
        assert_eq!(
            (a.city.as_str(), a.state.as_str(), a.zip.as_str()),
            ("Olympia", "WA", "98504")
        );
        assert!(a.is_po_box());
        assert!(us_address_in("Hà Nội, Đống Đa, VN 100000").is_none());
        let bad = ReturnTo {
            name: "A".into(),
            line1: "1 A St".into(),
            line2: None,
            city: "B".into(),
            region: "WA".into(),
            postal_code: "1234é".into(),
            country: "US".into(),
            phone: None,
        };
        assert!(!bad.valid());
        let phone = |p: &str| ReturnTo {
            postal_code: "98504".into(),
            phone: Some(p.into()),
            ..bad.clone()
        };
        assert!(phone("+1 (360) 555-0100").valid());
        assert!(phone("360.555.0100").valid());
        assert!(!phone("call me").valid());
        assert!(!phone("12").valid());
        assert!(
            !phone("+44 20 7946 0000").valid(),
            "a US address takes a US number"
        );
        // The ship-from JSON parses into the same shape, or reads as unset.
        let ship = parse_ship_from(
            r#"{"name":"Example Co","line1":"1 Example Way","city":"Seattle","region":"WA","postal_code":"98101","phone":"+13605550100"}"#,
        )
        .expect("valid ship-from");
        assert_eq!(ship.phone.as_deref(), Some("+13605550100"));
        assert!(parse_ship_from(r#"{"name":"Example Co"}"#).is_none());
        assert!(parse_ship_from("not json").is_none());
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

    /// Washington SOS as accepted on production (P417): statement and
    /// citation quote on separate lines, no comma after the street.
    fn wa_facts() -> Vec<FactText> {
        vec![
            f(
                2,
                "mailing",
                "apostille_mailing_address: PO BOX 40228 Olympia, WA 98504-0228\n\
                 PO BOX 40228 Olympia, WA 98504-0228",
            ),
            f(
                5,
                "address",
                "apostille_main_branch_street: Town Center 3 243 Israel Rd SE Tumwater, WA 98501",
            ),
            f(7, "contacts", "apostille_phone: 360-725-0377"),
        ]
    }

    #[test]
    fn wa_mail_destination_reads_the_po_box_without_garbling_the_city() {
        let d = destination(&wa_facts()).unwrap();
        let mail = d.mail.clone().unwrap();
        assert_eq!(mail.address.street, "PO Box 40228");
        assert_eq!(mail.address.city, "Olympia");
        assert_eq!(mail.address.state, "WA");
        assert_eq!(mail.address.zip, "98504-0228");
        assert_eq!(mail.fact_id, 2);
        // The Tumwater counter is a walk-in address: no courier cite, no FedEx.
        assert_eq!(d.fedex, None);
        assert_eq!(d.usps_only_fact, Some(2));
        let v = options("usa-wa-sos", Some(&d), &ready());
        let fedex = card(&v, "label_fedex");
        assert_eq!(fedex["enabled"], false);
        assert_eq!(fedex["fact_id"], 2);
        assert_eq!(card(&v, "label_usps")["enabled"], true);
        assert_eq!(v["destination"]["city"], "Olympia");
        assert!(v["fedex_destination"].is_null());
    }

    #[test]
    fn a_po_box_quoted_over_two_lines_is_still_the_destination() {
        // Staging's WA facts: the quote breaks after the box number.
        let facts = [
            f(
                10,
                "mailing",
                "submission_requirement: Send all documents with Request form and payment:",
            ),
            f(
                25,
                "po_box",
                "po_box: PO Box 40220\nPO Box 40220  \n Olympia, WA 98504-0220",
            ),
        ];
        let d = destination(&facts).unwrap();
        let mail = d.mail.clone().unwrap();
        assert_eq!(
            (mail.address.street.as_str(), mail.address.city.as_str()),
            ("PO Box 40220", "Olympia")
        );
        assert_eq!(mail.fact_id, 25);
        assert_eq!(d.fedex, None);
    }

    #[test]
    fn a_cited_courier_street_takes_fedex_while_usps_keeps_the_po_box() {
        let mut facts = wa_facts();
        facts.push(f(
            9,
            "address",
            "For FedEx or UPS deliveries use 801 Capitol Way S, Olympia, WA 98501.",
        ));
        let d = destination(&facts).unwrap();
        assert_eq!(d.mail.as_ref().unwrap().address.street, "PO Box 40228");
        let c = d.fedex.clone().unwrap();
        assert_eq!(c.address.street, "801 Capitol Way S");
        assert_eq!(c.fact_id, 9);
        let v = options("usa-wa-sos", Some(&d), &ready());
        assert_eq!(card(&v, "label_fedex")["enabled"], true);
        assert_eq!(v["fedex_destination"]["fact_id"], 9);
        assert_eq!(v["destination"]["fact_id"], 2);
    }

    #[test]
    fn a_street_mailing_address_serves_both_carriers_unless_usps_only() {
        let street = [f(
            3,
            "mailing",
            "Mail to 801 Capitol Way S, Olympia, WA 98501.",
        )];
        let d = destination(&street).unwrap();
        assert_eq!(d.fedex.as_ref().unwrap().fact_id, 3);
        assert_eq!(d.usps_only_fact, None);
        let usps_only = [
            street[0].clone(),
            f(4, "mailing", "Requests are accepted by USPS only."),
        ];
        let d = destination(&usps_only).unwrap();
        assert_eq!(d.fedex, None);
        assert_eq!(d.usps_only_fact, Some(4));
        assert!(!cites_courier("Supply a stamped envelope."));
        assert!(cites_courier(
            "Overnight (FedEx) packages: 801 Capitol Way S"
        ));
    }

    /// Washington with three accepted USPS statements that disagree
    /// (fixture boxes; #23 pairs box 40220 with the other box's ZIP+4),
    /// plus the Tumwater walk-in counter.
    fn disputed_wa_facts() -> Vec<FactText> {
        vec![
            f(
                21,
                "po_box",
                "po_box: PO Box 40220\nPO Box 40220\nOlympia, WA 98504-0220",
            ),
            f(
                22,
                "mailing",
                "apostille_mailing_address: PO Box 40234 Olympia, WA 98504-0234",
            ),
            f(
                23,
                "po_box",
                "Mail requests to PO Box 40220, Olympia, WA 98504-0234.",
            ),
            f(
                5,
                "address",
                "apostille_main_branch_street: Town Center 3 243 Israel Rd SE Tumwater, WA 98501",
            ),
        ]
    }

    const DISPUTED_TEXT: &str = "The office's pages list different PO boxes. Labels are paused for this office until it confirms which one is current.";

    #[test]
    fn different_po_boxes_pause_every_usps_kind_and_refuse_its_orders() {
        assert_eq!(DISPUTED_PO_BOX, DISPUTED_TEXT);
        let d = destination(&disputed_wa_facts()).unwrap();
        assert_eq!(d.disputed, [22, 21, 23], "mail facts first, then by id");
        // Ready everywhere and paused alike: the dispute wins, citing #22.
        for sw in [ready(), Switches::default()] {
            let v = options("usa-wa-sos", Some(&d), &sw);
            for kind in ["label_usps", "mail_kit", "address_label"] {
                let c = card(&v, kind);
                assert_eq!(c["enabled"], false, "{kind}");
                assert_eq!(c["reason"], DISPUTED_TEXT, "{kind}");
                assert_eq!(c["fact_id"], 22, "{kind}");
                assert!(c["from_cents"].is_null(), "{kind}");
            }
            assert!(v["destination"].is_null(), "no USPS destination to use");
            // FedEx reads only a courier street: none is cited, so it is
            // off with its own reason, never a box or the walk-in counter.
            assert!(v["fedex_destination"].is_null());
            let fedex = card(&v, "label_fedex");
            assert_eq!(fedex["enabled"], false);
            assert_eq!(
                fedex["reason"],
                "This office receives mail at a PO Box, which FedEx cannot deliver to, and no \
                 approved fact names a street address for couriers."
            );
        }
        // The order path never writes a disputed box (nor FedEx without
        // a courier street).
        let (_tmp, p2) = p2_store();
        for kind in KINDS {
            assert!(
                create_order(&p2, "cust-1", (7, "S"), &body(kind), &d, "test", 1).is_err(),
                "{kind}"
            );
        }
        let n: i64 = p2
            .query_row("SELECT COUNT(*) FROM customer_order", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 0);
    }

    #[test]
    fn punctuation_variants_of_one_po_box_agree() {
        let facts = [
            f(
                2,
                "mailing",
                "apostille_mailing_address: PO BOX 40220 Olympia, WA 98504-0220\n\
                 PO BOX 40220 Olympia, WA 98504-0220",
            ),
            f(25, "po_box", "P.O. Box 40220, Olympia, WA 98504-0220."),
        ];
        let d = destination(&facts).unwrap();
        assert!(d.disputed.is_empty(), "{:?}", d.disputed);
        let v = options("usa-wa-sos", Some(&d), &ready());
        for kind in ["label_usps", "mail_kit", "address_label"] {
            assert_eq!(card(&v, kind)["enabled"], true, "{kind}");
        }
        assert_eq!(v["destination"]["postal_code"], "98504-0220");
        assert_eq!(v["destination"]["fact_id"], 2);
    }

    #[test]
    fn a_five_digit_zip_agrees_with_its_zip_plus_four_only() {
        let po = |id, zip: &str| f(id, "po_box", &format!("PO Box 40220, Olympia, WA {zip}"));
        let d = destination(&[po(1, "98504"), po(2, "98504-0220")]).unwrap();
        assert!(d.disputed.is_empty(), "same box, one ZIP refines the other");
        let d = destination(&[po(1, "98504"), po(2, "98507-0220")]).unwrap();
        assert_eq!(d.disputed, [1, 2], "another ZIP is another box");
        // Box numbers compare as numbers; streets and other topics never.
        let d = destination(&[
            po(1, "98504-0220"),
            f(2, "po_box", "PO Box 040220, Olympia, WA 98504-0220"),
            f(
                3,
                "mailing",
                "Mail to 801 Capitol Way S, Olympia, WA 98501.",
            ),
            f(4, "address", "PO Box 40234, Olympia, WA 98504-0234"),
        ])
        .unwrap();
        assert!(d.disputed.is_empty(), "{:?}", d.disputed);
    }

    #[test]
    fn couriers_read_only_the_courier_street_while_the_box_is_disputed() {
        let mut facts = disputed_wa_facts();
        facts.push(f(
            30,
            "mailing",
            "courier: Overnight or courier deliveries go to 801 Capitol Way S, Olympia, WA 98501.",
        ));
        let d = destination(&facts).unwrap();
        assert_eq!(d.disputed, [22, 21, 23], "the courier street is no box");
        let c = d.fedex.clone().unwrap();
        assert_eq!(
            (c.address.street.as_str(), c.fact_id),
            ("801 Capitol Way S", 30)
        );
        let v = options("usa-wa-sos", Some(&d), &ready());
        assert_eq!(card(&v, "label_fedex")["enabled"], true);
        assert_eq!(v["fedex_destination"]["fact_id"], 30);
        assert_eq!(v["fedex_destination"]["postal_code"], "98501");
        assert_eq!(card(&v, "label_usps")["reason"], DISPUTED_TEXT);
        // The FedEx order ships to that street, never to a PO box.
        let (_tmp, p2) = p2_store();
        let id =
            create_order(&p2, "cust-1", (7, "S"), &body("label_fedex"), &d, "test", 1).unwrap();
        let to: String = p2
            .query_row(
                "SELECT dest_json FROM customer_order WHERE id = ?1",
                [&id],
                |r| r.get(0),
            )
            .unwrap();
        let to: Value = serde_json::from_str(&to).unwrap();
        assert_eq!(
            (&to["street1"], &to["postal"], &to["fact_id"]),
            (&json!("801 Capitol Way S"), &json!("98501"), &json!(30))
        );
        // Without a disputed box the mail address stays USPS's own.
        let mut calm = wa_facts();
        calm.push(facts.pop().unwrap());
        let d = destination(&calm).unwrap();
        assert!(d.disputed.is_empty());
        assert_eq!(d.ship_to("label_usps").map(|t| t.2), Some(2));
        assert_eq!(d.ship_to("label_fedex").map(|t| t.2), Some(30));
    }

    /// App state over `root` with no config: paid tiers read as paused.
    fn test_state(root: &std::path::Path) -> AppState {
        AppState {
            spa_index: std::path::PathBuf::from("test-spa/index.html"),
            static_dir: std::path::PathBuf::from("static"),
            store_root: root.to_path_buf(),
            bus: crate::ws_bus::Bus::new(),
            kek: Some(std::sync::Arc::new(config::Kek::generate())),
            admin_mtls_off: true,
            admin_allow_ips: config::loopback_allow_ips(),
            admin_spa_dir: root.join("admin-spa"),
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
            fakes: config::Modes::default(),
            fedex_live: crate::fedex_live::FedexLiveState::default(),
        }
    }

    #[tokio::test]
    async fn the_order_routes_refuse_a_disputed_box() {
        use axum::{body::Body, http::Request};
        use tower::ServiceExt as _;
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::new(tmp.path());
        store.init().unwrap();
        let conn = db::open(&store).unwrap();
        db::migrate(&conn).unwrap();
        config::migrate_config_tables(&conn).unwrap();
        let authority = db::add_authority(
            &conn,
            &db::NewAuthority {
                kind: "state",
                country: "USA",
                region: "Washington",
                name: "Synthetic WA SoS",
                slug: "usa-wa-sos",
                actor: "t",
                at: 1,
            },
        )
        .unwrap();
        for fact in disputed_wa_facts() {
            conn.execute(
                "INSERT INTO fact(authority_id, capture_id, topic, statement, status, created_by,
                                  created_at)
                 VALUES (?1, 0, ?2, ?3, 'accepted', 't', 1)",
                rusqlite::params![authority, fact.topic, fact.text],
            )
            .unwrap();
        }
        db::create_demo_session(&conn, "tok-disputed", "cust-disputed", 1, i64::MAX).unwrap();
        drop(conn);
        let app = p2_routes().with_state(test_state(tmp.path()));
        let read = |res: Response| async move {
            let bytes = axum::body::to_bytes(res.into_body(), 1 << 20)
                .await
                .unwrap();
            serde_json::from_slice::<Value>(&bytes).unwrap()
        };
        let res = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/orders/options?slug=usa-wa-sos")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let v = read(res).await;
        for kind in ["label_usps", "mail_kit", "address_label"] {
            assert_eq!(card(&v, kind)["reason"], DISPUTED_TEXT, "{kind}");
        }
        assert!(v["destination"].is_null());
        for kind in ["label_usps", "mail_kit", "address_label"] {
            let body = json!({
                "slug": "usa-wa-sos", "kind": kind,
                "return_to": {"name": "SYNTHETIC Customer", "line1": "1 Example St",
                              "city": "Springfield", "region": "IL", "postal_code": "62701"},
            });
            let req = Request::builder()
                .method("POST")
                .uri("/orders")
                .header("origin", "https://andrii.ai")
                .header("content-type", "application/json")
                .header(auth::CSRF_HEADER, "tok")
                .header(
                    "cookie",
                    format!("demo_session=tok-disputed; {}=tok", auth::CSRF_COOKIE),
                )
                .body(Body::from(body.to_string()))
                .unwrap();
            let res = app.clone().oneshot(req).await.unwrap();
            assert_eq!(res.status(), StatusCode::CONFLICT, "{kind}");
            let v = read(res).await;
            assert_eq!(
                (&v["error"], &v["reason"]),
                (&json!("option_unavailable"), &json!(DISPUTED_TEXT)),
                "{kind}"
            );
        }
        let p2 = db::open_p2(&store).unwrap();
        let orders: i64 = p2
            .query_row("SELECT COUNT(*) FROM customer_order", [], |r| r.get(0))
            .unwrap();
        assert_eq!(orders, 0, "no order ships to a disputed box");
    }

    /// `facts` with the conflict flag `/facts` would give each one: the
    /// shared rule over the statements (each text's first line).
    fn public_flags(mut facts: Vec<FactText>) -> Vec<FactText> {
        let statement = |f: &FactText| f.text.lines().next().unwrap_or("").to_string();
        let flags: Vec<bool> = facts
            .iter()
            .map(|a| {
                facts.iter().any(|b| {
                    a.id != b.id
                        && db::facts_disagree(&a.topic, &statement(a), &b.topic, &statement(b))
                })
            })
            .collect();
        for (fact, flag) in facts.iter_mut().zip(flags) {
            fact.disputed = flag;
        }
        facts
    }

    #[test]
    fn a_box_the_reader_cannot_place_is_still_disputed_by_the_shared_rule() {
        let mailing = || {
            f(
                2,
                "mailing",
                "apostille_mailing_address: PO BOX 40220 Olympia, WA 98504-0220\n\
                 PO BOX 40220 Olympia, WA 98504-0220",
            )
        };
        for (case, other) in [
            (
                "no city or ZIP anywhere",
                f(
                    9,
                    "po_box",
                    "po_box: PO Box 40234\nMail apostille requests to PO Box 40234.",
                ),
            ),
            (
                "no comma before the state",
                f(
                    9,
                    "po_box",
                    "po_box: PO Box 40234\nApostilles, PO Box 40234, Olympia WA 98504-0234",
                ),
            ),
            (
                "both boxes in one statement",
                f(
                    9,
                    "mailing",
                    "mail_instruction: PO Box 40220, Olympia, WA 98504-0220 or PO Box 40234, \
                     Olympia, WA 98504-0234",
                ),
            ),
            (
                "a hash before the number",
                f(9, "po_box", "po_box: PO Box #40234, Olympia, WA 98504-0234"),
            ),
        ] {
            let facts = public_flags(vec![mailing(), other]);
            assert!(
                facts.iter().all(|f| f.disputed),
                "{case}: /facts banners both"
            );
            let d = destination(&facts).unwrap();
            assert_eq!(d.disputed, [2, 9], "{case}");
            for kind in ["label_usps", "mail_kit", "address_label"] {
                assert!(d.ship_to(kind).is_none(), "{case}: {kind}");
            }
            let v = options("usa-wa-sos", Some(&d), &ready());
            for kind in ["label_usps", "mail_kit", "address_label"] {
                let c = card(&v, kind);
                assert_eq!(c["enabled"], false, "{case}: {kind}");
                assert_eq!(c["reason"], DISPUTED_TEXT, "{case}: {kind}");
                assert_eq!(c["fact_id"], 2, "{case}: {kind}");
            }
            assert!(v["destination"].is_null(), "{case}");
        }
        // The address reader alone also reads `#40234` (fail-closed extra).
        let unflagged = [
            mailing(),
            f(9, "po_box", "po_box: PO Box #40234, Olympia, WA 98504-0234"),
        ];
        assert_eq!(destination(&unflagged).unwrap().disputed, [2, 9]);
        // `No.` before the number is the same box, not box "No".
        let same = [
            mailing(),
            f(
                9,
                "po_box",
                "po_box: PO Box No. 40220, Olympia, WA 98504-0220",
            ),
        ];
        assert!(destination(&same).unwrap().disputed.is_empty());
        // A flagged USPS fact that names no box (a courier street) never
        // pauses USPS: the shared rule never compares it with a box.
        let mut courier = f(
            30,
            "mailing",
            "courier: Overnight deliveries go to 801 Capitol Way S, Olympia, WA 98501.",
        );
        courier.disputed = true;
        let d = destination(&[mailing(), courier]).unwrap();
        assert!(d.disputed.is_empty());
        assert_eq!(d.ship_to("label_usps").map(|t| t.2), Some(2));
    }

    #[test]
    fn couriers_never_take_the_walk_in_counter_or_a_box_spelled_out() {
        let counter = || {
            f(
                5,
                "address",
                "apostille_main_branch_street: Town Center 3 243 Israel Rd SE Tumwater, WA 98501",
            )
        };
        // A box the reader cannot place: USPS is off (never the counter),
        // and FedEx is off citing the box.
        let unplaced = f(
            9,
            "po_box",
            "po_box: PO Box 40234\nMail requests to PO Box 40234.",
        );
        let d = destination(&[unplaced.clone(), counter()]).unwrap();
        assert_eq!(d.mail, None);
        assert_eq!(d.fedex, None);
        assert_eq!(d.usps_only_fact, Some(9));
        for kind in KINDS {
            assert!(d.ship_to(kind).is_none(), "{kind}");
        }
        let v = options("usa-wa-sos", Some(&d), &ready());
        for kind in ["label_usps", "mail_kit", "address_label"] {
            let c = card(&v, kind);
            assert_eq!(c["enabled"], false, "{kind}");
            assert_eq!(
                c["reason"], "No staff-approved US mailing address for this office yet.",
                "{kind}"
            );
        }
        assert_eq!(card(&v, "label_fedex")["reason"], PO_BOX_NO_COURIER);
        assert_eq!(card(&v, "label_fedex")["fact_id"], 9);
        assert!(v["destination"].is_null() && v["fedex_destination"].is_null());
        // A street cited for couriers takes FedEx; USPS stays off.
        let cited = f(
            12,
            "address",
            "For FedEx or UPS deliveries use 801 Capitol Way S, Olympia, WA 98501.",
        );
        let d = destination(&[unplaced, counter(), cited]).unwrap();
        assert_eq!(d.ship_to("label_fedex").map(|t| t.2), Some(12));
        assert!(d.ship_to("label_usps").is_none());
        // An office with only its walk-in counter: USPS mail may go there,
        // FedEx never does.
        let d = destination(&[counter()]).unwrap();
        assert_eq!(d.ship_to("label_usps").map(|t| t.2), Some(5));
        assert_eq!(d.fedex, None);
        let v = options("usa-wa-sos", Some(&d), &ready());
        let fedex = card(&v, "label_fedex");
        assert_eq!(
            (&fedex["enabled"], &fedex["reason"], &fedex["fact_id"]),
            (&json!(false), &json!(NO_COURIER_STREET), &Value::Null)
        );
        assert_eq!(card(&v, "label_usps")["enabled"], true);
        let (_tmp, p2) = p2_store();
        assert!(create_order(&p2, "c", (7, "S"), &body("label_fedex"), &d, "test", 1).is_err());
        // Spelled-out boxes are boxes: never a FedEx street.
        for written in [
            "apostille_mailing_address: Post Office Box 40220, Olympia, WA 98504-0220",
            "apostille_mailing_address: P.O.Box 40220, Olympia, WA 98504-0220",
            "apostille_mailing_address: P. O. Box 40220, Olympia, WA 98504-0220",
            "apostille_mailing_address: Post Office Box 40220 Olympia, WA 98504-0220",
            "apostille_mailing_address: PO Box #40220 Olympia, WA 98504-0220",
        ] {
            let d = destination(&[f(2, "mailing", written), counter()]).unwrap();
            let mail = d.mail.clone().unwrap();
            assert!(mail.address.is_po_box(), "{written}: {:?}", mail.address);
            assert_eq!(
                (mail.address.city.as_str(), mail.address.zip.as_str()),
                ("Olympia", "98504-0220"),
                "{written}"
            );
            assert_eq!(d.fedex, None, "{written}");
            assert_eq!(d.usps_only_fact, Some(2), "{written}");
            assert!(d.disputed.is_empty(), "{written}");
        }
    }

    #[tokio::test]
    async fn options_gate_the_po_box_facts_the_public_read_banners() {
        use axum::{body::Body, http::Request};
        use tower::ServiceExt as _;
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::new(tmp.path());
        store.init().unwrap();
        let conn = db::open(&store).unwrap();
        db::migrate(&conn).unwrap();
        config::migrate_config_tables(&conn).unwrap();
        let mut ids = Vec::new();
        for (slug, facts) in [
            (
                "usa-wa-sos",
                vec![
                    (
                        "mailing",
                        "apostille_mailing_address: PO BOX 40220 Olympia, WA 98504-0220",
                    ),
                    // Neither address reader can place this box.
                    ("po_box", "po_box: PO Box 40234"),
                    (
                        "address",
                        "apostille_main_branch_street: Town Center 3 243 Israel Rd SE \
                         Tumwater, WA 98501",
                    ),
                ],
            ),
            (
                "usa-xx-sos",
                vec![(
                    "address",
                    "office: Walk-in at 100 Fixture Way, Olympia, WA 98501.",
                )],
            ),
        ] {
            let authority = db::add_authority(
                &conn,
                &db::NewAuthority {
                    kind: "state",
                    country: "USA",
                    region: slug,
                    name: "Synthetic SoS",
                    slug,
                    actor: "t",
                    at: 1,
                },
            )
            .unwrap();
            for (topic, statement) in facts {
                conn.execute(
                    "INSERT INTO fact(authority_id, capture_id, topic, statement, status,
                                      created_by, created_at)
                     VALUES (?1, 0, ?2, ?3, 'accepted', 't', 1)",
                    rusqlite::params![authority, topic, statement],
                )
                .unwrap();
                ids.push(conn.last_insert_rowid());
            }
        }
        // `/facts` banners the mailing box and the box it cannot place.
        let views = crate::approval::accepted_fact_views(&conn, tmp.path(), "usa-wa-sos").unwrap();
        let banned: Vec<i64> = views.iter().filter(|v| v.conflict).map(|v| v.id).collect();
        assert_eq!(banned, [ids[0], ids[1]]);
        db::create_demo_session(&conn, "tok-banner", "cust-banner", 1, i64::MAX).unwrap();
        drop(conn);
        let app = p2_routes().with_state(test_state(tmp.path()));
        let read = |res: Response| async move {
            let status = res.status();
            let bytes = axum::body::to_bytes(res.into_body(), 1 << 20)
                .await
                .unwrap();
            (status, serde_json::from_slice::<Value>(&bytes).unwrap())
        };
        let get = |slug: &str| {
            Request::builder()
                .uri(format!("/orders/options?slug={slug}"))
                .body(Body::empty())
                .unwrap()
        };
        let (status, v) = read(app.clone().oneshot(get("usa-wa-sos")).await.unwrap()).await;
        assert_eq!(status, StatusCode::OK);
        for kind in ["label_usps", "mail_kit", "address_label"] {
            assert_eq!(card(&v, kind)["reason"], DISPUTED_TEXT, "{kind}");
            assert_eq!(card(&v, kind)["fact_id"], ids[0], "{kind}");
        }
        assert!(v["destination"].is_null());
        assert_eq!(card(&v, "label_fedex")["reason"], PO_BOX_NO_COURIER);
        let post = |slug: &str, kind: &str| {
            let body = json!({
                "slug": slug, "kind": kind,
                "return_to": {"name": "SYNTHETIC Customer", "line1": "1 Example St",
                              "city": "Springfield", "region": "IL", "postal_code": "62701"},
            });
            Request::builder()
                .method("POST")
                .uri("/orders")
                .header("origin", "https://andrii.ai")
                .header("content-type", "application/json")
                .header(auth::CSRF_HEADER, "tok")
                .header(
                    "cookie",
                    format!("demo_session=tok-banner; {}=tok", auth::CSRF_COOKIE),
                )
                .body(Body::from(body.to_string()))
                .unwrap()
        };
        let (status, v) = read(
            app.clone()
                .oneshot(post("usa-wa-sos", "address_label"))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(v["reason"], DISPUTED_TEXT);
        // A walk-in-only office: FedEx is refused with its reason (409,
        // not the paused 503); the free label still ships there.
        let (_, v) = read(app.clone().oneshot(get("usa-xx-sos")).await.unwrap()).await;
        assert_eq!(card(&v, "label_fedex")["reason"], NO_COURIER_STREET);
        let (status, v) = read(
            app.clone()
                .oneshot(post("usa-xx-sos", "label_fedex"))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "{v}");
        assert_eq!(
            (&v["error"], &v["reason"]),
            (&json!("option_unavailable"), &json!(NO_COURIER_STREET))
        );
        // A paid USPS kind while paid tiers pause still answers 503.
        let (status, v) = read(
            app.clone()
                .oneshot(post("usa-xx-sos", "label_usps"))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{v}");
        assert_eq!(v["error"], "paid_tiers_paused");
        let (status, _) = read(
            app.clone()
                .oneshot(post("usa-xx-sos", "address_label"))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
    }

    fn ready() -> Switches {
        Switches {
            orders_enabled: true,
            stripe_key: true,
            kit_price_cents: Some(4900),
            fedex_ready: true,
            usps_ready: true,
            ship_from_ok: true,
            ..Default::default()
        }
    }

    fn card(v: &Value, kind: &str) -> Value {
        v["options"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["kind"] == kind)
            .unwrap()
            .clone()
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
            (d.mail.as_ref().map(|m| m.fact_id), d.usps_only_fact),
            (Some(203), Some(203)),
            "po_box outranks the street address"
        );
        // Not ready (no customer carrier buys, no ship-from): paid cards
        // stay paused even with Stripe on.
        let off = Switches {
            orders_enabled: true,
            stripe_key: true,
            kit_price_cents: Some(4900),
            ..Default::default()
        };
        let v = options("usa-wa-sos", Some(&d), &off);
        for kind in ["label_usps", "mail_kit"] {
            let card = v["options"]
                .as_array()
                .unwrap()
                .iter()
                .find(|c| c["kind"] == kind)
                .unwrap()
                .clone();
            assert_eq!(card["enabled"], false, "{kind}");
        }
        // Ready everywhere: the PO Box still disables FedEx, citing the fact.
        let sw = Switches {
            fedex_ready: true,
            usps_ready: true,
            ship_from_ok: true,
            ..off
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
    fn stripe_session_must_match_amount_currency_mode_and_order() {
        let s = json!({"metadata": {"order_id": "ord_a"}, "payment_status": "paid", "amount_total": 4900, "currency": "usd", "livemode": false});
        assert_eq!(
            judge_session(&s, "ord_a", 4900, "usd", false),
            Settled::Paid
        );
        assert!(
            matches!(
                judge_session(&s, "ord_b", 4900, "usd", false),
                Settled::Mismatch(_)
            ),
            "replayed onto another order"
        );
        assert!(
            matches!(
                judge_session(&s, "ord_a", 5900, "usd", false),
                Settled::Mismatch(_)
            ),
            "amount"
        );
        assert!(
            matches!(
                judge_session(&s, "ord_a", 4900, "eur", false),
                Settled::Mismatch(_)
            ),
            "currency"
        );
        assert!(
            matches!(
                judge_session(&s, "ord_a", 4900, "usd", true),
                Settled::Mismatch(_)
            ),
            "test payment for a live order"
        );
        let open = json!({"metadata": {"order_id": "ord_a"}, "payment_status": "unpaid", "amount_total": 4900, "currency": "usd", "livemode": false});
        assert_eq!(
            judge_session(&open, "ord_a", 4900, "usd", false),
            Settled::NotPaidYet
        );
        let sw = Switches {
            kit_price_cents: Some(4900),
            ..Default::default()
        };
        assert_eq!(price_lines("mail_kit", &sw).unwrap()[0].1, 4900);
        assert!(
            price_lines("label_fedex", &sw).is_none(),
            "no carrier rates yet: never a guessed price"
        );
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
                phone: None,
            },
        }
    }

    /// A PO box for mail and a cited courier street: every kind ships
    /// (a FedEx order never goes to the box).
    fn mail_and_courier() -> Destination {
        destination(&[
            f(203, "po_box", "Send to PO Box 4000, Olympia, WA 98504."),
            f(
                209,
                "address",
                "For FedEx or UPS deliveries use 801 Capitol Way S, Olympia, WA 98501.",
            ),
        ])
        .unwrap()
    }

    #[test]
    fn every_kind_inserts_its_legs_with_foreign_keys_on() {
        let (_tmp, p2) = p2_store();
        let fk: i64 = p2
            .query_row("PRAGMA foreign_keys", [], |r| r.get(0))
            .unwrap();
        assert_eq!(fk, 1, "the FK the backing submission satisfies is enforced");
        let dest = mail_and_courier();
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
    fn reconciler_picks_open_sessions_in_its_window() {
        let (_tmp, p2) = p2_store();
        let d =
            destination(&[f(203, "po_box", "Send to PO Box 4000, Olympia, WA 98504.")]).unwrap();
        let now = 10_000;
        for (age, status) in [(5, "open"), (60, "open"), (60, "paid"), (100_000, "open")] {
            let id = create_order(
                &p2,
                "cust-1",
                (7, "S"),
                &body("mail_kit"),
                &d,
                "test",
                now - age,
            )
            .unwrap();
            p2.execute(
                "INSERT INTO order_payment(order_id, session_id, checkout_url, amount_cents, currency, livemode, status, created_at)
                 VALUES (?1, ?2, 'https://checkout.stripe.com/x', 4900, 'usd', 0, ?3, ?4)",
                rusqlite::params![id, format!("cs_test_{id}"), status, now - age],
            )
            .unwrap();
        }
        assert_eq!(
            crate::reconcile::due(&p2, now).len(),
            1,
            "too new, paid and expired sessions are skipped"
        );
    }

    #[test]
    fn erasure_removes_orders_before_their_shipments() {
        // Review #6: the order tables' foreign keys made erasure fail.
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::new(tmp.path());
        store.init().unwrap();
        let hb = db::open(&store).unwrap();
        db::migrate(&hb).unwrap();
        let p2 = db::open_p2(&store).unwrap();
        db::migrate_p2(&p2).unwrap();
        migrate_p2_tables(&p2).unwrap();
        crate::forms_map::migrate_p2_tables(&p2).unwrap();
        let d = mail_and_courier();
        for kind in KINDS {
            create_order(&p2, "cust-1", (7, "S"), &body(kind), &d, "test", 1).unwrap();
        }
        let keep = create_order(&p2, "cust-2", (7, "S"), &body("mail_kit"), &d, "test", 1).unwrap();
        let report = db::erase_customer_data(&hb, &p2, &store, "cust-1", 2).unwrap();
        assert_eq!(report.orders, 4);
        assert_eq!(report.shipments, 8, "1 + 2 + 2 + 3 legs");
        let left: i64 = p2
            .query_row("SELECT COUNT(*) FROM customer_order", [], |r| r.get(0))
            .unwrap();
        assert_eq!(left, 1);
        assert!(
            order_view(&p2, &keep, "S").unwrap().is_some(),
            "other customers untouched"
        );
    }

    #[test]
    fn contract_seeds_match_the_served_shapes() {
        let seed: Value =
            serde_json::from_str(include_str!("../../../docs/contracts/order-options.json"))
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
            serde_json::from_str(include_str!("../../../docs/contracts/order.json")).unwrap();
        let (_tmp, p2) = p2_store();
        let id = create_order(&p2, "cust-1", (7, "S"), &body("mail_kit"), &d, "test", 1).unwrap();
        let mine = order_view(&p2, &id, "S").unwrap().unwrap();
        for key in order.as_object().unwrap().keys() {
            assert!(mine.get(key).is_some(), "order key {key}");
        }
    }
}
