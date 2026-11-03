//! `FormMapV1`: Spark maps an official PDF's AcroForm fields to labels,
//! help and prefill topics, grounded in the lopdf text layer; staff
//! accept into `form_template`; the customer fill is fill-and-flatten
//! of the official PDF.
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

/// Customer routes, merged into the `/api/v1/p2` nest (paths are
/// relative: `/orders`, `/orders/{id}`). CSRF double-submit on every
/// POST; owner-or-staff on every read.
pub fn p2_routes() -> Router<AppState> {
    Router::new()
}

/// Staff routes merged onto the ADMIN listener only (`AdminAuth` per
/// handler; host guard + CSRF layers inside, like `admin_workers::router`).
pub fn admin_router(state: AppState) -> Router {
    Router::<AppState>::new().with_state(state)
}

/// Published forms for one authority, shaped as
/// `docs/lanes/contracts/form.json` (`plan.rs` reads this).
/// B0 seam: empty until SA-23 lands.
#[allow(dead_code)]
pub fn accepted_for_slug(
    _conn: &Connection,
    _slug: &str,
) -> rusqlite::Result<Vec<serde_json::Value>> {
    Ok(Vec::new())
}
