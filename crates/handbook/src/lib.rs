//! Apostille Handbook library.
//!
//! Design arrives in a later prompt; this crate currently only reserves
//! the library root so the workspace builds.

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
