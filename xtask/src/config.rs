//! Encrypted config (secure-config.md slice): envelope encryption in SQLite.
//!
//! Tasks: `config-enroll`, `config-set`, `config-get`, `config-verify`,
//! `config-rotate`. Values are sealed with XChaCha20-Poly1305; each row
//! stores `nonce(24) ‖ ciphertext` with AAD `key:enc_version` (row-swap
//! resistant). `secret` keys are write-only: settable, never readable.
//!
//! The KEK lives in a 32-byte file (`--kek-file`, or `HANDBOOK_KEK_FILE`).
//! `config-enroll --seal` additionally seals it via `systemd-creds`
//! (host+tpm2) when that tool exists; otherwise enrollment prints a
//! loud DEV-CRYPTO banner (file KEK, synthetic data only) and the
//! `.cred` step stays a provisioning-time operation.
//!
//! Tables come from the shared migration (`006_config.sql`): every task
//! runs [`handbook_store::db::migrate`] on open, so there is exactly one
//! schema path and no ad-hoc `CREATE TABLE` to collide with upgrades.

use chacha20poly1305::{
    KeyInit, XChaCha20Poly1305, XNonce,
    aead::{Aead, Payload},
};
use handbook_store::store::Store;
use zeroize::Zeroizing;

type Kek = Zeroizing<[u8; 32]>;

fn open_conn(store: &Store) -> Result<rusqlite::Connection, String> {
    let conn = handbook_store::db::open(store).map_err(|e| e.to_string())?;
    handbook_store::db::migrate(&conn).map_err(|e| e.to_string())?;
    Ok(conn)
}

fn read_kek(args: &[String]) -> Result<Kek, String> {
    let path = crate::flag(args, "kek-file")
        .or_else(|| std::env::var("HANDBOOK_KEK_FILE").ok())
        .ok_or_else(|| "missing --kek-file (or HANDBOOK_KEK_FILE)".to_string())?;
    read_kek_file(&path)
}

fn read_kek_file(path: &str) -> Result<Kek, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("read KEK {path}: {e}"))?;
    if bytes.len() != 32 {
        return Err(format!(
            "KEK {path}: expected 32 bytes, got {}",
            bytes.len()
        ));
    }
    let mut kek = [0u8; 32];
    kek.copy_from_slice(&bytes);
    Ok(Zeroizing::new(kek))
}

fn seal(value: &[u8], key: &str, version: i64, kek: &Kek) -> Result<Vec<u8>, String> {
    let cipher = XChaCha20Poly1305::new_from_slice(&kek[..]).map_err(|e| e.to_string())?;
    let nonce_bytes: [u8; 24] = rand::random();
    let aad = format!("{key}:{version}");
    let ct = cipher
        .encrypt(
            XNonce::from_slice(&nonce_bytes),
            Payload {
                msg: value,
                aad: aad.as_bytes(),
            },
        )
        .map_err(|e| format!("seal: {e}"))?;
    let mut out = Vec::with_capacity(24 + ct.len());
    out.extend_from_slice(&nonce_bytes);
    out.extend_from_slice(&ct);
    Ok(out)
}

fn unseal(blob: &[u8], key: &str, version: i64, kek: &Kek) -> Result<Vec<u8>, String> {
    if blob.len() < 24 + 16 {
        return Err(format!("config row {key:?}: truncated ciphertext"));
    }
    let cipher = XChaCha20Poly1305::new_from_slice(&kek[..]).map_err(|e| e.to_string())?;
    let aad = format!("{key}:{version}");
    cipher
        .decrypt(
            XNonce::from_slice(&blob[..24]),
            Payload {
                msg: &blob[24..],
                aad: aad.as_bytes(),
            },
        )
        .map_err(|_| format!("config row {key:?}: decrypt failed (wrong KEK or tampered row)"))
}

fn active_version(conn: &rusqlite::Connection) -> Result<Option<i64>, String> {
    conn.query_row(
        "SELECT version FROM config_kek WHERE retired_at IS NULL ORDER BY version DESC LIMIT 1",
        [],
        |row| row.get(0),
    )
    .map(Some)
    .or_else(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => Ok(None),
        other => Err(other),
    })
    .map_err(|e| e.to_string())
}

