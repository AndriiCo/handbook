//! Passkey storage + account linking (spec `customer-auth.md` §1).
//!
//! WebAuthn types stay in `web`; this module sees JSON blobs,
//! credential ids, and rule primitives. Counters live inside the
//! stored `Passkey` JSON and move via the crate's
//! `update_credential` flow — there is deliberately no counter
//! column to drift (spec decision 0.5).

use rusqlite::{Connection, OptionalExtension};

/// Challenge TTL, seconds (5 min, single-use).
pub const CHALLENGE_TTL_SECS: i64 = 300;

/// Step-up / re-auth freshness window, seconds (10 min).
pub const STEP_UP_WINDOW_SECS: i64 = 600;

/// Challenge kinds (spec §1.4).
pub const KIND_REGISTER: &str = "register";
pub const KIND_LOGIN: &str = "login";
/// Reserved for Phase-4 conditional mediation (spec §2.5).
#[allow(dead_code)]
pub const KIND_DISCOVERABLE: &str = "discoverable";

/// Fresh 256-bit challenge id, hex (same shape as session ids).
pub fn mint_challenge_id() -> String {
    crate::mint_session_id()
}

/// Fresh opaque user id: 16 random bytes, hex (also the
/// WebAuthn user handle — `Uuid::parse_str` accepts this form).
pub fn mint_user_id() -> String {
    use rand::RngCore as _;
    let mut bytes = [0u8; 16];
    rand::rng().fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub struct NewChallenge<'a> {
    pub kind: &'a str,
    pub user_id: Option<&'a str>,
    pub state_json: &'a str,
    pub now: i64,
}

/// Store one ceremony state; caller swept expired rows first.
pub fn store_challenge(conn: &Connection, id: &str, c: &NewChallenge<'_>) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO webauthn_challenge(id, user_id, kind, state_json, created_at, expires_at, consumed_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL)",
        rusqlite::params![id, c.user_id, c.kind, c.state_json, c.now, c.now + CHALLENGE_TTL_SECS],
    )?;
    Ok(())
}

/// One `webauthn_challenge` row as selected.
type ChallengeRow = (Option<String>, String, String, i64, Option<i64>);

/// Take a challenge exactly once: missing/consumed/expired/wrong
/// kind → `None` (fail closed; caller maps to 410).
pub fn take_challenge(
    conn: &Connection,
    id: &str,
    expected_kind: &str,
    now: i64,
) -> rusqlite::Result<Option<(Option<String>, String)>> {
    let row: Option<ChallengeRow> = conn
        .query_row(
            "SELECT user_id, kind, state_json, expires_at, consumed_at
             FROM webauthn_challenge WHERE id = ?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .optional()?;
    let Some((user_id, kind, state, expires_at, consumed)) = row else {
        return Ok(None);
    };
    if consumed.is_some() || now >= expires_at || kind != expected_kind {
        return Ok(None);
    }
    conn.execute(
        "UPDATE webauthn_challenge SET consumed_at = ?1 WHERE id = ?2",
        rusqlite::params![now, id],
    )?;
    Ok(Some((user_id, state)))
}

/// Delete expired challenges; returns rows removed.
pub fn sweep_challenges(conn: &Connection, now: i64) -> rusqlite::Result<usize> {
    conn.execute(
        "DELETE FROM webauthn_challenge WHERE expires_at <= ?1",
        [now],
    )
}

/// True when challenge `id` was consumed by `user_id` within the
/// step-up window (unlink re-auth proof). Anything else → false.
pub fn proof_fresh(conn: &Connection, id: &str, user_id: &str, now: i64) -> rusqlite::Result<bool> {
    let row: Option<(Option<String>, Option<i64>)> = conn
        .query_row(
            "SELECT user_id, consumed_at FROM webauthn_challenge WHERE id = ?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    Ok(
        matches!(row, Some((Some(owner), Some(at))) if owner == user_id && now - at <= STEP_UP_WINDOW_SECS),
    )
}

/// One account row (spec §1.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Account {
    pub user_id: String,
    pub role: String,
    pub locked_at: Option<i64>,
    pub step_up_at: Option<i64>,
    pub fallback_until: Option<i64>,
}

