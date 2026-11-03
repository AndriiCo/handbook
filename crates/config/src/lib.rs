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

/// When each stored key was last written (`config.updated_at`, unix
/// seconds), ordered by key; removed keys are left out like
/// [`list_display`]. The admin page compares these with the process
/// start to flag boot-time keys that still need a restart.
pub fn list_updated_at(conn: &Connection) -> Result<Vec<(String, i64)>> {
    let rows: Vec<(String, i64)> = conn
        .prepare("SELECT key, updated_at FROM config ORDER BY key")?
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows
        .into_iter()
        .filter(|(key, _)| !keys::REMOVED.contains(&key.as_str()))
        .collect())
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
        if keys::REMOVED.contains(&key.as_str()) {
            continue;
        }
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

/// Delete one config row (the setup-key burn). Returns false when
/// no row existed. The caller writes the audit row.
pub fn delete_value(conn: &Connection, key: &str) -> Result<bool> {
    let removed = conn.execute("DELETE FROM config WHERE key = ?1", [key])?;
    Ok(removed > 0)
}

/// Verify a setup-key candidate against the sealed
/// `setup.key_hash` row (secure-config.md §9). The caller hashes
/// the candidate (SHA-256 hex) — this function only compares, so
/// secret bytes never cross into the web layer. False when no row
/// exists (window closed) or the row is not a secret.
/// Setup keys older than this stop verifying (M2): a minted
/// window must not linger in scrollback forever. Re-mint is the
/// recovery; no schema change (the row's `updated_at` carries it).
pub const SETUP_KEY_MAX_AGE_SECS: i64 = 72 * 3600;

pub fn verify_setup_key(
    conn: &Connection,
    kek: &Kek,
    candidate_hash_hex: &str,
    now: i64,
) -> Result<bool> {
    let row: Option<(String, Vec<u8>, i64, i64)> = conn
        .query_row(
            "SELECT type, value_enc, enc_version, updated_at FROM config WHERE key = ?1",
            [keys::SETUP_KEY_HASH],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .optional()?;
    let Some((type_text, value_enc, enc_version, updated_at)) = row else {
        return Ok(false);
    };
    if type_text != ConfigType::Secret.as_str() {
        return Ok(false);
    }
    if now.saturating_sub(updated_at) > SETUP_KEY_MAX_AGE_SECS {
        return Ok(false);
    }
    let stored = decrypt_value(kek, keys::SETUP_KEY_HASH, enc_version, &value_enc)?;
    Ok(ct_eq(&stored, candidate_hash_hex.as_bytes()))
}

/// Branchless fixed-shape compare (length + XOR accumulation):
/// no early exit on first mismatch.
fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter()
        .zip(b.iter())
        .fold(0u8, |acc, (x, y)| acc | (x ^ y))
        == 0
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
    let text = if is_path {
        std::fs::read_to_string(&raw)
            .map_err(|e| ConfigError::KekUnavailable(format!("{var}={raw}: {e}")))?
    } else {
        raw
    };
    let bytes =
        parse_dev_kek(&text).map_err(|why| ConfigError::KekUnavailable(format!("{var}: {why}")))?;
    let kek = Kek::from_bytes(bytes);
    if !cfg!(debug_assertions) && !dev_kek_loopback_holds(&kek) {
        return Err(ConfigError::KekUnavailable(format!(
            "{var}: dev KEK refused in a release build unless every bind IP is loopback"
        )));
    }
    if cfg!(debug_assertions) {
        eprintln!("DEV CRYPTO — {var}-backed dev KEK, NOT TPM-sealed, synthetic data only");
    } else {
        eprintln!(
            "DEV CRYPTO — {var}-backed dev KEK in a release build, allowed only because every bind IP is loopback; synthetic data only"
        );
    }
    Ok(Some((kek, source)))
}

/// Release-mode dev-KEK gate: a dev KEK may boot a release binary ONLY
/// when every sealed bind IP is loopback. Opens the store read-only via
/// `STORE_DIR` (default `store`) and decrypts just the bind rows with
/// the candidate key. Unreadable state refuses (false): a missing DB,
/// missing tables, or undecryptable rows never read as loopback.
/// Missing bind rows count as loopback — the loader defaults an unset
/// bind to 127.0.0.1 and an unset bind6 to nothing, so that is what
/// the server would actually bind.
fn dev_kek_loopback_holds(kek: &Kek) -> bool {
    let dir = std::env::var("STORE_DIR").unwrap_or_else(|_| "store".to_string());
    let db = std::path::Path::new(&dir).join("handbook.sqlite");
    let conn = match Connection::open_with_flags(&db, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY) {
        Ok(conn) => conn,
        Err(_) => return false,
    };
    binds_loopback_in(&conn, kek)
}

