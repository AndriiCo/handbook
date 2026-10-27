//! ACME client configuration: Google Public CA endpoints, domains, EAB, paths.
//!
//! The only CA we may use is Google PKI
//! (`docs/ops/certificates.md`), so the directory defaults to Google
//! prod; staging and custom URLs exist for the manual staging run
//! and for tests.

use std::path::{Path, PathBuf};

use crate::{AcmeError, Result};

/// Google Public CA production ACME directory (verified 2026-10-26).
pub const GOOGLE_PROD_DIRECTORY: &str = "https://dv.acme-v02.api.pki.goog/directory";
/// Google Public CA staging ACME directory (untrusted certs, same checks).
pub const GOOGLE_STAGING_DIRECTORY: &str = "https://dv.acme-v02.test-api.pki.goog/directory";
/// Domains covered by the single issued certificate.
pub const DEFAULT_DOMAINS: [&str; 2] = ["andrii.ai", "admin.andrii.ai"];

/// ACME client configuration. Build with [`AcmeConfig::builder`]
/// (validation happens in `build`) or [`AcmeConfig::from_stored`].
/// The EAB HMAC is redacted in `Debug`.
#[derive(Clone)]
pub struct AcmeConfig {
    directory_url: String,
    domains: Vec<String>,
    email: Option<String>,
    eab_kid: String,
    eab_hmac: Vec<u8>,
    challenge_dir: PathBuf,
    cert_dir: PathBuf,
}

impl std::fmt::Debug for AcmeConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AcmeConfig")
            .field("directory_url", &self.directory_url)
            .field("domains", &self.domains)
            .field("email", &self.email)
            .field("eab_kid", &self.eab_kid)
            .field("eab_hmac", &"<redacted>")
            .field("challenge_dir", &self.challenge_dir)
            .field("cert_dir", &self.cert_dir)
            .finish()
    }
}

/// DB-backed values for [`AcmeConfig::from_stored`]: one field
/// per `acme.*` config key (`docs/specs/config-db.md` §2.1).
/// Empty strings behave like missing values (defaults apply).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredAcme {
    /// `prod`, `staging`, or a full `https://` directory URL.
    pub directory: String,
    /// Comma-separated domains (empty = both prod names).
    pub domains_csv: String,
    /// Optional contact mailbox (no `mailto:` prefix).
    pub email: Option<String>,
    /// Google EAB key ID (required, with the HMAC).
    pub eab_kid: Option<String>,
    /// Google EAB HMAC, base64 (required, with the KID).
    pub eab_hmac_b64: Option<String>,
}

/// Builder for [`AcmeConfig`]; `build` validates every field.
pub struct AcmeConfigBuilder {
    directory_url: String,
    domains: Vec<String>,
    email: Option<String>,
    eab_kid: Option<String>,
    eab_hmac_b64: Option<String>,
    challenge_dir: PathBuf,
    cert_dir: PathBuf,
}

impl AcmeConfig {
    /// Start a builder. The challenge dir serves http-01 token
    /// files; the cert dir persists the pair + ACME account.
    pub fn builder(challenge_dir: PathBuf, cert_dir: PathBuf) -> AcmeConfigBuilder {
        AcmeConfigBuilder {
            directory_url: GOOGLE_PROD_DIRECTORY.to_string(),
            domains: DEFAULT_DOMAINS.iter().map(ToString::to_string).collect(),
            email: None,
            eab_kid: None,
            eab_hmac_b64: None,
            challenge_dir,
            cert_dir,
        }
    }

