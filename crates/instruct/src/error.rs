//! Instruction-worker error type: one enum, stage-named variants.

use std::fmt;

/// Every error the worker can return.
#[derive(Debug)]
pub enum Error {
    /// File/pack I/O.
    Io(std::io::Error),
    /// Pack parse or validation failure.
    Config(String),
    /// Trigger intake failure.
    Trigger(String),
    /// Model call failure (atlas error text).
    Model(String),
    /// Token budget exceeded before sending.
    Budget(String),
    /// Draft/proposal validation failure.
    Validate(String),
    /// Store failure.
    Db(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(e) => write!(f, "instruct io: {e}"),
            Error::Config(msg) => write!(f, "instruct config: {msg}"),
            Error::Trigger(msg) => write!(f, "instruct trigger: {msg}"),
            Error::Model(msg) => write!(f, "instruct model: {msg}"),
            Error::Budget(msg) => write!(f, "instruct budget: {msg}"),
            Error::Validate(msg) => write!(f, "instruct validate: {msg}"),
            Error::Db(e) => write!(f, "instruct db: {e}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}

impl From<atlas::error::Error> for Error {
    fn from(e: atlas::error::Error) -> Self {
        Error::Model(e.to_string())
    }
}

impl From<rusqlite::Error> for Error {
    fn from(e: rusqlite::Error) -> Self {
        Error::Db(e.to_string())
    }
}

impl From<contracts::error::HandbookError> for Error {
    fn from(e: contracts::error::HandbookError) -> Self {
        Error::Db(e.to_string())
    }
}

/// Fallible worker result.
pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_names_the_stage() {
        assert_eq!(
            format!("{}", Error::Budget("too big".to_string())),
            "instruct budget: too big"
        );
    }
}