/// True when the sealed binds decrypt to an all-loopback listener set
/// (pure over the connection, so both directions unit-test without
/// env or filesystem state).
fn binds_loopback_in(conn: &Connection, kek: &Kek) -> bool {
    let bind_text = match get_text_for_job(conn, kek, keys::NET_BIND) {
        Ok(None) => None,
        Ok(Some(v)) if v.trim().is_empty() => None,
        Ok(Some(v)) => Some(v),
        // Wrong type, bad ciphertext, missing table: refuse.
        Err(_) => return false,
    };
    let bind6_text = match get_text_for_job(conn, kek, keys::NET_BIND6) {
        Ok(None) => None,
        Ok(Some(v)) if v.trim().is_empty() => None,
        Ok(Some(v)) => Some(v),
        Err(_) => return false,
    };
    let bind: std::net::IpAddr = match bind_text {
        None => std::net::IpAddr::from([127, 0, 0, 1]),
        Some(text) => match text.trim().parse() {
            Ok(ip) => ip,
            Err(_) => return false,
        },
    };
    let bind6: Option<std::net::IpAddr> = match bind6_text {
        None => None,
        Some(text) => match text.trim().parse() {
            Ok(ip) => Some(ip),
            Err(_) => return false,
        },
    };
    RuntimeConfig::dev_seam_allowed(true, bind, bind6)
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
    fn setup_key_verify_and_burn() {
        let conn = memory();
        let kek = Kek::generate();
        // No row: window closed.
        assert!(!verify_setup_key(&conn, &kek, &"a".repeat(64), 100).expect("verify"));
        assert!(!delete_value(&conn, keys::SETUP_KEY_HASH).expect("delete"));
        let hash = "b".repeat(64);
        put(
            &conn,
            &kek,
            keys::SETUP_KEY_HASH,
            ConfigType::Secret,
            hash.as_bytes(),
        )
        .expect("set");
        assert!(verify_setup_key(&conn, &kek, &hash, 100).expect("verify"));
        assert!(!verify_setup_key(&conn, &kek, &"c".repeat(64), 100).expect("verify"));
        assert!(!verify_setup_key(&conn, &kek, "short", 100).expect("verify"));
        // Burn removes the row; a second burn is a no-op false.
        assert!(delete_value(&conn, keys::SETUP_KEY_HASH).expect("delete"));
        assert!(!delete_value(&conn, keys::SETUP_KEY_HASH).expect("delete"));
        assert!(!verify_setup_key(&conn, &kek, &hash, 100).expect("verify"));
    }

    #[test]
    fn setup_key_verify_expires_after_72h() {
        let conn = memory();
        let kek = Kek::generate();
        let hash = "b".repeat(64);
        put(
            &conn,
            &kek,
            keys::SETUP_KEY_HASH,
            ConfigType::Secret,
            hash.as_bytes(),
        )
        .expect("set");
        // Test rows stamp updated_at = 100.
        assert!(verify_setup_key(&conn, &kek, &hash, 100).expect("verify"));
        assert!(
            verify_setup_key(&conn, &kek, &hash, 100 + SETUP_KEY_MAX_AGE_SECS).expect("verify")
        );
        assert!(
            !verify_setup_key(&conn, &kek, &hash, 100 + SETUP_KEY_MAX_AGE_SECS + 1)
                .expect("verify")
        );
    }

    #[test]
    fn setup_key_verify_refuses_non_secret_row() {
        let conn = memory();
        let kek = Kek::generate();
        put(
            &conn,
            &kek,
            keys::SETUP_KEY_HASH,
            ConfigType::Text,
            b"not-a-hash-row",
        )
        .expect("set");
        assert!(!verify_setup_key(&conn, &kek, "not-a-hash-row", 100).expect("verify"));
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
    pub const NET_HTTP_PORT: &str = "net.http_port";
    pub const NET_PUBLIC_HOST: &str = "net.public_host";
    pub const TLS_CERT_FILE: &str = "tls.cert_file";
    pub const TLS_KEY_FILE: &str = "tls.key_file";
    pub const ADMIN_MTLS_OFF: &str = "admin.mtls_off";
    pub const ADMIN_ALLOW_IPS: &str = "admin.allow_ips";
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
    /// Live FedEx buys (SA-19): master switch. Default false —
    /// sandbox fixtures only until staff opts in.
    pub const LABELS_LIVE: &str = "labels.live";
    /// Max live buys per UTC day. Default 2.
    pub const LABELS_LIVE_DAILY_CAP: &str = "labels.live_daily_cap";
    /// Max USD per label. Default 60.
    pub const LABELS_LIVE_MAX_USD: &str = "labels.live_max_usd";
    /// Comma-separated allowlisted destination ZIPs. Default empty
    /// (nothing buyable until staff lists ZIPs).
    pub const LABELS_LIVE_SHIP_TO_ALLOW: &str = "labels.live_ship_to_allow";
    /// FedEx OAuth client id. Secret: presence-only in UI.
    pub const LABELS_FEDEX_CLIENT_ID: &str = "labels.fedex_client_id";
    /// FedEx OAuth client secret. Secret: presence-only in UI.
    pub const LABELS_FEDEX_CLIENT_SECRET: &str = "labels.fedex_client_secret";
    /// FedEx account number (shipper account). Default empty.
    pub const LABELS_FEDEX_ACCOUNT: &str = "labels.fedex_account";
    /// Native QUIC/HTTP-3 listener (single-443 spec §2): bind UDP
    /// on `net.https_port`, serve h3, send Alt-Svc. Default false.
    pub const QUIC_ENABLED: &str = "quic.enabled";
    pub const AUTH_RP_ID: &str = "auth.rp_id";
    pub const AUTH_RP_NAME: &str = "auth.rp_name";
    pub const AUTH_ORIGIN: &str = "auth.origin";
    pub const GOOGLE_CLIENT_ID: &str = "google.client_id";
    pub const GOOGLE_CLIENT_SECRET: &str = "google.client_secret";
    pub const APPLE_CLIENT_ID: &str = "apple.client_id";
    pub const APPLE_TEAM_ID: &str = "apple.team_id";
    pub const APPLE_KEY_ID: &str = "apple.key_id";
    pub const APPLE_PRIVATE_KEY: &str = "apple.private_key";
    pub const META_CLIENT_ID: &str = "meta.client_id";
    pub const META_CLIENT_SECRET: &str = "meta.client_secret";
    /// Model API root (`docs/meta-api-shapes.md`). Default prod.
    pub const META_BASE_URL: &str = "meta.base_url";
    /// Model API key (`LLM_{id}_{secret}`). Secret: presence-only in UI.
    pub const META_API_KEY: &str = "meta.api_key";
    /// Gmail inbound poll (`xtask mail-poll`): OAuth client, refresh
    /// token, and overridable API roots. Distinct from `mailer.*`
    /// (SMTP outbound).
    pub const MAIL_CLIENT_ID: &str = "mail.client_id";
    pub const MAIL_CLIENT_SECRET: &str = "mail.client_secret";
    pub const MAIL_REFRESH_TOKEN: &str = "mail.refresh_token";
    pub const MAIL_BASE_URL: &str = "mail.base_url";
    pub const MAIL_TOKEN_URL: &str = "mail.token_url";
    pub const MAILER_PROVIDER: &str = "mailer.provider";
    pub const MAILER_HOST: &str = "mailer.host";
    pub const MAILER_PORT: &str = "mailer.port";
    pub const MAILER_USERNAME: &str = "mailer.username";
    pub const MAILER_PASSWORD: &str = "mailer.password";
    pub const MAILER_FROM: &str = "mailer.from";
    pub const MAILER_DIR: &str = "mailer.dir";
    /// Follower notifications (phase 1 email): master switch, off
    /// unless the operator enables it after setting a postal address.
    pub const NOTIFY_ENABLED: &str = "notify.enabled";
    /// Max digest emails per subscriber per day (parsed as u64,
    /// defaulted by the worker when missing or malformed).
    pub const NOTIFY_DAILY_CAP: &str = "notify.daily_cap";
    /// CAN-SPAM postal address footer (operator-supplied, required
    /// before any digest sends; confirm mails carry it too).
    pub const NOTIFY_POSTAL_ADDRESS: &str = "notify.postal_address";
    /// Setup-wizard gate (secure-config.md §9): SHA-256 hex of the
    /// CLI-minted setup key. Writers are mint/burn only — admin
    /// PUT and setup PUT both refuse `setup.*`.
    pub const SETUP_KEY_HASH: &str = "setup.key_hash";
    // ---- MISSION WIN (P315 B0): commerce, carriers, Meta pipeline ----
    /// Stripe secret API key (test or live). Secret: presence-only in UI.
    pub const STRIPE_SECRET_KEY: &str = "stripe.secret_key";
    /// Stripe webhook signing secret (`whsec_…`). Secret.
    pub const STRIPE_WEBHOOK_SECRET: &str = "stripe.webhook_secret";
    /// Customer orders master switch. Default false (paid tiers 503).
    pub const ORDERS_ENABLED: &str = "orders.enabled";
    /// Service fee per paid label order, in cents. Default 0.
    pub const ORDERS_FEE_LABEL_CENTS: &str = "orders.fee_label_cents";
    /// Mailing-kit price (kit + both prepaid legs), in cents.
    pub const ORDERS_KIT_PRICE_CENTS: &str = "orders.kit_price_cents";
    /// ISO 4217 lowercase currency for Checkout. Default `usd`.
    pub const ORDERS_CURRENCY: &str = "orders.currency";
    /// Ship-from (kit fulfilment) address as one JSON object.
    pub const ORDERS_SHIP_FROM: &str = "orders.ship_from";
    /// Soft cap on paid orders per UTC day (default 20): once this many
    /// payments are captured today in the current mode, checkout opens no
    /// new session; sessions already open may still complete.
    pub const ORDERS_DAILY_CAP: &str = "orders.daily_cap";
    /// Max total per order, in cents. Default 15000.
    pub const ORDERS_MAX_ORDER_CENTS: &str = "orders.max_order_cents";
    /// FedEx API root: `https://apis-sandbox.fedex.com` or the production
    /// root; anything else (or unset) reads as production.
    pub const LABELS_FEDEX_BASE_URL: &str = "labels.fedex_base_url";
    /// Customer-paid FedEx buys switch (separate from staff `labels.live`).
    pub const LABELS_CUSTOMER_LIVE: &str = "labels.customer_live";
    /// Max customer label buys per UTC day. Default 10.
    pub const LABELS_CUSTOMER_DAILY_CAP: &str = "labels.customer_daily_cap";
    /// Max USD per customer label. Default 60.
    pub const LABELS_CUSTOMER_MAX_USD: &str = "labels.customer_max_usd";
    /// USPS OAuth client id / secret (Labels, Payments, Tracking v3).
    pub const USPS_CLIENT_ID: &str = "usps.client_id";
    pub const USPS_CLIENT_SECRET: &str = "usps.client_secret";
    /// USPS customer registration id + mailer ids (payment auth).
    pub const USPS_CRID: &str = "usps.crid";
    pub const USPS_MID: &str = "usps.mid";
    pub const USPS_MANIFEST_MID: &str = "usps.manifest_mid";
    /// USPS EPA account number. Secret: presence-only in UI.
    pub const USPS_EPA_ACCOUNT: &str = "usps.epa_account";
    /// Live USPS postage switch. Default false (TEM only).
    pub const USPS_LIVE: &str = "usps.live";
    /// Max live USPS labels per UTC day. Default 10.
    pub const USPS_LIVE_DAILY_CAP: &str = "usps.live_daily_cap";
    /// Max USD per live USPS label. Default 40.
    pub const USPS_LIVE_MAX_USD: &str = "usps.live_max_usd";
    /// Compose a page plan after each accept batch. Default false
    /// (staff press Compose; batch approvals never churn plans).
    pub const PLAN_AUTO_COMPOSE: &str = "plan.auto_compose";
    /// Max Compile runs per UTC day. Default 6.
    pub const COMPILE_DAILY_RUNS: &str = "compile.daily_runs";
    /// Carrier tracking poller switch. Default false.
    pub const TRACKING_POLL_ENABLED: &str = "tracking.poll_enabled";
    /// Max Meta model calls per UTC day, counted from `model_call`
    /// rows (every pipeline). Default 400.
    pub const META_DAILY_CALL_CAP: &str = "meta.daily_call_cap";

    /// Keys removed from the registry (admin client certificates and the
    /// split-port admin listener are gone). A stored row is ignored: the
    /// boot prints one warning and the admin list leaves it out.
    pub const REMOVED: &[&str] = &[
        "net.admin_https_port",
        "tls.client_ca_file",
        "admin.require_cert",
    ];

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
        (NET_HTTP_PORT, super::ConfigType::Text),
        (NET_PUBLIC_HOST, super::ConfigType::Text),
        (TLS_CERT_FILE, super::ConfigType::Text),
        (TLS_KEY_FILE, super::ConfigType::Text),
        (ADMIN_MTLS_OFF, super::ConfigType::Bool),
        (ADMIN_ALLOW_IPS, super::ConfigType::Text),
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
        (LABELS_LIVE, super::ConfigType::Bool),
        (LABELS_LIVE_DAILY_CAP, super::ConfigType::Text),
        (LABELS_LIVE_MAX_USD, super::ConfigType::Text),
        (LABELS_LIVE_SHIP_TO_ALLOW, super::ConfigType::Text),
        (LABELS_FEDEX_CLIENT_ID, super::ConfigType::Secret),
        (LABELS_FEDEX_CLIENT_SECRET, super::ConfigType::Secret),
        (LABELS_FEDEX_ACCOUNT, super::ConfigType::Text),
        (QUIC_ENABLED, super::ConfigType::Bool),
        (AUTH_RP_ID, super::ConfigType::Text),
        (AUTH_RP_NAME, super::ConfigType::Text),
        (AUTH_ORIGIN, super::ConfigType::Text),
        (GOOGLE_CLIENT_ID, super::ConfigType::Text),
        (GOOGLE_CLIENT_SECRET, super::ConfigType::Secret),
        (APPLE_CLIENT_ID, super::ConfigType::Text),
        (APPLE_TEAM_ID, super::ConfigType::Text),
        (APPLE_KEY_ID, super::ConfigType::Text),
        (APPLE_PRIVATE_KEY, super::ConfigType::Secret),
        (META_CLIENT_ID, super::ConfigType::Text),
        (META_CLIENT_SECRET, super::ConfigType::Secret),
        (META_BASE_URL, super::ConfigType::Text),
        (META_API_KEY, super::ConfigType::Secret),
        (MAIL_CLIENT_ID, super::ConfigType::Text),
        (MAIL_CLIENT_SECRET, super::ConfigType::Secret),
        (MAIL_REFRESH_TOKEN, super::ConfigType::Secret),
        (MAIL_BASE_URL, super::ConfigType::Text),
        (MAIL_TOKEN_URL, super::ConfigType::Text),
        (MAILER_PROVIDER, super::ConfigType::Text),
        (MAILER_HOST, super::ConfigType::Text),
        (MAILER_PORT, super::ConfigType::Text),
        (MAILER_USERNAME, super::ConfigType::Text),
        (MAILER_PASSWORD, super::ConfigType::Secret),
        (MAILER_FROM, super::ConfigType::Text),
        (MAILER_DIR, super::ConfigType::Text),
        (NOTIFY_ENABLED, super::ConfigType::Bool),
        (NOTIFY_DAILY_CAP, super::ConfigType::Text),
        (NOTIFY_POSTAL_ADDRESS, super::ConfigType::Text),
        (SETUP_KEY_HASH, super::ConfigType::Secret),
        (STRIPE_SECRET_KEY, super::ConfigType::Secret),
        (STRIPE_WEBHOOK_SECRET, super::ConfigType::Secret),
        (ORDERS_ENABLED, super::ConfigType::Bool),
        (ORDERS_FEE_LABEL_CENTS, super::ConfigType::Text),
        (ORDERS_KIT_PRICE_CENTS, super::ConfigType::Text),
        (ORDERS_CURRENCY, super::ConfigType::Text),
        (ORDERS_SHIP_FROM, super::ConfigType::Text),
        (ORDERS_DAILY_CAP, super::ConfigType::Text),
        (ORDERS_MAX_ORDER_CENTS, super::ConfigType::Text),
        (LABELS_FEDEX_BASE_URL, super::ConfigType::Text),
        (LABELS_CUSTOMER_LIVE, super::ConfigType::Bool),
        (LABELS_CUSTOMER_DAILY_CAP, super::ConfigType::Text),
        (LABELS_CUSTOMER_MAX_USD, super::ConfigType::Text),
        (USPS_CLIENT_ID, super::ConfigType::Secret),
        (USPS_CLIENT_SECRET, super::ConfigType::Secret),
        (USPS_CRID, super::ConfigType::Text),
        (USPS_MID, super::ConfigType::Text),
        (USPS_MANIFEST_MID, super::ConfigType::Text),
        (USPS_EPA_ACCOUNT, super::ConfigType::Secret),
        (USPS_LIVE, super::ConfigType::Bool),
        (USPS_LIVE_DAILY_CAP, super::ConfigType::Text),
        (USPS_LIVE_MAX_USD, super::ConfigType::Text),
        (PLAN_AUTO_COMPOSE, super::ConfigType::Bool),
        (COMPILE_DAILY_RUNS, super::ConfigType::Text),
        (TRACKING_POLL_ENABLED, super::ConfigType::Bool),
        (META_DAILY_CALL_CAP, super::ConfigType::Text),
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

/// Read-through cache for runtime switches and caps (`orders.enabled`,
/// `usps.live`, daily caps): handlers read per request without a
/// decrypt each time, and an operator change in admin `/config` lands
/// within one TTL (or at once — the PUT calls [`CachedReader::invalidate`]),
/// so installing keys never needs a restart. Secrets are never cached.
#[derive(Debug)]
pub struct CachedReader {
    ttl: std::time::Duration,
    rows: std::sync::Mutex<std::collections::HashMap<String, (std::time::Instant, CachedValue)>>,
}

#[derive(Debug, Clone)]
enum CachedValue {
    Text(Option<String>),
    Bool(Option<bool>),
}

impl Default for CachedReader {
    fn default() -> Self {
        CachedReader::new(std::time::Duration::from_secs(30))
    }
}

impl CachedReader {
    /// Empty cache with the given TTL (30 s in production).
    pub fn new(ttl: std::time::Duration) -> Self {
        CachedReader {
            ttl,
            rows: std::sync::Mutex::new(std::collections::HashMap::new()),
        }
    }

    /// Cached [`get_text_for_job`]. Errors are never cached.
    pub fn text(&self, conn: &Connection, kek: &Kek, key: &str) -> Result<Option<String>> {
        if let Some(CachedValue::Text(v)) = self.fresh(key) {
            return Ok(v);
        }
        let v = get_text_for_job(conn, kek, key)?;
        self.put(key, CachedValue::Text(v.clone()));
        Ok(v)
    }

    /// Cached [`get_bool_for_job`]. Errors are never cached.
    pub fn bool(&self, conn: &Connection, kek: &Kek, key: &str) -> Result<Option<bool>> {
        if let Some(CachedValue::Bool(v)) = self.fresh(key) {
            return Ok(v);
        }
        let v = get_bool_for_job(conn, kek, key)?;
        self.put(key, CachedValue::Bool(v));
        Ok(v)
    }

    /// Drop every cached row (called after any config write).
    pub fn invalidate(&self) {
        self.rows.lock().expect("config cache poisoned").clear();
    }

    fn fresh(&self, key: &str) -> Option<CachedValue> {
        let rows = self.rows.lock().expect("config cache poisoned");
        rows.get(key)
            .filter(|(at, _)| at.elapsed() < self.ttl)
            .map(|(_, v)| v.clone())
    }

    fn put(&self, key: &str, value: CachedValue) {
        self.rows
            .lock()
            .expect("config cache poisoned")
            .insert(key.to_string(), (std::time::Instant::now(), value));
    }
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

impl From<String> for SecretText {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl std::fmt::Debug for SecretText {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SecretText(<redacted>)")
    }
}

/// Every runtime setting, read once at boot from sealed DB rows.
///
/// One parsed `admin.allow_ips` entry: a base address plus prefix
/// length. Bare IPs parse as host routes (/32, /128). No new
/// dependency: the matcher is a few integer ops below.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IpNet {
    addr: std::net::IpAddr,
    prefix: u8,
}

impl IpNet {
    /// True when `ip` falls inside this network. Both sides are
    /// canonicalized first, so a v4-mapped peer (`::ffff:203.0.113.7`)
    /// matches a v4 range. Prefix 0 is match-all for its family
    /// (shifting a u32 by 32 panics under overflow-checks, so the
    /// zero case returns early instead of shifting).
    pub fn contains(&self, ip: &std::net::IpAddr) -> bool {
        match (self.addr.to_canonical(), ip.to_canonical()) {
            (std::net::IpAddr::V4(net), std::net::IpAddr::V4(addr)) => {
                if self.prefix == 0 {
                    return true;
                }
                let shift = 32 - self.prefix.min(32);
                (u32::from(net) >> shift) == (u32::from(addr) >> shift)
            }
            (std::net::IpAddr::V6(net), std::net::IpAddr::V6(addr)) => {
                if self.prefix == 0 {
                    return true;
                }
                let shift = 128 - self.prefix.min(128);
                (u128::from(net) >> shift) == (u128::from(addr) >> shift)
            }
            _ => false,
        }
    }
}

impl std::fmt::Display for IpNet {
    /// Canonical `addr/prefix` form for log lines (the boot line
    /// and the admin 403 text both render through here).
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}", self.addr, self.prefix)
    }
}