    /// Read config from DB-backed values (goal-025 stream 4,
    /// zero-env; replaces the old env mapping 1:1). `directory` is
    /// `prod` (default), `staging`, or a full directory URL;
    /// `domains_csv` is comma-separated (default: both prod names);
    /// `email` is an optional contact; `eab_kid` / `eab_hmac_b64`
    /// are the required Google EAB pair. Certs persist under
    /// `<store_dir>/acme/`.
    ///
    /// # Errors
    ///
    /// Missing EAB values or anything [`AcmeConfigBuilder::build`]
    /// rejects.
    pub fn from_stored(
        store_dir: &Path,
        challenge_dir: PathBuf,
        stored: StoredAcme,
    ) -> Result<Self> {
        let directory_url = match stored.directory.trim() {
            "" | "prod" => GOOGLE_PROD_DIRECTORY.to_string(),
            "staging" => GOOGLE_STAGING_DIRECTORY.to_string(),
            url => url.to_string(),
        };
        let domains: Vec<String> = if stored.domains_csv.trim().is_empty() {
            DEFAULT_DOMAINS.iter().map(ToString::to_string).collect()
        } else {
            stored
                .domains_csv
                .split(',')
                .map(|d| d.trim().to_string())
                .collect()
        };
        let mut builder = AcmeConfig::builder(challenge_dir, store_dir.join("acme"))
            .directory(directory_url)
            .domains(domains);
        if let Some(email) = stored.email.filter(|e| !e.trim().is_empty()) {
            builder = builder.email(email);
        }
        match (stored.eab_kid, stored.eab_hmac_b64) {
            (Some(kid), Some(hmac)) if !kid.trim().is_empty() && !hmac.trim().is_empty() => {
                builder = builder.eab(kid, hmac);
            }
            _ => {}
        }
        builder.build()
    }

    /// ACME directory URL (Google prod unless overridden).
    pub fn directory_url(&self) -> &str {
        &self.directory_url
    }

    /// Domains on the certificate (one order, one cert).
    pub fn domains(&self) -> &[String] {
        &self.domains
    }

    /// Optional `mailto:` contact (without the prefix).
    pub fn email(&self) -> Option<&str> {
        self.email.as_deref()
    }

    /// Google EAB key ID (public identifier, safe to log).
    pub fn eab_kid(&self) -> &str {
        &self.eab_kid
    }

    /// Raw EAB HMAC key bytes (decoded from base64 at build).
    pub fn eab_hmac(&self) -> &[u8] {
        &self.eab_hmac
    }

    /// Directory whose files answer `/.well-known/acme-challenge/`.
    pub fn challenge_dir(&self) -> &Path {
        &self.challenge_dir
    }

    /// Directory holding `cert.pem`, `key.pem`, `account.json`.
    pub fn cert_dir(&self) -> &Path {
        &self.cert_dir
    }

    /// True when pointing at Google staging (certs untrusted by design).
    pub fn staging(&self) -> bool {
        self.directory_url == GOOGLE_STAGING_DIRECTORY
    }
}

impl AcmeConfigBuilder {
    /// ACME directory URL (default: Google prod).
    pub fn directory(mut self, url: String) -> Self {
        self.directory_url = url;
        self
    }

    /// Domains for the single certificate (default: both prod names).
    pub fn domains(mut self, domains: Vec<String>) -> Self {
        self.domains = domains;
        self
    }

    /// Contact email for the ACME account (no `mailto:` prefix).
    pub fn email(mut self, email: String) -> Self {
        self.email = Some(email);
        self
    }

    /// Google EAB pair: key ID plus base64-encoded HMAC key
    /// (standard or URL-safe alphabet, padding optional).
    pub fn eab(mut self, kid: String, hmac_b64: String) -> Self {
        self.eab_kid = Some(kid);
        self.eab_hmac_b64 = Some(hmac_b64);
        self
    }

