//! Admin mTLS client certificates: issue / verify / revoke.
//!
//! Tasks: `admin-cert-issue --cn <name>`, `admin-cert-verify
//! --cert <pem>`, `admin-cert-revoke (--cn <name> | --fingerprint
//! <hex>)`. A local CA (`<store>/admin-certs/ca.pem`, key 0600)
//! signs client certs; issuance is recorded in a self-contained
//! `admin_cert` table (fingerprint → cn, expiry, revocation).
//!
//! Verification is registry-bound: the presented PEM's DER sha256
//! must match a live registry row. Full chain validation happens at
//! the TLS handshake when axum mTLS lands; this registry is the
//! issuance/revocation source of truth, not a chain validator.

use handbook_store::store::Store;
use rcgen::{
    BasicConstraints, Certificate, CertificateParams, DnType, ExtendedKeyUsagePurpose, IsCa,
    KeyPair,
};

const CA_CN: &str = "handbook-admin-ca";

fn ensure_tables(conn: &rusqlite::Connection) -> Result<(), String> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS admin_cert(
           fingerprint TEXT PRIMARY KEY,
           cn TEXT NOT NULL,
           issued_at INTEGER NOT NULL,
           expires_at INTEGER NOT NULL,
           revoked_at INTEGER,
           cert_path TEXT NOT NULL DEFAULT ''
         );",
    )
    .map_err(|e| e.to_string())
}

fn open_conn(store: &Store) -> Result<rusqlite::Connection, String> {
    let conn = handbook_store::db::open(store).map_err(|e| e.to_string())?;
    ensure_tables(&conn)?;
    Ok(conn)
}

fn certs_dir(store: &Store) -> std::path::PathBuf {
    store.root().join("admin-certs")
}

fn valid_cn(cn: &str) -> bool {
    !cn.is_empty()
        && cn.len() <= 64
        && cn
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

fn write_private(path: &std::path::Path, bytes: &[u8]) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(path)
            .map_err(|e| format!("write {}: {e}", path.display()))?;
        use std::io::Write as _;
        file.write_all(bytes)
            .map_err(|e| format!("write {}: {e}", path.display()))?;
        Ok(())
    }
    #[cfg(not(unix))]
    {
        std::fs::write(path, bytes).map_err(|e| format!("write {}: {e}", path.display()))
    }
}

fn ca_params() -> CertificateParams {
    let mut params = CertificateParams::default();
    params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    params.distinguished_name.push(DnType::CommonName, CA_CN);
    params
}

/// Load the CA (creating + self-signing it on first use). Returns a
/// signer `Certificate` carrying the CA DN plus the CA key pair.
/// On reload the signer cert is a fresh self-signature over the same
/// key and DN (rcgen generates, never parses): it exists only to feed
/// `signed_by`, which copies the issuer DN — the persisted `ca.pem`
/// stays the trust anchor.
fn ensure_ca(store: &Store) -> Result<(Certificate, KeyPair), String> {
    let dir = certs_dir(store);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let key_path = dir.join("ca-key.pem");
    let cert_path = dir.join("ca.pem");
    if key_path.exists() && cert_path.exists() {
        let pem = std::fs::read_to_string(&key_path).map_err(|e| e.to_string())?;
        let key = KeyPair::from_pem(&pem).map_err(|e| format!("parse CA key: {e}"))?;
        let signer = ca_params()
            .self_signed(&key)
            .map_err(|e| format!("rebuild CA: {e}"))?;
        return Ok((signer, key));
    }
    let key = KeyPair::generate().map_err(|e| format!("generate CA key: {e}"))?;
    let cert = ca_params()
        .self_signed(&key)
        .map_err(|e| format!("self-sign CA: {e}"))?;
    std::fs::write(&cert_path, cert.pem().as_bytes()).map_err(|e| e.to_string())?;
    write_private(&key_path, key.serialize_pem().as_bytes())?;
    Ok((cert, key))
}

