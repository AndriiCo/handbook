//! USPS v3 for customer labels (OAuth, payment authorization, labels,
//! prices). Filled in by the USPS slice; until then USPS customer
//! labels stay paused.

use rusqlite::Connection;

use crate::AppState;
use crate::customer_labels::LabelLink;

/// USPS customer labels can be quoted and bought now.
pub fn customer_ready(_state: &AppState, _meta: &Connection, _stripe_live: bool) -> bool {
    false
}

/// The USPS link for one customer order (blocking).
pub(crate) fn customer_link(
    _state: &AppState,
    _meta: &Connection,
    _stripe_live: bool,
) -> Result<Box<dyn LabelLink>, String> {
    Err("usps customer labels are not available".to_string())
}