    /// Validate and finish. Every failure names the offending field;
    /// secrets never appear in the message.
    ///
    /// # Errors
    ///
    /// Non-HTTPS directory, empty/bad domain list, bad email shape,
    /// or missing/undecodable EAB credentials.
    pub fn build(self) -> Result<AcmeConfig> {
        if !self.directory_url.starts_with("https://") {
            return Err(AcmeError::Config(
                "directory must be an https:// URL".to_string(),
            ));
        }
        if self.domains.is_empty() || self.domains.iter().any(|d| !valid_domain(d)) {
            return Err(AcmeError::Config(
                "domains must be non-empty DNS names (letters, digits, dots, dashes)".to_string(),
            ));
        }
        if let Some(email) = &self.email
            && (email.contains(' ') || !email.contains('@'))
        {
            return Err(AcmeError::Config(
                "email must look like a mailbox (user@host)".to_string(),
            ));
        }
        let Some(kid) = self.eab_kid.filter(|k| !k.trim().is_empty()) else {
            return Err(AcmeError::Config("EAB key id is required".to_string()));
        };
        let Some(hmac_b64) = self.eab_hmac_b64.filter(|h| !h.trim().is_empty()) else {
            return Err(AcmeError::Config("EAB HMAC key is required".to_string()));
        };
        let eab_hmac = decode_b64(hmac_b64.trim())
            .ok_or_else(|| AcmeError::Config("EAB HMAC is not valid base64".to_string()))?;
        if eab_hmac.is_empty() {
            return Err(AcmeError::Config(
                "EAB HMAC decodes to zero bytes".to_string(),
            ));
        }
        Ok(AcmeConfig {
            directory_url: self.directory_url,
            domains: self.domains,
            email: self.email,
            eab_kid: kid,
            eab_hmac,
            challenge_dir: self.challenge_dir,
            cert_dir: self.cert_dir,
        })
    }
}

/// Conservative DNS-name shape: dot-separated labels of ASCII
/// letters, digits, and dashes, no empty labels.
fn valid_domain(domain: &str) -> bool {
    !domain.is_empty()
        && domain.len() <= 253
        && domain.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && label
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
                && !label.starts_with('-')
                && !label.ends_with('-')
        })
}

