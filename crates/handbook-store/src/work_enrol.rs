//! Worker enrolment and per-worker credentials (`work.sqlite`).
//!
//! A worker with no token files a request ([`request`]) and gets a
//! pairing code it prints in its log; staff approve it by typing that
//! code ([`approve`]); the worker's next [`poll`] mints its `w2.` token,
//! stores only the token's hash and hands the plaintext out once.
//! Sockets present the token and [`authenticate`] maps it to a named,
//! revocable credential. Codes, poll secrets and tokens are stored as
//! SHA-256 only. Tables live in [`crate::work::migrate`]'s DDL.
//! Spec: `docs/specs/worker-enrolment.md`.

use rusqlite::{Connection, OptionalExtension};
use sha2::{Digest, Sha256};

use contracts::Result;

use crate::db::in_transaction;
use crate::work::audit_entity;

/// How long a request waits for a decision.
pub const ENROL_TTL_SECS: i64 = 15 * 60;
/// Requests pending at once, from everyone.
pub const MAX_PENDING: i64 = 5;
/// Requests one peer IP may file per hour.
pub const MAX_PER_IP_PER_HOUR: i64 = 3;
/// Wrong pairing codes before a request is denied.
pub const MAX_CODE_TRIES: i64 = 3;
/// Decided and expired requests older than this are pruned.
const PRUNE_AFTER_SECS: i64 = 30 * 24 * 3600;

/// Crockford base32 (no I, L, O, U).
const CODE_ALPHABET: &[u8] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

/// Constant-time equality of two hex digests of equal length.
fn digest_eq(a: &str, b: &str) -> bool {
    a.len() == b.len()
        && a.bytes()
            .zip(b.bytes())
            .fold(0u8, |diff, (x, y)| diff | (x ^ y))
            == 0
}

fn random_hex(len_bytes: usize) -> String {
    let bytes: Vec<u8> = (0..len_bytes).map(|_| rand::random::<u8>()).collect();
    hex(&bytes)
}

fn is_hex(text: &str, len: usize) -> bool {
    text.len() == len && text.bytes().all(|b| b.is_ascii_hexdigit())
}

fn decode_hex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) || !text.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).ok())
        .collect()
}

/// A fresh pairing code, `XXXX-XX`.
fn new_code() -> String {
    let chars: String = (0..6)
        .map(|_| CODE_ALPHABET[rand::random::<u8>() as usize % CODE_ALPHABET.len()] as char)
        .collect();
    format!("{}-{}", &chars[..4], &chars[4..])
}

/// A typed code as stored: upper case, no separators, Crockford's
/// look-alikes folded (`O` → `0`, `I`/`L` → `1`).
fn normalize_code(code: &str) -> String {
    code.chars()
        .filter(|c| !matches!(c, '-' | ' '))
        .map(|c| match c.to_ascii_uppercase() {
            'O' => '0',
            'I' | 'L' => '1',
            other => other,
        })
        .collect()
}

fn code_hash(code: &str) -> String {
    sha256_hex(normalize_code(code).as_bytes())
}

// ---------------------------------------------------------------------------
// Names
// ---------------------------------------------------------------------------

fn name_part(part: &str, max: usize) -> bool {
    (1..=max).contains(&part.len())
        && part.bytes().all(|b| {
            b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'.' | b'_' | b'-')
        })
}

/// A user name as a worker reports it (the name rule's user part).
pub fn valid_username(user: &str) -> bool {
    name_part(user, 32)
}

/// A host name as a worker reports it (the name rule's host part).
pub fn valid_hostname(host: &str) -> bool {
    name_part(host, 63)
}

/// `user@host`, optionally ` YYYY-MM-DD`. No `:`, since `lease_owner`
/// is `<name>:<lease token>`.
pub fn valid_name(name: &str) -> bool {
    let (base, date) = match name.split_once(' ') {
        Some((base, date)) => (base, Some(date)),
        None => (name, None),
    };
    let date_ok = date.is_none_or(|d| {
        d.len() == 10
            && d.bytes().enumerate().all(|(i, b)| match i {
                4 | 7 => b == b'-',
                _ => b.is_ascii_digit(),
            })
    });
    date_ok
        && base
            .split_once('@')
            .is_some_and(|(user, host)| valid_username(user) && valid_hostname(host))
}

