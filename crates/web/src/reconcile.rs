//! Reconciler: re-fetches open Checkout sessions (staging, which Stripe
//! cannot reach, and missed webhooks) and drives paid orders to labels.
//!
//! Owner: lane SA-24 commerce (`docs/lanes/sa-24.md`). The
//! builder wires the hooks below in `main.rs` (B0, P316) and edits
//! nothing else here; the lane fills them in.

use crate::AppState;

/// Background worker, spawned once at boot after the notify worker.
/// Off unless its config switch is on; never on a tight timer.
pub fn spawn(_state: &AppState) {}