/// gcloud emits standard base64; accept the URL-safe and unpadded
/// spellings too so paste variants all work.
fn decode_b64(input: &str) -> Option<Vec<u8>> {
    use base64::Engine as _;
    [input.to_string(), format!("{input}=="), format!("{input}=")]
        .into_iter()
        .flat_map(|candidate| {
            [
                base64::engine::general_purpose::STANDARD.decode(&candidate),
                base64::engine::general_purpose::URL_SAFE.decode(&candidate),
                base64::engine::general_purpose::STANDARD_NO_PAD.decode(input),
                base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(input),
            ]
        })
        .find_map(std::result::Result::ok)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn builder() -> AcmeConfigBuilder {
        AcmeConfig::builder(PathBuf::from("acme-dir"), PathBuf::from("cert-dir"))
            .eab("kid-1".to_string(), "aGk".to_string())
    }

    #[test]
    fn defaults_cover_both_prod_names_on_google_prod() {
        let config = builder().build().unwrap();
        assert_eq!(config.directory_url(), GOOGLE_PROD_DIRECTORY);
        assert!(!config.staging());
        assert_eq!(
            config.domains(),
            &["andrii.ai".to_string(), "admin.andrii.ai".to_string()]
        );
        assert_eq!(config.email(), None);
        assert_eq!(config.eab_kid(), "kid-1");
        assert_eq!(config.eab_hmac(), b"hi");
    }

    #[test]
    fn staging_and_custom_urls_parse() {
        let staging = builder()
            .directory("staging".to_string())
            .build()
            .expect_err("bare 'staging' is not a URL");
        assert!(staging.to_string().contains("https"), "{staging}");
        let staging = builder()
            .directory(GOOGLE_STAGING_DIRECTORY.to_string())
            .build()
            .unwrap();
        assert!(staging.staging());
        assert!(
            builder()
                .directory("http://x/y".to_string())
                .build()
                .is_err()
        );
    }

    #[test]
    fn domains_and_email_are_validated() {
        assert!(builder().domains(Vec::new()).build().is_err());
        for bad in [
            "",
            "has space",
            "under_score",
            "-lead.test",
            "trail-.test",
            "double..dot",
        ] {
            assert!(
                builder().domains(vec![bad.to_string()]).build().is_err(),
                "domain {bad:?} must fail"
            );
        }
        assert!(
            builder()
                .email("ops@example.com".to_string())
                .build()
                .is_ok()
        );
        for bad in ["no-at-sign", "two @ signs"] {
            assert!(
                builder().email(bad.to_string()).build().is_err(),
                "email {bad:?} must fail"
            );
        }
    }

    #[test]
    fn eab_is_required_and_base64_checked() {
        let bare = AcmeConfig::builder(PathBuf::from("a"), PathBuf::from("c"));
        assert!(bare.build().is_err(), "missing EAB must fail");
        assert!(
            AcmeConfig::builder(PathBuf::from("a"), PathBuf::from("c"))
                .eab("kid".to_string(), "!!!not-b64!!!".to_string())
                .build()
                .is_err()
        );
        // Padded, unpadded, and URL-safe spellings all decode.
        for spelling in ["aGk=", "aGk", "aGk"] {
            let config = AcmeConfig::builder(PathBuf::from("a"), PathBuf::from("c"))
                .eab("kid".to_string(), spelling.to_string())
                .build()
                .unwrap();
            assert_eq!(config.eab_hmac(), b"hi", "spelling {spelling}");
        }
    }

    #[test]
    fn debug_redacts_the_hmac() {
        let shown = format!("{:?}", builder().build().unwrap());
        assert!(shown.contains("kid-1"), "{shown}");
        assert!(!shown.contains("aGk"), "{shown}");
        assert!(shown.contains("redacted"), "{shown}");
    }

    fn stored() -> StoredAcme {
        StoredAcme {
            directory: "prod".to_string(),
            domains_csv: String::new(),
            email: None,
            eab_kid: Some("kid-1".to_string()),
            eab_hmac_b64: Some("aGk".to_string()),
        }
    }

    #[test]
    fn from_stored_maps_selectors_and_defaults() {
        let config =
            AcmeConfig::from_stored(Path::new("store"), PathBuf::from("acme-dir"), stored())
                .unwrap();
        assert_eq!(config.directory_url(), GOOGLE_PROD_DIRECTORY);
        assert_eq!(
            config.domains(),
            &["andrii.ai".to_string(), "admin.andrii.ai".to_string()]
        );
        assert_eq!(config.email(), None);
        assert_eq!(config.cert_dir(), Path::new("store/acme"));
        assert_eq!(config.challenge_dir(), Path::new("acme-dir"));

        let staging = AcmeConfig::from_stored(
            Path::new("store"),
            PathBuf::from("acme-dir"),
            StoredAcme {
                directory: "staging".to_string(),
                domains_csv: "a.test, b.test".to_string(),
                email: Some("ops@example.test".to_string()),
                eab_kid: Some("kid-1".to_string()),
                eab_hmac_b64: Some("aGk".to_string()),
            },
        )
        .unwrap();
        assert!(staging.staging());
        assert_eq!(
            staging.domains(),
            &["a.test".to_string(), "b.test".to_string()]
        );
        assert_eq!(staging.email(), Some("ops@example.test"));

        // Empty selector/CSV behave like missing values.
        let empty = AcmeConfig::from_stored(
            Path::new("store"),
            PathBuf::from("acme-dir"),
            StoredAcme {
                directory: String::new(),
                domains_csv: "  ".to_string(),
                email: Some("  ".to_string()),
                eab_kid: Some("kid-1".to_string()),
                eab_hmac_b64: Some("aGk".to_string()),
            },
        )
        .unwrap();
        assert_eq!(empty.directory_url(), GOOGLE_PROD_DIRECTORY);
        assert_eq!(empty.domains().len(), 2);
        assert_eq!(empty.email(), None);
    }

    #[test]
    fn from_stored_requires_the_full_eab_pair() {
        for (kid, hmac) in [
            (None, None),
            (Some("kid-1".to_string()), None),
            (None, Some("aGk".to_string())),
            (Some("  ".to_string()), Some("aGk".to_string())),
        ] {
            assert!(
                AcmeConfig::from_stored(
                    Path::new("store"),
                    PathBuf::from("acme-dir"),
                    StoredAcme {
                        directory: "prod".to_string(),
                        domains_csv: String::new(),
                        email: None,
                        eab_kid: kid,
                        eab_hmac_b64: hmac,
                    },
                )
                .is_err(),
                "half or missing EAB must fail"
            );
        }
    }
}
