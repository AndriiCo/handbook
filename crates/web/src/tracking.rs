//! Carrier tracking poller (USPS Tracking v3, FedEx Track): writes
//! `tracking_event` + `order_event`, moves orders along
//! (`in_transit`, then `delivered` once the return leg arrives) and
//! publishes `tracking.update` on `billing:{customer_ref}` and
//! `shipment:{id}`.
//!
//! Off unless `tracking.poll_enabled` (read every tick through the
//! config cache). Never a tight timer: a tick every [`TICK`] polls at
//! most [`PER_TICK`] numbers, each number at most once per
//! [`MIN_GAP_SECS`], and a delivered number is never polled again.

use handbook_store::{db, store::Store};
use labels::track::{TrackingScan, TrackingStatus};
use labels::{HttpTransport, UreqTransport};
use rusqlite::Connection;
use serde_json::{Value, json};

use crate::{AppState, unix_now};

const TICK: std::time::Duration = std::time::Duration::from_secs(300);
const PER_TICK: i64 = 20;
const MIN_GAP_SECS: i64 = 3_600;

/// Boot DDL on the P2 DB (`p2.sqlite`, customer PII lives here):
/// `CREATE TABLE IF NOT EXISTS` only — no ALTER, DROP or RENAME.
pub fn migrate_p2_tables(p2: &Connection) -> rusqlite::Result<()> {
    p2.execute_batch(
        "CREATE TABLE IF NOT EXISTS tracking_event (
            id INTEGER PRIMARY KEY,
            order_id TEXT NOT NULL,
            leg TEXT NOT NULL,
            shipment_id INTEGER NOT NULL,
            carrier TEXT NOT NULL CHECK (carrier IN ('fedex', 'usps')),
            tracking_no TEXT NOT NULL,
            code TEXT NOT NULL,
            text TEXT NOT NULL,
            at TEXT NOT NULL,
            place TEXT NOT NULL DEFAULT '',
            received_at INTEGER NOT NULL,
            UNIQUE (carrier, tracking_no, code, at)
         );
         CREATE INDEX IF NOT EXISTS idx_tracking_event_order ON tracking_event(order_id, received_at);
         CREATE TABLE IF NOT EXISTS tracking_poll (
            carrier TEXT NOT NULL,
            tracking_no TEXT NOT NULL,
            last_polled_at INTEGER NOT NULL,
            last_status TEXT NOT NULL DEFAULT '',
            polls INTEGER NOT NULL DEFAULT 0,
            last_error TEXT NOT NULL DEFAULT '',
            PRIMARY KEY (carrier, tracking_no)
         );",
    )
}

/// `tracking.poll_enabled` (default off).
fn enabled(state: &AppState, meta: &Connection) -> bool {
    state.kek.as_deref().is_some_and(|kek| {
        state
            .config_cache
            .bool(meta, kek, config::keys::TRACKING_POLL_ENABLED)
            .ok()
            .flatten()
            .unwrap_or(false)
    })
}

/// Background worker, spawned once at boot after the notify worker.
pub fn spawn(state: &AppState) {
    let state = state.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(TICK).await;
            let job = state.clone();
            let _ = tokio::task::spawn_blocking(move || tick(&job)).await;
        }
    });
}

