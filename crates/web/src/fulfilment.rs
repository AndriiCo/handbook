//! Kit fulfilment queue (To pack / Awaiting label / Shipped) and the
//! printed packet (`render_packet_cover`).
//!
//! The hooks below are wired in `main.rs`; the routes are still to come.

use axum::Router;

use crate::AppState;

/// Staff routes merged onto the ADMIN listener only (`AdminAuth` per
/// handler; host guard + CSRF layers inside, like `admin_workers::router`).
pub fn admin_router(state: AppState) -> Router {
    Router::<AppState>::new().with_state(state)
}
