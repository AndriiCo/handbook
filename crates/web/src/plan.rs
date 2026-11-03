//! `RenderPlanV1`: Spark composes the authority page from accepted
//! facts, `validate_plan` + a Wonder entailment pass gate it, staff
//! accept it, and `GET /api/v1/authorities/{slug}/plan` serves it
//! (always 200, model-free, re-filtered against current accepted facts).
//!
//! Owner: lane SA-22 plan (`docs/lanes/sa-22.md`). The
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

/// Public routes, merged into the `/apostille` API router (paths start
/// `/api/v1/…`). Rate-limit every handler; no model calls on a public read.
pub fn public_routes() -> Router<AppState> {
    Router::new()
}

/// Staff routes merged onto the ADMIN listener only (`AdminAuth` per
/// handler; host guard + CSRF layers inside, like `admin_workers::router`).
pub fn admin_router(state: AppState) -> Router {
    Router::<AppState>::new().with_state(state)
}
