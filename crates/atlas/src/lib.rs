//! Atlas: rerunnable convention-source research harness.
//!
//! Task packs (TOML) describe sources, models, effort, and tasks;
//! the [`task`] runner fetches, extracts (text/HTML/PDF/SVG/audio),
//! budgets tokens, summarizes, and extracts structured JSON via the
//! Meta Model API — fast sync chat or managed background batches.
//! Every run leaves a JSONL ledger + debug dumps under `out/`.
//!
//! Spec: `docs/specs/atlas-harness.md`.

pub mod batch;
pub mod cached;
pub mod chunk;
pub mod config;
pub mod error;
pub mod extract;
pub mod ledger;
pub mod model;
pub mod render;
pub mod source;
pub mod summarize;
pub mod task;
