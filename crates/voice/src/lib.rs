//! Telnyx Call Control voice lane (spec: `docs/specs/voice-telnyx.md`).
//!
//! Outbound staff calls: dial a public office, record with consent,
//! read a live transcript. The [`client::TelnyxClient`] is blocking
//! (runs on `spawn_blocking` in the web layer) and takes a base-URL
//! override so tests dial a stub server — never the real API.

pub mod calls;
pub mod client;
pub mod scripts;
pub mod store;
pub mod webhook;

pub use calls::CallStatus;
pub use client::{DialOutcome, TelnyxClient, VoiceConfig, VoiceError};
pub use store::{NewCall, TranscriptSegment, VoiceCall};
pub use webhook::{EventType, WebhookEnvelope, WebhookError, verify_signature};
