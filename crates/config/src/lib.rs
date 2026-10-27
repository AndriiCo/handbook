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
/// Dev sources print a loud banner and are REFUSED outside debug
/// builds — a dev-marked KEK can never start a release binary.
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
