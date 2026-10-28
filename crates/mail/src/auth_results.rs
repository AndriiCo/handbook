//! `Authentication-Results` parsing (RFC 8601) into trust evidence.
//!
//! Gmail prepends its delivery stamp; forwarding hops may add more.
//! Only the stamp whose `authserv-id` is `mx.google.com` is trusted,
//! and of those the topmost (headers prepend, so topmost = final
//! delivery). Comments in parentheses are stripped before parsing.
//! Missing methods record `"none"` — absence of evidence, explicit.

/// Delivery-authentication verdicts from one trusted stamp.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthResults {
    pub dkim: String,
    pub dkim_domain: String,
    pub spf: String,
    pub spf_mailfrom: String,
    pub dmarc: String,
    pub from_domain: String,
}

impl AuthResults {
    /// All-`none`: no trusted stamp (or no header at all).
    pub fn none() -> Self {
        AuthResults {
            dkim: "none".to_string(),
            dkim_domain: String::new(),
            spf: "none".to_string(),
            spf_mailfrom: String::new(),
            dmarc: "none".to_string(),
            from_domain: String::new(),
        }
    }
}

/// Parse the trusted stamp from `Authentication-Results` header values
/// (topmost first, as collected). Returns [`AuthResults::none`] when no
/// `mx.google.com` stamp exists.
pub fn parse(values: &[String]) -> AuthResults {
    for value in values {
        let stripped = strip_comments(value);
        let mut clauses = stripped.split(';');
        let authserv = clauses.next().unwrap_or("").trim().to_ascii_lowercase();
        if authserv != "mx.google.com" {
            continue;
        }
        let mut out = AuthResults::none();
        for clause in clauses {
            let mut pairs = clause.split_whitespace();
            let Some(method) = pairs.next() else { continue };
            let (method, verdict) = match method.split_once('=') {
                Some((m, v)) => (m.trim().to_ascii_lowercase(), v.trim().to_string()),
                None => continue,
            };
            let param = |name: &str| {
                pairs
                    .clone()
                    .filter_map(|p| p.split_once('='))
                    .find(|(k, _)| k.trim().eq_ignore_ascii_case(name))
                    .map(|(_, v)| v.trim().to_string())
                    .unwrap_or_default()
            };
            match method.as_str() {
                "dkim" => {
                    out.dkim = verdict;
                    out.dkim_domain = param("header.d");
                }
                "spf" => {
                    out.spf = verdict;
                    out.spf_mailfrom = param("smtp.mailfrom");
                }
                "dmarc" => {
                    out.dmarc = verdict;
                    out.from_domain = param("header.from");
                }
                _ => {}
            }
        }
        return out;
    }
    AuthResults::none()
}

/// Remove `(comment)` groups (non-nested; RFC comments nest, but Gmail
/// stamps never nest them — an unbalanced paren ends stripping).
fn strip_comments(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut depth = 0u32;
    for ch in value.chars() {
        match ch {
            '(' => depth += 1,
            ')' if depth > 0 => depth -= 1,
            _ if depth == 0 => out.push(ch),
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_topmost_google_stamp() {
        let values = vec![
            "mx.google.com; dkim=pass header.d=example.gov; spf=pass smtp.mailfrom=x@example.gov; dmarc=pass header.from=example.gov"
                .to_string(),
            "mx.evil.test; dkim=pass header.d=example.gov".to_string(),
        ];
        let parsed = parse(&values);
        assert_eq!(parsed.dkim, "pass");
        assert_eq!(parsed.dkim_domain, "example.gov");
        assert_eq!(parsed.spf, "pass");
        assert_eq!(parsed.spf_mailfrom, "x@example.gov");
        assert_eq!(parsed.dmarc, "pass");
        assert_eq!(parsed.from_domain, "example.gov");
    }

    #[test]
    fn skips_forged_stamps_for_the_google_one() {
        let values = vec![
            // Non-google stamps are skipped even above the google one
            // (forwarding chains); only mx.google.com is trusted.
            "mx.forwarder.test; dkim=pass header.d=example.gov".to_string(),
            "mx.google.com; dkim=fail header.d=example.gov; dmarc=fail header.from=example.gov"
                .to_string(),
        ];
        let parsed = parse(&values);
        assert_eq!(parsed.dkim, "fail");
        assert_eq!(parsed.spf, "none");
        assert_eq!(parsed.dmarc, "fail");
    }

    #[test]
    fn strips_comments_before_parsing() {
        let values =
            vec!["mx.google.com; dkim=pass (good sig) header.d=example.gov; spf=none".to_string()];
        let parsed = parse(&values);
        assert_eq!(parsed.dkim, "pass");
        assert_eq!(parsed.dkim_domain, "example.gov");
        assert_eq!(parsed.spf, "none");
    }

    #[test]
    fn missing_header_is_all_none() {
        let parsed = parse(&[]);
        assert_eq!(parsed, AuthResults::none());
        let parsed = parse(&["mx.other.test; dkim=pass".to_string()]);
        assert_eq!(parsed, AuthResults::none());
    }
}
