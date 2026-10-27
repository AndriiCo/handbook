//! Sealed KEK storage: the [`Sealer`] abstraction over software and hardware TPMs.
//!
//! Per `docs/specs/tpm-seal.md`: config-row encryption keys are sealed
//! through this trait so the same call sites run on a software TPM today
//! and a hardware TPM later. [`SoftwareSealer`] is the default pure-Rust
//! provider (dev, CI, tests); [`HardwareTpmSealer`] is the compile seam
//! for the hardware path (systemd-creds enroll + tmpfs collect).
//!
//! Unsealing yields the existing [`Kek`](crate::Kek), which feeds
//! `set_value`, `get_display`, `get_secret_for_job`, and `rotate`
//! unchanged — the integration point is the `Kek` value, and no row-path
//! signature changes.

use chacha20poly1305::{
    KeyInit, XChaCha20Poly1305, XNonce,
    aead::{Aead, Payload},
};

use crate::{ConfigError, Kek};

/// Magic prefix of every software-sealed blob. The trailing `1` is the
/// format version; a v2 format gets a new magic, never a silent
/// reinterpretation.
const SOFTWARE_MAGIC: &[u8; 8] = b"SWSEAL1:";
/// Exact sealed-blob length: magic(8) + nonce(24) + ct(32 + 16 tag).
const SEALED_BLOB_LEN: usize = 8 + crate::NONCE_LEN + 32 + 16;

/// Every error a [`Sealer`] can return. Messages never carry key bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SealError {
    /// Operation needs a different provider (the hardware-stub path).
    /// The message names the real enroll/collect command or file.
    Unsupported(String),
    /// Bad magic, truncation, or failed AEAD open.
    Corrupt(String),
    /// Keyfile or environment I/O failure.
    Unavailable(String),
}

impl std::fmt::Display for SealError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SealError::Unsupported(why) => write!(f, "seal operation unsupported: {why}"),
            SealError::Corrupt(why) => write!(f, "sealed blob corrupt: {why}"),
            SealError::Unavailable(why) => write!(f, "sealer unavailable: {why}"),
        }
    }
}

impl std::error::Error for SealError {}

impl From<SealError> for ConfigError {
    fn from(e: SealError) -> Self {
        match e {
            SealError::Corrupt(_) => ConfigError::Crypto,
            SealError::Unsupported(why) | SealError::Unavailable(why) => {
                ConfigError::KekUnavailable(why)
            }
        }
    }
}

/// Seal/unseal one KEK. Object-safe (`&dyn Sealer` works) and
/// `Send + Sync` for future server use.
pub trait Sealer: Send + Sync {
    /// Provider name for logs (`"software"`, `"hardware-tpm"`).
    fn name(&self) -> &'static str;
    /// Wrap 32 KEK bytes into an opaque provider-tagged blob.
    ///
    /// # Errors
    ///
    /// [`SealError::Unsupported`] when this provider cannot seal
    /// (the hardware stub points at `xtask config-enroll --seal`).
    fn seal(&self, kek: &[u8; 32]) -> Result<Vec<u8>, SealError>;
    /// Open a blob sealed by this provider back into a [`Kek`].
    ///
    /// # Errors
    ///
    /// [`SealError::Corrupt`] on bad magic, truncation, or failed
    /// authentication; [`SealError::Unsupported`] when this provider
    /// cannot unseal in-process.
    fn unseal(&self, blob: &[u8]) -> Result<Kek, SealError>;
}

/// Default pure-Rust provider: wraps the KEK with XChaCha20-Poly1305
/// under a local wrapping key. Runs everywhere with zero new
/// dependencies — but it is explicitly NOT a hardware security
/// boundary, only an API-compatible stand-in for dev, CI, and tests.
pub struct SoftwareSealer {
    wrapping: zeroize::Zeroizing<[u8; 32]>,
}

impl SoftwareSealer {
    /// Ephemeral wrapping key (CSPRNG). For tests, never persisted.
    pub fn generate() -> Self {
        use rand::RngCore as _;
        let mut key = [0u8; 32];
        rand::rng().fill_bytes(&mut key);
        SoftwareSealer::from_wrapping_key(key)
    }

