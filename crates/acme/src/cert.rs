//! Certificate, key, and account persistence under the store.
//!
//! Layout inside the cert dir (`<store>/acme/`): `cert.pem` (full
//! chain, world-readable), `key.pem` and `account.json` (owner-only:
//! the key signs nothing else, but the account file holds the ACME
//! account private key). Writes are atomic temp-file + rename so a
//! crash never leaves a half-written pair behind.

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::{AcmeError, Result};

/// Full-chain certificate file inside the cert dir.
pub const CERT_FILE: &str = "cert.pem";
/// Certificate private-key file inside the cert dir (owner-only).
pub const KEY_FILE: &str = "key.pem";
/// Serialized ACME account credentials (owner-only: holds a key).
pub const ACCOUNT_FILE: &str = "account.json";

/// A certificate chain plus its matching private key, both PEM bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersistedCert {
    /// Full chain PEM as the CA returned it.
    pub cert_pem: Vec<u8>,
    /// PKCS#8 PEM of the key the CSR was built from.
    pub key_pem: Vec<u8>,
}

/// Leaf facts the renewal loop needs.
pub struct LeafInfo {
    /// Expiry of the leaf (first) certificate.
    pub not_after: SystemTime,
    /// DER of the leaf, for the ARI certificate identifier.
    pub leaf_der: Vec<u8>,
}

/// Path of the chain file inside `dir`.
pub fn cert_path(dir: &Path) -> PathBuf {
    dir.join(CERT_FILE)
}

/// Path of the key file inside `dir`.
pub fn key_path(dir: &Path) -> PathBuf {
    dir.join(KEY_FILE)
}

/// Path of the account-credential file inside `dir`.
pub fn account_path(dir: &Path) -> PathBuf {
    dir.join(ACCOUNT_FILE)
}

/// Atomically replace the persisted pair, creating the dir. The key
/// lands owner-only (0600); the chain is world-readable (0644).
///
/// # Errors
///
/// Directory creation or file writes fail.
pub fn save_pair(dir: &Path, cert_pem: &[u8], key_pem: &[u8]) -> Result<()> {
    std::fs::create_dir_all(dir).map_err(|e| {
        AcmeError::Io(std::io::Error::new(
            e.kind(),
            format!("create {}: {e}", dir.display()),
        ))
    })?;
    write_atomic(&cert_path(dir), cert_pem, 0o644)?;
    write_atomic(&key_path(dir), key_pem, 0o600)?;
    Ok(())
}

/// Atomically replace the account-credential JSON (owner-only).
///
/// # Errors
///
/// Directory creation or file writes fail.
pub fn save_account_json(dir: &Path, json: &str) -> Result<()> {
    std::fs::create_dir_all(dir).map_err(|e| {
        AcmeError::Io(std::io::Error::new(
            e.kind(),
            format!("create {}: {e}", dir.display()),
        ))
    })?;
    write_atomic(&account_path(dir), json.as_bytes(), 0o600)?;
    Ok(())
}

/// Load the persisted pair. `None` when either file is missing or
/// unreadable — the caller re-issues in that case.
pub fn load_pair(dir: &Path) -> Option<PersistedCert> {
    let cert_pem = std::fs::read(cert_path(dir)).ok()?;
    let key_pem = std::fs::read(key_path(dir)).ok()?;
    if cert_pem.is_empty() || key_pem.is_empty() {
        return None;
    }
    Some(PersistedCert { cert_pem, key_pem })
}

/// Load the persisted account-credential JSON, if present.
pub fn load_account_json(dir: &Path) -> Option<String> {
    std::fs::read_to_string(account_path(dir)).ok()
}