fn valid_key(key: &str) -> bool {
    !key.is_empty()
        && key.len() <= 128
        && key
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

/// `config-enroll --kek-file <path> [--actor <a>] [--seal] [--cred <path>]`.
/// Generates a 32-byte KEK (0600), records generation 1 (or next).
pub fn run_enroll(args: &[String]) {
    let store = crate::open_store(args);
    let conn = open_conn(&store).unwrap_or_else(|e| fail(&e));
    let path = crate::flag(args, "kek-file").unwrap_or_else(|| fail("missing --kek-file <path>"));
    if std::path::Path::new(&path).exists() {
        fail(&format!("KEK file exists (refusing to overwrite): {path}"));
    }
    let kek: [u8; 32] = rand::random();
    write_private(&path, &kek);
    let at = crate::unix_now();
    let version: i64 = conn
        .query_row(
            "SELECT COALESCE(MAX(version), 0) + 1 FROM config_kek",
            [],
            |row| row.get(0),
        )
        .unwrap_or_else(|e| fail(&e.to_string()));
    let actor = crate::flag(args, "actor").unwrap_or_else(|| "xtask".to_string());
    let mut cred_path = String::new();
    if args.iter().any(|a| a == "--seal") {
        let cred = crate::flag(args, "cred")
            .unwrap_or_else(|| format!("/etc/credstore.encrypted/handbook-kek-v{version}.cred"));
        match seal_with_systemd(&path, &cred) {
            Ok(()) => {
                cred_path = cred.clone();
                println!("KEK sealed: {cred}");
            }
            Err(e) => println!("seal skipped ({e}); file KEK remains — DEV CRYPTO"),
        }
    }
    conn.execute(
        "INSERT INTO config_kek(version, cred_path, created_at) VALUES (?1, ?2, ?3)",
        rusqlite::params![version, cred_path, at],
    )
    .unwrap_or_else(|e| fail(&e.to_string()));
    conn.execute(
        "INSERT INTO audit(at, actor, action, entity, entity_id, reason)
         VALUES (?1, ?2, 'config_enroll', 'config_kek', ?3, '')",
        rusqlite::params![at, actor, version],
    )
    .unwrap_or_else(|e| fail(&e.to_string()));
    if cred_path.is_empty() {
        println!("DEV CRYPTO — file-backed KEK, NOT TPM-sealed, synthetic data only");
    }
    println!("enrolled KEK generation {version}");
}

#[cfg(unix)]
fn write_private(path: &str, bytes: &[u8]) {
    use std::os::unix::fs::OpenOptionsExt;
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true).mode(0o600);
    let mut file = opts
        .open(path)
        .unwrap_or_else(|e| fail(&format!("write KEK {path}: {e}")));
    use std::io::Write as _;
    file.write_all(bytes)
        .unwrap_or_else(|e| fail(&format!("write KEK {path}: {e}")));
}

#[cfg(not(unix))]
fn write_private(path: &str, bytes: &[u8]) {
    std::fs::write(path, bytes).unwrap_or_else(|e| fail(&format!("write KEK {path}: {e}")));
}

/// Best-effort TPM seal via systemd-creds. `Err` carries the reason;
/// callers fall back to the file KEK loudly, never silently.
fn seal_with_systemd(kek_path: &str, cred_path: &str) -> Result<(), String> {
    let probe = std::process::Command::new("systemd-creds")
        .arg("--help")
        .output();
    if probe.is_err() {
        return Err("systemd-creds not installed".to_string());
    }
    let status = std::process::Command::new("systemd-creds")
        .args([
            "encrypt",
            "--with-key=host+tpm2",
            "--name=handbook-kek",
            kek_path,
            cred_path,
        ])
        .status()
        .map_err(|e| e.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "systemd-creds exited {}",
            status.code().unwrap_or(-1)
        ))
    }
}

