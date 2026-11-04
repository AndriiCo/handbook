//! W-3 fake modes: local stand-ins for every outside service, so a judge
//! or developer can run the whole product with no accounts and no money
//! moving (`docs/specs/config-db.md` § Fake mode).
//!
//! The `<area>.mode` rows live in the config crate ([`config::Modes`]);
//! its loader refuses a fake on any boot whose binds are not all
//! loopback, unless the host is a labelled staging target that is not
//! production. This module holds the web side: the config writers'
//! matching refusal, so a production console can never save a row that
//! would stop its own next boot.

use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde_json::json;

/// 422 when `key` is a `<area>.mode` row, `value` asks for `fake`, and
/// this boot's guard is closed; `None` lets the write through.
pub(crate) fn refuse_fake_row(
    modes: &config::Modes,
    key: &str,
    value: &serde_json::Value,
) -> Option<Response> {
    let mode_row = config::Integration::ALL.iter().any(|i| i.key() == key);
    let asks_fake = value.as_str().and_then(config::Mode::parse) == Some(config::Mode::Fake);
    (mode_row && asks_fake && !modes.fakes_allowed).then(|| {
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"error": "fake mode runs only on loopback binds or a staging target"})),
        )
            .into_response()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_fake_mode_row_on_a_closed_guard_is_refused() {
        let closed = config::Modes::default();
        let mut open = config::Modes::default();
        open.fakes_allowed = true;
        let fake = json!("fake");
        assert!(refuse_fake_row(&closed, "labels.mode", &fake).is_some());
        assert!(refuse_fake_row(&closed, "mailer.mode", &json!(" fake ")).is_some());
        assert!(refuse_fake_row(&closed, "labels.mode", &json!("live")).is_none());
        assert!(refuse_fake_row(&closed, "mailer.host", &fake).is_none());
        assert!(refuse_fake_row(&open, "labels.mode", &fake).is_none());
    }
}
