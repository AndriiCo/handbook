//! Apostille Handbook library: evidence-store metadata and file paths.
//!
//! See `docs/design.md`. This crate owns the SQLite schema (`migrations/`),
//! content-addressed blob I/O, and foundation queries. Collection and
//! maintenance entry points live in `xtask`.

pub mod analyze;
pub mod db;
pub mod model;
pub mod runner;
pub mod store;
pub mod topics;

/// Crate version, for smoke tests and future tooling.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_matches_package() {
        assert_eq!(version(), "0.1.0");
    }
}