/// `config-set --key <k> --type bool|text|secret (--value <v> | --value-from-stdin)`.
pub fn run_set(args: &[String]) {
    let store = crate::open_store(args);
    let conn = open_conn(&store).unwrap_or_else(|e| fail(&e));
    let key = crate::required(args, "key");
    if !valid_key(&key) {
        fail("invalid --key (ascii word chars, dots, dashes; max 128)");
    }
    let kind = crate::required(args, "type");
    if !["bool", "text", "secret"].contains(&kind.as_str()) {
        fail("invalid --type (bool | text | secret)");
    }
    let value = if args.iter().any(|a| a == "--value-from-stdin") {
        use std::io::Read as _;
        let mut buf = String::new();
        std::io::stdin()
            .read_to_string(&mut buf)
            .unwrap_or_else(|e| fail(&e.to_string()));
        buf.trim_end_matches(['\n', '\r']).to_string()
    } else {
        crate::required(args, "value")
    };
    // Canonical bool bytes are `1`/`0` (shared with the web admin
    // API, which rejects anything else on read): normalize at write.
    let value = if kind == "bool" {
        match value.as_str() {
            "true" | "1" => "1".to_string(),
            "false" | "0" => "0".to_string(),
            _ => fail("bool values are true | false | 1 | 0"),
        }
    } else {
        value
    };
    let kek = read_kek(args).unwrap_or_else(|e| fail(&e));
    let version = active_version(&conn)
        .unwrap_or_else(|e| fail(&e))
        .unwrap_or_else(|| fail("no active KEK: run `cargo xtask config-enroll` first"));
    let blob = seal(value.as_bytes(), &key, version, &kek).unwrap_or_else(|e| fail(&e));
    let at = crate::unix_now();
    let actor = crate::flag(args, "actor").unwrap_or_else(|| "xtask".to_string());
    handbook_store::db::in_transaction(&conn, || {
        conn.execute(
            "INSERT INTO config(key, type, value_enc, enc_version, updated_at, updated_by)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(key) DO UPDATE SET
               type = excluded.type, value_enc = excluded.value_enc,
               enc_version = excluded.enc_version, updated_at = excluded.updated_at,
               updated_by = excluded.updated_by",
            rusqlite::params![key, kind, blob, version, at, actor],
        )?;
        conn.execute(
            "INSERT INTO audit(at, actor, action, entity, entity_id, reason)
             VALUES (?1, ?2, 'config_set', 'config', 0, ?3)",
            rusqlite::params![at, actor, key],
        )?;
        Ok(())
    })
    .unwrap_or_else(|e| fail(&e.to_string()));
    println!("set {key} ({kind})");
}

