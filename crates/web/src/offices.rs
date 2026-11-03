//! `OfficeSetV1`: Spark structures accepted address/hours/walk-in/
//! appointment/po_box facts into offices; `address_ok` + `hours_ok`
//! guards; staff accept; `offices_accepted` on `authority:{slug}`.
//!
//! Owner: lane SA-23 ground (`docs/lanes/sa-23.md`). The
//! builder wires the hooks below in `main.rs` (B0, P316) and edits
//! nothing else here; the lane fills them in.

use axum::Router;
use rusqlite::Connection;

use crate::AppState;

/// Boot DDL on the metadata DB (`handbook.sqlite`): `CREATE TABLE IF
/// NOT EXISTS` only — no ALTER, DROP or RENAME (AGENTS.md §7).
pub fn migrate_tables(_conn: &Connection) -> rusqlite::Result<()> {
    Ok(())
}

/// Staff routes merged onto the ADMIN listener only (`AdminAuth` per
/// handler; host guard + CSRF layers inside, like `admin_workers::router`).
pub fn admin_router(state: AppState) -> Router {
    Router::<AppState>::new().with_state(state)
}

/// Accepted offices for one authority, shaped as
/// `docs/lanes/contracts/office.json` (`plan.rs` reads this).
/// B0 seam: empty until SA-23 lands.
#[allow(dead_code)]
pub fn accepted_for_slug(
    _conn: &Connection,
    _slug: &str,
) -> rusqlite::Result<Vec<serde_json::Value>> {
    Ok(Vec::new())
}