// ---------------------------------------------------------------------------
// Requests
// ---------------------------------------------------------------------------

/// What a worker sends to enrol (already checked by the caller).
#[derive(Debug, Clone)]
pub struct NewRequest<'a> {
    pub username: &'a str,
    pub hostname: &'a str,
    pub version: &'a str,
    pub started_at: i64,
    pub capabilities_json: &'a str,
    pub peer_ip: &'a str,
    /// Hex SHA-256 of the worker's 32-byte poll secret.
    pub poll_hash: &'a str,
    pub now: i64,
}

/// A filed request: the worker polls `public_id` and prints `code`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Filed {
    pub public_id: String,
    pub code: String,
    pub expires_at: i64,
}

/// Why a request was not filed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// [`MAX_PENDING`] requests are already waiting.
    TooManyPending,
    /// The peer IP filed [`MAX_PER_IP_PER_HOUR`] in the past hour.
    TooManyFromIp,
    /// A field broke its rule.
    Invalid,
}

/// File one request. Returns the plaintext code once; only its hash
/// is stored.
pub fn request(
    conn: &Connection,
    req: &NewRequest<'_>,
) -> Result<std::result::Result<Filed, Refusal>> {
    if !valid_username(req.username) || !valid_hostname(req.hostname) || !is_hex(req.poll_hash, 64)
    {
        return Ok(Err(Refusal::Invalid));
    }
    in_transaction(conn, || {
        conn.execute(
            "DELETE FROM worker_enrolment WHERE created_at < ?1",
            [req.now - PRUNE_AFTER_SECS],
        )?;
        let pending: i64 = conn.query_row(
            "SELECT COUNT(*) FROM worker_enrolment WHERE status = 'pending' AND expires_at > ?1",
            [req.now],
            |row| row.get(0),
        )?;
        if pending >= MAX_PENDING {
            return Ok(Err(Refusal::TooManyPending));
        }
        let from_ip: i64 = conn.query_row(
            "SELECT COUNT(*) FROM worker_enrolment WHERE peer_ip = ?1 AND created_at > ?2",
            rusqlite::params![req.peer_ip, req.now - 3600],
            |row| row.get(0),
        )?;
        if from_ip >= MAX_PER_IP_PER_HOUR {
            return Ok(Err(Refusal::TooManyFromIp));
        }
        let public_id = random_hex(16);
        let code = new_code();
        let expires_at = req.now + ENROL_TTL_SECS;
        conn.execute(
            "INSERT INTO worker_enrolment(public_id, username, hostname, version, started_at,
                 capabilities_json, peer_ip, poll_hash, code_hash, expires_at, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            rusqlite::params![
                public_id,
                req.username,
                req.hostname,
                req.version,
                req.started_at,
                req.capabilities_json,
                req.peer_ip,
                req.poll_hash.to_ascii_lowercase(),
                code_hash(&code),
                expires_at,
                req.now
            ],
        )?;
        let id = conn.last_insert_rowid();
        audit_entity(
            conn,
            req.now,
            &format!("{}@{}", req.username, req.hostname),
            "worker_enrol_request",
            "worker_enrolment",
            id,
            req.peer_ip,
        )?;
        Ok(Ok(Filed {
            public_id,
            code,
            expires_at,
        }))
    })
}

/// What a poll finds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Poll {
    /// Still waiting for staff.
    Pending { expires_at: i64 },
    /// Approved: the token, minted now and never again.
    Approved { name: String, token: String },
    /// `denied`, `delivered` or `expired`.
    Gone(&'static str),
    /// Unknown id or wrong poll secret (the same answer for both).
    NotFound,
}