    /// Wrap under explicit wrapping-key bytes (callers own custody).
    pub fn from_wrapping_key(key: [u8; 32]) -> Self {
        SoftwareSealer {
            wrapping: zeroize::Zeroizing::new(key),
        }
    }

    /// File-rooted wrapping key: reads 32 raw bytes, or generates and
    /// writes them (`0600` on unix) when the file is absent.
    ///
    /// # Errors
    ///
    /// [`SealError::Corrupt`] when the file exists with a wrong length;
    /// [`SealError::Unavailable`] on I/O failure.
    pub fn load_or_create_keyfile(path: &std::path::Path) -> Result<Self, SealError> {
        match std::fs::read(path) {
            Ok(bytes) if bytes.len() == 32 => {
                let mut key = [0u8; 32];
                key.copy_from_slice(&bytes);
                Ok(SoftwareSealer::from_wrapping_key(key))
            }
            Ok(bytes) => Err(SealError::Corrupt(format!(
                "{}: want 32 raw bytes, got {}",
                path.display(),
                bytes.len()
            ))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let sealer = SoftwareSealer::generate();
                write_private(path, sealer.wrapping.as_slice())?;
                Ok(sealer)
            }
            Err(e) => Err(SealError::Unavailable(format!("{}: {e}", path.display()))),
        }
    }

    fn cipher(&self) -> XChaCha20Poly1305 {
        XChaCha20Poly1305::new_from_slice(self.wrapping.as_slice())
            .expect("wrapping key is 32 bytes")
    }
}

impl std::fmt::Debug for SoftwareSealer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SoftwareSealer(redacted)")
    }
}

impl Sealer for SoftwareSealer {
    fn name(&self) -> &'static str {
        "software"
    }

    fn seal(&self, kek: &[u8; 32]) -> Result<Vec<u8>, SealError> {
        use rand::RngCore as _;
        let mut nonce_bytes = [0u8; crate::NONCE_LEN];
        rand::rng().fill_bytes(&mut nonce_bytes);
        let ciphertext = self
            .cipher()
            .encrypt(
                XNonce::from_slice(&nonce_bytes),
                Payload {
                    msg: kek,
                    aad: SOFTWARE_MAGIC,
                },
            )
            .expect("XChaCha20-Poly1305 seal cannot fail");
        let mut blob = Vec::with_capacity(SEALED_BLOB_LEN);
        blob.extend_from_slice(SOFTWARE_MAGIC);
        blob.extend_from_slice(&nonce_bytes);
        blob.extend_from_slice(&ciphertext);
        Ok(blob)
    }

    fn unseal(&self, blob: &[u8]) -> Result<Kek, SealError> {
        if blob.len() != SEALED_BLOB_LEN {
            return Err(SealError::Corrupt(format!(
                "want {SEALED_BLOB_LEN} bytes, got {}",
                blob.len()
            )));
        }
        if &blob[..SOFTWARE_MAGIC.len()] != SOFTWARE_MAGIC {
            return Err(SealError::Corrupt("bad magic".to_string()));
        }
        let body = &blob[SOFTWARE_MAGIC.len()..];
        let (nonce, ciphertext) = body.split_at(crate::NONCE_LEN);
        let plain = self
            .cipher()
            .decrypt(
                nonce.into(),
                Payload {
                    msg: ciphertext,
                    aad: SOFTWARE_MAGIC,
                },
            )
            .map_err(|_| SealError::Corrupt("authentication failed".to_string()))?;
        let mut kek = [0u8; 32];
        kek.copy_from_slice(&plain);
        Ok(Kek::from_bytes(kek))
    }
}

/// Hardware-TPM provider seam (stub): both operations fail with a
/// message naming the real path. Seal happens once via `cargo xtask
/// config-enroll --seal` (systemd-creds, host+tpm2); unseal happens at
/// unit activation via `LoadCredentialEncrypted=`, collected by
/// `config::load_kek()`. A future in-process provider implements this
/// same trait without touching callers.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HardwareTpmSealer;