/// `admin-cert-issue --cn <name> [--days <n>] [--actor <a>]`.
pub fn run_issue(args: &[String]) {
    let store = crate::open_store(args);
    let conn = open_conn(&store).unwrap_or_else(|e| fail(&e));
    let cn = crate::required(args, "cn");
    if !valid_cn(&cn) {
        fail("invalid --cn (ascii word chars, dots, dashes; max 64)");
    }
    let days: i64 = crate::flag(args, "days")
        .and_then(|d| d.parse().ok())
        .unwrap_or(90);
    if !(1..=825).contains(&days) {
        fail("--days must be 1..825");
    }
    let actor = crate::flag(args, "actor").unwrap_or_else(|| "xtask".to_string());
    issue(&store, &conn, &cn, days, &actor).unwrap_or_else(|e| fail(&e));
}

fn issue(
    store: &Store,
    conn: &rusqlite::Connection,
    cn: &str,
    days: i64,
    actor: &str,
) -> Result<(), String> {
    let (issuer, issuer_key) = ensure_ca(store)?;
    let client_key = KeyPair::generate().map_err(|e| format!("generate client key: {e}"))?;
    let mut params = CertificateParams::new(vec![cn.to_string()]).map_err(|e| e.to_string())?;
    params.distinguished_name.push(DnType::CommonName, cn);
    params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ClientAuth];
    let cert = params
        .signed_by(&client_key, &issuer, &issuer_key)
        .map_err(|e| format!("sign client cert: {e}"))?;
    let fingerprint = handbook_store::store::sha256_hex(cert.der());
    let at = crate::unix_now();
    let expires_at = at.saturating_add(days.saturating_mul(86_400));
    let dir = certs_dir(store);
    let cert_path = dir.join(format!("client-{cn}-{}.pem", &fingerprint[..12]));
    let key_path = dir.join(format!("client-{cn}-{}.key.pem", &fingerprint[..12]));
    std::fs::write(&cert_path, cert.pem().as_bytes()).map_err(|e| e.to_string())?;
    write_private(&key_path, client_key.serialize_pem().as_bytes())?;
    handbook_store::db::in_transaction(conn, || {
        conn.execute(
            "INSERT INTO admin_cert(fingerprint, cn, issued_at, expires_at, cert_path)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![fingerprint, cn, at, expires_at, cert_path.to_string_lossy()],
        )?;
        conn.execute(
            "INSERT INTO audit(at, actor, action, entity, entity_id, reason)
             VALUES (?1, ?2, 'admin_cert_issue', 'admin_cert', 0, ?3)",
            rusqlite::params![at, actor, cn],
        )?;
        Ok(())
    })
    .map_err(|e| e.to_string())?;
    println!(
        "issued {cn}\n  cert: {}\n  key: {}\n  fingerprint: {fingerprint}",
        cert_path.display(),
        key_path.display()
    );
    Ok(())
}

/// Read the first CERTIFICATE block's DER from a PEM file.
fn cert_der(path: &str) -> Result<Vec<u8>, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("read {path}: {e}"))?;
    let mut reader = std::io::BufReader::new(bytes.as_slice());
    let mut certs = rustls_pemfile::certs(&mut reader);
    match certs.next() {
        Some(Ok(der)) => Ok(der.to_vec()),
        _ => Err(format!("no CERTIFICATE block in {path}")),
    }
}

/// Verify one PEM against the registry. `Ok(cn)` on success.
pub fn verify(store: &Store, cert_path: &str, now: i64) -> Result<String, String> {
    let conn = open_conn(store)?;
    let der = cert_der(cert_path)?;
    let fingerprint = handbook_store::store::sha256_hex(&der);
    let row: Option<(String, i64, Option<i64>)> = conn
        .query_row(
            "SELECT cn, expires_at, revoked_at FROM admin_cert WHERE fingerprint = ?1",
            [&fingerprint],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(other),
        })
        .map_err(|e| e.to_string())?;
    let Some((cn, expires_at, revoked_at)) = row else {
        return Err("certificate not issued by this CA (unknown fingerprint)".to_string());
    };
    if revoked_at.is_some() {
        return Err(format!("certificate for {cn:?} is revoked"));
    }
    if now >= expires_at {
        return Err(format!("certificate for {cn:?} is expired"));
    }
    Ok(cn)
}

