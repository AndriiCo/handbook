//! Evidence-store metadata (SQLite schema in `migrations/`) and
//! content-addressed blob I/O.
//!
//! Every fallible API returns [`contracts::HandbookError`].

pub mod db;
pub mod store;
