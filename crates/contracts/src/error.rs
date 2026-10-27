//! The single error type for handbook subsystem APIs.
//!
//! Two variants cover everything the store and agent layers can fail
//! with: SQLite failures and file-store failures. Callers match on the
//! variant when the distinction matters, otherwise render with
//! [`Display`](std::fmt::Display).

/// Every error the handbook subsystems can return.
#[derive(Debug)]
pub enum HandbookError {
    /// SQLite failure (query, constraint, or schema errors).
    Db(rusqlite::Error),
    /// File-store failure (paths, blobs, directories).
    Io(std::io::Error),
}

impl std::fmt::Display for HandbookError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HandbookError::Db(e) => write!(f, "database error: {e}"),
            HandbookError::Io(e) => write!(f, "file-store error: {e}"),
        }
    }
}

impl std::error::Error for HandbookError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            HandbookError::Db(e) => Some(e),
            HandbookError::Io(e) => Some(e),
        }
    }
}

impl From<rusqlite::Error> for HandbookError {
    fn from(e: rusqlite::Error) -> Self {
        HandbookError::Db(e)
    }
}

impl From<std::io::Error> for HandbookError {
    fn from(e: std::io::Error) -> Self {
        HandbookError::Io(e)
    }
}

/// Fallible subsystem result.
pub type Result<T> = std::result::Result<T, HandbookError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_names_the_layer() {
        let db: HandbookError = rusqlite::Error::InvalidQuery.into();
        assert!(format!("{db}").starts_with("database error:"));
        let io: HandbookError = std::io::Error::new(std::io::ErrorKind::NotFound, "missing").into();
        assert!(format!("{io}").starts_with("file-store error:"));
    }

    #[test]
    fn sources_are_preserved() {
        use std::error::Error as _;
        let io: HandbookError = std::io::Error::new(std::io::ErrorKind::NotFound, "missing").into();
        assert!(io.source().is_some());
    }
}