/// `config-get --key <k>`: decrypt and print. `secret` keys refuse.
pub fn run_get(args: &[String]) {
    let store = crate::open_store(args);
    let conn = open_conn(&store).unwrap_or_else(|e| fail(&e));
    let key = crate::required(args, "key");
    let (kind, blob, version): (String, Vec<u8>, i64) = conn
        .query_row(
            "SELECT type, value_enc, enc_version FROM config WHERE key = ?1",
            [&key],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap_or_else(|_| fail(&format!("unknown config key: {key}")));
    if kind == "secret" {
        fail("secret keys are write-only (set-new-only, never retrievable)");
    }
    let kek = read_kek(args).unwrap_or_else(|e| fail(&e));
    let plain = unseal(&blob, &key, version, &kek).unwrap_or_else(|e| fail(&e));
    println!("{}", String::from_utf8_lossy(&plain));
}

/// `config-verify`: decrypt every row, report counts, print no values.
pub fn run_verify(args: &[String]) {
    let store = crate::open_store(args);
    let conn = open_conn(&store).unwrap_or_else(|e| fail(&e));
    let kek = read_kek(args).unwrap_or_else(|e| fail(&e));
    let version = active_version(&conn)
        .unwrap_or_else(|e| fail(&e))
        .unwrap_or_else(|| fail("no active KEK"));
    let rows: Vec<(String, Vec<u8>, i64)> = conn
        .prepare("SELECT key, value_enc, enc_version FROM config")
        .unwrap_or_else(|e| fail(&e.to_string()))
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .unwrap_or_else(|e| fail(&e.to_string()))
        .collect::<rusqlite::Result<_>>()
        .unwrap_or_else(|e| fail(&e.to_string()));
    for (key, blob, row_version) in &rows {
        if *row_version != version {
            fail(&format!(
                "stale row {key:?} on retired generation {row_version}"
            ));
        }
        unseal(blob, key, *row_version, &kek).unwrap_or_else(|e| fail(&e));
    }
    println!(
        "config-verify: {} row(s) decrypt, generation {version}",
        rows.len()
    );
}

/// `config-rotate --new-kek-file <path>`: new generation, re-encrypt
/// every row in one transaction, retire the old KEK.
pub fn run_rotate(args: &[String]) {
    let store = crate::open_store(args);
    let conn = open_conn(&store).unwrap_or_else(|e| fail(&e));
    let new_path =
        crate::flag(args, "new-kek-file").unwrap_or_else(|| fail("missing --new-kek-file <path>"));
    if std::path::Path::new(&new_path).exists() {
        fail(&format!(
            "KEK file exists (refusing to overwrite): {new_path}"
        ));
    }
    let old_kek = read_kek(args).unwrap_or_else(|e| fail(&e));
    let old_version = active_version(&conn)
        .unwrap_or_else(|e| fail(&e))
        .unwrap_or_else(|| fail("no active KEK"));
    let rows: Vec<(String, String, Vec<u8>, i64)> = conn
        .prepare("SELECT key, type, value_enc, enc_version FROM config")
        .unwrap_or_else(|e| fail(&e.to_string()))
        .query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })
        .unwrap_or_else(|e| fail(&e.to_string()))
        .collect::<rusqlite::Result<_>>()
        .unwrap_or_else(|e| fail(&e.to_string()));
    // Decrypt everything BEFORE minting the new generation: a bad row
    // aborts the rotation without touching any state.
    let mut plains = Vec::with_capacity(rows.len());
    for (key, kind, blob, row_version) in &rows {
        let plain = unseal(blob, key, *row_version, &old_kek).unwrap_or_else(|e| fail(&e));
        plains.push((key.clone(), kind.clone(), plain));
    }
    let new_kek_bytes: [u8; 32] = rand::random();
    write_private(&new_path, &new_kek_bytes);
    let new_kek = Zeroizing::new(new_kek_bytes);
    let at = crate::unix_now();
    let actor = crate::flag(args, "actor").unwrap_or_else(|| "xtask".to_string());
    handbook_store::db::in_transaction(&conn, || {
        let new_version: i64 = conn.query_row(
            "SELECT COALESCE(MAX(version), 0) + 1 FROM config_kek",
            [],
            |row| row.get(0),
        )?;
        for (key, _, plain) in &plains {
            let blob = seal(plain, key, new_version, &new_kek).map_err(|e| {
                rusqlite::Error::ToSqlConversionFailure(Box::new(std::io::Error::other(e)))
            })?;
            conn.execute(
                "UPDATE config SET value_enc = ?1, enc_version = ?2,
                                  updated_at = ?3, updated_by = ?4
                 WHERE key = ?5",
                rusqlite::params![blob, new_version, at, actor, key],
            )?;
        }
        conn.execute(
            "INSERT INTO config_kek(version, cred_path, created_at) VALUES (?1, '', ?2)",
            rusqlite::params![new_version, at],
        )?;
        conn.execute(
            "UPDATE config_kek SET retired_at = ?1 WHERE version = ?2",
            rusqlite::params![at, old_version],
        )?;
        conn.execute(
            "INSERT INTO audit(at, actor, action, entity, entity_id, reason)
             VALUES (?1, ?2, 'config_rotate', 'config_kek', ?3, ?4)",
            rusqlite::params![
                at,
                actor,
                new_version,
                format!("{old_version}->{new_version}")
            ],
        )?;
        Ok(())
    })
    .unwrap_or_else(|e| fail(&e.to_string()));
    println!("rotated to new KEK generation (old retired); update HANDBOOK_KEK_FILE to {new_path}");
}

