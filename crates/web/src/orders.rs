//! Customer orders (`label_fedex`, `label_usps`, `mail_kit`,
//! `address_label`): `customer_order`, `order_shipment`, `order_event`
//! in p2.sqlite, a backing `form_submission` per order, the state
//! machine, quotes, and `billing:{customer_ref}` events.
//!
//! Owner: lane SA-24 commerce (`docs/lanes/sa-24.md`). The
//! builder wires the hooks below in `main.rs` (B0, P316) and edits
//! nothing else here; the lane fills them in.

use axum::Router;
use rusqlite::Connection;

use crate::AppState;

/// Boot DDL on the P2 DB (`p2.sqlite`, customer PII lives here):
/// `CREATE TABLE IF NOT EXISTS` only — no ALTER, DROP or RENAME.
pub fn migrate_p2_tables(_p2: &Connection) -> rusqlite::Result<()> {
    Ok(())
}

/// Customer routes, merged into the `/api/v1/p2` nest (paths are
/// relative: `/orders`, `/orders/{id}`). CSRF double-submit on every
/// POST; owner-or-staff on every read.
pub fn p2_routes() -> Router<AppState> {
    Router::new()
}
