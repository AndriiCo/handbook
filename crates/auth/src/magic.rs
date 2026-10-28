//! Magic-link fallback (spec `customer-auth.md` §12): token store,
//! link build, mail send. HTTP(SMTP) goes through [`Mailer`] so
//! tests run offline; `web` provides the live impls. Tokens are
//! Bearer: the DB keeps hashes only, and tokens never appear in
//! URLs' query strings (access logs record full URIs) — the
//! emailed link carries the token in the fragment (`#t=`).

use rusqlite::{Connection, OptionalExtension as _};

/// Minimal send surface. Secrets/links never appear in errors
/// (mirrors the `HttpTransport` precedent: test doubles in, live
/// transports in `web`).
pub trait Mailer {
    fn send(&self, to: &str, subject: &str, text_body: &str) -> Result<(), MailError>;
}

/// Send failure, static-shape only: bodies are dropped because
/// provider errors can echo the link or the address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MailError {
    /// Transport failure (connect/auth/IO); the String is a
    /// static-shape tag, never a provider body.
    Transport(String),
    /// Provider refused the message (SMTP 5xx / API 4xx-5xx);
    /// status kept for audit, body dropped.
    Rejected(u16),
}

/// Subject line for the magic-link message (single template).
pub const MAGIC_SUBJECT: &str = "Your Apostille Handbook sign-in link";

/// `https://host/apostille/login#t=<token>` — fragment, so
/// the token never touches server logs (see `request_id_and_log`).
/// Returns `None` unless the token is 64 lowercase-or-not hex.
pub fn build_link(link_base: &str, token: &str) -> Option<String> {
    let ok = token.len() == 64 && token.bytes().all(|b| b.is_ascii_hexdigit());
    if !ok {
        return None;
    }
    Some(format!("{}#t={token}", link_base.trim_end_matches('/')))
}

/// Pure composition — unit-tested without any mailer. No
/// account-existence language: the same text fits every send.
pub fn compose_body(link: &str) -> String {
    format!(
        "Sign in to Apostille Handbook (this link expires in 15 minutes and works once):\n\n{link}\n\nIf you did not ask for this, ignore this message."
    )
}

/// Request-time send; the caller resolved + stored the token
/// already. Pure composition over an injected [`Mailer`].
pub fn send_magic_link<M: Mailer>(
    mailer: &M,
    to: &str,
    link_base: &str,
    token: &str,
) -> Result<(), MailError> {
    let link =
        build_link(link_base, token).ok_or_else(|| MailError::Transport("bad token".into()))?;
    mailer.send(to, MAGIC_SUBJECT, &compose_body(&link))
}

/// Store a fresh token row (hash only; the raw email is hashed
/// inside, never stored). The caller invalidated + swept first.
pub fn store_magic_token(
    conn: &Connection,
    token_hash: &str,
    email: &str,
    now: i64,
) -> rusqlite::Result<()> {
    let email_hash = crate::hash_str(&email.to_lowercase());
    conn.execute(
        "INSERT INTO magic_token(token_hash, email_hash, created_at, expires_at, consumed_at)
         VALUES (?1, ?2, ?3, ?4, NULL)",
        rusqlite::params![
            token_hash,
            email_hash,
            now,
            now.saturating_add(crate::login::MAGIC_LINK_TTL_SECS),
        ],
    )?;
    Ok(())
}

/// Atomically take a live token (take-and-burn): guarded UPDATE
/// (the `consume_invite` shape), then read back the `email_hash`
/// from the row we just consumed. Missing/consumed/expired →
/// `Ok(None)` (one shape — no replay-vs-expired oracle).
pub fn take_magic_token(
    conn: &Connection,
    token_hash: &str,
    now: i64,
) -> rusqlite::Result<Option<String>> {
    let touched = conn.execute(
        "UPDATE magic_token SET consumed_at = ?1
         WHERE token_hash = ?2 AND consumed_at IS NULL AND expires_at > ?1",
        rusqlite::params![now, token_hash],
    )?;
    if touched != 1 {
        return Ok(None);
    }
    let email_hash: String = conn.query_row(
        "SELECT email_hash FROM magic_token WHERE token_hash = ?1",
        [token_hash],
        |r| r.get(0),
    )?;
    Ok(Some(email_hash))
}

