//! `FactChecksV1`: Wonder cross-checks each proposed fact against the
//! whole capture sidecar (supported / contradicted / not_found + quote;
//! the quote must be a sidecar substring or the verdict is not_found).
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

/// Cross-check verdicts for one authority's facts, keyed by fact id
/// (`plan.rs` provenance reads this). B0 seam: empty until SA-23 lands.
#[allow(dead_code)]
pub fn checks_for_slug(
    _conn: &Connection,
    _slug: &str,
) -> rusqlite::Result<Vec<serde_json::Value>> {
    Ok(Vec::new())
}