/// Loopback-only allowlist: the fail-closed default (first boot can
/// still reach setup over localhost).
pub fn loopback_allow_ips() -> Vec<IpNet> {
    vec![
        IpNet {
            addr: std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST),
            prefix: 8,
        },
        IpNet {
            addr: std::net::IpAddr::V6(std::net::Ipv6Addr::LOCALHOST),
            prefix: 128,
        },
    ]
}

/// Parse a comma-separated `admin.allow_ips` value: `203.0.113.0/24`,
/// `2001:db8::/32`, or a bare IP (host route). Empty overall input
/// is an error (unset means loopback, never "allow all").
pub fn parse_allow_ips(csv: &str) -> std::result::Result<Vec<IpNet>, String> {
    let mut out = Vec::new();
    for part in csv.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let (addr_text, prefix) = match part.split_once('/') {
            Some((ip, len)) => {
                let max = if ip.contains(':') { 128 } else { 32 };
                let bits: u8 = len
                    .trim()
                    .parse()
                    .map_err(|_| format!("admin.allow_ips: bad prefix in {part:?}"))?;
                if bits > max {
                    return Err(format!("admin.allow_ips: prefix out of range in {part:?}"));
                }
                (ip.trim(), bits)
            }
            None => {
                let max = if part.contains(':') { 128 } else { 32 };
                (part, max)
            }
        };
        let addr: std::net::IpAddr = addr_text
            .parse()
            .map_err(|_| format!("admin.allow_ips: bad address in {part:?}"))?;
        out.push(IpNet { addr, prefix });
    }
    if out.is_empty() {
        return Err("admin.allow_ips: no usable entries".to_string());
    }
    Ok(out)
}

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
    pub quic_enabled: bool,
    pub http_port: u16,
    pub public_host: String,
    pub tls_cert: Option<String>,
    pub tls_key: Option<String>,
    pub admin_mtls_off: bool,
    pub admin_allow_ips: Vec<IpNet>,
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
    pub labels_live: bool,
    pub labels_live_daily_cap: u32,
    /// Max USD per label, in cents.
    pub labels_live_max_usd_cents: i64,
    /// Allowlisted destination ZIPs (CSV parsed at load).
    pub labels_live_ship_to_allow: Vec<String>,
    pub labels_fedex_client_id: Option<SecretText>,
    pub labels_fedex_client_secret: Option<SecretText>,
    pub labels_fedex_account: Option<String>,
    pub auth_rp_id: String,
    pub auth_rp_name: String,
    pub auth_origin: String,
    pub google_client_id: Option<String>,
    pub google_client_secret: Option<SecretText>,
    pub apple_client_id: Option<String>,
    pub apple_team_id: Option<String>,
    pub apple_key_id: Option<String>,
    pub apple_private_key: Option<SecretText>,
    pub meta_client_id: Option<String>,
    pub meta_client_secret: Option<SecretText>,
    pub meta_base_url: String,
    pub meta_api_key: Option<SecretText>,
    pub mail_client_id: Option<String>,
    pub mail_client_secret: Option<SecretText>,
    pub mail_refresh_token: Option<SecretText>,
    pub mail_base_url: String,
    pub mail_token_url: String,
    pub mailer_provider: String,
    pub mailer_host: Option<String>,
    pub mailer_port: Option<String>,
    pub mailer_username: Option<String>,
    pub mailer_password: Option<SecretText>,
    pub mailer_from: Option<String>,
    pub mailer_dir: String,
    /// Follower notifications master switch (fail-closed: off until
    /// the operator sets a postal address and enables it).
    pub notify_enabled: bool,
    /// Max digest emails per subscriber per day.
    pub notify_daily_cap: u64,
    /// CAN-SPAM postal footer (operator-supplied; digests refuse to
    /// send while empty).
    pub notify_postal_address: String,
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
            quic_enabled: false,
            http_port: 80,
            public_host: "andrii.ai".to_string(),
            tls_cert: None,
            tls_key: None,
            admin_mtls_off: false,
            admin_allow_ips: loopback_allow_ips(),
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
            labels_live: false,
            labels_live_daily_cap: 2,
            labels_live_max_usd_cents: 6_000,
            labels_live_ship_to_allow: Vec::new(),
            labels_fedex_client_id: None,
            labels_fedex_client_secret: None,
            labels_fedex_account: None,
            auth_rp_id: "andrii.ai".to_string(),
            auth_rp_name: "Apostille Handbook".to_string(),
            auth_origin: "https://andrii.ai".to_string(),
            google_client_id: None,
            google_client_secret: None,
            apple_client_id: None,
            apple_team_id: None,
            apple_key_id: None,
            apple_private_key: None,
            meta_client_id: None,
            meta_client_secret: None,
            // Model API prod root (`docs/meta-api-shapes.md`); keep in
            // sync with `handbook_agent::model::META_BASE_URL`.
            meta_base_url: "https://api.meta.ai/v1".to_string(),
            meta_api_key: None,
            mail_client_id: None,
            mail_client_secret: None,
            mail_refresh_token: None,
            // Gmail/token roots (`docs/gmail-shapes.md`); keep in sync
            // with `mail::client::{GMAIL_BASE_URL, TOKEN_URL}`.
            mail_base_url: "https://gmail.googleapis.com".to_string(),
            mail_token_url: "https://oauth2.googleapis.com/token".to_string(),
            // Unset until configured (the OAuth unset→503 contract):
            // dev sets `file`, prod sets `smtp` — never a silent prod
            // default that files real users' links instead of mailing.
            mailer_provider: String::new(),
            mailer_host: None,
            mailer_port: None,
            mailer_username: None,
            mailer_password: None,
            mailer_from: None,
            mailer_dir: "mail".to_string(),
            notify_enabled: false,
            notify_daily_cap: 3,
            notify_postal_address: String::new(),
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
        overlay_required_text(
            conn,
            kek,
            keys::META_BASE_URL,
            &mut warnings,
            &mut cfg.meta_base_url,
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
        overlay_optional_text(
            conn,
            kek,
            keys::GOOGLE_CLIENT_ID,
            &mut warnings,
            &mut cfg.google_client_id,
        )?;
        overlay_optional_text(
            conn,
            kek,
            keys::APPLE_CLIENT_ID,
            &mut warnings,
            &mut cfg.apple_client_id,
        )?;
        overlay_optional_text(
            conn,
            kek,
            keys::APPLE_TEAM_ID,
            &mut warnings,
            &mut cfg.apple_team_id,
        )?;
        overlay_optional_text(
            conn,
            kek,
            keys::APPLE_KEY_ID,
            &mut warnings,
            &mut cfg.apple_key_id,
        )?;
        overlay_optional_text(
            conn,
            kek,
            keys::META_CLIENT_ID,
            &mut warnings,
            &mut cfg.meta_client_id,
        )?;
        overlay_optional_text(
            conn,
            kek,
            keys::MAIL_CLIENT_ID,
            &mut warnings,
            &mut cfg.mail_client_id,
        )?;
        overlay_required_text(
            conn,
            kek,
            keys::MAIL_BASE_URL,
            &mut warnings,
            &mut cfg.mail_base_url,
        )?;
        overlay_required_text(
            conn,
            kek,
            keys::MAIL_TOKEN_URL,
            &mut warnings,
            &mut cfg.mail_token_url,
        )?;
        overlay_required_text(
            conn,
            kek,
            keys::MAILER_PROVIDER,
            &mut warnings,
            &mut cfg.mailer_provider,
        )?;
        overlay_optional_text(
            conn,
            kek,
            keys::MAILER_HOST,
            &mut warnings,
            &mut cfg.mailer_host,
        )?;
        overlay_optional_text(
            conn,
            kek,
            keys::MAILER_PORT,
            &mut warnings,
            &mut cfg.mailer_port,
        )?;
        overlay_optional_text(
            conn,
            kek,
            keys::MAILER_USERNAME,
            &mut warnings,
            &mut cfg.mailer_username,
        )?;
        overlay_optional_text(
            conn,
            kek,
            keys::MAILER_FROM,
            &mut warnings,
            &mut cfg.mailer_from,
        )?;
        overlay_required_text(
            conn,
            kek,
            keys::MAILER_DIR,
            &mut warnings,
            &mut cfg.mailer_dir,
        )?;
        overlay_bool(
            conn,
            kek,
            keys::ADMIN_MTLS_OFF,
            &mut warnings,
            &mut cfg.admin_mtls_off,
        )?;
        // Fail closed: an unparsable allowlist locks the admin
        // listener to loopback (with a loud warning), never to the
        // previous value and never to the open internet.
        match get_text_for_job(conn, kek, keys::ADMIN_ALLOW_IPS) {
            Ok(None) => {}
            Ok(Some(v)) if v.trim().is_empty() => {
                warnings.push(format!(
                    "{}: empty value; loopback only",
                    keys::ADMIN_ALLOW_IPS
                ));
            }
            Ok(Some(v)) => match parse_allow_ips(&v) {
                Ok(nets) => cfg.admin_allow_ips = nets,
                Err(e) => {
                    warnings.push(format!("{e}; loopback only"));
                    cfg.admin_allow_ips = loopback_allow_ips();
                }
            },
            Err(ConfigError::BadValue(why)) => {
                warnings.push(format!("{}: {why}; loopback only", keys::ADMIN_ALLOW_IPS));
                cfg.admin_allow_ips = loopback_allow_ips();
            }
            Err(e) => return Err(e),
        }
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
        // Removed keys: a stored row is read by nothing. Warn once so
        // the operator can see it is inert; unset stays silent.
        for key in keys::REMOVED {
            if get_row(conn, key)?.is_some() {
                warnings.push(format!("{key} was removed and its stored value is IGNORED"));
            }
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
        // Live FedEx buys (SA-19): off by default; caps parse
        // leniently (garbage warns + keeps defaults, mirroring the
        // notify-cap overlay).
        overlay_bool(
            conn,
            kek,
            keys::LABELS_LIVE,
            &mut warnings,
            &mut cfg.labels_live,
        )?;
        match get_text_for_job(conn, kek, keys::LABELS_LIVE_DAILY_CAP) {
            Ok(None) => {}
            Ok(Some(v)) => match v.trim().parse::<u32>() {
                Ok(n) if (1..=10).contains(&n) => cfg.labels_live_daily_cap = n,
                _ => warnings.push(format!(
                    "{}: {v:?} is not 1-10; using default 2",
                    keys::LABELS_LIVE_DAILY_CAP
                )),
            },
            Err(ConfigError::BadValue(why)) => warnings.push(format!(
                "{}: {why}; using default 2",
                keys::LABELS_LIVE_DAILY_CAP
            )),
            Err(e) => return Err(e),
        }
        match get_text_for_job(conn, kek, keys::LABELS_LIVE_MAX_USD) {
            Ok(None) => {}
            Ok(Some(v)) => match v.trim().parse::<f64>() {
                Ok(usd) if (1.0..=10_000.0).contains(&usd) => {
                    cfg.labels_live_max_usd_cents = (usd * 100.0).round() as i64;
                }
                _ => warnings.push(format!(
                    "{}: {v:?} is not 1-10000 USD; using default 60",
                    keys::LABELS_LIVE_MAX_USD
                )),
            },
            Err(ConfigError::BadValue(why)) => warnings.push(format!(
                "{}: {why}; using default 60",
                keys::LABELS_LIVE_MAX_USD
            )),
            Err(e) => return Err(e),
        }
        match get_text_for_job(conn, kek, keys::LABELS_LIVE_SHIP_TO_ALLOW) {
            Ok(None) => {}
            Ok(Some(v)) => {
                cfg.labels_live_ship_to_allow = v
                    .split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
                    .collect();
            }
            Err(ConfigError::BadValue(why)) => warnings.push(format!(
                "{}: {why}; using empty allowlist",
                keys::LABELS_LIVE_SHIP_TO_ALLOW
            )),
            Err(e) => return Err(e),
        }
        overlay_secret(
            conn,
            kek,
            keys::LABELS_FEDEX_CLIENT_ID,
            &mut warnings,
            &mut cfg.labels_fedex_client_id,
        )?;
        overlay_secret(
            conn,
            kek,
            keys::LABELS_FEDEX_CLIENT_SECRET,
            &mut warnings,
            &mut cfg.labels_fedex_client_secret,
        )?;
        overlay_optional_text(
            conn,
            kek,
            keys::LABELS_FEDEX_ACCOUNT,
            &mut warnings,
            &mut cfg.labels_fedex_account,
        )?;
        overlay_secret(
            conn,
            kek,
            keys::GOOGLE_CLIENT_SECRET,
            &mut warnings,
            &mut cfg.google_client_secret,
        )?;
        overlay_secret(
            conn,
            kek,
            keys::APPLE_PRIVATE_KEY,
            &mut warnings,
            &mut cfg.apple_private_key,
        )?;
        overlay_secret(
            conn,
            kek,
            keys::META_CLIENT_SECRET,
            &mut warnings,
            &mut cfg.meta_client_secret,
        )?;
        overlay_secret(
            conn,
            kek,
            keys::META_API_KEY,
            &mut warnings,
            &mut cfg.meta_api_key,
        )?;
        overlay_secret(
            conn,
            kek,
            keys::MAILER_PASSWORD,
            &mut warnings,
            &mut cfg.mailer_password,
        )?;
        overlay_secret(
            conn,
            kek,
            keys::MAIL_CLIENT_SECRET,
            &mut warnings,
            &mut cfg.mail_client_secret,
        )?;
        overlay_secret(
            conn,
            kek,
            keys::MAIL_REFRESH_TOKEN,
            &mut warnings,
            &mut cfg.mail_refresh_token,
        )?;
        overlay_bool(
            conn,
            kek,
            keys::NOTIFY_ENABLED,
            &mut warnings,
            &mut cfg.notify_enabled,
        )?;
        // Daily cap: missing/malformed → default 3 (warn on garbage,
        // mirroring the port overlay's leniency).
        match get_text_for_job(conn, kek, keys::NOTIFY_DAILY_CAP) {
            Ok(None) => {}
            Ok(Some(v)) => match v.trim().parse::<u64>() {
                Ok(n) if (1..=50).contains(&n) => cfg.notify_daily_cap = n,
                _ => warnings.push(format!(
                    "{}: {v:?} is not 1-50; using default 3",
                    keys::NOTIFY_DAILY_CAP
                )),
            },
            Err(ConfigError::BadValue(why)) => warnings.push(format!(
                "{}: {why}; using default 3",
                keys::NOTIFY_DAILY_CAP
            )),
            Err(e) => return Err(e),
        }
        // Postal footer: empty → default (empty warns); the worker
        // refuses digest sends while it stays empty.
        overlay_required_text(
            conn,
            kek,
            keys::NOTIFY_POSTAL_ADDRESS,
            &mut warnings,
            &mut cfg.notify_postal_address,
        )?;
        // H6 fix baked into the migration: a persistent dev-bypass
        // flag must never boot a release binary (spec §3) — except on
        // a loopback-only host, where local `up` runs keep the seam.
        if cfg.admin_mtls_off
            && !Self::dev_seam_allowed(!cfg!(debug_assertions), cfg.bind, cfg.bind6)
        {
            return Err(ConfigError::BadValue(
                "admin.mtls_off is dev-only; refusing release boot on non-loopback binds",
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

    /// Release-mode dev-seam gate: dev bypasses (a dev KEK, or
    /// `admin.mtls_off`) may boot a release binary ONLY when every
    /// bind IP is loopback. Pure over explicit inputs so both
    /// directions unit-test on any profile; production passes
    /// `!cfg!(debug_assertions)` as `is_release`.
    pub fn dev_seam_allowed(
        is_release: bool,
        bind: std::net::IpAddr,
        bind6: Option<std::net::IpAddr>,
    ) -> bool {
        if !is_release {
            return true;
        }
        bind.is_loopback() && bind6.is_none_or(|ip| ip.is_loopback())
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
    fn registry_holds_92_typed_keys() {
        assert_eq!(ALL_KEYS.len(), 92);
        for (key, _) in ALL_KEYS {
            assert!(valid_key_name(key), "key {key:?} must be a valid name");
        }
        // Twelve secrets (EAB HMAC, Telnyx API key, Google/Apple/Meta
        // client secrets, Meta Model API key, Gmail client secret +
        // refresh token, mailer password, setup key hash, FedEx client
        // id + secret); eight bools (labels.live is the eighth); rest
        // text (auth.* adds three, google/apple/meta ids six, mailer.*
        // six, meta.* one, mail.* three, notify.* two, labels.* four).
        // MISSION WIN (P316) adds 26: five secrets (stripe.secret_key,
        // stripe.webhook_secret, usps.client_id/client_secret/
        // epa_account), five bools (orders.enabled, labels.customer_live,
        // usps.live, plan.auto_compose, tracking.poll_enabled), 16 text.
        // P373 removes tls.client_ca_file and net.admin_https_port (text)
        // and admin.require_cert (bool).
        let mut counts = (0, 0, 0);
        for (_, key_type) in ALL_KEYS {
            match key_type {
                ConfigType::Bool => counts.0 += 1,
                ConfigType::Text => counts.1 += 1,
                ConfigType::Secret => counts.2 += 1,
            }
        }
        assert_eq!(counts, (12, 63, 17));
    }

    #[test]
    fn allow_ips_parses_ranges_hosts_and_v6() {
        let nets = parse_allow_ips("203.0.113.0/24, 198.51.100.7, 2001:db8::/32").unwrap();
        assert_eq!(nets.len(), 3);
        let inside: std::net::IpAddr = "203.0.113.200".parse().unwrap();
        let outside: std::net::IpAddr = "203.0.114.1".parse().unwrap();
        assert!(nets.iter().any(|n| n.contains(&inside)));
        assert!(!nets.iter().any(|n| n.contains(&outside)));
        let host: std::net::IpAddr = "198.51.100.7".parse().unwrap();
        let other: std::net::IpAddr = "198.51.100.8".parse().unwrap();
        assert!(nets.iter().any(|n| n.contains(&host)));
        assert!(!nets.iter().any(|n| n.contains(&other)));
        // A v4-mapped peer canonicalizes into its v4 range …
        let mapped: std::net::IpAddr = "::ffff:203.0.113.200".parse().unwrap();
        assert!(nets[0].contains(&mapped));
        // … while a true v6 address never matches a v4 range.
        let v6: std::net::IpAddr = "2001:db9::1".parse().unwrap();
        assert!(!nets.iter().any(|n| n.contains(&v6)));
        // Loopback default holds localhost only.
        let local: std::net::IpAddr = "127.0.0.1".parse().unwrap();
        let lan: std::net::IpAddr = "192.168.1.2".parse().unwrap();
        assert!(loopback_allow_ips().iter().any(|n| n.contains(&local)));
        assert!(!loopback_allow_ips().iter().any(|n| n.contains(&lan)));
        // Garbage fails closed (the loader warns + keeps loopback).
        assert!(parse_allow_ips("").is_err());
        assert!(parse_allow_ips(" , ").is_err());
        assert!(parse_allow_ips("10.0.0.0/33").is_err());
    }

    #[test]
    fn allow_ips_zero_prefix_matches_all_without_panic() {
        // Prefix 0 used to shift a u32 by 32 and panic every admin
        // request under overflow-checks; now it is match-all.
        let any4 = parse_allow_ips("0.0.0.0/0").unwrap();
        let any6 = parse_allow_ips("::/0").unwrap();
        let v4a: std::net::IpAddr = "203.0.113.7".parse().unwrap();
        let v4b: std::net::IpAddr = "10.255.0.1".parse().unwrap();
        let v6a: std::net::IpAddr = "::1".parse().unwrap();
        let v6b: std::net::IpAddr = "2001:db8::1".parse().unwrap();
        assert!(any4.iter().any(|n| n.contains(&v4a)));
        assert!(any4.iter().any(|n| n.contains(&v4b)));
        assert!(any6.iter().any(|n| n.contains(&v6a)));
        assert!(any6.iter().any(|n| n.contains(&v6b)));
        // … but families still do not cross: v6 /0 takes no v4 peer.
        assert!(!any6.iter().any(|n| n.contains(&v4a)));
        assert!(!any4.iter().any(|n| n.contains(&v6a)));
    }

    #[test]
    fn allow_ips_mapped_peer_matches_host_route() {
        // The operator's stack may present ::ffff:203.0.113.7 for a
        // v4 client: it must match a 203.0.113.7 range entry.
        let nets = parse_allow_ips("203.0.113.7").unwrap();
        let mapped: std::net::IpAddr = "::ffff:203.0.113.7".parse().unwrap();
        let plain: std::net::IpAddr = "203.0.113.7".parse().unwrap();
        let other: std::net::IpAddr = "::ffff:203.0.113.8".parse().unwrap();
        assert!(nets.iter().any(|n| n.contains(&mapped)));
        assert!(nets.iter().any(|n| n.contains(&plain)));
        assert!(!nets.iter().any(|n| n.contains(&other)));
        assert!(parse_allow_ips("not-an-ip").is_err());
    }

    #[test]
    fn dev_seam_gate_both_directions() {
        use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
        let loopback = IpAddr::from([127, 0, 0, 1]);
        let lan: IpAddr = "192.168.1.2".parse().unwrap();
        let any = IpAddr::from(Ipv4Addr::UNSPECIFIED);
        let v6loop = IpAddr::from(Ipv6Addr::LOCALHOST);
        let v6wan: IpAddr = "2001:db8::1".parse().unwrap();
        // Dev profile: everything boots (existing behavior).
        assert!(RuntimeConfig::dev_seam_allowed(false, loopback, None));
        assert!(RuntimeConfig::dev_seam_allowed(false, lan, None));
        assert!(RuntimeConfig::dev_seam_allowed(false, any, Some(v6wan)));
        // Release profile: loopback-only hosts keep the seam …
        assert!(RuntimeConfig::dev_seam_allowed(true, loopback, None));
        assert!(RuntimeConfig::dev_seam_allowed(
            true,
            loopback,
            Some(v6loop)
        ));
        // … anything else refuses.
        assert!(!RuntimeConfig::dev_seam_allowed(true, lan, None));
        assert!(!RuntimeConfig::dev_seam_allowed(true, any, None));
        assert!(!RuntimeConfig::dev_seam_allowed(
            true,
            loopback,
            Some(v6wan)
        ));
        assert!(!RuntimeConfig::dev_seam_allowed(true, any, Some(v6loop)));
    }

    #[test]
    fn sealed_binds_loopback_both_directions() {
        let kek = Kek::generate();
        // No rows yet: loader defaults (127.0.0.1, no v6) read loopback.
        assert!(super::binds_loopback_in(&memory(), &kek));
        // Explicit loopback rows hold.
        let conn = memory();
        put(&conn, &kek, keys::NET_BIND, ConfigType::Text, b"127.0.0.1");
        assert!(super::binds_loopback_in(&conn, &kek));
        put(&conn, &kek, keys::NET_BIND6, ConfigType::Text, b"::1");
        assert!(super::binds_loopback_in(&conn, &kek));
        // A LAN/WAN bind refuses …
        put(&conn, &kek, keys::NET_BIND, ConfigType::Text, b"0.0.0.0");
        assert!(!super::binds_loopback_in(&conn, &kek));
        put(&conn, &kek, keys::NET_BIND, ConfigType::Text, b"127.0.0.1");
        put(
            &conn,
            &kek,
            keys::NET_BIND6,
            ConfigType::Text,
            b"2001:db8::1",
        );
        assert!(!super::binds_loopback_in(&conn, &kek));
        // … as does garbage the loader would only warn over (fail closed).
        put(&conn, &kek, keys::NET_BIND, ConfigType::Text, b"not-an-ip");
        assert!(!super::binds_loopback_in(&conn, &kek));
        // A wrong KEK never reads as loopback.
        let conn = memory();
        put(&conn, &kek, keys::NET_BIND, ConfigType::Text, b"127.0.0.1");
        assert!(!super::binds_loopback_in(&conn, &Kek::generate()));
        // Missing tables refuse (never loopback by default).
        let bare = Connection::open_in_memory().expect("open memory db");
        assert!(!super::binds_loopback_in(&bare, &kek));
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
        assert!(!cfg.quic_enabled);
        assert_eq!(cfg.http_port, 80);
        assert_eq!(cfg.public_host, "andrii.ai");
        assert_eq!(cfg.tls_cert, None);
        assert_eq!(cfg.tls_key, None);
        assert!(!cfg.admin_mtls_off);
        assert_eq!(cfg.admin_allow_ips, loopback_allow_ips());
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
        assert_eq!(cfg.meta_base_url, "https://api.meta.ai/v1");
        assert_eq!(cfg.meta_api_key, None);
        assert_eq!(cfg.mail_client_id, None);
        assert_eq!(cfg.mail_client_secret, None);
        assert_eq!(cfg.mail_refresh_token, None);
        assert_eq!(cfg.mail_base_url, "https://gmail.googleapis.com");
        assert_eq!(cfg.mail_token_url, "https://oauth2.googleapis.com/token");
        assert_eq!(cfg.auth_rp_id, "andrii.ai");
        assert_eq!(cfg.auth_rp_name, "Apostille Handbook");
        assert_eq!(cfg.auth_origin, "https://andrii.ai");
        assert_eq!(cfg.google_client_id, None);
        assert_eq!(cfg.google_client_secret, None);
        assert_eq!(cfg.mailer_provider, "");
        assert_eq!(cfg.mailer_host, None);
        assert_eq!(cfg.mailer_port, None);
        assert_eq!(cfg.mailer_username, None);
        assert_eq!(cfg.mailer_password, None);
        assert_eq!(cfg.mailer_from, None);
        assert_eq!(cfg.mailer_dir, "mail");
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
            ("net.admin_https_port", ConfigType::Text, "8444"),
            (keys::NET_HTTP_PORT, ConfigType::Text, "8080"),
            (keys::NET_PUBLIC_HOST, ConfigType::Text, "example.test"),
            (keys::TLS_CERT_FILE, ConfigType::Text, "/x/cert.pem"),
            (keys::TLS_KEY_FILE, ConfigType::Text, "/x/key.pem"),
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
            (
                keys::META_BASE_URL,
                ConfigType::Text,
                "https://meta-stub.test",
            ),
            (keys::META_API_KEY, ConfigType::Secret, "LLM_1_stub"),
            (
                keys::MAIL_CLIENT_ID,
                ConfigType::Text,
                "id-x.apps.googleusercontent.com",
            ),
            (
                keys::MAIL_CLIENT_SECRET,
                ConfigType::Secret,
                "mail-secret-x",
            ),
            (keys::MAIL_REFRESH_TOKEN, ConfigType::Secret, "rtok-x"),
            (
                keys::MAIL_BASE_URL,
                ConfigType::Text,
                "https://gmail-stub.test",
            ),
            (
                keys::MAIL_TOKEN_URL,
                ConfigType::Text,
                "https://sts-stub.test/token",
            ),
            (keys::QUIC_ENABLED, ConfigType::Bool, "1"),
            (keys::AUTH_RP_ID, ConfigType::Text, "example.test"),
            (keys::AUTH_RP_NAME, ConfigType::Text, "Example RP"),
            (keys::AUTH_ORIGIN, ConfigType::Text, "https://example.test"),
            (
                keys::GOOGLE_CLIENT_ID,
                ConfigType::Text,
                "id-x.apps.googleusercontent.com",
            ),
            (keys::GOOGLE_CLIENT_SECRET, ConfigType::Secret, "secret-x"),
            (keys::MAILER_PROVIDER, ConfigType::Text, "smtp"),
            (keys::MAILER_HOST, ConfigType::Text, "smtp.example.test"),
            (keys::MAILER_PORT, ConfigType::Text, "587"),
            (keys::MAILER_USERNAME, ConfigType::Text, "user@example.test"),
            (keys::MAILER_PASSWORD, ConfigType::Secret, "app-password-x"),
            (keys::MAILER_FROM, ConfigType::Text, "noreply@example.test"),
            (keys::MAILER_DIR, ConfigType::Text, "mail-x"),
        ] {
            put(&conn, &kek, key, key_type, plaintext.as_bytes());
        }
        let (cfg, warnings) = RuntimeConfig::load(&conn, &kek).expect("load full db");
        // The removed `net.admin_https_port` row (set above) is the
        // ONLY warning; every other key overlays cleanly.
        assert_eq!(warnings.len(), 1, "warnings: {warnings:?}");
        assert!(
            warnings[0].contains("net.admin_https_port") && warnings[0].contains("removed"),
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
        assert!(cfg.quic_enabled);
        assert_eq!(cfg.http_port, 8080);
        assert_eq!(cfg.public_host, "example.test");
        assert_eq!(cfg.tls_cert.as_deref(), Some("/x/cert.pem"));
        assert_eq!(cfg.tls_key.as_deref(), Some("/x/key.pem"));
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
        assert_eq!(cfg.meta_base_url, "https://meta-stub.test");
        assert_eq!(
            cfg.meta_api_key.as_ref().map(SecretText::as_str),
            Some("LLM_1_stub")
        );
        assert_eq!(
            cfg.mail_client_id.as_deref(),
            Some("id-x.apps.googleusercontent.com")
        );
        assert_eq!(
            cfg.mail_client_secret.as_ref().map(SecretText::as_str),
            Some("mail-secret-x")
        );
        assert_eq!(
            cfg.mail_refresh_token.as_ref().map(SecretText::as_str),
            Some("rtok-x")
        );
        assert_eq!(cfg.mail_base_url, "https://gmail-stub.test");
        assert_eq!(cfg.mail_token_url, "https://sts-stub.test/token");
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
        assert_eq!(
            cfg.google_client_id.as_deref(),
            Some("id-x.apps.googleusercontent.com")
        );
        assert_eq!(
            cfg.google_client_secret.as_ref().map(SecretText::as_str),
            Some("secret-x")
        );
        assert_eq!(cfg.apple_client_id, None);
        assert_eq!(cfg.apple_team_id, None);
        assert_eq!(cfg.apple_key_id, None);
        assert_eq!(cfg.apple_private_key, None);
        assert_eq!(cfg.meta_client_id, None);
        assert_eq!(cfg.meta_client_secret, None);
        assert_eq!(cfg.mailer_provider, "smtp");
        assert_eq!(cfg.mailer_host.as_deref(), Some("smtp.example.test"));
        assert_eq!(cfg.mailer_port.as_deref(), Some("587"));
        assert_eq!(cfg.mailer_username.as_deref(), Some("user@example.test"));
        assert_eq!(
            cfg.mailer_password.as_ref().map(SecretText::as_str),
            Some("app-password-x")
        );
        assert_eq!(cfg.mailer_from.as_deref(), Some("noreply@example.test"));
        assert_eq!(cfg.mailer_dir, "mail-x");
    }

    #[test]
    fn removed_keys_warn_when_stored_and_stay_out_of_the_admin_list() {
        let conn = memory();
        let kek = Kek::generate();
        let (_, warnings) = RuntimeConfig::load(&conn, &kek).expect("load empty db");
        assert!(warnings.is_empty());
        put(
            &conn,
            &kek,
            "tls.client_ca_file",
            ConfigType::Text,
            b"/x/ca.pem",
        );
        put(&conn, &kek, "admin.require_cert", ConfigType::Bool, b"1");
        put(&conn, &kek, keys::NET_PORT, ConfigType::Text, b"3000");
        // A stored require_cert used to refuse the boot without a CA;
        // removed, it is inert.
        let (_, warnings) = RuntimeConfig::load(&conn, &kek).expect("load");
        assert_eq!(warnings.len(), 2, "warnings: {warnings:?}");
        assert!(
            warnings
                .iter()
                .all(|w| w.contains("removed") && w.contains("IGNORED")),
            "warnings: {warnings:?}"
        );
        let listed: Vec<String> = list_display(&conn, &kek)
            .expect("list")
            .into_iter()
            .map(|(key, _)| key)
            .collect();
        assert_eq!(listed, vec![keys::NET_PORT.to_string()]);
        for key in keys::REMOVED {
            assert!(
                !ALL_KEYS.iter().any(|(k, _)| k == key),
                "{key} is removed but still registered"
            );
        }
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
