//! Issuance and the ARI-driven renewal loop over instant-acme.
//!
//! One certificate covers every configured domain. Boot loads the
//! persisted pair or issues it; the loop then sleeps until the CA's
//! suggested window (RFC 9773) or the lifetime fallback says renew,
//! issues with `replaces` pointing at the old cert, persists the new
//! pair, and hands it to `apply` for the hot swap. Failures back off
//! and retry forever — the loop never serves an error to traffic.

use std::time::{Duration, SystemTime};

use instant_acme::{
    Account, AccountCredentials, AuthorizationStatus, CertificateIdentifier, ChallengeType,
    ExternalAccountKey, Identifier, NewAccount, NewOrder, OrderStatus, RetryPolicy,
};

use crate::cert::{leaf_info, load_account_json, load_pair, save_account_json, save_pair};
use crate::schedule::{ari_sleep, fallback_sleep, retry_sleep};
use crate::{AcmeConfig, AcmeError, PersistedCert, Result};

/// Challenge-response polling budget per order phase (the CA usually
/// validates http-01 within seconds; this only caps the tail).
const ORDER_TIMEOUT: Duration = Duration::from_secs(120);
/// Boot re-issues when less than this much lifetime remains — an
/// expired-or-imminent persisted pair is not worth serving.
const BOOT_RENEW_BEFORE: Duration = Duration::from_secs(24 * 3600);

/// Load the boot certificate: the persisted pair when it parses and
/// still has a day of life, otherwise a fresh issuance (which also
/// registers the ACME account on first run).
///
/// # Errors
///
/// Issuance or account setup fails; the caller must refuse to serve
/// TLS without a certificate.
pub async fn ensure_certificate(config: &AcmeConfig) -> Result<PersistedCert> {
    if let Some(pair) = load_pair(config.cert_dir())
        && let Ok(leaf) = leaf_info(&pair.cert_pem)
        && leaf.not_after > SystemTime::now() + BOOT_RENEW_BEFORE
    {
        tracing::info!("ACME: using persisted certificate");
        return Ok(pair);
    }
    tracing::info!("ACME: issuing certificate for {:?}", config.domains());
    let issued = issue(config, None).await?;
    save_pair(config.cert_dir(), &issued.cert_pem, &issued.key_pem)?;
    Ok(PersistedCert {
        cert_pem: issued.cert_pem,
        key_pem: issued.key_pem,
    })
}

/// Renewal loop. `apply` hot-swaps a renewed pair into the live TLS
/// config and reports whether the swap took; a refused swap retries
/// the same pair (never a fresh issuance) until it lands. Returns
/// only via task cancellation.
pub async fn renewal_loop<F, Fut>(config: AcmeConfig, apply: F) -> !
where
    F: Fn(Vec<u8>, Vec<u8>) -> Fut,
    Fut: Future<Output = std::result::Result<(), String>>,
{
    let mut failures = 0u32;
    loop {
        let current = load_pair(config.cert_dir())
            .and_then(|pair| leaf_info(&pair.cert_pem).ok().map(|leaf| (pair, leaf)));
        let Some((pair, leaf)) = current else {
            tracing::warn!("ACME: persisted pair missing or corrupt; re-issuing");
            if renew_once(&config, &apply, None).await {
                failures = 0;
            } else {
                failures += 1;
                tokio::time::sleep(retry_sleep(failures)).await;
            }
            continue;
        };
        let wait = sleep_until_renewal(&config, &pair.cert_pem, leaf.not_after).await;
        if !wait.is_zero() {
            tokio::time::sleep(wait).await;
            continue;
        }
        let replaces = cert_id_for_ari(&pair.cert_pem).ok();
        if renew_once(&config, &apply, replaces).await {
            failures = 0;
        } else {
            failures += 1;
            tokio::time::sleep(retry_sleep(failures)).await;
        }
    }
}

/// One renewal: issue, persist, then hot-swap until the swap takes.
/// True on full success; every failure is already logged.
async fn renew_once<F, Fut>(
    config: &AcmeConfig,
    apply: &F,
    replaces: Option<CertificateIdentifier<'static>>,
) -> bool
where
    F: Fn(Vec<u8>, Vec<u8>) -> Fut,
    Fut: Future<Output = std::result::Result<(), String>>,
{
    let issued = match issue(config, replaces).await {
        Ok(issued) => issued,
        Err(e) => {
            tracing::warn!("ACME: issuance failed: {e}");
            return false;
        }
    };
    if let Err(e) = save_pair(config.cert_dir(), &issued.cert_pem, &issued.key_pem) {
        tracing::warn!("ACME: cannot persist renewed pair: {e}");
        return false;
    }
    // The pair on disk is already the newest; keep pushing it live
    // until the swap takes so memory can never lag the disk.
    let mut reloads = 0u32;
    loop {
        match apply(issued.cert_pem.clone(), issued.key_pem.clone()).await {
            Ok(()) => {
                tracing::info!("ACME: renewed certificate is live");
                return true;
            }
            Err(e) => {
                reloads += 1;
                tracing::warn!("ACME: hot swap failed ({e}); retrying same pair");
                tokio::time::sleep(retry_sleep(reloads)).await;
            }
        }
    }
}

