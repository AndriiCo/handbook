//! Carrier tracking poller (USPS Tracking v3, FedEx Track) plus
//! customer-entered tracking; writes `tracking_event` + `order_event`
//! and publishes `tracking.update` on `shipment:{id}`.
//!
//! Owner: lane SA-25 carriers (`docs/lanes/sa-25.md`). The
//! builder wires the hooks below in `main.rs` (B0, P316) and edits
//! nothing else here; the lane fills them in.

use rusqlite::Connection;

use crate::AppState;

/// Boot DDL on the P2 DB (`p2.sqlite`, customer PII lives here):
/// `CREATE TABLE IF NOT EXISTS` only — no ALTER, DROP or RENAME.
pub fn migrate_p2_tables(_p2: &Connection) -> rusqlite::Result<()> {
    Ok(())
}

/// Background worker, spawned once at boot after the notify worker.
/// Off unless its config switch is on; never on a tight timer.
pub fn spawn(_state: &AppState) {}