/// Parse the leaf (first) certificate: expiry plus DER bytes.
///
/// # Errors
///
/// No PEM certificate block, or the leaf does not parse as X.509.
pub fn leaf_info(cert_pem: &[u8]) -> Result<LeafInfo> {
    let mut reader = cert_pem;
    let leaf: rustls_pki_types::CertificateDer<'static> = rustls_pemfile::certs(&mut reader)
        .next()
        .ok_or_else(|| AcmeError::Cert("chain holds no certificate block".to_string()))?
        .map_err(|e| AcmeError::Cert(format!("certificate PEM is corrupt: {e}")))?;
    let (_, parsed) = x509_parser::parse_x509_certificate(leaf.as_ref())
        .map_err(|e| AcmeError::Cert(format!("leaf is not valid X.509: {e}")))?;
    let stamp = parsed.validity().not_after.to_datetime().unix_timestamp();
    let not_after = SystemTime::UNIX_EPOCH
        .checked_add(std::time::Duration::from_secs(stamp.max(0) as u64))
        .ok_or_else(|| AcmeError::Cert("leaf expiry out of range".to_string()))?;
    Ok(LeafInfo {
        not_after,
        leaf_der: leaf.as_ref().to_vec(),
    })
}

/// Temp-file + rename write; `mode` applies on unix (both our hosts
/// are unix; other platforms keep the default umask perms).
fn write_atomic(path: &Path, bytes: &[u8], mode: u32) -> Result<()> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes)?;
    #[cfg(unix)]
    std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(mode))?;
    let _ = mode;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    /// rcgen self-signed leaf (test-only fixture, generated at
    /// runtime; rcgen defaults expire in year 4096).
    fn self_signed() -> Vec<u8> {
        use rcgen::{CertificateParams, DistinguishedName, KeyPair};
        let mut params = CertificateParams::new(vec!["acme.test".to_string()]).unwrap();
        params.distinguished_name = DistinguishedName::new();
        let key = KeyPair::generate().unwrap();
        params.self_signed(&key).unwrap().pem().into_bytes()
    }

    #[test]
    fn pair_roundtrips_atomically_with_owner_only_key() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("acme");
        save_pair(&dir, b"chain-bytes", b"key-bytes").unwrap();
        assert_eq!(
            load_pair(&dir),
            Some(PersistedCert {
                cert_pem: b"chain-bytes".to_vec(),
                key_pem: b"key-bytes".to_vec(),
            })
        );
        // No temp leftovers; key is 0600, chain is 0644.
        assert!(!dir.join("cert.tmp").exists());
        assert!(!dir.join("key.tmp").exists());
        use std::os::unix::fs::PermissionsExt as _;
        let key_mode = std::fs::metadata(key_path(&dir))
            .unwrap()
            .permissions()
            .mode();
        let cert_mode = std::fs::metadata(cert_path(&dir))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(key_mode & 0o777, 0o600, "key mode {key_mode:o}");
        assert_eq!(cert_mode & 0o777, 0o644, "cert mode {cert_mode:o}");
        // Overwrite replaces both files.
        save_pair(&dir, b"chain-2", b"key-2").unwrap();
        assert_eq!(load_pair(&dir).unwrap().cert_pem, b"chain-2");
    }

    #[test]
    fn missing_or_empty_pair_loads_as_none() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(load_pair(tmp.path()), None);
        save_pair(tmp.path(), b"c", b"k").unwrap();
        std::fs::remove_file(key_path(tmp.path())).unwrap();
        assert_eq!(load_pair(tmp.path()), None);
    }

    #[test]
    fn leaf_info_reads_expiry_and_der() {
        let chain = self_signed();
        let info = leaf_info(&chain).unwrap();
        assert!(!info.leaf_der.is_empty());
        let remaining = info
            .not_after
            .duration_since(SystemTime::now())
            .expect("fresh fixture expires in the future");
        assert!(
            remaining > std::time::Duration::from_secs(365 * 86400),
            "{remaining:?}"
        );
        assert!(leaf_info(b"not pem at all").is_err());
        assert!(leaf_info(b"-----BEGIN FOO-----\nAAAA\n-----END FOO-----\n").is_err());
    }
}