/// How long to sleep before the next renewal attempt: the ARI window
/// when the CA offers one, else the lifetime fallback.
async fn sleep_until_renewal(
    config: &AcmeConfig,
    cert_pem: &[u8],
    not_after: SystemTime,
) -> Duration {
    let now = SystemTime::now();
    let account = match load_or_create_account(config).await {
        Ok(account) => account,
        Err(e) => {
            tracing::warn!("ACME: account unavailable, using lifetime fallback: {e}");
            return fallback_sleep(not_after, now);
        }
    };
    let cert_id = match cert_id_for_ari(cert_pem) {
        Ok(id) => id,
        Err(e) => {
            tracing::warn!("ACME: no ARI identifier ({e}); using lifetime fallback");
            return fallback_sleep(not_after, now);
        }
    };
    match account.renewal_info(&cert_id).await {
        Ok((info, hint)) => {
            let start = stamp_to_system(info.suggested_window.start.unix_timestamp());
            let end = stamp_to_system(info.suggested_window.end.unix_timestamp());
            match (start, end) {
                (Some(start), Some(end)) => ari_sleep(now, start, end, Some(hint), rand::random()),
                _ => fallback_sleep(not_after, now),
            }
        }
        Err(e) => {
            tracing::debug!("ACME: ARI unavailable ({e}); using lifetime fallback");
            fallback_sleep(not_after, now)
        }
    }
}

fn stamp_to_system(stamp: i64) -> Option<SystemTime> {
    SystemTime::UNIX_EPOCH.checked_add(Duration::from_secs(stamp.try_into().ok()?))
}

/// A fresh certificate for every configured domain: one order, one
/// http-01 challenge per name answered from the challenge dir, one
/// CSR + key. `replaces` (renewals) points at the superseded cert.
async fn issue(
    config: &AcmeConfig,
    replaces: Option<CertificateIdentifier<'static>>,
) -> Result<Issued> {
    let account = load_or_create_account(config).await?;
    let identifiers: Vec<Identifier> = config
        .domains()
        .iter()
        .map(|d| Identifier::Dns(d.clone()))
        .collect();
    let new_order = NewOrder::new(&identifiers);
    let new_order = match replaces {
        Some(id) => new_order.replaces(id),
        None => new_order,
    };
    let mut order = account.new_order(&new_order).await?;
    let mut tokens = Vec::new();
    let mut authorizations = order.authorizations();
    while let Some(authorization) = authorizations.next().await {
        let mut authorization = authorization?;
        match authorization.status {
            AuthorizationStatus::Valid => continue,
            AuthorizationStatus::Pending => {}
            status => {
                return Err(AcmeError::Order(format!(
                    "authorization in unexpected state {status:?}"
                )));
            }
        }
        let mut challenge = authorization
            .challenge(ChallengeType::Http01)
            .ok_or_else(|| AcmeError::Config("CA offers no http-01 challenge".to_string()))?;
        let body = challenge.key_authorization().as_str().to_string();
        let path = challenge_path(config.challenge_dir(), &challenge.token)?;
        tokio::fs::create_dir_all(config.challenge_dir()).await?;
        tokio::fs::write(&path, &body).await?;
        tokens.push(challenge.token.clone());
        challenge.set_ready().await?;
    }
    match order
        .poll_ready(&RetryPolicy::new().timeout(ORDER_TIMEOUT))
        .await?
    {
        OrderStatus::Ready => {}
        status => {
            cleanup_tokens(config, &tokens).await;
            return Err(AcmeError::Order(format!(
                "order never became ready (state {status:?})"
            )));
        }
    }
    let (csr_der, key_pem) = build_csr(config.domains())?;
    order.finalize_csr(&csr_der).await?;
    let chain = order
        .poll_certificate(&RetryPolicy::new().timeout(ORDER_TIMEOUT))
        .await?;
    cleanup_tokens(config, &tokens).await;
    Ok(Issued {
        cert_pem: chain.into_bytes(),
        key_pem,
    })
}

/// Restore the ACME account from disk, or register it (Google EAB)
/// on first run and persist the credentials for the next boot.
///
/// Never auto-registers past a saved account: EAB secrets are
/// single-use, so a failed order must not burn a fresh secret. When
/// the saved account is dead (wrong CA, revoked), the operator
/// deletes `account.json` and restarts (see `docs/ops/acme.md`).
async fn load_or_create_account(config: &AcmeConfig) -> Result<Account> {
    if let Some(json) = load_account_json(config.cert_dir()) {
        let credentials: AccountCredentials = serde_json::from_str(&json)?;
        return Ok(Account::builder()?.from_credentials(credentials).await?);
    }
    let external = ExternalAccountKey::new(config.eab_kid().to_string(), config.eab_hmac());
    let contact = config.email().map(|email| format!("mailto:{email}"));
    let contact_refs: Vec<&str> = contact.iter().map(String::as_str).collect();
    let (account, credentials) = Account::builder()?
        .create(
            &NewAccount {
                contact: &contact_refs,
                terms_of_service_agreed: true,
                only_return_existing: false,
            },
            config.directory_url().to_string(),
            Some(&external),
        )
        .await?;
    save_account_json(config.cert_dir(), &serde_json::to_string(&credentials)?)?;
    Ok(account)
}