/// Why a take failed (peek for the audit reason — the take
/// itself is uniform-`None` by design, so the response stays a
/// single 410 with no replay-vs-expired oracle).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenState {
    Live,
    Consumed,
    Expired,
    Missing,
}

/// Classify one row without touching it (failure-path only).
pub fn inspect_magic_token(
    conn: &Connection,
    token_hash: &str,
    now: i64,
) -> rusqlite::Result<TokenState> {
    let row: Option<(i64, Option<i64>)> = conn
        .query_row(
            "SELECT expires_at, consumed_at FROM magic_token WHERE token_hash = ?1",
            [token_hash],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    Ok(match row {
        None => TokenState::Missing,
        Some((_, Some(_))) => TokenState::Consumed,
        Some((expires_at, None)) if expires_at <= now => TokenState::Expired,
        Some(_) => TokenState::Live,
    })
}

/// Delete expired rows; returns the count (sweep runs on every
/// request, mirroring `sweep_challenges`).
pub fn sweep_magic_tokens(conn: &Connection, now: i64) -> rusqlite::Result<usize> {
    let touched = conn.execute("DELETE FROM magic_token WHERE expires_at <= ?1", [now])?;
    Ok(touched)
}

/// Burn outstanding rows for one address (re-request supersedes:
/// exactly one live token per address). Burned rows keep their
/// `consumed_at` until the sweep reaps them by expiry.
pub fn invalidate_email_tokens(
    conn: &Connection,
    email: &str,
    now: i64,
) -> rusqlite::Result<usize> {
    let email_hash = crate::hash_str(&email.to_lowercase());
    let touched = conn.execute(
        "UPDATE magic_token SET consumed_at = ?1
         WHERE email_hash = ?2 AND consumed_at IS NULL",
        rusqlite::params![now, email_hash],
    )?;
    Ok(touched)
}

/// Delete one row outright (send failure: no dangling token the
/// user can never receive — re-request mints fresh).
pub fn delete_magic_token(conn: &Connection, token_hash: &str) -> rusqlite::Result<()> {
    conn.execute(
        "DELETE FROM magic_token WHERE token_hash = ?1",
        [token_hash],
    )?;
    Ok(())
}

/// Distinct live owners of one email hash (any method, any
/// verified flag, including relay; Meta hashes are NULL and never
/// match). The owner is resolved fresh at consume (§12.5), so a
/// lock/disable landing between request and consume fails closed.
pub fn owners_by_email_hash(conn: &Connection, email_hash: &str) -> rusqlite::Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT DISTINCT user_id FROM identity_link
         WHERE email_hash = ?1 AND disabled_at IS NULL",
    )?;
    let owners = stmt
        .query_map([email_hash], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(owners)
}

/// Rule-2 flip: inbox control proven by the consume, so the
/// unverified hint rows for this owner+address become verified
/// (passkey register leaves them unverified, §2.2). Returns the
/// flipped count. Never inserts — no `magic_link` rows in v1.
pub fn verify_email_rows(
    conn: &Connection,
    user_id: &str,
    email_hash: &str,
) -> rusqlite::Result<usize> {
    let touched = conn.execute(
        "UPDATE identity_link SET verified_email = 1
         WHERE user_id = ?1 AND email_hash = ?2
           AND verified_email = 0 AND disabled_at IS NULL",
        rusqlite::params![user_id, email_hash],
    )?;
    Ok(touched)
}

