//! Instruction worker: triggers → drafted sections → proposed facts.
//!
//! Server-side background worker (timer-driven `xtask
//! instruct-sweep`). Packs carry the general template; findings
//! carry the particular; humans decide via the existing review
//! queue. Spec: `docs/specs/instruction-worker.md`.

pub mod change;
pub mod draft;
pub mod error;
pub mod fixture;
pub mod propose;
pub mod render;
pub mod template;
pub mod trigger;
pub mod worker;

/// Prompt pack version, recorded in watermarks and `created_by`.
pub const PROMPT_VERSION: &str = "instruct-v1";