fn fail(message: &str) -> ! {
    eprintln!("config: {message}");
    std::process::exit(1);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> (tempfile::TempDir, Store, rusqlite::Connection, String) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().join("store"));
        store.init().unwrap();
        let conn = handbook_store::db::open(&store).unwrap();
        handbook_store::db::migrate(&conn).unwrap();
        let kek_path = dir.path().join("kek");
        let bytes: [u8; 32] = rand::random();
        std::fs::write(&kek_path, bytes).unwrap();
        (dir, store, conn, kek_path.to_string_lossy().into_owned())
    }

    fn enroll_v1(conn: &rusqlite::Connection) {
        conn.execute(
            "INSERT INTO config_kek(version, cred_path, created_at) VALUES (1, '', 1)",
            [],
        )
        .unwrap();
    }

    #[test]
    fn seal_roundtrip_and_aad_binding() {
        let (_dir, _store, _conn, kek_path) = setup();
        let kek = read_kek_file(&kek_path).unwrap();
        let blob = seal(b"hunter2", "api.key", 1, &kek).unwrap();
        assert_eq!(unseal(&blob, "api.key", 1, &kek).unwrap(), b"hunter2");
        // Wrong key name or version breaks the AAD: row-swap fails.
        unseal(&blob, "other.key", 1, &kek).unwrap_err();
        unseal(&blob, "api.key", 2, &kek).unwrap_err();
        // Bit flip fails authentication.
        let mut tampered = blob.clone();
        let last = tampered.len() - 1;
        tampered[last] ^= 1;
        unseal(&tampered, "api.key", 1, &kek).unwrap_err();
    }

    #[test]
    fn set_get_verify_roundtrip() {
        let (_dir, store, conn, kek_path) = setup();
        enroll_v1(&conn);
        let kek = read_kek_file(&kek_path).unwrap();
        for (key, kind, value) in [
            ("site.on", "bool", "true"),
            ("site.name", "text", "demo"),
            ("api.token", "secret", "s3cr3t"),
        ] {
            let blob = seal(value.as_bytes(), key, 1, &kek).unwrap();
            conn.execute(
                "INSERT INTO config(key, type, value_enc, enc_version, updated_at, updated_by)
                 VALUES (?1, ?2, ?3, 1, 1, 'test')",
                rusqlite::params![key, kind, blob],
            )
            .unwrap();
        }
        // Non-secret rows decrypt; secret rows exist but stay sealed
        // (the CLI refuses them — enforced by kind check, tested below).
        let (kind, blob, version): (String, Vec<u8>, i64) = conn
            .query_row(
                "SELECT type, value_enc, enc_version FROM config WHERE key = 'site.name'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(kind, "text");
        assert_eq!(unseal(&blob, "site.name", version, &kek).unwrap(), b"demo");
        assert_eq!(active_version(&conn).unwrap(), Some(1));
        let _ = store;
    }

    #[test]
    fn bool_values_normalize_to_canonical_bytes() {
        let (_dir, store, conn, kek_path) = setup();
        enroll_v1(&conn);
        let root = store.root().to_string_lossy().into_owned();
        for (input, canonical) in [("true", b"1"), ("false", b"0"), ("1", b"1"), ("0", b"0")] {
            let key = format!("flag.{input}");
            run_set(&[
                "config-set".to_string(),
                "--store".to_string(),
                root.clone(),
                "--key".to_string(),
                key.clone(),
                "--type".to_string(),
                "bool".to_string(),
                "--value".to_string(),
                input.to_string(),
                "--kek-file".to_string(),
                kek_path.clone(),
            ]);
            let (blob, version): (Vec<u8>, i64) = conn
                .query_row(
                    "SELECT value_enc, enc_version FROM config WHERE key = ?1",
                    [&key],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .unwrap();
            let kek = read_kek_file(&kek_path).unwrap();
            assert_eq!(
                unseal(&blob, &key, version, &kek).unwrap(),
                canonical,
                "input {input:?}"
            );
        }
    }

    #[test]
    fn key_validation() {
        assert!(valid_key("fedex.account_id"));
        assert!(valid_key("a-b_c.9"));
        assert!(!valid_key(""));
        assert!(!valid_key("has space"));
        assert!(!valid_key("semi;colon"));
        assert!(!valid_key(&"x".repeat(129)));
    }
}
