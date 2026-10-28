//! Typed encrypted config: XChaCha20-Poly1305 envelope over SQLite rows.
//!
//! Per `docs/specs/secure-config.md`: every config value is stored
//! encrypted (`nonce(24) ‖ ciphertext`), bound to its key name and
//! KEK generation via AEAD associated data. `SECRET` keys are
//! write-only — the admin display path cannot decrypt them by
//! construction ([`BoolOrTextRow`] has no secret variant).
//!
//! The KEK arrives via systemd credentials on the server
//! (`$CREDENTIALS_DIRECTORY/handbook-kek`, TPM2-sealed) or, on Mac
//! dev only, via `HANDBOOK_DEV_KEYFILE` / `HANDBOOK_DEV_KEK` with a
//! loud banner. Dev KEKs are refused outside debug builds.

use chacha20poly1305::{
    KeyInit, XChaCha20Poly1305, XNonce,
    aead::{Aead, Payload},
};
use rusqlite::{Connection, OptionalExtension};

pub mod seal;

pub use seal::{HardwareTpmSealer, SealError, Sealer, SoftwareSealer};

/// XChaCha20-Poly1305 nonce length in bytes.
pub const NONCE_LEN: usize = 24;
/// Max `text` plaintext length (4 KiB).
pub const MAX_TEXT_LEN: usize = 4096;
/// Max `secret` plaintext length (8 KiB).
pub const MAX_SECRET_LEN: usize = 8192;
/// Max key-name length.
pub const MAX_KEY_LEN: usize = 128;

/// 256-bit key-encryption key, zeroized on drop and redacted in logs.
pub struct Kek(zeroize::Zeroizing<[u8; 32]>);

impl Kek {
    /// Wrap 32 raw key bytes (systemd credential payload).
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Kek(zeroize::Zeroizing::new(bytes))
    }

    /// Fresh CSPRNG key (enroll / rotate path).
    pub fn generate() -> Self {
        use rand::RngCore as _;
        let mut bytes = [0u8; 32];
        rand::rng().fill_bytes(&mut bytes);
        Kek::from_bytes(bytes)
    }

    /// Seal this KEK's bytes through any [`Sealer`] (software TPM today,
    /// hardware TPM later). The bytes never leave except inside the
    /// returned opaque blob.
    ///
    /// # Errors
    ///
    /// Forwards [`SealError`] from the provider.
    pub fn seal_with(&self, sealer: &dyn Sealer) -> std::result::Result<Vec<u8>, SealError> {
        sealer.seal(&self.0)
    }

    /// Best-effort `mlock`: keeps the key out of swap when the
    /// platform allows it. Failure is non-fatal (dev sandboxes deny
    /// it); the key stays zeroized-on-drop either way.
    pub fn lock_memory(&self) {
        let ptr = self.0.as_ptr().cast_mut().cast::<libc::c_void>();
        // SAFETY: `ptr`/`len` borrow live key bytes; mlock only pins.
        let _ = unsafe { libc::mlock(ptr, self.0.len()) };
    }

    fn cipher(&self) -> XChaCha20Poly1305 {
        XChaCha20Poly1305::new_from_slice(self.0.as_slice()).expect("KEK is 32 bytes")
    }
}

impl std::fmt::Debug for Kek {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Kek(redacted)")
    }
}

/// Config value type; `Secret` is write-only via the admin API.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigType {
    Bool,
    Text,
    Secret,
}

impl ConfigType {
    /// Wire form stored in `config.type`.
    pub fn as_str(self) -> &'static str {
        match self {
            ConfigType::Bool => "bool",
            ConfigType::Text => "text",
            ConfigType::Secret => "secret",
        }
    }

    /// Parse the wire form; unknown input is `None`.
    pub fn parse(input: &str) -> Option<Self> {
        match input {
            "bool" => Some(ConfigType::Bool),
            "text" => Some(ConfigType::Text),
            "secret" => Some(ConfigType::Secret),
            _ => None,
        }
    }
}

impl std::fmt::Display for ConfigType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Every error the config crate can return.
#[derive(Debug)]
pub enum ConfigError {
    /// AEAD open failed (wrong KEK, swapped row, or tampering).
    Crypto,
    /// SQLite failure.
    Db(rusqlite::Error),
    /// Key name outside the allowed alphabet.
    BadKeyName,
    /// Plaintext violates its type's shape or length cap.
    BadValue(&'static str),
    /// No KEK source available (message names the source, never bytes).
    KekUnavailable(String),
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigError::Crypto => write!(f, "config decryption failed"),
            ConfigError::Db(e) => write!(f, "config database error: {e}"),
            ConfigError::BadKeyName => write!(f, "invalid config key name"),
            ConfigError::BadValue(why) => write!(f, "invalid config value: {why}"),
            ConfigError::KekUnavailable(why) => write!(f, "config KEK unavailable: {why}"),
        }
    }
}

impl std::error::Error for ConfigError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ConfigError::Db(e) => Some(e),
            _ => None,
        }
    }
}

impl From<rusqlite::Error> for ConfigError {
    fn from(e: rusqlite::Error) -> Self {
        ConfigError::Db(e)
    }
}

/// Fallible config result.
pub type Result<T> = std::result::Result<T, ConfigError>;

/// Key names: ASCII alphanumerics plus `.`, `_`, `-` (e.g.
/// `fedex.account_id`). Anything else is rejected before touching SQL.
pub fn valid_key_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_KEY_LEN
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'_' || b == b'-')
}

/// AEAD associated data: `"{key}:{version}"`. Binds each ciphertext
/// to its row, killing cut-and-paste between keys or generations.
/// This exact string form is shared with `xtask config-*`: rows sealed
/// by either side must decrypt on the other, so do not change it here
/// without changing both plus a migration story.
fn aad(key: &str, version: i64) -> Vec<u8> {
    format!("{key}:{version}").into_bytes()
}

/// Seal one value: fresh random 24-byte nonce, XChaCha20-Poly1305.
/// Returns `nonce ‖ ciphertext`.
pub fn encrypt_value(kek: &Kek, key: &str, version: i64, plaintext: &[u8]) -> Vec<u8> {
    use rand::RngCore as _;
    let mut nonce_bytes = [0u8; NONCE_LEN];
    rand::rng().fill_bytes(&mut nonce_bytes);
    let nonce = XNonce::from_slice(&nonce_bytes);
    let ciphertext = kek
        .cipher()
        .encrypt(
            nonce,
            Payload {
                msg: plaintext,
                aad: &aad(key, version),
            },
        )
        .expect("XChaCha20-Poly1305 seal cannot fail");
    let mut blob = Vec::with_capacity(NONCE_LEN + ciphertext.len());
    blob.extend_from_slice(nonce.as_slice());
    blob.extend_from_slice(&ciphertext);
    blob
}

/// Open one value. Any mismatch (wrong KEK, wrong key name or
/// version, truncated or tampered blob) is [`ConfigError::Crypto`].
pub fn decrypt_value(kek: &Kek, key: &str, version: i64, blob: &[u8]) -> Result<Vec<u8>> {
    if blob.len() < NONCE_LEN + 16 {
        return Err(ConfigError::Crypto);
    }
    let (nonce, ciphertext) = blob.split_at(NONCE_LEN);
    kek.cipher()
        .decrypt(
            nonce.into(),
            Payload {
                msg: ciphertext,
                aad: &aad(key, version),
            },
        )
        .map_err(|_| ConfigError::Crypto)
}

/// Create the `config` / `config_kek` tables (spec §1.4 schema).
pub fn migrate_config_tables(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS config(
           key TEXT PRIMARY KEY,
           type TEXT NOT NULL CHECK(type IN ('bool','text','secret')),
           value_enc BLOB NOT NULL,
           enc_version INTEGER NOT NULL,
           updated_at INTEGER NOT NULL,
           updated_by TEXT NOT NULL
         ) STRICT;
         CREATE TABLE IF NOT EXISTS config_kek(
           version INTEGER PRIMARY KEY,
           cred_path TEXT NOT NULL,
           created_at INTEGER NOT NULL,
           retired_at INTEGER
         ) STRICT;",
    )?;
    Ok(())
}

/// Displayable value kinds: `bool` and `text` only. There is
/// deliberately no secret variant: the private `decrypt_display`
/// takes only a [`BoolOrTextRow`], so returning a secret from the
/// admin path is a compile error, not a policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoolOrText {
    Bool,
    Text,
}

