//! Compile run: discover → capture → extract → cross-check → offices →
//! compose, streamed over `run:{id}` (ids from `DraftRuns::reserve_id`),
//! capped per run and per day (`compile.daily_runs`), never on a timer.
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

/// Staff routes merged onto the ADMIN listener only (`AdminAuth` per
/// handler; host guard + CSRF layers inside, like `admin_workers::router`).
pub fn admin_router(state: AppState) -> Router {
    Router::<AppState>::new().with_state(state)
}