/// Arm the fallback window (magic login worked); mirrors
/// `set_step_up`/`clear_fallback`.
pub fn set_fallback(conn: &Connection, user_id: &str, until: i64) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE account SET fallback_until = ?1 WHERE user_id = ?2",
        rusqlite::params![until, user_id],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn memory() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        crate::migrate_auth_tables(&conn).unwrap();
        conn
    }

    fn link(conn: &Connection, user: &str, method: &str, email: &str, verified: i64) {
        let hash = crate::hash_str(&email.to_lowercase());
        conn.execute(
            "INSERT INTO identity_link(user_id, method, provider_sub, email_hash, verified_email, is_relay, disabled_at)
             VALUES (?1, ?2, ?3, ?4, ?5, 0, NULL)",
            rusqlite::params![
                user,
                method,
                format!("sub-{user}-{method}-{email}"),
                hash,
                verified
            ],
        )
        .unwrap();
    }

    #[test]
    fn build_link_ok_shape_and_rejects_other() {
        let token = "ab".repeat(32);
        assert_eq!(
            build_link("https://andrii.ai/apostille/login", &token).unwrap(),
            format!("https://andrii.ai/apostille/login#t={token}")
        );
        // Trailing slash tolerated; uppercase hex tolerated.
        assert!(
            build_link("https://andrii.ai/x/", &token.to_uppercase())
                .unwrap()
                .ends_with(&format!("#t={}", token.to_uppercase()))
        );
        for bad in ["", "xyz", &"ab".repeat(31), &"zz".repeat(32), "ab cd"] {
            assert_eq!(build_link("https://andrii.ai/x", bad), None, "{bad:?}");
        }
    }

    #[test]
    fn send_composes_subject_body_link() {
        struct Stub(std::cell::RefCell<Vec<(String, String, String)>>);
        impl Mailer for Stub {
            fn send(&self, to: &str, subject: &str, body: &str) -> Result<(), MailError> {
                self.0
                    .borrow_mut()
                    .push((to.into(), subject.into(), body.into()));
                Ok(())
            }
        }
        let stub = Stub(std::cell::RefCell::new(Vec::new()));
        let token = "cd".repeat(32);
        send_magic_link(
            &stub,
            "someone@example.test",
            "https://andrii.ai/apostille/login",
            &token,
        )
        .expect("send");
        let sent = stub.0.borrow();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].0, "someone@example.test");
        assert_eq!(sent[0].1, MAGIC_SUBJECT);
        assert!(sent[0].2.contains(&format!("#t={token}")), "{}", sent[0].2);
        assert!(sent[0].2.contains("15 minutes"), "{}", sent[0].2);
        // Bad token shape never reaches the transport.
        assert_eq!(
            send_magic_link(&stub, "a@b.test", "https://andrii.ai/x", "nope"),
            Err(MailError::Transport("bad token".into()))
        );
        assert_eq!(sent.len(), 1);
    }

    #[test]
    fn store_take_take_again_single_use() {
        let conn = memory();
        let token = crate::login::mint_magic_token();
        let hash = crate::login::token_hash(&token);
        store_magic_token(&conn, &hash, "Someone@Example.test", 1_000).unwrap();
        let want = crate::hash_str("someone@example.test");
        assert_eq!(take_magic_token(&conn, &hash, 1_100).unwrap(), Some(want));
        assert_eq!(take_magic_token(&conn, &hash, 1_100).unwrap(), None);
    }

    #[test]
    fn take_miss_and_expiry_are_none() {
        let conn = memory();
        assert_eq!(
            take_magic_token(&conn, &"aa".repeat(32), 1_000).unwrap(),
            None
        );
        let token = crate::login::mint_magic_token();
        let hash = crate::login::token_hash(&token);
        store_magic_token(&conn, &hash, "a@example.test", 1_000).unwrap();
        // expires_at is exclusive (> now required).
        assert_eq!(
            take_magic_token(&conn, &hash, 1_000 + crate::login::MAGIC_LINK_TTL_SECS).unwrap(),
            None
        );
    }

    #[test]
    fn inspect_classifies_without_touching() {
        let conn = memory();
        assert_eq!(
            inspect_magic_token(&conn, &"aa".repeat(32), 1_000).unwrap(),
            TokenState::Missing
        );
        let live = crate::login::token_hash(&crate::login::mint_magic_token());
        let dead = crate::login::token_hash(&crate::login::mint_magic_token());
        let old = crate::login::token_hash(&crate::login::mint_magic_token());
        store_magic_token(&conn, &live, "a@example.test", 1_000).unwrap();
        store_magic_token(&conn, &dead, "b@example.test", 1_000).unwrap();
        store_magic_token(&conn, &old, "c@example.test", 1_000).unwrap();
        take_magic_token(&conn, &dead, 1_100).unwrap();
        assert_eq!(
            inspect_magic_token(&conn, &live, 1_100).unwrap(),
            TokenState::Live
        );
        assert_eq!(
            inspect_magic_token(&conn, &dead, 1_100).unwrap(),
            TokenState::Consumed
        );
        assert_eq!(
            inspect_magic_token(&conn, &old, 1_000 + 900).unwrap(),
            TokenState::Expired
        );
        // The peek burned nothing: the live token still takes.
        assert!(take_magic_token(&conn, &live, 1_100).unwrap().is_some());
    }

    #[test]
    fn sweep_reaps_only_expired() {
        let conn = memory();
        let h1 = crate::login::token_hash(&crate::login::mint_magic_token());
        let h2 = crate::login::token_hash(&crate::login::mint_magic_token());
        store_magic_token(&conn, &h1, "a@example.test", 1_000).unwrap();
        store_magic_token(&conn, &h2, "b@example.test", 10_000).unwrap();
        assert_eq!(sweep_magic_tokens(&conn, 1_000 + 900).unwrap(), 1);
        assert_eq!(take_magic_token(&conn, &h1, 1_000 + 900).unwrap(), None);
        assert!(take_magic_token(&conn, &h2, 1_000 + 900).unwrap().is_some());
    }

    #[test]
    fn invalidate_burns_only_that_address() {
        let conn = memory();
        let h1 = crate::login::token_hash(&crate::login::mint_magic_token());
        let h2 = crate::login::token_hash(&crate::login::mint_magic_token());
        store_magic_token(&conn, &h1, "a@example.test", 1_000).unwrap();
        store_magic_token(&conn, &h2, "b@example.test", 1_000).unwrap();
        assert_eq!(
            invalidate_email_tokens(&conn, "a@example.test", 1_100).unwrap(),
            1
        );
        assert_eq!(take_magic_token(&conn, &h1, 1_100).unwrap(), None);
        assert!(take_magic_token(&conn, &h2, 1_100).unwrap().is_some());
    }

    #[test]
    fn storage_holds_hashes_only() {
        let conn = memory();
        let token = crate::login::mint_magic_token();
        let hash = crate::login::token_hash(&token);
        store_magic_token(&conn, &hash, "Someone@Example.test", 1_000).unwrap();
        let (stored_hash, stored_email): (String, String) = conn
            .query_row("SELECT token_hash, email_hash FROM magic_token", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap();
        assert_eq!(stored_hash, hash);
        assert_ne!(stored_hash, token);
        assert_eq!(stored_email, crate::hash_str("someone@example.test"));
    }

    #[test]
    fn owners_distinct_live_only() {
        let conn = memory();
        let hash = crate::hash_str("dup@example.test");
        link(&conn, "u1", "email", "dup@example.test", 0);
        link(&conn, "u1", "google", "dup@example.test", 1);
        link(&conn, "u2", "apple", "dup@example.test", 1);
        // Disabled rows never resolve.
        conn.execute(
            "INSERT INTO identity_link(user_id, method, provider_sub, email_hash, verified_email, is_relay, disabled_at)
             VALUES ('u3', 'email', 'sub-u3', ?1, 1, 0, 500)",
            [hash.clone()],
        )
        .unwrap();
        let mut owners = owners_by_email_hash(&conn, &hash).unwrap();
        owners.sort();
        assert_eq!(owners, vec!["u1".to_string(), "u2".to_string()]);
        assert!(
            owners_by_email_hash(&conn, &"ff".repeat(32))
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn verify_flips_only_that_owner_and_address() {
        let conn = memory();
        link(&conn, "u1", "email", "flip@example.test", 0);
        link(&conn, "u1", "email", "other@example.test", 0);
        link(&conn, "u2", "email", "flip@example.test", 0);
        let hash = crate::hash_str("flip@example.test");
        assert_eq!(verify_email_rows(&conn, "u1", &hash).unwrap(), 1);
        let v: i64 = conn
            .query_row(
                "SELECT verified_email FROM identity_link
                 WHERE user_id = 'u1' AND email_hash = ?1",
                [&hash],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(v, 1);
        // Second call flips nothing (idempotent).
        assert_eq!(verify_email_rows(&conn, "u1", &hash).unwrap(), 0);
    }

    #[test]
    fn fallback_roundtrip() {
        let conn = memory();
        conn.execute(
            "INSERT INTO account(user_id, role, customer_ref, created_at, locked_at, step_up_at, fallback_until)
             VALUES ('u1', 'customer', 'u1', 1, NULL, NULL, NULL)",
            [],
        )
        .unwrap();
        set_fallback(&conn, "u1", 7_776_000).unwrap();
        let got = crate::passkey::get_account(&conn, "u1").unwrap().unwrap();
        assert_eq!(got.fallback_until, Some(7_776_000));
        crate::passkey::clear_fallback(&conn, "u1").unwrap();
        let got = crate::passkey::get_account(&conn, "u1").unwrap().unwrap();
        assert_eq!(got.fallback_until, None);
    }
}
