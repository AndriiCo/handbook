//! Gmail watch-mail poll (Phase B): historyId-watermarked poll of one
//! mailbox into immutable `email` captures.
//!
//! `client` speaks Gmail + OAuth2 over blocking ureq (voice precedent);
//! `auth_results` parses `Authentication-Results` into trust evidence;
//! `mime` extracts headers + a text sidecar from RFC 2822 bytes
//! (deliberately minimal — raw bytes are always preserved, so a fuller
//! parser can re-derive sidecars later); `poll` orchestrates the
//! watermark walk with at-least-once + idempotent-ingest semantics.

pub mod auth_results;
pub mod client;
pub mod mime;
pub mod poll;
