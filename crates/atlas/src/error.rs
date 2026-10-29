//! Atlas error type: one enum for the whole harness.
//!
//! Variants name the failing stage so `--dry-run` and the ledger can
//! report precisely. Secrets never appear in messages (callers pass
//! redacted values in).

use std::fmt;

/// Every error the harness can return.
#[derive(Debug)]
pub enum Error {
    /// File/pack/ledger I/O.
    Io(std::io::Error),
    /// Pack parse or validation failure (`path: reason`).
    Config(String),
    /// Source fetch failure (`source: reason`).
    Fetch(String),
    /// Local extraction failure (`doc: reason`).
    Extract(String),
    /// Model API failure (transport, auth, billing, bad output).
    Model(String),
    /// Token budget exceeded before sending.
    Budget(String),
    /// Batch job failure (`job: reason`).
    Batch(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(e) => write!(f, "atlas io: {e}"),
            Error::Config(msg) => write!(f, "atlas config: {msg}"),
            Error::Fetch(msg) => write!(f, "atlas fetch: {msg}"),
            Error::Extract(msg) => write!(f, "atlas extract: {msg}"),
            Error::Model(msg) => write!(f, "atlas model: {msg}"),
            Error::Budget(msg) => write!(f, "atlas budget: {msg}"),
            Error::Batch(msg) => write!(f, "atlas batch: {msg}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}

/// Fallible harness result.
pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_names_the_stage() {
        let err = Error::Budget("input too large".to_string());
        assert_eq!(format!("{err}"), "atlas budget: input too large");
        let io: Error = std::io::Error::new(std::io::ErrorKind::NotFound, "x").into();
        assert!(format!("{io}").starts_with("atlas io:"));
    }
}
