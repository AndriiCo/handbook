//! Change header: before/after/reason carried in `fact.reasoning`.
//!
//! No schema change: the review queue already renders `reasoning`
//! as `reasoning_untrusted`, so a parseable header gives reviewers
//! before/after inline. Grammar (a future column migration can
//! adopt it verbatim):
//!
//! ```text
//! SUPERSEDES #<accepted fact id | 0 when new>
//! BEFORE: <old statement | (none)>
//! AFTER: <new statement>
//! REASON: <why, citing trigger capture ids>
//! ```

use crate::error::{Error, Result};

/// One proposed change with its review trail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    /// Accepted fact id this replaces (0 when there is none).
    pub supersedes: i64,
    pub before: Option<String>,
    pub after: String,
    pub reason: String,
}

impl Change {
    /// Render the header (`fact.reasoning` body).
    pub fn header(&self) -> String {
        format!(
            "SUPERSEDES #{}\nBEFORE: {}\nAFTER: {}\nREASON: {}",
            self.supersedes,
            self.before.as_deref().unwrap_or("(none)"),
            self.after,
            self.reason
        )
    }

    /// Parse a header back (round-trip tested; the queue never
    /// parses — humans read it — but tests and migrations do).
    pub fn parse(text: &str) -> Result<Self> {
        let mut supersedes = None;
        let mut before = None;
        let mut after = None;
        let mut reason = None;
        for line in text.lines() {
            if let Some(id) = line.strip_prefix("SUPERSEDES #") {
                supersedes = Some(
                    id.trim()
                        .parse::<i64>()
                        .map_err(|_| Error::Validate(format!("bad SUPERSEDES id: {line}")))?,
                );
            } else if let Some(value) = line.strip_prefix("BEFORE: ") {
                before = Some(value.to_string());
            } else if let Some(value) = line.strip_prefix("AFTER: ") {
                after = Some(value.to_string());
            } else if let Some(value) = line.strip_prefix("REASON: ") {
                reason = Some(value.to_string());
            }
        }
        let (Some(supersedes), Some(before), Some(after), Some(reason)) =
            (supersedes, before, after, reason)
        else {
            return Err(Error::Validate(
                "change header is missing lines".to_string(),
            ));
        };
        Ok(Change {
            supersedes,
            before: (before != "(none)").then_some(before),
            after,
            reason,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_round_trips() {
        let change = Change {
            supersedes: 41,
            before: Some("filing: $20".to_string()),
            after: "filing: $30".to_string(),
            reason: "capture #7 raises the fee".to_string(),
        };
        let text = change.header();
        assert!(text.starts_with("SUPERSEDES #41\n"));
        assert_eq!(Change::parse(&text).unwrap(), change);
        let fresh = Change {
            supersedes: 0,
            before: None,
            after: "walkin: none".to_string(),
            reason: "capture #8".to_string(),
        };
        assert!(fresh.header().contains("BEFORE: (none)"));
        assert_eq!(Change::parse(&fresh.header()).unwrap(), fresh);
    }

    #[test]
    fn truncated_header_fails() {
        assert!(Change::parse("AFTER: x").is_err());
        assert!(Change::parse("SUPERSEDES #x\nBEFORE: a\nAFTER: b\nREASON: c").is_err());
    }
}