fn tick(state: &AppState) {
    let store = Store::new(&state.store_root);
    let Ok(meta) = db::open(&store) else {
        return;
    };
    if !enabled(state, &meta) {
        return;
    }
    // One bearer per carrier per tick, fetched on first use.
    let usps: std::cell::OnceCell<Option<(String, String)>> = std::cell::OnceCell::new();
    let fedex: std::cell::OnceCell<Option<(String, String)>> = std::cell::OnceCell::new();
    let transport = UreqTransport::new();
    let fetch = |carrier: &str, tracking_no: &str| -> Result<Fetched, String> {
        match carrier {
            "usps" => {
                let (base, bearer) = usps
                    .get_or_init(|| crate::usps::bearer(state, &meta).and_then(Result::ok))
                    .clone()
                    .ok_or("usps credentials are not set")?;
                let response = transport
                    .get(&labels::usps::tracking_url(&base, tracking_no), &bearer)
                    .map_err(|e| e.to_string())?;
                answer(response).map(|json| labels::usps::parse_tracking(&json))
            }
            "fedex" => {
                let (base, bearer) = fedex
                    .get_or_init(|| {
                        crate::customer_labels::fedex_bearer(state, &meta).and_then(Result::ok)
                    })
                    .clone()
                    .ok_or("fedex credentials are not set")?;
                let url = format!("{base}{}", labels::track::FEDEX_TRACK_PATH);
                let response = transport
                    .post_json(
                        &url,
                        &bearer,
                        None,
                        &labels::track::fedex_track_body(tracking_no),
                    )
                    .map_err(|e| e.to_string())?;
                answer(response).map(|json| labels::track::parse_fedex_track(&json))
            }
            _ => Err("no tracking for this carrier".to_string()),
        }
    };
    poll_once(state, unix_now(), &fetch);
}

fn answer(response: labels::HttpResponse) -> Result<Value, String> {
    if !(200..300).contains(&response.status) {
        return Err(format!("tracking: http {}", response.status));
    }
    serde_json::from_slice(&response.body).map_err(|e| e.to_string())
}

pub(crate) type Fetched = (Option<TrackingStatus>, Vec<TrackingScan>);

/// One leg due a poll.
struct Due {
    order_id: String,
    customer_ref: String,
    status: String,
    leg: String,
    shipment_id: i64,
    carrier: String,
    tracking_no: String,
}