impl Sealer for HardwareTpmSealer {
    fn name(&self) -> &'static str {
        "hardware-tpm"
    }

    fn seal(&self, _kek: &[u8; 32]) -> Result<Vec<u8>, SealError> {
        Err(SealError::Unsupported(
            "seal to hardware TPM via `cargo xtask config-enroll --seal` \
             (systemd-creds encrypt --with-key=host+tpm2)"
                .to_string(),
        ))
    }

    fn unseal(&self, _blob: &[u8]) -> Result<Kek, SealError> {
        Err(SealError::Unsupported(
            "hardware unseal happens in systemd at unit activation \
             (LoadCredentialEncrypted=); collect via config::load_kek()"
                .to_string(),
        ))
    }
}

#[cfg(unix)]
fn write_private(path: &std::path::Path, bytes: &[u8]) -> Result<(), SealError> {
    use std::os::unix::fs::OpenOptionsExt as _;
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true).mode(0o600);
    let mut file = opts
        .open(path)
        .map_err(|e| SealError::Unavailable(format!("{}: {e}", path.display())))?;
    use std::io::Write as _;
    file.write_all(bytes)
        .map_err(|e| SealError::Unavailable(format!("{}: {e}", path.display())))
}

#[cfg(not(unix))]
fn write_private(path: &std::path::Path, bytes: &[u8]) -> Result<(), SealError> {
    std::fs::write(path, bytes)
        .map_err(|e| SealError::Unavailable(format!("{}: {e}", path.display())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ConfigType, SetValue};

    fn memory() -> rusqlite::Connection {
        let conn = rusqlite::Connection::open_in_memory().expect("open memory db");
        crate::migrate_config_tables(&conn).expect("migrate config tables");
        conn
    }

    fn seals_open(before: &Kek, after: &Kek) {
        let blob = crate::encrypt_value(before, "probe.key", 1, b"probe");
        assert_eq!(
            crate::decrypt_value(after, "probe.key", 1, &blob).expect("open"),
            b"probe"
        );
    }

    #[test]
    fn software_roundtrip_preserves_kek() {
        let sealer = SoftwareSealer::generate();
        assert_eq!(sealer.name(), "software");
        let kek = Kek::generate();
        let blob = kek.seal_with(&sealer).expect("seal");
        assert_eq!(blob.len(), SEALED_BLOB_LEN);
        assert_eq!(&blob[..SOFTWARE_MAGIC.len()], SOFTWARE_MAGIC);
        let back = sealer.unseal(&blob).expect("unseal");
        seals_open(&kek, &back);
        seals_open(&back, &kek);
        // Fresh nonce per seal: same KEK, different bytes.
        let again = kek.seal_with(&sealer).expect("seal again");
        assert_ne!(blob, again);
    }

    #[test]
    fn trait_object_seal_path() {
        let sealer: Box<dyn Sealer> = Box::new(SoftwareSealer::generate());
        let kek = Kek::generate();
        let blob = kek.seal_with(sealer.as_ref()).expect("seal_with");
        let back = sealer.unseal(&blob).expect("unseal");
        seals_open(&kek, &back);
    }

    #[test]
    fn wrong_key_tamper_truncation_and_magic_fail_closed() {
        let a = SoftwareSealer::generate();
        let b = SoftwareSealer::generate();
        let kek = Kek::generate();
        let blob = kek.seal_with(&a).expect("seal");
        assert!(matches!(b.unseal(&blob), Err(SealError::Corrupt(_))));
        let mut flipped = blob.clone();
        let last = flipped.len() - 1;
        flipped[last] ^= 0x01;
        assert!(matches!(a.unseal(&flipped), Err(SealError::Corrupt(_))));
        assert!(matches!(
            a.unseal(&blob[..SEALED_BLOB_LEN - 1]),
            Err(SealError::Corrupt(_))
        ));
        let mut bad_magic = blob.clone();
        bad_magic[0] = b'X';
        assert!(matches!(a.unseal(&bad_magic), Err(SealError::Corrupt(_))));
    }

    #[test]
    fn hardware_stub_names_the_real_path() {
        let hw = HardwareTpmSealer;
        assert_eq!(hw.name(), "hardware-tpm");
        let seal_err = hw.seal(&[0u8; 32]).expect_err("stub seal fails");
        assert!(
            seal_err.to_string().contains("config-enroll --seal"),
            "{seal_err}"
        );
        let blob = SoftwareSealer::generate()
            .seal(&[1u8; 32])
            .expect("software seal");
        let unseal_err = hw.unseal(&blob).expect_err("stub unseal fails");
        assert!(
            unseal_err.to_string().contains("LoadCredentialEncrypted"),
            "{unseal_err}"
        );
        assert!(matches!(seal_err, SealError::Unsupported(_)));
    }

    #[test]
    fn keyfile_creates_private_and_reloads() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("wrap.key");
        let first = SoftwareSealer::load_or_create_keyfile(&path).expect("create");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(&path).expect("stat").permissions().mode();
            assert_eq!(mode & 0o777, 0o600, "keyfile is 0600");
        }
        let second = SoftwareSealer::load_or_create_keyfile(&path).expect("reload");
        let kek = Kek::generate();
        let blob = kek.seal_with(&first).expect("seal");
        let back = second.unseal(&blob).expect("unseal with reloaded key");
        seals_open(&kek, &back);
        std::fs::write(&path, b"short").expect("clobber");
        assert!(matches!(
            SoftwareSealer::load_or_create_keyfile(&path),
            Err(SealError::Corrupt(_))
        ));
    }

    #[test]
    fn errors_map_to_config_error() {
        let crypto: ConfigError = SealError::Corrupt("x".to_string()).into();
        assert!(matches!(crypto, ConfigError::Crypto));
        let unavail: ConfigError = SealError::Unavailable("x".to_string()).into();
        assert!(matches!(unavail, ConfigError::KekUnavailable(_)));
        let unsup: ConfigError = SealError::Unsupported("x".to_string()).into();
        assert!(matches!(unsup, ConfigError::KekUnavailable(_)));
    }

    #[test]
    fn debug_impls_redact_key_material() {
        let sealer = SoftwareSealer::from_wrapping_key([0xabu8; 32]);
        let shown = format!("{sealer:?}");
        assert_eq!(shown, "SoftwareSealer(redacted)");
        assert!(!shown.contains("ab"));
    }

    #[test]
    fn sealed_kek_drives_set_get_rotate_unchanged() {
        let conn = memory();
        let sealer = SoftwareSealer::generate();
        let kek = Kek::generate();
        // Seal → persist bytes → unseal, exactly as a restart would.
        let blob = kek.seal_with(&sealer).expect("seal");
        let rt = sealer.unseal(&blob).expect("unseal");
        let put = |key: &str, key_type: ConfigType, plaintext: &[u8]| {
            crate::set_value(
                &conn,
                &rt,
                &SetValue {
                    version: 1,
                    key,
                    key_type,
                    plaintext,
                    now: 100,
                    actor: "seal-test",
                },
            )
            .expect("set");
        };
        put("site.on", ConfigType::Bool, b"1");
        put("site.name", ConfigType::Text, b"synthetic");
        put("api.token", ConfigType::Secret, b"synthetic-secret");
        assert_eq!(
            crate::get_display(&conn, &rt, "site.name").expect("get"),
            Some(crate::DisplayValue::Text("synthetic".to_string()))
        );
        assert_eq!(
            crate::get_display(&conn, &rt, "api.token").expect("get"),
            Some(crate::DisplayValue::Secret { set: true })
        );
        assert_eq!(
            crate::get_secret_for_job(&conn, &rt, "api.token").expect("job"),
            Some(b"synthetic-secret".to_vec())
        );
        let rotated_kek = Kek::generate();
        crate::record_kek(&conn, 2, "sealed:test", 200).expect("record");
        assert_eq!(
            crate::rotate(&conn, &rt, &rotated_kek, 2, 200, "seal-test").expect("rotate"),
            3
        );
        assert_eq!(crate::verify_all(&conn, &rotated_kek).expect("verify"), 3);
    }
}
