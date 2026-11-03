//! Reconciler: re-fetches open Checkout sessions (staging, which Stripe
//! cannot reach, and missed webhooks) and drives paid orders to labels.
//!
//! One tick a minute, only while Stripe keys are
//! configured; each open session is re-read at most once per tick and
//! settled through `orders::settle` (Stripe's own record decides).

use handbook_store::{db, store::Store};

use crate::{AppState, unix_now};

/// Sessions live an hour; one more pass after that marks them expired.
const MAX_AGE_SECS: i64 = 3_600 + 1_800;
/// Give the buyer a moment on the Checkout page before polling.
const MIN_AGE_SECS: i64 = 20;
const TICK: std::time::Duration = std::time::Duration::from_secs(60);

/// Open sessions due a re-read (pure query; tested with the order tests).
pub fn due(p2: &rusqlite::Connection, now: i64) -> Vec<String> {
    p2.prepare(
        "SELECT order_id FROM order_payment WHERE status = 'open' AND created_at <= ?1 AND created_at >= ?2
         ORDER BY created_at DESC LIMIT 20",
    )
    .and_then(|mut s| s.query_map([now - MIN_AGE_SECS, now - MAX_AGE_SECS], |r| r.get(0))?.collect())
    .unwrap_or_default()
}

fn tick(state: &AppState) {
    let store = Store::new(&state.store_root);
    let (Ok(meta), Ok(p2)) = (db::open(&store), db::open_p2(&store)) else {
        return;
    };
    let Some(keys) = crate::stripe::keys(state, &meta) else {
        return;
    };
    if crate::orders::migrate_p2_tables(&p2).is_err() {
        return;
    }
    for order_id in due(&p2, unix_now()) {
        match crate::orders::settle(state, &keys, &order_id) {
            Ok(crate::orders::Settled::Mismatch(why)) => {
                // settle already flagged the row and the order for staff.
                tracing::warn!(order = %order_id, %why, "reconcile: session does not match the order");
            }
            Ok(_) => {}
            Err(e) => tracing::warn!(order = %order_id, error = %e, "reconcile: settle failed"),
        }
    }
}

/// Background worker, spawned once at boot after the notify worker.
/// Off unless Stripe keys are configured; never on a tight timer.
pub fn spawn(state: &AppState) {
    let state = state.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(TICK).await;
            let st = state.clone();
            let _ = tokio::task::spawn_blocking(move || tick(&st)).await;
        }
    });
}