/// Poll every due leg once (blocking). Returns how many new scans landed.
pub(crate) fn poll_once(
    state: &AppState,
    now: i64,
    fetch: &dyn Fn(&str, &str) -> Result<Fetched, String>,
) -> usize {
    let Ok(p2) = db::open_p2(&Store::new(&state.store_root)) else {
        return 0;
    };
    let due: Vec<Due> = p2
        .prepare(
            "SELECT o.id, o.customer_ref, o.status, os.role, os.shipment_id, os.carrier,
                    os.tracking_number
             FROM order_shipment os
             JOIN customer_order o ON o.id = os.order_id
             LEFT JOIN tracking_poll t
                    ON t.carrier = os.carrier AND t.tracking_no = os.tracking_number
             WHERE os.tracking_number IS NOT NULL AND os.tracking_number != ''
               AND os.carrier IN ('fedex', 'usps')
               AND o.status IN ('labels_ready', 'shipped', 'in_transit')
               AND (t.last_polled_at IS NULL OR t.last_polled_at <= ?1)
               AND COALESCE(t.last_status, '') != 'delivered'
             ORDER BY COALESCE(t.last_polled_at, 0) LIMIT ?2",
        )
        .and_then(|mut stmt| {
            stmt.query_map(rusqlite::params![now - MIN_GAP_SECS, PER_TICK], |r| {
                Ok(Due {
                    order_id: r.get(0)?,
                    customer_ref: r.get(1)?,
                    status: r.get(2)?,
                    leg: r.get(3)?,
                    shipment_id: r.get(4)?,
                    carrier: r.get(5)?,
                    tracking_no: r.get(6)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()
        })
        .unwrap_or_default();
    let mut landed = 0;
    for leg in due {
        let result = fetch(&leg.carrier, &leg.tracking_no);
        let (status_text, error) = match &result {
            Ok((status, _)) => (status.map(status_word).unwrap_or(""), String::new()),
            Err(e) => ("", e.chars().take(200).collect()),
        };
        let _ = p2.execute(
            "INSERT INTO tracking_poll(carrier, tracking_no, last_polled_at, last_status, polls, last_error)
             VALUES (?1, ?2, ?3, ?4, 1, ?5)
             ON CONFLICT(carrier, tracking_no) DO UPDATE SET
                last_polled_at = excluded.last_polled_at,
                last_status = CASE WHEN excluded.last_status = '' THEN last_status
                                   ELSE excluded.last_status END,
                polls = polls + 1,
                last_error = excluded.last_error",
            rusqlite::params![leg.carrier, leg.tracking_no, now, status_text, error],
        );
        let Ok((status, scans)) = result else {
            continue;
        };
        landed += record(state, &p2, &leg, status, &scans, now);
    }
    landed
}

fn status_word(status: TrackingStatus) -> &'static str {
    match status {
        TrackingStatus::PreTransit => "pre_transit",
        TrackingStatus::InTransit => "in_transit",
        TrackingStatus::Delivered => "delivered",
        TrackingStatus::Alert => "alert",
    }
}

/// Store new scans of one leg and move its order; returns new scans.
fn record(
    state: &AppState,
    p2: &Connection,
    leg: &Due,
    status: Option<TrackingStatus>,
    scans: &[TrackingScan],
    now: i64,
) -> usize {
    let mut fresh: Vec<&TrackingScan> = Vec::new();
    for scan in scans {
        let inserted = p2
            .execute(
                "INSERT OR IGNORE INTO tracking_event(order_id, leg, shipment_id, carrier, tracking_no,
                                                      code, text, at, place, received_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                rusqlite::params![
                    leg.order_id,
                    leg.leg,
                    leg.shipment_id,
                    leg.carrier,
                    leg.tracking_no,
                    scan.code,
                    scan.text.chars().take(200).collect::<String>(),
                    scan.at,
                    scan.place,
                    now
                ],
            )
            .unwrap_or(0);
        if inserted == 1 {
            fresh.push(scan);
        }
    }
    // Carriers list the newest scan first: that one is the timeline line.
    if let Some(newest) = fresh.first() {
        let text = if newest.place.is_empty() {
            newest.text.clone()
        } else {
            format!("{} ({})", newest.text, newest.place)
        };
        let _ = p2.execute(
            "INSERT INTO order_event(order_id, at, kind, text, leg) VALUES (?1, ?2, 'tracking.update', ?3, ?4)",
            rusqlite::params![leg.order_id, now, text, leg.leg],
        );
    }
    // Orders move on the carrier's word: any movement means in transit;
    // the return leg delivered means the documents are back home.
    let mut current = leg.status.clone();
    let moving = matches!(
        status,
        Some(TrackingStatus::InTransit | TrackingStatus::Delivered)
    );
    if moving
        && matches!(current.as_str(), "labels_ready" | "shipped")
        && crate::orders::set_status(p2, &leg.order_id, &current, "in_transit", now)
    {
        current = "in_transit".to_string();
    }
    if status == Some(TrackingStatus::Delivered)
        && leg.leg == "return"
        && current == "in_transit"
        && crate::orders::set_status(p2, &leg.order_id, "in_transit", "delivered", now)
    {
        crate::orders::event(
            p2,
            &leg.order_id,
            "order.delivered",
            "Your documents are back with you",
            now,
        );
    }
    if status == Some(TrackingStatus::Alert) && !fresh.is_empty() {
        crate::ws_bus::publish_ops(
            &state.bus,
            "tracking.alert",
            json!({"order_id": leg.order_id, "leg": leg.leg}),
            now,
        );
    }
    if !fresh.is_empty() {
        let billing = format!("billing:{}", leg.customer_ref);
        state.bus.publish(
            &billing,
            json!({"type": "tracking.update", "topic": billing, "order_id": leg.order_id,
                   "leg": leg.leg, "at": now})
            .to_string(),
        );
        let shipment = format!("shipment:{}", leg.shipment_id);
        state.bus.publish(
            &shipment,
            json!({"type": "tracking.update", "topic": shipment, "shipment_id": leg.shipment_id,
                   "status": status.map(status_word), "at": now})
            .to_string(),
        );
    }
    fresh.len()
}
