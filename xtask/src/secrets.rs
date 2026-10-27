//! Secret scanner: keeps PII/secrets out of commits and CI.
//!
//! `cargo xtask secrets-scan` checks every tracked file;
//! `secrets-scan --staged` checks what `git commit` is about to
//! record (the pre-commit hook). Any hit fails the command.
//!
//! This file deliberately contains no scannable secret: needles are
//! written so they never match their own source lines, and test
//! fixtures are assembled from fragments.

use std::path::Path;

/// One scanner finding.
pub struct Hit {
    pub file: String,
    pub line: usize,
    pub rule: &'static str,
}

/// Basenames / extensions that must never be committed.
pub fn filename_hit(path: &str) -> Option<&'static str> {
    let base = path.rsplit('/').next().unwrap_or(path);
    if base == ".env" || base.starts_with(".env.") {
        return Some("filename-dotenv");
    }
    if base == "id_rsa"
        || base == "id_ed25519"
        || base == "id_ecdsa"
        || base.starts_with("id_rsa.")
        || base.starts_with("id_ed25519.")
    {
        // `*.pub` is fine (handled by falling through: only the exact
        // private names and their non-pub suffixed forms match).
        if !base.ends_with(".pub") {
            return Some("filename-ssh-key");
        }
    }
    match Path::new(base).extension().and_then(|e| e.to_str()) {
        Some("pem" | "key" | "p12" | "pfx" | "jks" | "asc" | "gpg") => {
            Some("filename-key-material")
        }
        _ => None,
    }
}

/// NUL byte in the head = binary: skip (rules are text-only).
fn is_binary(bytes: &[u8]) -> bool {
    bytes.iter().take(8192).any(|b| *b == 0)
}

/// Scan one file's bytes; binary files are skipped, not failed.
pub fn scan_bytes(file: &str, bytes: &[u8]) -> Vec<Hit> {
    if is_binary(bytes) {
        return Vec::new();
    }
    if let Some(rule) = filename_hit(file) {
        return vec![Hit {
            file: file.to_string(),
            line: 0,
            rule,
        }];
    }
    let text = String::from_utf8_lossy(bytes);
    let mut hits = Vec::new();
    for (index, line) in text.lines().enumerate() {
        for rule in line_hits(line) {
            hits.push(Hit {
                file: file.to_string(),
                line: index + 1,
                rule,
            });
        }
    }
    hits
}

/// True when `rest` starts with `n` chars from `alphabet`.
fn takes(rest: &str, n: usize, alphabet: &[u8]) -> bool {
    rest.bytes().take(n).len() == n && rest.bytes().take(n).all(|b| alphabet.contains(&b))
}

const B64ISH: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789_-";
const UPPER_DIGIT: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";

