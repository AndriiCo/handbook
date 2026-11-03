//! Stripe hosted Checkout + the webhook (the only CSRF-exempt POST:
//! raw body, HMAC via aws_lc_rs, `stripe_event` dedupe, server-side
//! session re-fetch for amount/paid/livemode).
//!
//! Owner: lane SA-24 commerce (`docs/lanes/sa-24.md`). The
//! builder wires the hooks below in `main.rs` (B0, P316) and edits
//! nothing else here; the lane fills them in.

use axum::Router;

use crate::AppState;

/// Public routes, merged into the `/apostille` API router (paths start
/// `/api/v1/…`). Rate-limit every handler; no model calls on a public read.
pub fn public_routes() -> Router<AppState> {
    Router::new()
}