/// A config row the admin display path is allowed to decrypt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoolOrTextRow {
    pub key: String,
    pub kind: BoolOrText,
    pub value_enc: Vec<u8>,
    pub enc_version: i64,
}

/// What the admin API returns for one key. Secrets surface as
/// `Secret { set }` — presence only, never bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DisplayValue {
    Bool(bool),
    Text(String),
    Secret { set: bool },
}

/// Validate a plaintext against its type's shape and length cap.
fn check_plaintext(key_type: ConfigType, plaintext: &[u8]) -> Result<()> {
    match key_type {
        ConfigType::Bool if plaintext == b"0" || plaintext == b"1" => Ok(()),
        ConfigType::Bool => Err(ConfigError::BadValue("bool must be one byte 0 or 1")),
        ConfigType::Text if plaintext.len() > MAX_TEXT_LEN => {
            Err(ConfigError::BadValue("text exceeds 4 KiB"))
        }
        ConfigType::Text if std::str::from_utf8(plaintext).is_err() => {
            Err(ConfigError::BadValue("text must be UTF-8"))
        }
        ConfigType::Text => Ok(()),
        ConfigType::Secret if plaintext.len() > MAX_SECRET_LEN => {
            Err(ConfigError::BadValue("secret exceeds 8 KiB"))
        }
        ConfigType::Secret if plaintext.is_empty() => {
            Err(ConfigError::BadValue("secret must be non-empty"))
        }
        ConfigType::Secret => Ok(()),
    }
}

/// One config write: value plus audit attribution.
pub struct SetValue<'a> {
    pub version: i64,
    pub key: &'a str,
    pub key_type: ConfigType,
    pub plaintext: &'a [u8],
    pub now: i64,
    pub actor: &'a str,
}

/// Set (insert or overwrite) one key, sealing with the active KEK
/// generation. Secrets are set-new-only: the response path never
/// echoes bytes. The caller writes the audit row (key+version only).
pub fn set_value(conn: &Connection, kek: &Kek, set: &SetValue<'_>) -> Result<()> {
    if !valid_key_name(set.key) {
        return Err(ConfigError::BadKeyName);
    }
    check_plaintext(set.key_type, set.plaintext)?;
    let blob = encrypt_value(kek, set.key, set.version, set.plaintext);
    conn.execute(
        "INSERT INTO config(key, type, value_enc, enc_version, updated_at, updated_by)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(key) DO UPDATE SET
           type = excluded.type, value_enc = excluded.value_enc,
           enc_version = excluded.enc_version,
           updated_at = excluded.updated_at, updated_by = excluded.updated_by",
        rusqlite::params![
            set.key,
            set.key_type.as_str(),
            blob,
            set.version,
            set.now,
            set.actor
        ],
    )?;
    Ok(())
}

/// Decrypt one displayable row. The signature is the enforcement:
/// only [`BoolOrTextRow`] (bool/text) can be passed in.
fn decrypt_display(kek: &Kek, row: &BoolOrTextRow) -> Result<DisplayValue> {
    let plain = decrypt_value(kek, &row.key, row.enc_version, &row.value_enc)?;
    match row.kind {
        BoolOrText::Bool => match plain.as_slice() {
            b"0" => Ok(DisplayValue::Bool(false)),
            b"1" => Ok(DisplayValue::Bool(true)),
            _ => Err(ConfigError::BadValue("bool row holds non-bool bytes")),
        },
        BoolOrText::Text => String::from_utf8(plain)
            .map(DisplayValue::Text)
            .map_err(|_| ConfigError::BadValue("text row is not UTF-8")),
    }
}

/// Admin display read for one key: bool/text decrypt, secret → `set`
/// flag only (its ciphertext is never opened on this path).
pub fn get_display(conn: &Connection, kek: &Kek, key: &str) -> Result<Option<DisplayValue>> {
    let row: Option<(String, Vec<u8>, i64)> = conn
        .query_row(
            "SELECT type, value_enc, enc_version FROM config WHERE key = ?1",
            [key],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    let Some((type_text, value_enc, enc_version)) = row else {
        return Ok(None);
    };
    match type_text.as_str() {
        "bool" => Ok(Some(decrypt_display(
            kek,
            &BoolOrTextRow {
                key: key.to_string(),
                kind: BoolOrText::Bool,
                value_enc,
                enc_version,
            },
        )?)),
        "text" => Ok(Some(decrypt_display(
            kek,
            &BoolOrTextRow {
                key: key.to_string(),
                kind: BoolOrText::Text,
                value_enc,
                enc_version,
            },
        )?)),
        _ => Ok(Some(DisplayValue::Secret { set: true })),
    }
}

/// Admin display read for all keys, ordered by key.
pub fn list_display(conn: &Connection, kek: &Kek) -> Result<Vec<(String, DisplayValue)>> {
    let rows: Vec<(String, String, Vec<u8>, i64)> = conn
        .prepare("SELECT key, type, value_enc, enc_version FROM config ORDER BY key")?
        .query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut out = Vec::with_capacity(rows.len());
    for (key, type_text, value_enc, enc_version) in &rows {
        let value = match type_text.as_str() {
            "bool" => decrypt_display(
                kek,
                &BoolOrTextRow {
                    key: key.clone(),
                    kind: BoolOrText::Bool,
                    value_enc: value_enc.clone(),
                    enc_version: *enc_version,
                },
            )?,
            "text" => decrypt_display(
                kek,
                &BoolOrTextRow {
                    key: key.clone(),
                    kind: BoolOrText::Text,
                    value_enc: value_enc.clone(),
                    enc_version: *enc_version,
                },
            )?,
            _ => DisplayValue::Secret { set: true },
        };
        out.push((key.clone(), value));
    }
    Ok(out)
}

/// Read a secret's bytes for server-side job code (e.g. the
/// transcribe job's API key). NEVER called from the admin API layer —
/// the type-level split (the private display decryptor cannot take
/// a secret) is what keeps that true under review.
pub fn get_secret_for_job(conn: &Connection, kek: &Kek, key: &str) -> Result<Option<Vec<u8>>> {
    let row: Option<(String, Vec<u8>, i64)> = conn
        .query_row(
            "SELECT type, value_enc, enc_version FROM config WHERE key = ?1",
            [key],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    let Some((type_text, value_enc, enc_version)) = row else {
        return Ok(None);
    };
    if type_text != "secret" {
        return Err(ConfigError::BadValue("job read asked for a non-secret key"));
    }
    decrypt_value(kek, key, enc_version, &value_enc).map(Some)
}

/// Re-encrypt every row from `old_kek` to `new_kek` under
/// `new_version`, inside one transaction. Returns rows rotated.
pub fn rotate(
    conn: &Connection,
    old_kek: &Kek,
    new_kek: &Kek,
    new_version: i64,
    now: i64,
    actor: &str,
) -> Result<usize> {
    conn.execute_batch("BEGIN IMMEDIATE")?;
    let outcome = (|| {
        let rows: Vec<(String, String, Vec<u8>, i64)> = conn
            .prepare("SELECT key, type, value_enc, enc_version FROM config")?
            .query_map([], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for (key, type_text, value_enc, enc_version) in &rows {
            let plain = decrypt_value(old_kek, key, *enc_version, value_enc)?;
            let blob = encrypt_value(new_kek, key, new_version, &plain);
            conn.execute(
                "UPDATE config SET value_enc = ?1, enc_version = ?2,
                                  updated_at = ?3, updated_by = ?4
                 WHERE key = ?5",
                rusqlite::params![blob, new_version, now, actor, key],
            )?;
            let _ = type_text;
        }
        Ok(rows.len())
    })();
    match outcome {
        Ok(count) => {
            conn.execute_batch("COMMIT")?;
            Ok(count)
        }
        Err(e) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(e)
        }
    }
}

/// Decrypt-all self-test (values never leave): opens every row and
/// checks AAD. Returns rows verified.
pub fn verify_all(conn: &Connection, kek: &Kek) -> Result<usize> {
    let rows: Vec<(String, Vec<u8>, i64)> = conn
        .prepare("SELECT key, value_enc, enc_version FROM config")?
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for (key, value_enc, enc_version) in &rows {
        decrypt_value(kek, key, *enc_version, value_enc)?;
    }
    Ok(rows.len())
}

/// Active KEK generation (`retired_at IS NULL`), if any.
pub fn active_kek_version(conn: &Connection) -> Result<Option<i64>> {
    Ok(conn
        .query_row(
            "SELECT version FROM config_kek WHERE retired_at IS NULL
             ORDER BY version DESC LIMIT 1",
            [],
            |row| row.get(0),
        )
        .optional()?)
}

/// Record a KEK generation (enroll path; `cred_path` is the sealed
/// `.cred` path, never key bytes).
pub fn record_kek(conn: &Connection, version: i64, cred_path: &str, now: i64) -> Result<()> {
    conn.execute(
        "INSERT INTO config_kek(version, cred_path, created_at, retired_at)
         VALUES (?1, ?2, ?3, NULL)",
        rusqlite::params![version, cred_path, now],
    )?;
    Ok(())
}

/// Retire a KEK generation after a verified rotate + reboot.
pub fn retire_kek(conn: &Connection, version: i64, now: i64) -> Result<()> {
    conn.execute(
        "UPDATE config_kek SET retired_at = ?1 WHERE version = ?2",
        rusqlite::params![now, version],
    )?;
    Ok(())
}

/// Where the KEK came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KekSource {
    /// `$CREDENTIALS_DIRECTORY/handbook-kek` (systemd, TPM2-sealed).
    SystemdCredentials,
    /// `HANDBOOK_DEV_KEYFILE` (Mac dev only).
    DevFile,
    /// `HANDBOOK_DEV_KEK` (Mac dev / headless CI only).
    DevEnv,
}

/// Parse a dev KEK: `dev:` prefix (mandatory marker) + 64 hex chars.
/// The prefix is what the prod refusal keys on.
pub fn parse_dev_kek(text: &str) -> std::result::Result<[u8; 32], &'static str> {
    let hex = text
        .trim()
        .strip_prefix("dev:")
        .ok_or("dev KEK needs a dev: prefix")?;
    if hex.len() != 64 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("dev KEK needs 64 hex chars after dev:");
    }
    let mut out = [0u8; 32];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).map_err(|_| "bad hex")?;
    }
    Ok(out)
}