/// A worker polls its request with the poll secret (hex). On an
/// approved request this mints the credential, stores the token's
/// hash, marks the request delivered, and returns the token.
pub fn poll(conn: &Connection, public_id: &str, poll_secret_hex: &str, now: i64) -> Result<Poll> {
    let Some(secret) = decode_hex(poll_secret_hex) else {
        return Ok(Poll::NotFound);
    };
    let presented = sha256_hex(&secret);
    in_transaction(conn, || {
        let row: Option<(i64, String, String, i64, Option<String>)> = conn
            .query_row(
                "SELECT id, poll_hash, status, expires_at, name FROM worker_enrolment
                 WHERE public_id = ?1",
                [public_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .optional()?;
        let Some((id, poll_hash, status, expires_at, name)) = row else {
            return Ok(Poll::NotFound);
        };
        if !digest_eq(&presented, &poll_hash) {
            return Ok(Poll::NotFound);
        }
        match status.as_str() {
            "denied" => Ok(Poll::Gone("denied")),
            "delivered" => Ok(Poll::Gone("delivered")),
            "pending" if expires_at <= now => Ok(Poll::Gone("expired")),
            "pending" => Ok(Poll::Pending { expires_at }),
            _ => {
                let name = name.unwrap_or_default();
                let (credential_id, token) = insert_credential(conn, &name, Some(id), "", now)?;
                conn.execute(
                    "UPDATE worker_enrolment SET status = 'delivered', credential_id = ?1
                     WHERE id = ?2",
                    rusqlite::params![credential_id, id],
                )?;
                audit_entity(
                    conn,
                    now,
                    &name,
                    "worker_token_delivered",
                    "worker_credential",
                    credential_id,
                    "",
                )?;
                Ok(Poll::Approved { name, token })
            }
        }
    })
}

/// Insert a credential; returns its row id and the plaintext token.
fn insert_credential(
    conn: &Connection,
    name: &str,
    enrolment_id: Option<i64>,
    created_by: &str,
    now: i64,
) -> Result<(i64, String)> {
    let public_id = random_hex(16);
    let token = format!("w2.{public_id}.{}", random_hex(32));
    let created_by = if created_by.is_empty() {
        conn.query_row(
            "SELECT COALESCE(decided_by, '') FROM worker_enrolment WHERE id = ?1",
            [enrolment_id],
            |r| r.get::<_, String>(0),
        )
        .optional()?
        .unwrap_or_default()
    } else {
        created_by.to_string()
    };
    conn.execute(
        "INSERT INTO worker_credential(public_id, name, token_hash, enrolment_id, created_at, created_by)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        rusqlite::params![public_id, name, sha256_hex(token.as_bytes()), enrolment_id, now, created_by],
    )?;
    Ok((conn.last_insert_rowid(), token))
}

/// Mint a credential with no enrolment (`xtask up`'s local worker,
/// whose operator already holds the store). Any active credential with
/// the same name is revoked first. Returns the plaintext token.
pub fn mint_direct(conn: &Connection, name: &str, actor: &str, now: i64) -> Result<String> {
    in_transaction(conn, || {
        conn.execute(
            "UPDATE worker_credential SET revoked_at = ?1, revoked_by = ?2
             WHERE name = ?3 AND revoked_at IS NULL",
            rusqlite::params![now, actor, name],
        )?;
        let (id, token) = insert_credential(conn, name, None, actor, now)?;
        audit_entity(
            conn,
            now,
            actor,
            "worker_token_minted",
            "worker_credential",
            id,
            name,
        )?;
        Ok(token)
    })
}

/// A staff decision on a request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    Approved {
        name: String,
    },
    /// The code did not match; `tries_left` more before a deny.
    WrongCode {
        tries_left: i64,
    },
    /// The last wrong code denied the request.
    DeniedAfterTries,
    /// An active credential (or another approved request) has the name.
    NameTaken,
    /// Not pending any more (decided or expired), or no such request.
    NotPending,
    /// The name broke the rule.
    BadName,
}