/// `admin-cert-verify --cert <pem>`.
pub fn run_verify(args: &[String]) {
    let store = crate::open_store(args);
    let cert = crate::required(args, "cert");
    match verify(&store, &cert, crate::unix_now()) {
        Ok(cn) => println!("OK {cn}"),
        Err(e) => fail(&e),
    }
}

/// `admin-cert-revoke (--cn <name> | --fingerprint <hex>) [--actor <a>]`.
pub fn run_revoke(args: &[String]) {
    let store = crate::open_store(args);
    let conn = open_conn(&store).unwrap_or_else(|e| fail(&e));
    let actor = crate::flag(args, "actor").unwrap_or_else(|| "xtask".to_string());
    let (sql, value) = match (crate::flag(args, "cn"), crate::flag(args, "fingerprint")) {
        (Some(cn), None) => ("cn = ?1", cn),
        (None, Some(fp)) => ("fingerprint = ?1", fp),
        _ => fail("pass exactly one of --cn or --fingerprint"),
    };
    let at = crate::unix_now();
    let changed = handbook_store::db::in_transaction(&conn, || {
        let changed = conn.execute(
            &format!("UPDATE admin_cert SET revoked_at = ?1 WHERE {sql} AND revoked_at IS NULL"),
            rusqlite::params![at, value],
        )?;
        conn.execute(
            "INSERT INTO audit(at, actor, action, entity, entity_id, reason)
             VALUES (?1, ?2, 'admin_cert_revoke', 'admin_cert', 0, ?3)",
            rusqlite::params![at, actor, value],
        )?;
        Ok(changed)
    })
    .unwrap_or_else(|e| fail(&e.to_string()));
    if changed == 0 {
        fail("no live certificate matched");
    }
    println!("revoked {changed} certificate(s)");
}

fn fail(message: &str) -> ! {
    eprintln!("admin-cert: {message}");
    std::process::exit(1);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> (tempfile::TempDir, Store, rusqlite::Connection) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().join("store"));
        store.init().unwrap();
        let conn = handbook_store::db::open(&store).unwrap();
        handbook_store::db::migrate(&conn).unwrap();
        ensure_tables(&conn).unwrap();
        (dir, store, conn)
    }

    #[test]
    fn issue_verify_revoke_roundtrip() {
        let (_dir, store, conn) = setup();
        issue(&store, &conn, "ops-1", 90, "test").unwrap();
        let cert: String = conn
            .query_row("SELECT cert_path FROM admin_cert", [], |row| row.get(0))
            .unwrap();
        assert_eq!(verify(&store, &cert, 1_800_000_000).unwrap(), "ops-1");
        // Unknown PEM fails closed.
        verify(&store, "xtask/src/main.rs", 1_800_000_000).unwrap_err();
        // Revoke by cn; verify then refuses.
        let at = 1_800_000_001;
        conn.execute("UPDATE admin_cert SET revoked_at = ?1", [at])
            .unwrap();
        let err = verify(&store, &cert, 1_800_000_002).unwrap_err();
        assert!(err.contains("revoked"), "{err}");
    }

    #[test]
    fn expiry_is_enforced() {
        let (_dir, store, conn) = setup();
        issue(&store, &conn, "short", 1, "test").unwrap();
        let cert: String = conn
            .query_row("SELECT cert_path FROM admin_cert", [], |row| row.get(0))
            .unwrap();
        let expires: i64 = conn
            .query_row("SELECT expires_at FROM admin_cert", [], |row| row.get(0))
            .unwrap();
        let err = verify(&store, &cert, expires).unwrap_err();
        assert!(err.contains("expired"), "{err}");
    }

    #[test]
    fn cn_validation() {
        assert!(valid_cn("ops-1"));
        assert!(valid_cn("admin.andrii.ai"));
        assert!(!valid_cn(""));
        assert!(!valid_cn("../evil"));
        assert!(!valid_cn("has space"));
    }
}