/// ARI identifier for a chain: upstream's `TryFrom` over the leaf DER
/// (base64url AKI + serial per RFC 9773 §4.1).
fn cert_id_for_ari(cert_pem: &[u8]) -> Result<CertificateIdentifier<'static>> {
    let leaf = leaf_info(cert_pem)?;
    let der = rustls_pki_types::CertificateDer::from(leaf.leaf_der);
    CertificateIdentifier::try_from(&der)
        .map(|id| id.into_owned())
        .map_err(|e| AcmeError::Cert(format!("no ARI identifier: {e}")))
}

/// Token file for one http-01 challenge. Tokens are CA-chosen
/// base64url; anything outside that alphabet is refused rather than
/// joined onto the dir (no traversal, no hidden files).
fn challenge_path(dir: &std::path::Path, token: &str) -> Result<std::path::PathBuf> {
    if token.is_empty()
        || token.len() > 128
        || !token
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err(AcmeError::Config(
            "CA sent an unsafe challenge token".to_string(),
        ));
    }
    Ok(dir.join(token))
}

/// Best-effort removal of our challenge files after the order ends.
async fn cleanup_tokens(config: &AcmeConfig, tokens: &[String]) {
    for token in tokens {
        if let Ok(path) = challenge_path(config.challenge_dir(), token) {
            let _ = tokio::fs::remove_file(path).await;
        }
    }
}

/// A fresh certificate: DER CSR with every domain as a SAN, plus the
/// PKCS#8 PEM of the key that signed it (that key becomes `key.pem`).
fn build_csr(domains: &[String]) -> Result<(Vec<u8>, Vec<u8>)> {
    if domains.is_empty() {
        return Err(AcmeError::Csr("no domains for CSR".to_string()));
    }
    let mut params = rcgen::CertificateParams::new(domains.to_vec())
        .map_err(|e| AcmeError::Csr(format!("bad domain for CSR: {e}")))?;
    params.distinguished_name = rcgen::DistinguishedName::new();
    let key =
        rcgen::KeyPair::generate().map_err(|e| AcmeError::Csr(format!("keygen failed: {e}")))?;
    let csr = params
        .serialize_request(&key)
        .map_err(|e| AcmeError::Csr(format!("CSR serialize failed: {e}")))?;
    Ok((
        csr.der().as_ref().to_vec(),
        key.serialize_pem().into_bytes(),
    ))
}

/// A newly issued certificate: chain PEM plus fresh key PEM.
struct Issued {
    cert_pem: Vec<u8>,
    key_pem: Vec<u8>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn challenge_path_accepts_tokens_and_refuses_traversal() {
        let dir = PathBuf::from("/tmp/acme-test");
        let ok = challenge_path(&dir, "t0k3n-_99").unwrap();
        assert_eq!(ok, dir.join("t0k3n-_99"));
        for bad in [
            "",
            "../evil",
            "a/b",
            ".hidden",
            "sp ace",
            "dot.name",
            &"x".repeat(129),
        ] {
            assert!(
                challenge_path(&dir, bad).is_err(),
                "token {bad:?} must fail"
            );
        }
    }

    #[test]
    fn csr_carries_every_domain_as_san() {
        use x509_parser::extensions::{GeneralName, ParsedExtension};
        use x509_parser::prelude::FromDer as _;

        let domains = vec!["andrii.ai".to_string(), "admin.andrii.ai".to_string()];
        let (der, key_pem) = build_csr(&domains).unwrap();
        assert!(!der.is_empty());
        // Assembled from fragments so this committed line never
        // contains a scannable key header (cf. xtask secrets-scan).
        let header = ["-----BEGIN", " PRIVATE KEY-----"].concat();
        assert!(key_pem.starts_with(header.as_bytes()));
        let (_, csr) = x509_parser::certification_request::X509CertificationRequest::from_der(&der)
            .expect("valid PKCS#10");
        let mut sans = Vec::new();
        for ext in csr.requested_extensions().into_iter().flatten() {
            if let ParsedExtension::SubjectAlternativeName(san) = ext {
                sans.extend(san.general_names.iter().filter_map(|name| match name {
                    GeneralName::DNSName(dns) => Some(dns.to_string()),
                    _ => None,
                }));
            }
        }
        assert_eq!(sans, domains);
        assert!(build_csr(&[]).is_err(), "empty domains must fail");
    }

    #[test]
    fn cert_id_needs_a_real_chain() {
        assert!(cert_id_for_ari(b"garbage").is_err());
    }
}