/// All rules matching one line. Needles are split or context-gated
/// so these source lines never match themselves.
fn line_hits(line: &str) -> Vec<&'static str> {
    let mut out = Vec::new();
    // PEM private key: the BEGIN needle is spelled with an escape so
    // this line does not contain the scanned sequence.
    let begin = "-----BEGIN\u{20}";
    if line.contains(begin) && line.contains("PRIVATE KEY") {
        out.push("pem-private-key");
    }
    if let Some(i) = line.find("AKIA")
        && takes(&line[i + 4..], 16, UPPER_DIGIT)
    {
        out.push("aws-access-key");
    }
    for prefix in ["ghp_", "gho_", "ghs_", "github_pat_"] {
        if let Some(i) = line.find(prefix)
            && takes(&line[i + prefix.len()..], 20, B64ISH)
        {
            out.push("github-token");
            break;
        }
    }
    for prefix in ["sk_live_", "sk_test_", "rk_live_", "rk_test_"] {
        if let Some(i) = line.find(prefix)
            && takes(&line[i + prefix.len()..], 8, B64ISH)
        {
            out.push("stripe-key");
            break;
        }
    }
    // Slack prefixes are stored dash-less and joined at match time so
    // these source lines never contain a scannable token prefix.
    for prefix in ["xoxb", "xoxp", "xoxa", "xoxr", "xoxs"] {
        if line.contains(&[prefix, "-"].concat()) {
            out.push("slack-token");
            break;
        }
    }
    if let Some(i) = line.find("AIza")
        && takes(&line[i + 4..], 35, B64ISH)
    {
        out.push("google-api-key");
    }
    // META_API_KEY with an assigned value (a `=`/`:` separator is
    // required, so prose mentions like "`META_API_KEY` spelling"
    // never match).
    if let Some(i) = line.find("META_API_KEY") {
        let rest = line[i + "META_API_KEY".len()..].trim_start_matches([' ', '\t', '"', '\'', '`']);
        let rest = rest.strip_prefix(['=', ':']).unwrap_or("");
        let rest = rest.trim_start_matches([' ', '\t', '"', '\'', '`']);
        if takes(rest, 8, B64ISH) {
            out.push("meta-api-key");
        }
    }
    // Generic KEY=VALUE assignments. Markers sit in an array with no
    // `=`/`:` nearby, so these source lines never match themselves.
    let lower = line.to_lowercase();
    for marker in [
        "api_key",
        "apikey",
        "api-key",
        "api_secret",
        "secret_key",
        "secretkey",
        "access_token",
        "auth_token",
        "client_secret",
    ] {
        if let Some(i) = lower.find(marker) {
            let rest = &line[i + marker.len()..];
            let mut chars = rest.chars();
            let mut ok = false;
            for _ in 0..8 {
                match chars.next() {
                    Some(' ' | '\t' | '"' | '\'' | '`') => continue,
                    Some('=' | ':') => {
                        ok = true;
                        break;
                    }
                    _ => break,
                }
            }
            if ok {
                let after: String = chars.collect();
                let value = after.trim_start_matches([' ', '\t', '"', '\'', '`']);
                if takes(
                    value,
                    16,
                    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789_-.+/=",
                ) {
                    out.push("generic-secret-assignment");
                    break;
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Assemble a realistic-looking AWS key from fragments so this
    /// source never contains a scannable secret itself.
    fn aws_example() -> String {
        ["AK", "IA", "IOSFODNN7EXAMPLE"].concat()
    }

    #[test]
    fn detects_aws_key() {
        assert!(line_hits(&format!("key={}", aws_example())).contains(&"aws-access-key"));
        assert!(line_hits("prefix AKIA short").is_empty());
    }

    #[test]
    fn detects_pem_key_but_not_certificate_mention() {
        let begin: String = ["-----BEGIN", " "].concat();
        assert!(
            line_hits(&format!("{begin}OPENSSH PRIVATE KEY-----")).contains(&"pem-private-key")
        );
        assert!(line_hits("-----BEGIN CERTIFICATE-----").is_empty());
    }

    #[test]
    fn detects_provider_tokens() {
        let gh: String = ["ghp_", &"a".repeat(24)].concat();
        assert!(line_hits(&gh).contains(&"github-token"));
        let sk: String = ["sk_live_", &"b".repeat(24)].concat();
        assert!(line_hits(&sk).contains(&"stripe-key"));
        let sx: String = ["token=xoxb", "-1234"].concat();
        assert!(line_hits(&sx).contains(&"slack-token"));
        let gz: String = ["AIza", &"c".repeat(35)].concat();
        assert!(line_hits(&gz).contains(&"google-api-key"));
    }

    #[test]
    fn meta_key_needs_a_value() {
        assert!(line_hits("set META_API_KEY in the environment").is_empty());
        let with_value: String = ["META_API_KEY=", &"d".repeat(24)].concat();
        assert!(line_hits(&with_value).contains(&"meta-api-key"));
    }

    #[test]
    fn generic_assignment_needs_separator_and_value() {
        assert!(line_hits("the api_key belongs to the user").is_empty());
        assert!(line_hits("api_key = ").is_empty());
        let hit: String = ["api_key=", &"e".repeat(20)].concat();
        assert!(line_hits(&hit).contains(&"generic-secret-assignment"));
    }

    #[test]
    fn filename_rules_catch_key_material() {
        assert_eq!(filename_hit("x/server.pem"), Some("filename-key-material"));
        assert_eq!(filename_hit("x/.env"), Some("filename-dotenv"));
        assert_eq!(filename_hit("x/.env.prod"), Some("filename-dotenv"));
        assert_eq!(filename_hit("x/id_ed25519"), Some("filename-ssh-key"));
        assert_eq!(filename_hit("x/id_ed25519.pub"), None);
        assert_eq!(filename_hit("src/main.rs"), None);
    }

    #[test]
    fn binary_files_are_skipped() {
        assert!(scan_bytes("x/bin", b"AK\x00IAIOSFODNN7EXAMPLE".as_slice()).is_empty());
    }
}
