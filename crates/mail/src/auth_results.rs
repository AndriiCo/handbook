//! `Authentication-Results` parsing (RFC 8601) into trust evidence.
//!
//! Gmail prepends its delivery stamp; forwarding hops may add more.
//! Only the stamp whose `authserv-id` is `mx.google.com` is trusted,
//! and of those the topmost (headers prepend, so topmost = final
//! delivery). Comments in parentheses are stripped before parsing.
//! Missing methods record `"none"` — absence of evidence, explicit.
//!
//! [`AuthResults::verified_for`] is the trust gate: the poller grants
//! `official` only when the stamp authenticates the From domain the
//! poller itself parsed (DMARC pass for that domain, or a passing DKIM
//! signature aligned with it). SPF alone never counts — it vouches for
//! the envelope sender, not the From a reader sees.

/// One `dkim=` clause. A message may carry several signatures (the
/// sender's own, its mail provider's), and Gmail stamps each one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DkimVerdict {
    pub result: String,
    /// Signing domain: `header.d`, else the domain of `header.i`
    /// (Gmail stamps `header.i=@example.gov`).
    pub domain: String,
}

/// Delivery-authentication verdicts from one trusted stamp.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthResults {
    /// First `dkim=` clause (evidence summary; see `dkim_all`).
    pub dkim: String,
    pub dkim_domain: String,
    /// Every `dkim=` clause, in stamp order.
    pub dkim_all: Vec<DkimVerdict>,
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
            dkim_all: Vec::new(),
            spf: "none".to_string(),
            spf_mailfrom: String::new(),
            dmarc: "none".to_string(),
            from_domain: String::new(),
        }
    }

    /// How this stamp authenticates mail whose From domain is `from`:
    /// `"dmarc"` (DMARC pass evaluated for that same domain), `"dkim"`
    /// (a passing signature aligned with it), else `None`. The stamp's
    /// own `header.from` must match: a DMARC pass Gmail computed for a
    /// different domain than the one we parsed says nothing about ours.
    pub fn verified_for(&self, from: &str) -> Option<&'static str> {
        let from = normalize_domain(from);
        if from.is_empty() {
            return None;
        }
        if self.dmarc.eq_ignore_ascii_case("pass") && normalize_domain(&self.from_domain) == from {
            return Some("dmarc");
        }
        self.dkim_all
            .iter()
            .any(|sig| sig.result.eq_ignore_ascii_case("pass") && aligned(&sig.domain, &from))
            .then_some("dkim")
    }
}

/// Relaxed alignment without a public-suffix list: the signing domain
/// equals the From domain, or one is a subdomain of the other. A
/// single-label signer (`gov`) never aligns.
fn aligned(signer: &str, from: &str) -> bool {
    let signer = normalize_domain(signer);
    if !signer.contains('.') {
        return false;
    }
    signer == from || from.ends_with(&format!(".{signer}")) || signer.ends_with(&format!(".{from}"))
}

fn normalize_domain(domain: &str) -> String {
    domain.trim().trim_end_matches('.').to_ascii_lowercase()
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
                    let mut domain = param("header.d");
                    if domain.is_empty() {
                        domain = param("header.i")
                            .rsplit_once('@')
                            .map(|(_, d)| d.to_string())
                            .unwrap_or_default();
                    }
                    if out.dkim_all.is_empty() {
                        out.dkim = verdict.clone();
                        out.dkim_domain = domain.clone();
                    }
                    out.dkim_all.push(DkimVerdict {
                        result: verdict,
                        domain,
                    });
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
    fn keeps_every_dkim_signature() {
        // Gmail's real shape: `header.i=@domain`, one clause per
        // signature. The aligned pass must survive a later clause.
        let values = vec![
            "mx.google.com; dkim=pass header.i=@example.gov header.s=s1 header.b=AbC; dkim=pass header.i=@esp.test header.s=s2; spf=pass smtp.mailfrom=bounce@esp.test; dmarc=none header.from=example.gov"
                .to_string(),
        ];
        let parsed = parse(&values);
        assert_eq!(parsed.dkim, "pass");
        assert_eq!(parsed.dkim_domain, "example.gov");
        let domains: Vec<&str> = parsed.dkim_all.iter().map(|d| d.domain.as_str()).collect();
        assert_eq!(domains, ["example.gov", "esp.test"]);
        assert_eq!(parsed.verified_for("example.gov"), Some("dkim"));
        assert_eq!(parsed.verified_for("Example.GOV."), Some("dkim"));
        // Reversed order: the aligned signature is still found.
        let reversed = parse(&[
            "mx.google.com; dkim=pass header.d=esp.test; dkim=pass header.d=example.gov"
                .to_string(),
        ]);
        assert_eq!(reversed.verified_for("example.gov"), Some("dkim"));
    }

    #[test]
    fn verification_needs_dmarc_for_the_same_domain_or_aligned_dkim() {
        let stamp = |s: &str| parse(&[format!("mx.google.com; {s}")]);
        let dmarc = stamp("dmarc=pass header.from=example.gov");
        assert_eq!(dmarc.verified_for("example.gov"), Some("dmarc"));
        // A DMARC pass Gmail computed for another domain (the attacker's
        // own, with our parser fooled by the display name) says nothing.
        assert_eq!(dmarc.verified_for("sos.example.gov"), None);
        assert_eq!(dmarc.verified_for(""), None);
        let failed =
            stamp("dkim=fail header.d=example.gov; spf=pass; dmarc=fail header.from=example.gov");
        assert_eq!(failed.verified_for("example.gov"), None);
        // SPF alone vouches for the envelope, not the From.
        assert_eq!(
            stamp("spf=pass smtp.mailfrom=ops@example.gov").verified_for("example.gov"),
            None
        );
        // Relaxed alignment: parent or child signer, never a bare TLD
        // or an unrelated look-alike.
        assert_eq!(
            stamp("dkim=pass header.d=example.gov").verified_for("sos.example.gov"),
            Some("dkim")
        );
        assert_eq!(
            stamp("dkim=pass header.d=mail.example.gov").verified_for("example.gov"),
            Some("dkim")
        );
        assert_eq!(
            stamp("dkim=pass header.d=gov").verified_for("example.gov"),
            None
        );
        assert_eq!(
            stamp("dkim=pass header.d=badexample.gov").verified_for("example.gov"),
            None
        );
        assert_eq!(
            stamp("dkim=neutral header.d=example.gov").verified_for("example.gov"),
            None
        );
        assert_eq!(AuthResults::none().verified_for("example.gov"), None);
    }

    #[test]
    fn missing_header_is_all_none() {
        let parsed = parse(&[]);
        assert_eq!(parsed, AuthResults::none());
        let parsed = parse(&["mx.other.test; dkim=pass".to_string()]);
        assert_eq!(parsed, AuthResults::none());
    }
}