/// Create the account if missing (first registration); existing
/// rows are untouched.
pub fn ensure_account(conn: &Connection, user_id: &str, now: i64) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO account(user_id, role, customer_ref, created_at, locked_at, step_up_at, fallback_until)
         VALUES (?1, 'customer', NULL, ?2, NULL, NULL, NULL)
         ON CONFLICT(user_id) DO NOTHING",
        rusqlite::params![user_id, now],
    )?;
    Ok(())
}

pub fn get_account(conn: &Connection, user_id: &str) -> rusqlite::Result<Option<Account>> {
    conn.query_row(
        "SELECT user_id, role, locked_at, step_up_at, fallback_until
         FROM account WHERE user_id = ?1",
        [user_id],
        |r| {
            Ok(Account {
                user_id: r.get(0)?,
                role: r.get(1)?,
                locked_at: r.get(2)?,
                step_up_at: r.get(3)?,
                fallback_until: r.get(4)?,
            })
        },
    )
    .optional()
}

/// Record a UV/step-up success (rule 4 + re-auth proofs).
pub fn set_step_up(conn: &Connection, user_id: &str, now: i64) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE account SET step_up_at = ?1 WHERE user_id = ?2",
        rusqlite::params![now, user_id],
    )?;
    Ok(())
}

/// Clear the magic-link fallback window (passkey login worked).
pub fn clear_fallback(conn: &Connection, user_id: &str) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE account SET fallback_until = NULL WHERE user_id = ?1",
        [user_id],
    )?;
    Ok(())
}

/// Lock the account (rule 6 tail: last verified link revoked).
pub fn lock_account(conn: &Connection, user_id: &str, now: i64) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE account SET locked_at = ?1 WHERE user_id = ?2",
        rusqlite::params![now, user_id],
    )?;
    Ok(())
}

/// Insert a credential + its `identity_link` companion row
/// atomically (rule 1). A globally duplicate credential id is an
/// `Err` (caller maps to 409 + audit).
pub fn store_credential(
    conn: &Connection,
    user_id: &str,
    credential_id: &str,
    passkey_json: &str,
    label: &str,
    now: i64,
) -> rusqlite::Result<()> {
    conn.execute_batch("BEGIN IMMEDIATE")?;
    let done = (|| {
        conn.execute(
            "INSERT INTO webauthn_credential(credential_id, user_id, passkey_json, label, created_at, last_used_at, revoked_at)
             VALUES (?1, ?2, ?3, ?4, ?5, NULL, NULL)",
            rusqlite::params![credential_id, user_id, passkey_json, label, now],
        )?;
        conn.execute(
            "INSERT INTO identity_link(user_id, method, provider_sub, email_hash, verified_email, is_relay, disabled_at)
             VALUES (?1, 'passkey', ?2, NULL, 0, 0, NULL)",
            rusqlite::params![user_id, credential_id],
        )?;
        Ok::<_, rusqlite::Error>(())
    })();
    match done {
        Ok(()) => {
            conn.execute_batch("COMMIT")?;
            Ok(())
        }
        Err(e) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(e)
        }
    }
}