/// Load the KEK: systemd credentials first, then the dev fallbacks.
///
/// This is the hardware-path collection point (systemd unseals via
/// TPM2 before exec); sealed-blob flows use [`Sealer`] providers
/// instead. Dev sources print a loud banner and are REFUSED outside
/// debug builds — a dev-marked KEK can never start a release binary.
pub fn load_kek() -> Result<(Kek, KekSource)> {
    if let Ok(dir) = std::env::var("CREDENTIALS_DIRECTORY") {
        let path = std::path::Path::new(&dir).join("handbook-kek");
        if let Ok(bytes) = std::fs::read(&path) {
            if bytes.len() == 32 {
                let mut raw = [0u8; 32];
                raw.copy_from_slice(&bytes);
                return Ok((Kek::from_bytes(raw), KekSource::SystemdCredentials));
            }
            return Err(ConfigError::KekUnavailable(format!(
                "{}: want 32 raw bytes",
                path.display()
            )));
        }
    }
    dev_kek_from("HANDBOOK_DEV_KEYFILE", KekSource::DevFile, true)?
        .or(dev_kek_from("HANDBOOK_DEV_KEK", KekSource::DevEnv, false)?)
        .map_or_else(
            || {
                Err(ConfigError::KekUnavailable(
                    "set CREDENTIALS_DIRECTORY/handbook-kek (server) or HANDBOOK_DEV_KEK (dev)"
                        .to_string(),
                ))
            },
            Ok,
        )
}

