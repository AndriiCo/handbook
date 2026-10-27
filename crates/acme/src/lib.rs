//! In-binary ACME issuance and renewal (Google Public CA only).
//!
//! The web binary issues its own TLS certificate at boot and renews
//! it without restarting: http-01 challenges are answered from the
//! `ACME_CHALLENGE_DIR` files the public router already serves,
//! the pair is persisted under `<store>/acme/`, and each renewal is
//! hot-swapped into the live rustls config. Renewal timing follows
//! the CA's ARI window ([RFC 9773](https://www.rfc-editor.org/rfc/rfc9773.html))
//! with a certificate-lifetime fallback when ARI is unavailable.
//!
//! Live-CA behavior is covered by the documented manual staging run
//! in `docs/ops/acme.md`, not by CI: these tests pin the scheduling
//! math, config parsing, persistence round-trips, and CSR shape.

mod cert;
mod config;
mod manager;
mod schedule;

pub use cert::{
    ACCOUNT_FILE, CERT_FILE, KEY_FILE, LeafInfo, PersistedCert, leaf_info, load_pair, save_pair,
};
pub use config::{AcmeConfig, DEFAULT_DOMAINS, GOOGLE_PROD_DIRECTORY, GOOGLE_STAGING_DIRECTORY};
pub use manager::{ensure_certificate, renewal_loop};
pub use schedule::{
    FALLBACK_POLL_INTERVAL, FALLBACK_RENEW_BEFORE, MAX_ARI_SLEEP, ari_sleep, fallback_due,
    fallback_sleep, retry_sleep,
};

/// Every error the ACME crate can return. Secrets (EAB HMAC,
/// account keys) never appear in these messages.
#[derive(Debug)]
pub enum AcmeError {
    /// Config parsing or validation failed (names the variable or field).
    Config(String),
    /// The ACME server or transport failed.
    Acme(instant_acme::Error),
    /// An order or authorization ended in an unexpected state.
    Order(String),
    /// A persisted or issued certificate did not parse.
    Cert(String),
    /// CSR generation failed.
    Csr(String),
    /// Challenge, account, or certificate file I/O failed.
    Io(std::io::Error),
    /// Account-credential JSON did not parse.
    Json(serde_json::Error),
}

impl std::fmt::Display for AcmeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AcmeError::Config(why) => write!(f, "ACME config: {why}"),
            AcmeError::Acme(e) => write!(f, "ACME protocol: {e}"),
            AcmeError::Order(why) => write!(f, "ACME order: {why}"),
            AcmeError::Cert(why) => write!(f, "ACME certificate: {why}"),
            AcmeError::Csr(why) => write!(f, "ACME CSR: {why}"),
            AcmeError::Io(e) => write!(f, "ACME file I/O: {e}"),
            AcmeError::Json(e) => write!(f, "ACME account JSON: {e}"),
        }
    }
}

impl std::error::Error for AcmeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            AcmeError::Acme(e) => Some(e),
            AcmeError::Io(e) => Some(e),
            AcmeError::Json(e) => Some(e),
            _ => None,
        }
    }
}

impl From<instant_acme::Error> for AcmeError {
    fn from(e: instant_acme::Error) -> Self {
        AcmeError::Acme(e)
    }
}

impl From<std::io::Error> for AcmeError {
    fn from(e: std::io::Error) -> Self {
        AcmeError::Io(e)
    }
}

impl From<serde_json::Error> for AcmeError {
    fn from(e: serde_json::Error) -> Self {
        AcmeError::Json(e)
    }
}

/// Result over [`AcmeError`].
pub type Result<T> = std::result::Result<T, AcmeError>;
