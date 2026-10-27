//! Synthetic-only gate: real media waits for the terms read (G3).
//!
//! The slice runs on synthetic fixtures until someone reads the Meta
//! API data-handling terms for customer media. This module is that
//! tripwire in code: every intake and every Model API call takes the
//! policy + source and fails closed for real media while
//! `terms_read` is false.

use std::fmt;

/// Where the bytes came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum MediaSource {
    /// CI/dev fixtures (`dev-seed`, tests). Always allowed.
    SyntheticFixture,
    /// Anything a customer or staffer uploaded. Blocked until the
    /// Meta data-handling terms are read.
    RealUpload,
}

/// Slice media gate. The web layer owns one instance and flips
/// `terms_read` once SEC confirms the read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MediaPolicy {
    pub terms_read: bool,
}

impl MediaPolicy {
    pub fn synthetic_only() -> Self {
        Self { terms_read: false }
    }

    pub fn terms_read() -> Self {
        Self { terms_read: true }
    }
}

/// Enforce the gate.
///
/// # Errors
///
/// Returns [`PolicyError::TermsNotRead`] for real media while the
/// terms are unread.
pub fn check(policy: &MediaPolicy, source: MediaSource) -> Result<(), PolicyError> {
    match (policy.terms_read, source) {
        (_, MediaSource::SyntheticFixture) | (true, MediaSource::RealUpload) => Ok(()),
        (false, MediaSource::RealUpload) => Err(PolicyError::TermsNotRead),
    }
}

/// [`check`] failure.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum PolicyError {
    TermsNotRead,
}

impl fmt::Display for PolicyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PolicyError::TermsNotRead => write!(
                f,
                "real media is blocked until the meta api data-handling terms are read"
            ),
        }
    }
}

impl std::error::Error for PolicyError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn synthetic_flows_before_terms() {
        let policy = MediaPolicy::synthetic_only();
        assert!(check(&policy, MediaSource::SyntheticFixture).is_ok());
        assert_eq!(
            check(&policy, MediaSource::RealUpload).unwrap_err(),
            PolicyError::TermsNotRead
        );
        assert!(check(&MediaPolicy::terms_read(), MediaSource::RealUpload).is_ok());
    }
}