fn dev_kek_from(var: &str, source: KekSource, is_path: bool) -> Result<Option<(Kek, KekSource)>> {
    let Ok(raw) = std::env::var(var) else {
        return Ok(None);
    };
    if !cfg!(debug_assertions) {
        return Err(ConfigError::KekUnavailable(format!(
            "{var}: dev KEK refused in a release build"
        )));
    }
    eprintln!("DEV CRYPTO — {var}-backed dev KEK, NOT TPM-sealed, synthetic data only");
    let text = if is_path {
        std::fs::read_to_string(&raw)
            .map_err(|e| ConfigError::KekUnavailable(format!("{var}={raw}: {e}")))?
    } else {
        raw
    };
    let bytes =
        parse_dev_kek(&text).map_err(|why| ConfigError::KekUnavailable(format!("{var}: {why}")))?;
    Ok(Some((Kek::from_bytes(bytes), source)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn memory() -> Connection {
        let conn = Connection::open_in_memory().expect("open memory db");
        migrate_config_tables(&conn).expect("migrate config tables");
        conn
    }

    fn put(
        conn: &Connection,
        kek: &Kek,
        key: &str,
        key_type: ConfigType,
        plaintext: &[u8],
    ) -> Result<()> {
        set_value(
            conn,
            kek,
            &SetValue {
                version: 1,
                key,
                key_type,
                plaintext,
                now: 100,
                actor: "t",
            },
        )
    }

    #[test]
    fn envelope_roundtrips() {
        let kek = Kek::generate();
        let blob = encrypt_value(&kek, "mailer.enabled", 1, b"1");
        assert!(blob.len() > NONCE_LEN);
        // Fresh nonce per seal: same input, different bytes.
        let again = encrypt_value(&kek, "mailer.enabled", 1, b"1");
        assert_ne!(blob, again);
        assert_eq!(
            decrypt_value(&kek, "mailer.enabled", 1, &blob).expect("open"),
            b"1"
        );
    }

    #[test]
    fn aad_binds_key_name_version_and_kek() {
        let kek = Kek::generate();
        let other = Kek::generate();
        let blob = encrypt_value(&kek, "a.key", 1, b"value");
        // Wrong key name, wrong version, wrong KEK, truncation, flip.
        assert!(decrypt_value(&kek, "b.key", 1, &blob).is_err());
        assert!(decrypt_value(&kek, "a.key", 2, &blob).is_err());
        assert!(decrypt_value(&other, "a.key", 1, &blob).is_err());
        assert!(decrypt_value(&kek, "a.key", 1, &blob[..NONCE_LEN + 4]).is_err());
        let mut flipped = blob.clone();
        let last = flipped.len() - 1;
        flipped[last] ^= 0x01;
        assert!(decrypt_value(&kek, "a.key", 1, &flipped).is_err());
    }

    #[test]
    fn typed_set_and_display() {
        let conn = memory();
        let kek = Kek::generate();
        put(&conn, &kek, "mailer.enabled", ConfigType::Bool, b"1").expect("set bool");
        put(
            &conn,
            &kek,
            "mailer.host",
            ConfigType::Text,
            b"mail.example.test",
        )
        .expect("set text");
        put(&conn, &kek, "mailer.key", ConfigType::Secret, b"s3cr3t").expect("set secret");
        assert_eq!(
            get_display(&conn, &kek, "mailer.enabled").expect("get"),
            Some(DisplayValue::Bool(true))
        );
        assert_eq!(
            get_display(&conn, &kek, "mailer.host").expect("get"),
            Some(DisplayValue::Text("mail.example.test".to_string()))
        );
        // Write-only: presence only, never bytes.
        assert_eq!(
            get_display(&conn, &kek, "mailer.key").expect("get"),
            Some(DisplayValue::Secret { set: true })
        );
        assert_eq!(get_display(&conn, &kek, "missing").expect("get"), None);
        let listed = list_display(&conn, &kek).expect("list");
        assert_eq!(listed.len(), 3);
        assert_eq!(listed[0].0, "mailer.enabled");
    }

    #[test]
    fn secrets_flow_only_to_jobs() {
        let conn = memory();
        let kek = Kek::generate();
        put(&conn, &kek, "job.key", ConfigType::Secret, b"bytes").expect("set");
        assert_eq!(
            get_secret_for_job(&conn, &kek, "job.key").expect("job read"),
            Some(b"bytes".to_vec())
        );
        assert_eq!(
            get_secret_for_job(&conn, &kek, "missing").expect("missing"),
            None
        );
        put(&conn, &kek, "plain", ConfigType::Text, b"x").expect("set");
        get_secret_for_job(&conn, &kek, "plain").expect_err("non-secret refused");
    }

    #[test]
    fn values_are_validated() {
        let conn = memory();
        let kek = Kek::generate();
        put(&conn, &kek, "b", ConfigType::Bool, b"yes").expect_err("bad bool");
        put(&conn, &kek, "t", ConfigType::Text, &[0xff]).expect_err("non-utf8");
        put(&conn, &kek, "s", ConfigType::Secret, b"").expect_err("empty");
        put(&conn, &kek, "bad name!", ConfigType::Text, b"x").expect_err("bad name");
        assert!(valid_key_name("fedex.account_id"));
        assert!(!valid_key_name(""));
    }

    #[test]
    fn rotate_rewraps_everything() {
        let conn = memory();
        let old = Kek::generate();
        let new = Kek::generate();
        put(&conn, &old, "a", ConfigType::Text, b"1").expect("set");
        put(&conn, &old, "b", ConfigType::Bool, b"0").expect("set");
        record_kek(
            &conn,
            2,
            "/etc/credstore.encrypted/handbook-kek-v2.cred",
            200,
        )
        .expect("record");
        assert_eq!(rotate(&conn, &old, &new, 2, 200, "t").expect("rotate"), 2);
        assert_eq!(verify_all(&conn, &new).expect("verify"), 2);
        assert_eq!(
            get_display(&conn, &new, "a").expect("get"),
            Some(DisplayValue::Text("1".to_string()))
        );
        verify_all(&conn, &old).expect_err("old KEK no longer opens rows");
        retire_kek(&conn, 1, 300).expect("retire");
        assert_eq!(active_kek_version(&conn).expect("active"), Some(2));
    }

    #[test]
    fn dev_kek_parsing_and_redaction() {
        let bytes = parse_dev_kek(&format!("dev:{}", "ab".repeat(32))).expect("parse");
        assert_eq!(bytes, [0xab; 32]);
        parse_dev_kek(&"ab".repeat(32)).expect_err("prefix required");
        parse_dev_kek("dev:short").expect_err("length checked");
        assert_eq!(format!("{:?}", Kek::from_bytes([7u8; 32])), "Kek(redacted)");
        assert_eq!(ConfigType::parse("secret"), Some(ConfigType::Secret));
        assert_eq!(ConfigType::parse("yaml"), None);
    }
}

// ---------------------------------------------------------------------------
// DB-backed runtime config (goal-025 stream 4, zero-env).
//
// `docs/specs/config-db.md` owns the registry: every non-bootstrap
// runtime setting lives in a sealed `config` row and is read once at
// boot into [`RuntimeConfig`]. Missing row ≡ compiled default, so a
// fresh store boots exactly like the old env-default boot. Nothing
// above this line changed for the migration (crypto untouched).
// ---------------------------------------------------------------------------

/// DB key names for every runtime setting (spec §2.1).
pub mod keys {
    pub const PATHS_STATIC_DIR: &str = "paths.static_dir";
    pub const PATHS_SPA_DIR: &str = "paths.spa_dir";
    pub const PATHS_ADMIN_SPA_DIR: &str = "paths.admin_spa_dir";
    pub const PATHS_ACME_CHALLENGE_DIR: &str = "paths.acme_challenge_dir";
    pub const NET_BIND: &str = "net.bind";
    pub const NET_BIND6: &str = "net.bind6";
    pub const NET_PORT: &str = "net.port";
    pub const NET_ADMIN_PORT: &str = "net.admin_port";
    pub const NET_HTTPS_PORT: &str = "net.https_port";
    pub const NET_ADMIN_HTTPS_PORT: &str = "net.admin_https_port";
    pub const NET_HTTP_PORT: &str = "net.http_port";
    pub const NET_PUBLIC_HOST: &str = "net.public_host";
    pub const TLS_CERT_FILE: &str = "tls.cert_file";
    pub const TLS_KEY_FILE: &str = "tls.key_file";
    pub const TLS_CLIENT_CA_FILE: &str = "tls.client_ca_file";
    pub const ADMIN_MTLS_OFF: &str = "admin.mtls_off";
    pub const MEDIA_TERMS_READ: &str = "media.terms_read";
    pub const ACME_ENABLED: &str = "acme.enabled";
    pub const ACME_DIRECTORY: &str = "acme.directory";
    pub const ACME_DOMAINS: &str = "acme.domains";
    pub const ACME_EMAIL: &str = "acme.email";
    pub const ACME_EAB_KID: &str = "acme.eab_kid";
    pub const ACME_EAB_HMAC: &str = "acme.eab_hmac";
    pub const USPS_BASE_URL: &str = "usps.base_url";
    pub const USPS_CERTIFIED: &str = "usps.certified";
    pub const TELNYX_BASE_URL: &str = "telnyx.base_url";
    pub const TELNYX_FROM_NUMBER: &str = "telnyx.from_number";
    pub const TELNYX_CONNECTION_ID: &str = "telnyx.connection_id";
    pub const TELNYX_WEBHOOK_PUBLIC_KEY: &str = "telnyx.webhook_public_key";
    pub const TELNYX_API_KEY: &str = "telnyx.api_key";
    /// Native QUIC/HTTP-3 listener (single-443 spec §2): bind UDP
    /// on `net.https_port`, serve h3, send Alt-Svc. Default false.
    pub const QUIC_ENABLED: &str = "quic.enabled";
    pub const AUTH_RP_ID: &str = "auth.rp_id";
    pub const AUTH_RP_NAME: &str = "auth.rp_name";
    pub const AUTH_ORIGIN: &str = "auth.origin";

    /// Every registered key with its type (admin UI + roundtrip tests).
    pub const ALL: &[(&str, super::ConfigType)] = &[
        (PATHS_STATIC_DIR, super::ConfigType::Text),
        (PATHS_SPA_DIR, super::ConfigType::Text),
        (PATHS_ADMIN_SPA_DIR, super::ConfigType::Text),
        (PATHS_ACME_CHALLENGE_DIR, super::ConfigType::Text),
        (NET_BIND, super::ConfigType::Text),
        (NET_BIND6, super::ConfigType::Text),
        (NET_PORT, super::ConfigType::Text),
        (NET_ADMIN_PORT, super::ConfigType::Text),
        (NET_HTTPS_PORT, super::ConfigType::Text),
        (NET_ADMIN_HTTPS_PORT, super::ConfigType::Text),
        (NET_HTTP_PORT, super::ConfigType::Text),
        (NET_PUBLIC_HOST, super::ConfigType::Text),
        (TLS_CERT_FILE, super::ConfigType::Text),
        (TLS_KEY_FILE, super::ConfigType::Text),
        (TLS_CLIENT_CA_FILE, super::ConfigType::Text),
        (ADMIN_MTLS_OFF, super::ConfigType::Bool),
        (MEDIA_TERMS_READ, super::ConfigType::Bool),
        (ACME_ENABLED, super::ConfigType::Bool),
        (ACME_DIRECTORY, super::ConfigType::Text),
        (ACME_DOMAINS, super::ConfigType::Text),
        (ACME_EMAIL, super::ConfigType::Text),
        (ACME_EAB_KID, super::ConfigType::Text),
        (ACME_EAB_HMAC, super::ConfigType::Secret),
        (USPS_BASE_URL, super::ConfigType::Text),
        (USPS_CERTIFIED, super::ConfigType::Bool),
        (TELNYX_BASE_URL, super::ConfigType::Text),
        (TELNYX_FROM_NUMBER, super::ConfigType::Text),
        (TELNYX_CONNECTION_ID, super::ConfigType::Text),
        (TELNYX_WEBHOOK_PUBLIC_KEY, super::ConfigType::Text),
        (TELNYX_API_KEY, super::ConfigType::Secret),
        (QUIC_ENABLED, super::ConfigType::Bool),
        (AUTH_RP_ID, super::ConfigType::Text),
        (AUTH_RP_NAME, super::ConfigType::Text),
        (AUTH_ORIGIN, super::ConfigType::Text),
    ];
}

/// Raw row fetch shared by the runtime `*_for_job` readers.
fn get_row(conn: &Connection, key: &str) -> Result<Option<(String, Vec<u8>, i64)>> {
    Ok(conn
        .query_row(
            "SELECT type, value_enc, enc_version FROM config WHERE key = ?1",
            [key],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?)
}

/// Read a `bool` row for server-side code (boot-time config load).
/// Missing key is `None`; a present-but-wrong type or non-bool
/// bytes are [`ConfigError::BadValue`] (the loader warns + defaults).
pub fn get_bool_for_job(conn: &Connection, kek: &Kek, key: &str) -> Result<Option<bool>> {
    let Some((type_text, value_enc, enc_version)) = get_row(conn, key)? else {
        return Ok(None);
    };
    if type_text != "bool" {
        return Err(ConfigError::BadValue("job read asked for a non-bool key"));
    }
    match decrypt_value(kek, key, enc_version, &value_enc)?.as_slice() {
        b"0" => Ok(Some(false)),
        b"1" => Ok(Some(true)),
        _ => Err(ConfigError::BadValue("bool row holds non-bool bytes")),
    }
}

/// Read a `text` row for server-side code (boot-time config load).
/// Missing key is `None`; a present-but-wrong type or non-UTF-8
/// bytes are [`ConfigError::BadValue`] (the loader warns + defaults).
pub fn get_text_for_job(conn: &Connection, kek: &Kek, key: &str) -> Result<Option<String>> {
    let Some((type_text, value_enc, enc_version)) = get_row(conn, key)? else {
        return Ok(None);
    };
    if type_text != "text" {
        return Err(ConfigError::BadValue("job read asked for a non-text key"));
    }
    String::from_utf8(decrypt_value(kek, key, enc_version, &value_enc)?)
        .map(Some)
        .map_err(|_| ConfigError::BadValue("text row is not UTF-8"))
}

/// Secret text that never renders in `Debug` (key material).
#[derive(Clone, PartialEq, Eq)]
pub struct SecretText(String);

impl SecretText {
    /// Borrow the secret bytes as text (caller must not log them).
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for SecretText {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SecretText(<redacted>)")
    }
}

/// Every runtime setting, read once at boot from sealed DB rows.
///
/// `Default` is exactly the old env-default boot (spec §2.1), so a
/// fresh store behaves like the old binary with no env set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeConfig {
    pub static_dir: String,
    pub spa_dir: String,
    pub admin_spa_dir: String,
    pub acme_challenge_dir: String,
    pub bind: std::net::IpAddr,
    pub bind6: Option<std::net::IpAddr>,
    pub port: u16,
    pub admin_port: u16,
    pub https_port: u16,
    /// DEPRECATED (single-443): parsed but never bound. Kept so
    /// existing rows do not become unknown keys mid-rollout.
    pub admin_https_port: u16,
    pub quic_enabled: bool,
    pub http_port: u16,
    pub public_host: String,
    pub tls_cert: Option<String>,
    pub tls_key: Option<String>,
    pub tls_client_ca: Option<String>,
    pub admin_mtls_off: bool,
    pub media_terms_read: bool,
    pub acme_enabled: bool,
    pub acme_directory: String,
    pub acme_domains_csv: String,
    pub acme_email: Option<String>,
    pub acme_eab_kid: Option<String>,
    pub acme_eab_hmac_b64: Option<SecretText>,
    pub usps_base_url: String,
    pub usps_certified: bool,
    pub telnyx_base_url: String,
    pub telnyx_from_number: Option<String>,
    pub telnyx_connection_id: Option<String>,
    pub telnyx_webhook_public_key: Option<String>,
    pub telnyx_api_key: Option<SecretText>,
    pub auth_rp_id: String,
    pub auth_rp_name: String,
    pub auth_origin: String,
}

impl Default for RuntimeConfig {
    /// Old env-default boot, key for key (spec §2.1).
    fn default() -> Self {
        Self {
            static_dir: "crates/web/static".to_string(),
            spa_dir: "web/build".to_string(),
            admin_spa_dir: "web/build-admin".to_string(),
            acme_challenge_dir: "acme".to_string(),
            bind: std::net::IpAddr::from([127, 0, 0, 1]),
            bind6: None,
            port: 3000,
            admin_port: 3001,
            https_port: 443,
            admin_https_port: 444,
            quic_enabled: false,
            http_port: 80,
            public_host: "andrii.ai".to_string(),
            tls_cert: None,
            tls_key: None,
            tls_client_ca: None,
            admin_mtls_off: false,
            media_terms_read: false,
            acme_enabled: false,
            acme_directory: "prod".to_string(),
            acme_domains_csv: "andrii.ai,admin.andrii.ai,www.andrii.ai".to_string(),
            acme_email: None,
            acme_eab_kid: None,
            acme_eab_hmac_b64: None,
            // Must equal `labels::USPS_BASE_TEST`; asserted by a web
            // test (this crate cannot depend on labels for one string).
            usps_base_url: "https://apis-tem.usps.com".to_string(),
            usps_certified: false,
            // Must equal `voice::TELNYX_BASE_URL`; asserted by a web
            // test (this crate cannot depend on voice for one string).
            telnyx_base_url: "https://api.telnyx.com".to_string(),
            telnyx_from_number: None,
            telnyx_connection_id: None,
            telnyx_webhook_public_key: None,
            telnyx_api_key: None,
            auth_rp_id: "andrii.ai".to_string(),
            auth_rp_name: "Apostille Handbook".to_string(),
            auth_origin: "https://andrii.ai".to_string(),
        }
    }
}

/// Lenient bool overlay: missing → keep default; wrong type/shape →
/// warn + keep default. Crypto/DB errors propagate (refuse the boot).
fn overlay_bool(
    conn: &Connection,
    kek: &Kek,
    key: &str,
    warnings: &mut Vec<String>,
    slot: &mut bool,
) -> Result<()> {
    match get_bool_for_job(conn, kek, key) {
        Ok(None) => Ok(()),
        Ok(Some(v)) => {
            *slot = v;
            Ok(())
        }
        Err(ConfigError::BadValue(why)) => {
            warnings.push(format!("{key}: {why}; using default"));
            Ok(())
        }
        Err(e) => Err(e),
    }
}

/// Lenient required-text overlay: missing or empty → default (empty
/// warns); wrong type → warn + default. Crypto/DB propagate.
fn overlay_required_text(
    conn: &Connection,
    kek: &Kek,
    key: &str,
    warnings: &mut Vec<String>,
    slot: &mut String,
) -> Result<()> {
    match get_text_for_job(conn, kek, key) {
        Ok(None) => Ok(()),
        Ok(Some(v)) if v.trim().is_empty() => {
            warnings.push(format!("{key}: empty value; using default"));
            Ok(())
        }
        Ok(Some(v)) => {
            *slot = v;
            Ok(())
        }
        Err(ConfigError::BadValue(why)) => {
            warnings.push(format!("{key}: {why}; using default"));
            Ok(())
        }
        Err(e) => Err(e),
    }
}

/// Lenient optional-text overlay: missing or empty → unset (silent);
/// wrong type → warn + unset. Crypto/DB errors propagate.
fn overlay_optional_text(
    conn: &Connection,
    kek: &Kek,
    key: &str,
    warnings: &mut Vec<String>,
    slot: &mut Option<String>,
) -> Result<()> {
    match get_text_for_job(conn, kek, key) {
        Ok(None) => Ok(()),
        Ok(Some(v)) if v.trim().is_empty() => Ok(()),
        Ok(Some(v)) => {
            *slot = Some(v);
            Ok(())
        }
        Err(ConfigError::BadValue(why)) => {
            warnings.push(format!("{key}: {why}; treating as unset"));
            Ok(())
        }
        Err(e) => Err(e),
    }
}

/// Lenient secret overlay: missing or empty → `None` (silent);
/// non-UTF-8 or wrong type → warn + `None`. Crypto/DB propagate.
fn overlay_secret(
    conn: &Connection,
    kek: &Kek,
    key: &str,
    warnings: &mut Vec<String>,
    slot: &mut Option<SecretText>,
) -> Result<()> {
    match get_secret_for_job(conn, kek, key) {
        Ok(None) => Ok(()),
        Ok(Some(bytes)) => match String::from_utf8(bytes) {
            Ok(s) if s.trim().is_empty() => Ok(()),
            Ok(s) => {
                *slot = Some(SecretText(s));
                Ok(())
            }
            Err(_) => {
                warnings.push(format!("{key}: not UTF-8; treating as missing"));
                Ok(())
            }
        },
        Err(ConfigError::BadValue(why)) => {
            warnings.push(format!("{key}: {why}; treating as missing"));
            Ok(())
        }
        Err(e) => Err(e),
    }
}

/// Lenient port overlay: missing or unparsable → default (unparsable
/// warns, mirroring the old `port_env` garbage→default leniency).
fn overlay_port(
    conn: &Connection,
    kek: &Kek,
    key: &str,
    warnings: &mut Vec<String>,
    slot: &mut u16,
) -> Result<()> {
    match get_text_for_job(conn, kek, key) {
        Ok(None) => Ok(()),
        Ok(Some(v)) => match v.trim().parse::<u16>() {
            Ok(p) => {
                *slot = p;
                Ok(())
            }
            Err(_) => {
                warnings.push(format!("{key}: {v:?} is not a u16 port; using default"));
                Ok(())
            }
        },
        Err(ConfigError::BadValue(why)) => {
            warnings.push(format!("{key}: {why}; using default"));
            Ok(())
        }
        Err(e) => Err(e),
    }
}

impl RuntimeConfig {
    /// Read-once boot load: overlay present rows onto [`Self::default`].
    /// Returns the config plus one warning per defaulted row (main
    /// prints them; behavior already matches the old lenient reads).
    ///
    /// # Errors
    ///
    /// `Crypto` (wrong KEK or tampered row) and `Db` refuse the boot:
    /// crypto failures must never silently default. `admin.mtls_off`
    /// set in a release build refuses too (dev-only seam).
    pub fn load(conn: &Connection, kek: &Kek) -> Result<(Self, Vec<String>)> {
        let mut warnings = Vec::new();
        let mut cfg = Self::default();
        overlay_required_text(
            conn,
            kek,
            keys::PATHS_STATIC_DIR,
            &mut warnings,
            &mut cfg.static_dir,
        )?;
        overlay_required_text(
            conn,
            kek,
            keys::PATHS_SPA_DIR,
            &mut warnings,
            &mut cfg.spa_dir,
        )?;
        overlay_required_text(
            conn,
            kek,
            keys::PATHS_ADMIN_SPA_DIR,
            &mut warnings,
            &mut cfg.admin_spa_dir,
        )?;
        overlay_required_text(
            conn,
            kek,
            keys::PATHS_ACME_CHALLENGE_DIR,
            &mut warnings,
            &mut cfg.acme_challenge_dir,
        )?;
        overlay_required_text(
            conn,
            kek,
            keys::NET_PUBLIC_HOST,
            &mut warnings,
            &mut cfg.public_host,
        )?;
        overlay_required_text(
            conn,
            kek,
            keys::ACME_DIRECTORY,
            &mut warnings,
            &mut cfg.acme_directory,
        )?;
        overlay_required_text(
            conn,
            kek,
            keys::ACME_DOMAINS,
            &mut warnings,
            &mut cfg.acme_domains_csv,
        )?;
        overlay_required_text(
            conn,
            kek,
            keys::USPS_BASE_URL,
            &mut warnings,
            &mut cfg.usps_base_url,
        )?;
        overlay_required_text(
            conn,
            kek,
            keys::TELNYX_BASE_URL,
            &mut warnings,
            &mut cfg.telnyx_base_url,
        )?;
        overlay_optional_text(
            conn,
            kek,
            keys::TELNYX_FROM_NUMBER,
            &mut warnings,
            &mut cfg.telnyx_from_number,
        )?;
        overlay_optional_text(
            conn,
            kek,
            keys::TELNYX_CONNECTION_ID,
            &mut warnings,
            &mut cfg.telnyx_connection_id,
        )?;
        overlay_optional_text(
            conn,
            kek,
            keys::TELNYX_WEBHOOK_PUBLIC_KEY,
            &mut warnings,
            &mut cfg.telnyx_webhook_public_key,
        )?;
        overlay_optional_text(
            conn,
            kek,
            keys::TLS_CERT_FILE,
            &mut warnings,
            &mut cfg.tls_cert,
        )?;
        overlay_optional_text(
            conn,
            kek,
            keys::TLS_KEY_FILE,
            &mut warnings,
            &mut cfg.tls_key,
        )?;
        overlay_optional_text(
            conn,
            kek,
            keys::TLS_CLIENT_CA_FILE,
            &mut warnings,
            &mut cfg.tls_client_ca,
        )?;
        overlay_optional_text(
            conn,
            kek,
            keys::ACME_EMAIL,
            &mut warnings,
            &mut cfg.acme_email,
        )?;
        overlay_optional_text(
            conn,
            kek,
            keys::ACME_EAB_KID,
            &mut warnings,
            &mut cfg.acme_eab_kid,
        )?;
        overlay_bool(
            conn,
            kek,
            keys::ADMIN_MTLS_OFF,
            &mut warnings,
            &mut cfg.admin_mtls_off,
        )?;
        overlay_bool(
            conn,
            kek,
            keys::MEDIA_TERMS_READ,
            &mut warnings,
            &mut cfg.media_terms_read,
        )?;
        overlay_bool(
            conn,
            kek,
            keys::ACME_ENABLED,
            &mut warnings,
            &mut cfg.acme_enabled,
        )?;
        overlay_bool(
            conn,
            kek,
            keys::USPS_CERTIFIED,
            &mut warnings,
            &mut cfg.usps_certified,
        )?;
        overlay_port(conn, kek, keys::NET_PORT, &mut warnings, &mut cfg.port)?;
        overlay_port(
            conn,
            kek,
            keys::NET_ADMIN_PORT,
            &mut warnings,
            &mut cfg.admin_port,
        )?;
        overlay_port(
            conn,
            kek,
            keys::NET_HTTPS_PORT,
            &mut warnings,
            &mut cfg.https_port,
        )?;
        overlay_port(
            conn,
            kek,
            keys::NET_ADMIN_HTTPS_PORT,
            &mut warnings,
            &mut cfg.admin_https_port,
        )?;
        // DEPRECATED (single-443): the value parses but no listener
        // binds it. A SET row warns loudly; unset stays silent so
        // existing DBs boot exactly as before.
        match get_text_for_job(conn, kek, keys::NET_ADMIN_HTTPS_PORT) {
            Ok(Some(_)) => warnings.push(format!(
                "{} is deprecated and IGNORED: single-443 serves admin on {} — no listener is bound to this port",
                keys::NET_ADMIN_HTTPS_PORT,
                keys::NET_HTTPS_PORT
            )),
            Ok(None) => {}
            // Wrong-typed rows already warned in overlay_port above.
            Err(ConfigError::BadValue(_)) => {}
            Err(e) => return Err(e),
        }
        overlay_bool(
            conn,
            kek,
            keys::QUIC_ENABLED,
            &mut warnings,
            &mut cfg.quic_enabled,
        )?;
        overlay_required_text(
            conn,
            kek,
            keys::AUTH_RP_ID,
            &mut warnings,
            &mut cfg.auth_rp_id,
        )?;
        overlay_required_text(
            conn,
            kek,
            keys::AUTH_RP_NAME,
            &mut warnings,
            &mut cfg.auth_rp_name,
        )?;
        overlay_required_text(
            conn,
            kek,
            keys::AUTH_ORIGIN,
            &mut warnings,
            &mut cfg.auth_origin,
        )?;
        overlay_port(
            conn,
            kek,
            keys::NET_HTTP_PORT,
            &mut warnings,
            &mut cfg.http_port,
        )?;
        // `net.bind`: missing/empty/garbage → 127.0.0.1 (old `BIND` leniency).
        match get_text_for_job(conn, kek, keys::NET_BIND) {
            Ok(None) => {}
            Ok(Some(v)) => match v.trim().parse::<std::net::IpAddr>() {
                Ok(ip) => cfg.bind = ip,
                Err(_) => warnings.push(format!(
                    "{}: {v:?} is not an IP; using 127.0.0.1",
                    keys::NET_BIND
                )),
            },
            Err(ConfigError::BadValue(why)) => {
                warnings.push(format!("{}: {why}; using 127.0.0.1", keys::NET_BIND))
            }
            Err(e) => return Err(e),
        }
        // `net.bind6`: missing/empty/garbage → unset (old `BIND6` leniency).
        match get_text_for_job(conn, kek, keys::NET_BIND6) {
            Ok(None) => {}
            Ok(Some(v)) if v.trim().is_empty() => {}
            Ok(Some(v)) => match v.trim().parse::<std::net::IpAddr>() {
                Ok(ip) => cfg.bind6 = Some(ip),
                Err(_) => {
                    warnings.push(format!("{}: {v:?} is not an IP; ignoring", keys::NET_BIND6))
                }
            },
            Err(ConfigError::BadValue(why)) => {
                warnings.push(format!("{}: {why}; ignoring", keys::NET_BIND6));
            }
            Err(e) => return Err(e),
        }
        // Secrets in the runtime set: non-UTF-8 or wrong-typed rows
        // warn + count as missing (consumers fail closed on `None`).
        overlay_secret(
            conn,
            kek,
            keys::ACME_EAB_HMAC,
            &mut warnings,
            &mut cfg.acme_eab_hmac_b64,
        )?;
        overlay_secret(
            conn,
            kek,
            keys::TELNYX_API_KEY,
            &mut warnings,
            &mut cfg.telnyx_api_key,
        )?;
        // H6 fix baked into the migration: a persistent dev-bypass
        // flag must never boot a release binary (spec §3).
        if cfg.admin_mtls_off && !cfg!(debug_assertions) {
            return Err(ConfigError::BadValue(
                "admin.mtls_off is dev-only; refusing release boot",
            ));
        }
        Ok((cfg, warnings))
    }

    /// IPs to bind: v4 plus optional v6 (replaces `BIND`/`BIND6`).
    pub fn bind_ips(&self) -> Vec<std::net::IpAddr> {
        let mut ips = vec![self.bind];
        if let Some(v6) = self.bind6 {
            ips.push(v6);
        }
        ips
    }
}

#[cfg(test)]
mod runtime_tests {
    use super::{keys::ALL as ALL_KEYS, *};

    fn memory() -> Connection {
        let conn = Connection::open_in_memory().expect("open memory db");
        migrate_config_tables(&conn).expect("migrate config tables");
        conn
    }

    fn put(conn: &Connection, kek: &Kek, key: &str, key_type: ConfigType, plaintext: &[u8]) {
        set_value(
            conn,
            kek,
            &SetValue {
                version: 1,
                key,
                key_type,
                plaintext,
                now: 1,
                actor: "t",
            },
        )
        .expect("set test row");
    }

    #[test]
    fn registry_holds_34_typed_keys() {
        assert_eq!(ALL_KEYS.len(), 34);
        for (key, _) in ALL_KEYS {
            assert!(valid_key_name(key), "key {key:?} must be a valid name");
        }
        // Two secrets (EAB HMAC, Telnyx API key); five bools
        // (quic.enabled is the fifth); rest text (auth.* adds three).
        let mut counts = (0, 0, 0);
        for (_, key_type) in ALL_KEYS {
            match key_type {
                ConfigType::Bool => counts.0 += 1,
                ConfigType::Text => counts.1 += 1,
                ConfigType::Secret => counts.2 += 1,
            }
        }
        assert_eq!(counts, (5, 27, 2));
    }

    #[test]
    fn defaults_match_legacy_env_defaults() {
        // Independent oracle: literals copied from the old env reads,
        // not from the implementation. Any drift breaks this test.
        let cfg = RuntimeConfig::default();
        assert_eq!(cfg.static_dir, "crates/web/static");
        assert_eq!(cfg.spa_dir, "web/build");
        assert_eq!(cfg.admin_spa_dir, "web/build-admin");
        assert_eq!(cfg.acme_challenge_dir, "acme");
        assert_eq!(cfg.bind, std::net::IpAddr::from([127, 0, 0, 1]));
        assert_eq!(cfg.bind6, None);
        assert_eq!(cfg.bind_ips(), vec![std::net::IpAddr::from([127, 0, 0, 1])]);
        assert_eq!(cfg.port, 3000);
        assert_eq!(cfg.admin_port, 3001);
        assert_eq!(cfg.https_port, 443);
        assert_eq!(cfg.admin_https_port, 444);
        assert!(!cfg.quic_enabled);
        assert_eq!(cfg.http_port, 80);
        assert_eq!(cfg.public_host, "andrii.ai");
        assert_eq!(cfg.tls_cert, None);
        assert_eq!(cfg.tls_key, None);
        assert_eq!(cfg.tls_client_ca, None);
        assert!(!cfg.admin_mtls_off);
        assert!(!cfg.media_terms_read);
        assert!(!cfg.acme_enabled);
        assert_eq!(cfg.acme_directory, "prod");
        assert_eq!(
            cfg.acme_domains_csv,
            "andrii.ai,admin.andrii.ai,www.andrii.ai"
        );
        assert_eq!(cfg.acme_email, None);
        assert_eq!(cfg.acme_eab_kid, None);
        assert_eq!(cfg.acme_eab_hmac_b64, None);
        assert_eq!(cfg.usps_base_url, "https://apis-tem.usps.com");
        assert!(!cfg.usps_certified);
        assert_eq!(cfg.telnyx_base_url, "https://api.telnyx.com");
        assert_eq!(cfg.telnyx_from_number, None);
        assert_eq!(cfg.telnyx_connection_id, None);
        assert_eq!(cfg.telnyx_webhook_public_key, None);
        assert_eq!(cfg.telnyx_api_key, None);
        assert_eq!(cfg.auth_rp_id, "andrii.ai");
        assert_eq!(cfg.auth_rp_name, "Apostille Handbook");
        assert_eq!(cfg.auth_origin, "https://andrii.ai");
    }

    #[test]
    fn missing_rows_yield_defaults_silently() {
        let conn = memory();
        let kek = Kek::generate();
        let (cfg, warnings) = RuntimeConfig::load(&conn, &kek).expect("load empty db");
        assert_eq!(cfg, RuntimeConfig::default());
        assert!(warnings.is_empty());
    }

    #[test]
    fn every_key_overlays_with_valid_values() {
        let conn = memory();
        let kek = Kek::generate();
        for (key, key_type, plaintext) in [
            (keys::PATHS_STATIC_DIR, ConfigType::Text, "static-x"),
            (keys::PATHS_SPA_DIR, ConfigType::Text, "spa-x"),
            (keys::PATHS_ADMIN_SPA_DIR, ConfigType::Text, "admin-spa-x"),
            (keys::PATHS_ACME_CHALLENGE_DIR, ConfigType::Text, "acme-x"),
            (keys::NET_BIND, ConfigType::Text, "127.0.0.2"),
            (keys::NET_BIND6, ConfigType::Text, "::1"),
            (keys::NET_PORT, ConfigType::Text, "0"),
            (keys::NET_ADMIN_PORT, ConfigType::Text, "0"),
            (keys::NET_HTTPS_PORT, ConfigType::Text, "8443"),
            (keys::NET_ADMIN_HTTPS_PORT, ConfigType::Text, "8444"),
            (keys::NET_HTTP_PORT, ConfigType::Text, "8080"),
            (keys::NET_PUBLIC_HOST, ConfigType::Text, "example.test"),
            (keys::TLS_CERT_FILE, ConfigType::Text, "/x/cert.pem"),
            (keys::TLS_KEY_FILE, ConfigType::Text, "/x/key.pem"),
            (keys::TLS_CLIENT_CA_FILE, ConfigType::Text, "/x/ca.pem"),
            (keys::ADMIN_MTLS_OFF, ConfigType::Bool, "1"),
            (keys::MEDIA_TERMS_READ, ConfigType::Bool, "1"),
            (keys::ACME_ENABLED, ConfigType::Bool, "1"),
            (keys::ACME_DIRECTORY, ConfigType::Text, "staging"),
            (keys::ACME_DOMAINS, ConfigType::Text, "a.test,b.test"),
            (keys::ACME_EMAIL, ConfigType::Text, "ops@example.test"),
            (keys::ACME_EAB_KID, ConfigType::Text, "kid-1"),
            (keys::ACME_EAB_HMAC, ConfigType::Secret, "aGk="),
            (keys::USPS_BASE_URL, ConfigType::Text, "https://stub.test"),
            (keys::USPS_CERTIFIED, ConfigType::Bool, "1"),
            (
                keys::TELNYX_BASE_URL,
                ConfigType::Text,
                "https://telnyx-stub.test",
            ),
            (keys::TELNYX_FROM_NUMBER, ConfigType::Text, "+15550001111"),
            (keys::TELNYX_CONNECTION_ID, ConfigType::Text, "conn-x"),
            (
                keys::TELNYX_WEBHOOK_PUBLIC_KEY,
                ConfigType::Text,
                "pubkey-x",
            ),
            (keys::TELNYX_API_KEY, ConfigType::Secret, "key-x"),
            (keys::QUIC_ENABLED, ConfigType::Bool, "1"),
            (keys::AUTH_RP_ID, ConfigType::Text, "example.test"),
            (keys::AUTH_RP_NAME, ConfigType::Text, "Example RP"),
            (keys::AUTH_ORIGIN, ConfigType::Text, "https://example.test"),
        ] {
            put(&conn, &kek, key, key_type, plaintext.as_bytes());
        }
        let (cfg, warnings) = RuntimeConfig::load(&conn, &kek).expect("load full db");
        // The deprecated `net.admin_https_port` row (set above) is the
        // ONLY warning; every other key overlays cleanly.
        assert_eq!(warnings.len(), 1, "warnings: {warnings:?}");
        assert!(
            warnings[0].contains(keys::NET_ADMIN_HTTPS_PORT) && warnings[0].contains("deprecated"),
            "warnings: {warnings:?}"
        );
        assert_eq!(cfg.static_dir, "static-x");
        assert_eq!(cfg.spa_dir, "spa-x");
        assert_eq!(cfg.admin_spa_dir, "admin-spa-x");
        assert_eq!(cfg.acme_challenge_dir, "acme-x");
        assert_eq!(
            cfg.bind,
            "127.0.0.2".parse::<std::net::IpAddr>().expect("parse")
        );
        assert_eq!(
            cfg.bind6,
            Some("::1".parse::<std::net::IpAddr>().expect("parse"))
        );
        assert_eq!(cfg.bind_ips().len(), 2);
        assert_eq!(cfg.port, 0);
        assert_eq!(cfg.admin_port, 0);
        assert_eq!(cfg.https_port, 8443);
        assert_eq!(cfg.admin_https_port, 8444);
        assert!(cfg.quic_enabled);
        assert_eq!(cfg.http_port, 8080);
        assert_eq!(cfg.public_host, "example.test");
        assert_eq!(cfg.tls_cert.as_deref(), Some("/x/cert.pem"));
        assert_eq!(cfg.tls_key.as_deref(), Some("/x/key.pem"));
        assert_eq!(cfg.tls_client_ca.as_deref(), Some("/x/ca.pem"));
        assert!(cfg.admin_mtls_off); // debug build allows the dev seam
        assert!(cfg.media_terms_read);
        assert!(cfg.acme_enabled);
        assert_eq!(cfg.acme_directory, "staging");
        assert_eq!(cfg.acme_domains_csv, "a.test,b.test");
        assert_eq!(cfg.acme_email.as_deref(), Some("ops@example.test"));
        assert_eq!(cfg.acme_eab_kid.as_deref(), Some("kid-1"));
        assert_eq!(
            cfg.acme_eab_hmac_b64.as_ref().map(SecretText::as_str),
            Some("aGk=")
        );
        assert_eq!(cfg.usps_base_url, "https://stub.test");
        assert!(cfg.usps_certified);
        assert_eq!(cfg.telnyx_base_url, "https://telnyx-stub.test");
        assert_eq!(cfg.telnyx_from_number.as_deref(), Some("+15550001111"));
        assert_eq!(cfg.telnyx_connection_id.as_deref(), Some("conn-x"));
        assert_eq!(cfg.telnyx_webhook_public_key.as_deref(), Some("pubkey-x"));
        assert_eq!(
            cfg.telnyx_api_key.as_ref().map(SecretText::as_str),
            Some("key-x")
        );
        assert_eq!(cfg.auth_rp_id, "example.test");
        assert_eq!(cfg.auth_rp_name, "Example RP");
        assert_eq!(cfg.auth_origin, "https://example.test");
    }

    #[test]
    fn deprecated_admin_https_port_warns_only_when_set() {
        let conn = memory();
        let kek = Kek::generate();
        let (_, warnings) = RuntimeConfig::load(&conn, &kek).expect("load empty db");
        assert!(warnings.is_empty());
        put(
            &conn,
            &kek,
            keys::NET_ADMIN_HTTPS_PORT,
            ConfigType::Text,
            b"8444",
        );
        let (cfg, warnings) = RuntimeConfig::load(&conn, &kek).expect("load");
        assert_eq!(cfg.admin_https_port, 8444); // still parsed, never bound
        assert_eq!(warnings.len(), 1, "warnings: {warnings:?}");
        assert!(
            warnings[0].contains(keys::NET_ADMIN_HTTPS_PORT)
                && warnings[0].contains("deprecated")
                && warnings[0].contains(keys::NET_HTTPS_PORT),
            "warnings: {warnings:?}"
        );
    }

    #[test]
    fn every_registered_key_roundtrips_through_admin_paths() {
        // The admin PUT path is `set_value`; the GET path is
        // `get_display` (+ `get_secret_for_job` for job reads). All
        // moved keys must survive both with their registered type.
        let conn = memory();
        let kek = Kek::generate();
        for (key, key_type) in ALL_KEYS {
            let plaintext: &[u8] = match key_type {
                ConfigType::Bool => b"1",
                ConfigType::Text => b"sample",
                ConfigType::Secret => b"sample-secret",
            };
            set_value(
                &conn,
                &kek,
                &SetValue {
                    version: 1,
                    key,
                    key_type: *key_type,
                    plaintext,
                    now: 1,
                    actor: "t",
                },
            )
            .expect("admin PUT path");
        }
        for (key, key_type) in ALL_KEYS {
            match key_type {
                ConfigType::Bool => assert_eq!(
                    get_display(&conn, &kek, key).expect("get"),
                    Some(DisplayValue::Bool(true)),
                    "key {key}"
                ),
                ConfigType::Text => assert_eq!(
                    get_display(&conn, &kek, key).expect("get"),
                    Some(DisplayValue::Text("sample".to_string())),
                    "key {key}"
                ),
                ConfigType::Secret => {
                    assert_eq!(
                        get_display(&conn, &kek, key).expect("get"),
                        Some(DisplayValue::Secret { set: true }),
                        "key {key}"
                    );
                    assert_eq!(
                        get_secret_for_job(&conn, &kek, key).expect("job read"),
                        Some(b"sample-secret".to_vec()),
                        "key {key}"
                    );
                }
            }
        }
    }

    #[test]
    fn garbage_warns_and_defaults() {
        let conn = memory();
        let kek = Kek::generate();
        put(&conn, &kek, keys::NET_PORT, ConfigType::Text, b"not-a-port");
        put(&conn, &kek, keys::NET_BIND, ConfigType::Text, b"nope");
        put(&conn, &kek, keys::NET_BIND6, ConfigType::Text, b"also-nope");
        put(&conn, &kek, keys::NET_PUBLIC_HOST, ConfigType::Text, b"");
        put(&conn, &kek, keys::MEDIA_TERMS_READ, ConfigType::Text, b"1"); // wrong type
        put(&conn, &kek, keys::TLS_CERT_FILE, ConfigType::Text, b""); // empty ≡ unset, silent
        let (cfg, warnings) = RuntimeConfig::load(&conn, &kek).expect("load garbage db");
        assert_eq!(cfg.port, 3000);
        assert_eq!(cfg.bind, std::net::IpAddr::from([127, 0, 0, 1]));
        assert_eq!(cfg.bind6, None);
        assert_eq!(cfg.public_host, "andrii.ai");
        assert!(!cfg.media_terms_read);
        assert_eq!(cfg.tls_cert, None);
        assert_eq!(warnings.len(), 5, "warnings: {warnings:?}");
        for key in [
            keys::NET_PORT,
            keys::NET_BIND,
            keys::NET_BIND6,
            keys::NET_PUBLIC_HOST,
            keys::MEDIA_TERMS_READ,
        ] {
            assert!(
                warnings.iter().any(|w| w.contains(key)),
                "warnings: {warnings:?}"
            );
        }
    }

    #[test]
    fn wrong_kek_refuses_the_boot() {
        let conn = memory();
        let kek = Kek::generate();
        let wrong = Kek::generate();
        put(&conn, &kek, keys::NET_PORT, ConfigType::Text, b"3000");
        let err = RuntimeConfig::load(&conn, &wrong).expect_err("wrong KEK must refuse");
        assert!(matches!(err, ConfigError::Crypto), "got: {err}");
    }

    #[test]
    fn debug_redacts_the_hmac() {
        let cfg = RuntimeConfig {
            acme_eab_hmac_b64: Some(SecretText("aGk=".to_string())),
            ..RuntimeConfig::default()
        };
        let shown = format!("{cfg:?}");
        assert!(!shown.contains("aGk"), "{shown}");
        assert!(shown.contains("redacted"), "{shown}");
    }
}
