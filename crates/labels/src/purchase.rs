//! Purchase options with a sandbox-only guard (hard rule).
//!
//! Option A is a customer-printable address label (no postage moves).
//! Option B is a fully-paid USPS label (digital stamp): real money,
//! so it may run ONLY against the USPS test environment (TEM) or a
//! loopback stub backend. Any other base URL — including USPS
//! production — fails closed with [`SandboxError`], and the web layer
//! turns that into a 403 plus an audit row. There is deliberately no
//! "force live" flag anywhere in this crate.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::format::{PaperSize, PrintFormat};

/// The two author-decided purchase options.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum PurchaseOption {
    /// Customer-printable address label (customer affixes postage).
    Printable { format: PrintFormat },
    /// Fully-paid USPS label: postage is bought at purchase time.
    Prepaid { paper: PaperSize },
}

impl PurchaseOption {
    /// Stable storage spelling for the `shipping_label` row.
    pub fn as_str(self) -> &'static str {
        match self {
            PurchaseOption::Printable { .. } => "printable",
            PurchaseOption::Prepaid { .. } => "prepaid",
        }
    }

    /// Layout/paper choice as stored on the label row.
    pub fn layout_str(self) -> &'static str {
        match self {
            PurchaseOption::Printable { format } => format.as_str(),
            PurchaseOption::Prepaid { paper } => paper.as_str(),
        }
    }
}

/// USPS test-environment host (carrier-apis §1).
pub const USPS_TEM_HOST: &str = "apis-tem.usps.com";

/// Loopback hosts allowed so tests can buy against stub backends
/// (ephemeral port, no carrier contact).
const LOOPBACK_HOSTS: [&str; 4] = ["127.0.0.1", "localhost", "::1", "[::1]"];

/// Sandbox-only guard for prepaid buys.
pub struct SandboxGuard;

impl SandboxGuard {
    /// Accept only TEM and loopback-stub base URLs. Everything else —
    /// USPS production, FedEx hosts, unknown hosts, unparseable URLs —
    /// fails closed.
    ///
    /// # Errors
    ///
    /// Returns [`SandboxError`] when the URL is not a sandbox URL.
    pub fn check_usps_base_url(base_url: &str) -> Result<(), SandboxError> {
        match host_of(base_url) {
            Some(host) if host.eq_ignore_ascii_case(USPS_TEM_HOST) => Ok(()),
            Some(host) if LOOPBACK_HOSTS.contains(&host.to_ascii_lowercase().as_str()) => Ok(()),
            _ => Err(SandboxError::NotSandbox),
        }
    }
}

/// [`SandboxGuard`] failure: the only prepaid failure that must never
/// be retried against another host.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum SandboxError {
    NotSandbox,
}

impl fmt::Display for SandboxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SandboxError::NotSandbox => {
                write!(
                    f,
                    "refusing prepaid postage outside the USPS test environment"
                )
            }
        }
    }
}

impl std::error::Error for SandboxError {}

/// Host part of an http(s) URL, lowercased except it is compared
/// case-insensitively by the caller. Returns `None` when the URL has
/// no recognizable host.
fn host_of(url: &str) -> Option<String> {
    let after_scheme = url.split("://").nth(1)?;
    let authority = after_scheme.split('/').next()?;
    let authority = authority.split('@').next_back()?;
    let host = if let Some(rest) = authority.strip_prefix('[') {
        rest.split(']').next()?
    } else {
        authority.split(':').next()?
    };
    if host.trim().is_empty() {
        return None;
    }
    Some(host.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn option_spellings() {
        assert_eq!(
            PurchaseOption::Printable {
                format: PrintFormat::WindowEnvelope
            }
            .as_str(),
            "printable"
        );
        assert_eq!(
            PurchaseOption::Prepaid {
                paper: PaperSize::Letter
            }
            .layout_str(),
            "letter"
        );
        let json = serde_json::to_string(&PurchaseOption::Prepaid {
            paper: PaperSize::FourBySix,
        })
        .unwrap();
        assert_eq!(json, r#"{"kind":"prepaid","paper":"four_by_six"}"#);
        let back: PurchaseOption = serde_json::from_str(&json).unwrap();
        assert!(matches!(
            back,
            PurchaseOption::Prepaid {
                paper: PaperSize::FourBySix
            }
        ));
    }

    #[test]
    fn tem_and_loopback_pass() {
        for url in [
            "https://apis-tem.usps.com",
            "https://apis-tem.usps.com/",
            "https://APIS-TEM.USPS.COM/labels/v3/label",
            "http://127.0.0.1:0",
            "http://127.0.0.1:41237",
            "http://localhost:8080",
        ] {
            SandboxGuard::check_usps_base_url(url)
                .unwrap_or_else(|_| panic!("must allow sandbox url {url}"));
        }
    }

    #[test]
    fn prod_and_unknown_hosts_fail_closed() {
        for url in [
            "https://apis.usps.com",
            "https://api.usps.com",
            "https://apis-sandbox.fedex.com",
            "https://apis.fedex.com",
            "https://apis-tem.usps.com.evil.example",
            "https://evil-apis-tem.usps.com",
            "not a url",
            "",
        ] {
            assert_eq!(
                SandboxGuard::check_usps_base_url(url),
                Err(SandboxError::NotSandbox),
                "must refuse {url:?}"
            );
        }
    }
}
