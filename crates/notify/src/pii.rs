//! Personal-data patterns: one definition for every gate.
//!
//! Email addresses, phone-like digit runs and SSN-shaped numbers. The
//! mail poller redacts them from email sidecars (the text the model
//! reads and follower quotes are cut from), the digest builder drops a
//! quote that still carries one, and `xtask pii-sweep` refuses a store
//! where any survive. Sharing the patterns means a redacted sidecar
//! passes the sweep by construction.

use std::sync::LazyLock;

use regex::Regex;

/// An address-shaped token (`local@domain.tld`).
pub const EMAIL: &str = r"[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}";
/// Ten digits in 3-3-4 groups, optional `+1`, any separators.
pub const PHONE: &str = r"\+?1?[-.\s()]*\d{3}[-.\s()]*\d{3}[-.\s()]*\d{4}";
/// `123-45-6789`.
pub const SSN: &str = r"\b\d{3}-\d{2}-\d{4}\b";

static EMAIL_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(EMAIL).expect("email pattern"));
static PHONE_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(PHONE).expect("phone pattern"));
static SSN_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(SSN).expect("ssn pattern"));

/// Names of the patterns `text` matches (`email`, `phone`, `ssn`), in
/// that order; empty when clean.
pub fn matches(text: &str) -> Vec<&'static str> {
    let mut out = Vec::new();
    if EMAIL_RE.is_match(text) {
        out.push("email");
    }
    if PHONE_RE.is_match(text) {
        out.push("phone");
    }
    if SSN_RE.is_match(text) {
        out.push("ssn");
    }
    out
}

/// True when any pattern matches.
pub fn has_personal_data(text: &str) -> bool {
    !matches(text).is_empty()
}

/// Replace every match with `[ssn]`, `[email]` or `[phone]`. Leading
/// whitespace a phone match swallowed is kept, so line breaks survive.
/// Repeats until clean (a replacement never joins digits, but the loop
/// makes "clean afterwards" a guarantee rather than an argument).
pub fn redact(text: &str) -> String {
    let mut out = text.to_string();
    for _ in 0..4 {
        if !has_personal_data(&out) {
            break;
        }
        out = SSN_RE.replace_all(&out, "[ssn]").into_owned();
        out = EMAIL_RE.replace_all(&out, "[email]").into_owned();
        out = PHONE_RE
            .replace_all(&out, |caps: &regex::Captures<'_>| {
                let hit = &caps[0];
                let lead = hit.len() - hit.trim_start().len();
                format!("{}[phone]", &hit[..lead])
            })
            .into_owned();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redaction_leaves_nothing_the_sweep_would_flag() {
        // Fictional values only: example.invalid, 555 numbers, 000 SSN.
        let text = "Reply to clerk@example.invalid or Jane.Doe+x@mail.example.gov.\n\
                    Call\n(555) 010-0199, +1 555.010.0123 or 5550100123.\n\
                    SSN 000-12-3456 on file. Fee: $30 per document, ZIP 98504-0234.";
        assert_eq!(matches(text), ["email", "phone", "ssn"]);
        let clean = redact(text);
        assert!(matches(&clean).is_empty(), "{clean}");
        assert!(clean.contains("Reply to [email] or [email]."), "{clean}");
        assert!(
            clean.contains("Call\n[phone], [phone] or [phone]."),
            "{clean}"
        );
        assert!(clean.contains("SSN [ssn] on file."), "{clean}");
        // Public facts survive: fees, postcodes, dates.
        assert!(
            clean.contains("$30 per document, ZIP 98504-0234."),
            "{clean}"
        );
        assert_eq!(
            redact("Closed Mon, 26 Oct 2026 09:00."),
            "Closed Mon, 26 Oct 2026 09:00."
        );
    }
}
