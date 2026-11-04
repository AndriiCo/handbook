//! Gmail watch-mail poll (Phase B): historyId-watermarked poll of one
//! mailbox into immutable `email` captures.
//!
//! `client` speaks Gmail + OAuth2 over blocking ureq (voice precedent);
//! `auth_results` parses `Authentication-Results` into trust evidence;
//! `mime` extracts headers + a text sidecar from RFC 2822 bytes (raw
//! bytes are always preserved, so a better parser can re-derive
//! sidecars later); `report` reads bounces and complaints; `poll`
//! orchestrates the watermark walk with at-least-once +
//! idempotent-ingest semantics; `fake` is the W-3 fake inbox, mail
//! composed in the staff console and captured by the poll's own step.

pub mod auth_results;
pub mod client;
pub mod fake;
pub mod mime;
pub mod poll;
pub mod report;

/// The personal-data patterns email sidecars are redacted with (and
/// `xtask pii-sweep` refuses), re-exported from `notify`.
pub use notify::pii;