fn name_taken(conn: &Connection, name: &str) -> Result<bool> {
    Ok(conn
        .query_row(
            "SELECT 1 FROM worker_credential WHERE name = ?1 AND revoked_at IS NULL
             UNION ALL
             SELECT 1 FROM worker_enrolment WHERE name = ?1 AND status = 'approved'
             LIMIT 1",
            [name],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

/// Staff approve a pending request with its pairing code and the
/// final worker name.
pub fn approve(
    conn: &Connection,
    public_id: &str,
    code: &str,
    name: &str,
    actor: &str,
    now: i64,
) -> Result<Decision> {
    if !valid_name(name) {
        return Ok(Decision::BadName);
    }
    in_transaction(conn, || {
        let row: Option<(i64, String, i64)> = conn
            .query_row(
                "SELECT id, code_hash, code_tries FROM worker_enrolment
                 WHERE public_id = ?1 AND status = 'pending' AND expires_at > ?2",
                rusqlite::params![public_id, now],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        let Some((id, stored, tries)) = row else {
            return Ok(Decision::NotPending);
        };
        if !digest_eq(&code_hash(code), &stored) {
            let tries = tries + 1;
            if tries >= MAX_CODE_TRIES {
                conn.execute(
                    "UPDATE worker_enrolment SET code_tries = ?1, status = 'denied',
                         decided_by = ?2, decided_at = ?3 WHERE id = ?4",
                    rusqlite::params![tries, actor, now, id],
                )?;
                audit_entity(
                    conn,
                    now,
                    actor,
                    "worker_enrol_deny",
                    "worker_enrolment",
                    id,
                    "wrong pairing code 3 times",
                )?;
                return Ok(Decision::DeniedAfterTries);
            }
            conn.execute(
                "UPDATE worker_enrolment SET code_tries = ?1 WHERE id = ?2",
                rusqlite::params![tries, id],
            )?;
            return Ok(Decision::WrongCode {
                tries_left: MAX_CODE_TRIES - tries,
            });
        }
        if name_taken(conn, name)? {
            return Ok(Decision::NameTaken);
        }
        conn.execute(
            "UPDATE worker_enrolment SET status = 'approved', name = ?1,
                 decided_by = ?2, decided_at = ?3 WHERE id = ?4",
            rusqlite::params![name, actor, now, id],
        )?;
        audit_entity(
            conn,
            now,
            actor,
            "worker_enrol_approve",
            "worker_enrolment",
            id,
            name,
        )?;
        Ok(Decision::Approved {
            name: name.to_string(),
        })
    })
}

/// Staff deny a pending request. `false` when it was not pending.
pub fn deny(conn: &Connection, public_id: &str, actor: &str, now: i64) -> Result<bool> {
    in_transaction(conn, || {
        let id: Option<i64> = conn
            .query_row(
                "SELECT id FROM worker_enrolment
                 WHERE public_id = ?1 AND status = 'pending' AND expires_at > ?2",
                rusqlite::params![public_id, now],
                |r| r.get(0),
            )
            .optional()?;
        let Some(id) = id else {
            return Ok(false);
        };
        conn.execute(
            "UPDATE worker_enrolment SET status = 'denied', decided_by = ?1, decided_at = ?2
             WHERE id = ?3",
            rusqlite::params![actor, now, id],
        )?;
        audit_entity(
            conn,
            now,
            actor,
            "worker_enrol_deny",
            "worker_enrolment",
            id,
            "",
        )?;
        Ok(true)
    })
}

/// One request as staff see it (never the code or hashes).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub public_id: String,
    pub username: String,
    pub hostname: String,
    pub version: String,
    pub started_at: i64,
    pub capabilities_json: String,
    pub peer_ip: String,
    pub created_at: i64,
    pub expires_at: i64,
    pub code_tries: i64,
}

/// Requests still waiting for a decision, oldest first.
pub fn pending(conn: &Connection, now: i64) -> Result<Vec<Request>> {
    let mut stmt = conn.prepare(
        "SELECT public_id, username, hostname, version, started_at, capabilities_json,
                peer_ip, created_at, expires_at, code_tries
         FROM worker_enrolment WHERE status = 'pending' AND expires_at > ?1
         ORDER BY created_at, id",
    )?;
    let rows = stmt
        .query_map([now], |r| {
            Ok(Request {
                public_id: r.get(0)?,
                username: r.get(1)?,
                hostname: r.get(2)?,
                version: r.get(3)?,
                started_at: r.get(4)?,
                capabilities_json: r.get(5)?,
                peer_ip: r.get(6)?,
                created_at: r.get(7)?,
                expires_at: r.get(8)?,
                code_tries: r.get(9)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

// ---------------------------------------------------------------------------
// Credentials
// ---------------------------------------------------------------------------

/// One credential as staff see it (never the token or its hash).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Credential {
    pub public_id: String,
    pub name: String,
    pub created_at: i64,
    pub created_by: String,
    pub last_seen_at: Option<i64>,
    pub last_version: Option<String>,
    pub revoked_at: Option<i64>,
    pub revoked_by: Option<String>,
}

/// Credentials, newest first; revoked ones only when asked.
pub fn credentials(conn: &Connection, include_revoked: bool) -> Result<Vec<Credential>> {
    let mut stmt = conn.prepare(
        "SELECT public_id, name, created_at, created_by, last_seen_at, last_version,
                revoked_at, revoked_by
         FROM worker_credential WHERE ?1 OR revoked_at IS NULL
         ORDER BY revoked_at IS NOT NULL, created_at DESC, id DESC",
    )?;
    let rows = stmt
        .query_map([include_revoked], |r| {
            Ok(Credential {
                public_id: r.get(0)?,
                name: r.get(1)?,
                created_at: r.get(2)?,
                created_by: r.get(3)?,
                last_seen_at: r.get(4)?,
                last_version: r.get(5)?,
                revoked_at: r.get(6)?,
                revoked_by: r.get(7)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Revoke one credential. `None` when unknown or already revoked.
pub fn revoke(conn: &Connection, public_id: &str, actor: &str, now: i64) -> Result<Option<i64>> {
    in_transaction(conn, || {
        let row: Option<(i64, String)> = conn
            .query_row(
                "SELECT id, name FROM worker_credential WHERE public_id = ?1 AND revoked_at IS NULL",
                [public_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let Some((id, name)) = row else {
            return Ok(None);
        };
        conn.execute(
            "UPDATE worker_credential SET revoked_at = ?1, revoked_by = ?2 WHERE id = ?3",
            rusqlite::params![now, actor, id],
        )?;
        audit_entity(
            conn,
            now,
            actor,
            "worker_revoke",
            "worker_credential",
            id,
            &name,
        )?;
        Ok(Some(now))
    })
}

/// Who a presented token speaks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Authn {
    Active { public_id: String, name: String },
    Revoked,
    Unknown,
}

/// Check a presented `w2.<id>.<secret>` token.
pub fn authenticate(conn: &Connection, token: &str) -> Result<Authn> {
    let Some((public_id, secret)) = token
        .strip_prefix("w2.")
        .and_then(|rest| rest.split_once('.'))
    else {
        return Ok(Authn::Unknown);
    };
    if !is_hex(public_id, 32) || !is_hex(secret, 64) {
        return Ok(Authn::Unknown);
    }
    let row: Option<(String, String, Option<i64>)> = conn
        .query_row(
            "SELECT name, token_hash, revoked_at FROM worker_credential WHERE public_id = ?1",
            [public_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    let Some((name, token_hash, revoked_at)) = row else {
        return Ok(Authn::Unknown);
    };
    if !digest_eq(&sha256_hex(token.as_bytes()), &token_hash) {
        return Ok(Authn::Unknown);
    }
    Ok(if revoked_at.is_some() {
        Authn::Revoked
    } else {
        Authn::Active {
            public_id: public_id.to_string(),
            name,
        }
    })
}

/// Whether a credential is still active (the per-ping re-check).
pub fn is_active(conn: &Connection, public_id: &str) -> Result<bool> {
    Ok(conn
        .query_row(
            "SELECT 1 FROM worker_credential WHERE public_id = ?1 AND revoked_at IS NULL",
            [public_id],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

/// Record that a credential connected (`hello` accepted).
pub fn touch(conn: &Connection, public_id: &str, version: &str, now: i64) -> Result<()> {
    conn.execute(
        "UPDATE worker_credential SET last_seen_at = ?1, last_version = ?2 WHERE public_id = ?3",
        rusqlite::params![now, version, public_id],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::work;

    fn db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        work::migrate(&conn).unwrap();
        conn
    }

    const SECRET: &str = "0101010101010101010101010101010101010101010101010101010101010101";

    fn file(conn: &Connection, ip: &str, now: i64) -> std::result::Result<Filed, Refusal> {
        let poll_hash = sha256_hex(&decode_hex(SECRET).unwrap());
        request(
            conn,
            &NewRequest {
                username: "chandra",
                hostname: "plant",
                version: "0.1.0+abc1234",
                started_at: now - 5,
                capabilities_json: "[\"scrape.urls\"]",
                peer_ip: ip,
                poll_hash: &poll_hash,
                now,
            },
        )
        .unwrap()
    }

    fn actions(conn: &Connection) -> Vec<String> {
        let mut stmt = conn
            .prepare("SELECT action FROM audit ORDER BY id")
            .unwrap();
        stmt.query_map([], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<Vec<String>>>()
            .unwrap()
    }

    #[test]
    fn names_follow_the_rule() {
        assert!(valid_name("chandra@plant"));
        assert!(valid_name("chandra@plant 2026-11-04"));
        assert!(valid_name("a.b_c-d@host-1.lan"));
        for bad in [
            "plant",
            "Chandra@plant",
            "chandra@plant:1",
            "chandra@plant 2026-1-04",
            "chandra@plant  2026-11-04",
            "@plant",
            "chandra@",
            "a@b c",
        ] {
            assert!(!valid_name(bad), "{bad}");
        }
    }

    #[test]
    fn codes_fold_look_alikes() {
        assert_eq!(normalize_code("k7qf-2m"), "K7QF2M");
        assert_eq!(normalize_code("O1IL 0o"), "011100");
        let code = new_code();
        assert_eq!(code.len(), 7);
        assert_eq!(&code[4..5], "-");
    }

    #[test]
    fn approve_then_poll_delivers_the_token_once() {
        let conn = db();
        let filed = file(&conn, "203.0.113.5", 1000).unwrap();
        assert_eq!(
            poll(&conn, &filed.public_id, SECRET, 1001).unwrap(),
            Poll::Pending {
                expires_at: 1000 + ENROL_TTL_SECS
            }
        );
        // A wrong poll secret looks like an unknown id.
        assert_eq!(
            poll(&conn, &filed.public_id, &"02".repeat(32), 1001).unwrap(),
            Poll::NotFound
        );
        assert_eq!(pending(&conn, 1001).unwrap().len(), 1);
        let lower = filed.code.to_ascii_lowercase();
        assert_eq!(
            approve(
                &conn,
                &filed.public_id,
                &lower,
                "chandra@plant",
                "staff-1",
                1002
            )
            .unwrap(),
            Decision::Approved {
                name: "chandra@plant".into()
            }
        );
        let Poll::Approved { name, token } = poll(&conn, &filed.public_id, SECRET, 1003).unwrap()
        else {
            panic!("expected the token");
        };
        assert_eq!(name, "chandra@plant");
        assert_eq!(
            poll(&conn, &filed.public_id, SECRET, 1004).unwrap(),
            Poll::Gone("delivered")
        );
        // Only the hash is stored.
        let stored: String = conn
            .query_row("SELECT token_hash FROM worker_credential", [], |r| r.get(0))
            .unwrap();
        assert_ne!(stored, token);
        let Authn::Active { public_id, name } = authenticate(&conn, &token).unwrap() else {
            panic!("token admits");
        };
        assert_eq!(name, "chandra@plant");
        let creds = credentials(&conn, false).unwrap();
        assert_eq!(creds[0].created_by, "staff-1");
        assert_eq!(
            authenticate(&conn, &format!("{}0", &token[..token.len() - 1])).unwrap(),
            Authn::Unknown
        );
        assert!(is_active(&conn, &public_id).unwrap());
        assert_eq!(
            revoke(&conn, &public_id, "staff-1", 1005).unwrap(),
            Some(1005)
        );
        assert_eq!(revoke(&conn, &public_id, "staff-1", 1006).unwrap(), None);
        assert_eq!(authenticate(&conn, &token).unwrap(), Authn::Revoked);
        assert!(!is_active(&conn, &public_id).unwrap());
        assert!(credentials(&conn, false).unwrap().is_empty());
        assert_eq!(credentials(&conn, true).unwrap().len(), 1);
        assert_eq!(
            actions(&conn),
            [
                "worker_enrol_request",
                "worker_enrol_approve",
                "worker_token_delivered",
                "worker_revoke"
            ]
        );
    }

    #[test]
    fn three_wrong_codes_deny() {
        let conn = db();
        let filed = file(&conn, "203.0.113.5", 1000).unwrap();
        let wrong = if filed.code == "AAAA-AA" {
            "BBBB-BB"
        } else {
            "AAAA-AA"
        };
        assert_eq!(
            approve(&conn, &filed.public_id, wrong, "chandra@plant", "s", 1001).unwrap(),
            Decision::WrongCode { tries_left: 2 }
        );
        approve(&conn, &filed.public_id, wrong, "chandra@plant", "s", 1001).unwrap();
        assert_eq!(
            approve(&conn, &filed.public_id, wrong, "chandra@plant", "s", 1001).unwrap(),
            Decision::DeniedAfterTries
        );
        assert_eq!(
            approve(
                &conn,
                &filed.public_id,
                &filed.code,
                "chandra@plant",
                "s",
                1002
            )
            .unwrap(),
            Decision::NotPending
        );
        assert_eq!(
            poll(&conn, &filed.public_id, SECRET, 1003).unwrap(),
            Poll::Gone("denied")
        );
    }

    #[test]
    fn limits_expiry_and_name_clashes() {
        let conn = db();
        for _ in 0..3 {
            file(&conn, "203.0.113.5", 1000).unwrap();
        }
        assert_eq!(
            file(&conn, "203.0.113.5", 1000),
            Err(Refusal::TooManyFromIp)
        );
        file(&conn, "203.0.113.6", 1000).unwrap();
        file(&conn, "203.0.113.7", 1000).unwrap();
        assert_eq!(
            file(&conn, "203.0.113.8", 1000),
            Err(Refusal::TooManyPending)
        );
        // Expired requests free the pending slots but still count per IP.
        let later = 1000 + ENROL_TTL_SECS + 1;
        assert!(pending(&conn, later).unwrap().is_empty());
        assert_eq!(
            file(&conn, "203.0.113.5", later),
            Err(Refusal::TooManyFromIp)
        );
        let filed = file(&conn, "203.0.113.9", later).unwrap();
        let old = pending(&conn, 1001).unwrap()[0].public_id.clone();
        assert_eq!(
            poll(&conn, &old, SECRET, later).unwrap(),
            Poll::Gone("expired")
        );
        // A name held by an active credential is refused, freed by revoke.
        let token = mint_direct(&conn, "chandra@plant", "xtask-up", later).unwrap();
        assert_eq!(
            approve(
                &conn,
                &filed.public_id,
                &filed.code,
                "chandra@plant",
                "s",
                later
            )
            .unwrap(),
            Decision::NameTaken
        );
        assert_eq!(
            approve(
                &conn,
                &filed.public_id,
                &filed.code,
                "chandra@plant 2026-11-04",
                "s",
                later
            )
            .unwrap(),
            Decision::Approved {
                name: "chandra@plant 2026-11-04".into()
            }
        );
        // Minting again revokes the old local credential.
        mint_direct(&conn, "chandra@plant", "xtask-up", later).unwrap();
        assert_eq!(authenticate(&conn, &token).unwrap(), Authn::Revoked);
        assert_eq!(
            approve(&conn, &filed.public_id, &filed.code, "bad name", "s", later).unwrap(),
            Decision::BadName
        );
    }

    #[test]
    fn invalid_requests_are_refused() {
        let conn = db();
        let bad = request(
            &conn,
            &NewRequest {
                username: "Chandra",
                hostname: "plant",
                version: "",
                started_at: 0,
                capabilities_json: "[]",
                peer_ip: "x",
                poll_hash: "00",
                now: 1,
            },
        )
        .unwrap();
        assert_eq!(bad, Err(Refusal::Invalid));
    }
}