/// True when this credential id exists on ANY account.
pub fn credential_known(conn: &Connection, credential_id: &str) -> rusqlite::Result<bool> {
    Ok(conn
        .query_row(
            "SELECT 1 FROM webauthn_credential WHERE credential_id = ?1",
            [credential_id],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

/// Non-revoked `(credential_id, passkey_json)` for one user, oldest
/// first (login/start allow-list).
pub fn credentials_for_user(
    conn: &Connection,
    user_id: &str,
) -> rusqlite::Result<Vec<(String, String)>> {
    let mut stmt = conn.prepare(
        "SELECT credential_id, passkey_json FROM webauthn_credential
         WHERE user_id = ?1 AND revoked_at IS NULL ORDER BY created_at",
    )?;
    stmt.query_map([user_id], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()
}

/// Owner + JSON for one non-revoked credential id.
pub fn credential_owner(
    conn: &Connection,
    credential_id: &str,
) -> rusqlite::Result<Option<(String, String)>> {
    conn.query_row(
        "SELECT user_id, passkey_json FROM webauthn_credential
         WHERE credential_id = ?1 AND revoked_at IS NULL",
        [credential_id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
    .optional()
}

/// Re-persist a `Passkey` JSON after `update_credential` changed it.
pub fn update_passkey_json(
    conn: &Connection,
    credential_id: &str,
    passkey_json: &str,
    now: i64,
) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE webauthn_credential SET passkey_json = ?1, last_used_at = ?2 WHERE credential_id = ?3",
        rusqlite::params![passkey_json, now, credential_id],
    )?;
    Ok(())
}

/// Revoke one credential (sets both `revoked_at` and the link's
/// `disabled_at`); returns rows touched.
pub fn revoke_credential(
    conn: &Connection,
    user_id: &str,
    credential_id: &str,
    now: i64,
) -> rusqlite::Result<usize> {
    conn.execute(
        "UPDATE webauthn_credential SET revoked_at = ?1
         WHERE user_id = ?2 AND credential_id = ?3 AND revoked_at IS NULL",
        rusqlite::params![now, user_id, credential_id],
    )?;
    conn.execute(
        "UPDATE identity_link SET disabled_at = ?1
         WHERE user_id = ?2 AND method = 'passkey' AND provider_sub = ?3 AND disabled_at IS NULL",
        rusqlite::params![now, user_id, credential_id],
    )
}

/// Disable one non-passkey identity link (OAuth unlink path;
/// passkeys go through [`revoke_credential`]); returns rows touched.
pub fn disable_link(
    conn: &Connection,
    user_id: &str,
    method: &str,
    provider_sub: &str,
    now: i64,
) -> rusqlite::Result<usize> {
    conn.execute(
        "UPDATE identity_link SET disabled_at = ?1
         WHERE user_id = ?2 AND method = ?3 AND provider_sub = ?4 AND disabled_at IS NULL",
        rusqlite::params![now, user_id, method, provider_sub],
    )
}

/// Live (non-disabled) method count for rule 6.
pub fn live_method_count(conn: &Connection, user_id: &str) -> rusqlite::Result<usize> {
    conn.query_row(
        "SELECT COUNT(*) FROM identity_link WHERE user_id = ?1 AND disabled_at IS NULL",
        [user_id],
        |r| r.get(0),
    )
}

/// Non-disabled identity links for the profile list.
pub fn links_for_user(conn: &Connection, user_id: &str) -> rusqlite::Result<Vec<(String, String)>> {
    let mut stmt = conn.prepare(
        "SELECT method, provider_sub FROM identity_link
         WHERE user_id = ?1 AND disabled_at IS NULL ORDER BY method, provider_sub",
    )?;
    stmt.query_map([user_id], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()
}

/// Label for one credential (profile display; empty when unknown).
pub fn credential_label(conn: &Connection, credential_id: &str) -> rusqlite::Result<String> {
    Ok(conn
        .query_row(
            "SELECT label FROM webauthn_credential WHERE credential_id = ?1",
            [credential_id],
            |r| r.get(0),
        )
        .optional()?
        .unwrap_or_default())
}

/// Owner of a verified, non-relay, non-Meta email (rule 2–3 match
/// surface for the duplicate-email hint). Relay/Meta/unverified
/// rows never match.
pub fn usable_email_owner(conn: &Connection, email: &str) -> rusqlite::Result<Option<String>> {
    let hash = crate::hash_str(&email.to_lowercase());
    conn.query_row(
        "SELECT user_id FROM identity_link
         WHERE email_hash = ?1 AND verified_email = 1
           AND is_relay = 0 AND method != 'meta' AND disabled_at IS NULL
         LIMIT 1",
        [hash],
        |r| r.get(0),
    )
    .optional()
}

/// Owner of ANY non-disabled `email` link (verified or not):
/// login allow-list resolution only, never OAuth matching (rule 2
/// still demands verified emails there). Callers return the same
/// 401 for unknown either way — no enumeration.
pub fn account_by_any_email(conn: &Connection, email: &str) -> rusqlite::Result<Option<String>> {
    let hash = crate::hash_str(&email.to_lowercase());
    conn.query_row(
        "SELECT user_id FROM identity_link
         WHERE email_hash = ?1 AND method = 'email' AND disabled_at IS NULL
         LIMIT 1",
        [hash],
        |r| r.get(0),
    )
    .optional()
}

/// Rule 5 router for passkey registration: passkeys carry no email,
/// so registration always mints (mode a) or links (mode b, caller
/// proved the session + step-up). `usable_email_match` is accepted
/// for shape-parity with OAuth/magic flows and must be false here.
pub fn registration_decision(
    link_session_user: Option<&str>,
    new_user_id: &str,
    step_up_ok: bool,
) -> Result<crate::login::NewIdentityDecision, &'static str> {
    match link_session_user {
        Some(uid) if uid == new_user_id && crate::login::explicit_link_permitted(step_up_ok) => {
            Ok(crate::login::NewIdentityDecision::CreateFreshAccount)
        }
        Some(_) => Err("link session mismatch or step-up failed"),
        None => Ok(crate::login::decide_unknown_sub(false)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn memory() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        crate::migrate_auth_tables(&conn).unwrap();
        conn
    }

    fn challenge(user: Option<&'static str>) -> NewChallenge<'static> {
        NewChallenge {
            kind: KIND_REGISTER,
            user_id: user,
            state_json: "{}",
            now: 1000,
        }
    }

    #[test]
    fn user_ids_are_32_lowercase_hex() {
        for _ in 0..8 {
            let id = mint_user_id();
            assert_eq!(id.len(), 32);
            assert!(id.bytes().all(|b| b.is_ascii_hexdigit()));
            assert_eq!(id, id.to_lowercase());
        }
        assert_ne!(mint_user_id(), mint_user_id());
    }

    #[test]
    fn challenge_take_once_then_gone() {
        let conn = memory();
        store_challenge(&conn, "c-1", &challenge(Some("u-1"))).unwrap();
        let taken = take_challenge(&conn, "c-1", KIND_REGISTER, 1100).unwrap();
        assert_eq!(taken, Some((Some("u-1".to_string()), "{}".to_string())));
        assert_eq!(
            take_challenge(&conn, "c-1", KIND_REGISTER, 1100).unwrap(),
            None
        );
        assert_eq!(
            take_challenge(&conn, "missing", KIND_REGISTER, 1100).unwrap(),
            None
        );
    }

    #[test]
    fn challenge_rejects_expiry_and_wrong_kind() {
        let conn = memory();
        store_challenge(&conn, "c-old", &challenge(None)).unwrap();
        assert_eq!(
            take_challenge(&conn, "c-old", KIND_REGISTER, 1000 + CHALLENGE_TTL_SECS).unwrap(),
            None
        );
        store_challenge(&conn, "c-kind", &challenge(None)).unwrap();
        assert_eq!(
            take_challenge(&conn, "c-kind", KIND_LOGIN, 1100).unwrap(),
            None
        );
        assert_eq!(
            sweep_challenges(&conn, 1000 + CHALLENGE_TTL_SECS + 1).unwrap(),
            2
        );
    }

    #[test]
    fn credential_insert_is_globally_unique_and_linked() {
        let conn = memory();
        ensure_account(&conn, "u-1", 1000).unwrap();
        ensure_account(&conn, "u-2", 1000).unwrap();
        store_credential(&conn, "u-1", "cred-1", "{}", "Phone", 1000).unwrap();
        assert!(credential_known(&conn, "cred-1").unwrap());
        // Same id on another account fails; the failed insert leaves
        // no orphan identity row (atomic).
        assert!(store_credential(&conn, "u-2", "cred-1", "{}", "", 1000).is_err());
        assert_eq!(live_method_count(&conn, "u-2").unwrap(), 0);
        assert_eq!(live_method_count(&conn, "u-1").unwrap(), 1);
        assert_eq!(credential_label(&conn, "cred-1").unwrap(), "Phone");
        assert_eq!(
            credentials_for_user(&conn, "u-1").unwrap(),
            vec![("cred-1".to_string(), "{}".to_string())]
        );
        assert_eq!(
            credential_owner(&conn, "cred-1").unwrap(),
            Some(("u-1".to_string(), "{}".to_string()))
        );
    }

    #[test]
    fn revoke_drops_count_and_listing() {
        let conn = memory();
        ensure_account(&conn, "u-1", 1000).unwrap();
        store_credential(&conn, "u-1", "cred-1", "{}", "", 1000).unwrap();
        store_credential(&conn, "u-1", "cred-2", "{}", "", 1000).unwrap();
        assert_eq!(revoke_credential(&conn, "u-1", "cred-1", 1100).unwrap(), 1);
        assert_eq!(live_method_count(&conn, "u-1").unwrap(), 1);
        assert_eq!(
            links_for_user(&conn, "u-1").unwrap(),
            vec![("passkey".to_string(), "cred-2".to_string())]
        );
        assert!(
            credentials_for_user(&conn, "u-1")
                .unwrap()
                .iter()
                .all(|(id, _)| id == "cred-2")
        );
        assert_eq!(credential_owner(&conn, "cred-1").unwrap(), None);
    }

    #[test]
    fn proof_covers_consumed_recent_own_only() {
        let conn = memory();
        store_challenge(&conn, "c-1", &challenge(Some("u-1"))).unwrap();
        assert!(!proof_fresh(&conn, "c-1", "u-1", 1100).unwrap()); // not consumed yet
        take_challenge(&conn, "c-1", KIND_REGISTER, 1100).unwrap();
        assert!(proof_fresh(&conn, "c-1", "u-1", 1100).unwrap());
        assert!(!proof_fresh(&conn, "c-1", "u-2", 1100).unwrap());
        assert!(!proof_fresh(&conn, "c-1", "u-1", 1100 + STEP_UP_WINDOW_SECS + 1).unwrap());
        assert!(!proof_fresh(&conn, "missing", "u-1", 1100).unwrap());
    }

    #[test]
    fn account_flags_roundtrip() {
        let conn = memory();
        ensure_account(&conn, "u-1", 1000).unwrap();
        ensure_account(&conn, "u-1", 1000).unwrap(); // idempotent
        set_step_up(&conn, "u-1", 1100).unwrap();
        lock_account(&conn, "u-1", 1200).unwrap();
        clear_fallback(&conn, "u-1").unwrap();
        let account = get_account(&conn, "u-1").unwrap().unwrap();
        assert_eq!(account.step_up_at, Some(1100));
        assert_eq!(account.locked_at, Some(1200));
        assert_eq!(account.fallback_until, None);
        assert_eq!(get_account(&conn, "nobody").unwrap(), None);
    }

    #[test]
    fn email_match_ignores_relay_meta_and_unverified() {
        let conn = memory();
        let apple = crate::hash_str("a@example.test");
        for (user, method, relay, verified) in [
            ("u-apple", "apple", 0, 1),
            ("u-relay", "apple", 1, 1),
            ("u-meta", "meta", 0, 1),
            ("u-unver", "google", 0, 0),
        ] {
            conn.execute(
                "INSERT INTO identity_link(user_id, method, provider_sub, email_hash, verified_email, is_relay, disabled_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL)",
                rusqlite::params![user, method, format!("sub-{user}"), apple, verified, relay],
            )
            .unwrap();
        }
        assert_eq!(
            usable_email_owner(&conn, "A@EXAMPLE.TEST").unwrap(),
            Some("u-apple".to_string())
        );
        assert_eq!(
            usable_email_owner(&conn, "nobody@example.test").unwrap(),
            None
        );
    }

    #[test]
    fn registration_router_never_merges() {
        use crate::login::NewIdentityDecision;
        assert_eq!(
            registration_decision(None, "new-1", false).unwrap(),
            NewIdentityDecision::CreateFreshAccount
        );
        assert_eq!(
            registration_decision(Some("u-1"), "u-1", true).unwrap(),
            NewIdentityDecision::CreateFreshAccount
        );
        assert!(registration_decision(Some("u-1"), "u-1", false).is_err());
        assert!(registration_decision(Some("u-1"), "u-2", true).is_err());
    }
}
