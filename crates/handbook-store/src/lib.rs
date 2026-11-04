//! Evidence-store metadata (SQLite schema in `migrations/`) and
//! content-addressed blob I/O.
//!
//! Every fallible API returns [`contracts::HandbookError`].

pub mod citation;
pub mod db;
pub mod history;
pub mod store;
pub mod work;

/// Canonical test fixtures (the Washington PO-Box pair). Test-only on
/// purpose: fixture values must never leak into real facts, so the
/// module does not exist in non-test builds.
#[cfg(test)]
mod fixtures;
