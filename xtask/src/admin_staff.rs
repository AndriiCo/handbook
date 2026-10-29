//! Staff lifecycle: enrolment invites + offboarding.
//!
//! Tasks: `admin-invite --role <staff|admin> --cn <name>` (mints a
//! single-use enrolment URL, stores the token hash only) and
//! `admin-offboard --user <id> --cn <name>` (cert-first,
//! per-step transactions, idempotent). Spec
//! `customer-auth.md` §§5.1/5.6.

/// Enrol-page origin default (staging passes `--origin`).
const DEFAULT_ORIGIN: &str = "https://admin.andrii.ai";

/// `admin-invite --role <staff|admin> --cn <name> [--days <n>]
/// [--actor <a>] [--store <path>] [--origin <url>]`.
pub fn run_invite(args: &[String]) {
    let store = crate::open_store(args);
    let conn = crate::open_db(&store);
    auth::migrate_auth_tables(&conn).unwrap_or_else(|e| fail(&e.to_string()));
    let role = crate::required(args, "role");
    if role != "staff" && role != "admin" {
        fail("--role must be staff or admin");
    }
    let cn = crate::required(args, "cn");
    if !crate::admin_cert::valid_cn(&cn) {
        fail("invalid --cn (ascii word chars, dots, dashes; max 64)");
    }
    // Mint order (spec §5.1): the invite binds to an ISSUED cert,
    // never to a future one.
    let live: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM admin_cert
             WHERE cn = ?1 AND revoked_at IS NULL AND expires_at > ?2",
            rusqlite::params![cn, crate::unix_now()],
            |r| r.get(0),
        )
        .unwrap_or_else(|e| fail(&e.to_string()));
    if live == 0 {
        fail(&format!(
            "no live certificate for cn {cn:?}; run admin-cert-issue first"
        ));
    }
    let days: i64 = crate::flag(args, "days")
        .and_then(|d| d.parse().ok())
        .unwrap_or(7);
    if !(1..=30).contains(&days) {
        fail("--days must be 1..30");
    }
    let actor = crate::flag(args, "actor").unwrap_or_else(|| "xtask".to_string());
    let origin = crate::flag(args, "origin").unwrap_or_else(|| DEFAULT_ORIGIN.to_string());
    if !origin.starts_with("https://") && !origin.starts_with("http://localhost") {
        fail("--origin must be https:// (or http://localhost for dev)");
    }
    let token = auth::login::mint_magic_token();
    let hash = auth::login::token_hash(&token);
    let now = crate::unix_now();
    handbook_store::db::in_transaction(&conn, || {
        auth::passkey::store_invite(
            &conn,
            &hash,
            &auth::passkey::NewInvite {
                role: &role,
                cn: &cn,
                created_by: &actor,
                now,
                ttl_secs: days.saturating_mul(86_400),
            },
        )?;
        conn.execute(
            "INSERT INTO audit(at, actor, action, entity, entity_id, reason)
             VALUES (?1, ?2, 'staff_invite_issue', 'staff_invite', 0, ?3)",
            rusqlite::params![now, actor, format!("{role}:{cn}")],
        )?;
        Ok(())
    })
    .unwrap_or_else(|e| fail(&e.to_string()));
    println!(
        "invite for {cn} ({role})\n  url: {}/enrol?invite={token}\n  expires in {days}d; single-use; token is NOT stored",
        origin.trim_end_matches('/')
    );
}

/// `admin-offboard --user <id> --cn <name> [--actor <a>]
/// [--store <path>]`. Cert first (a racing login can never
/// resurrect access), one transaction per step, every step
/// idempotent — re-runnable after partial failure.
pub fn run_offboard(args: &[String]) {
    let store = crate::open_store(args);
    let conn = crate::open_db(&store);
    auth::migrate_auth_tables(&conn).unwrap_or_else(|e| fail(&e.to_string()));
    let user_id = crate::required(args, "user");
    let cn = crate::required(args, "cn");
    if !crate::admin_cert::valid_cn(&cn) {
        fail("invalid --cn (ascii word chars, dots, dashes; max 64)");
    }
    let actor = crate::flag(args, "actor").unwrap_or_else(|| "xtask".to_string());
    let now = crate::unix_now();
    // 1. Certs: every live row for the CN goes, by CN (OPEN-15:
    // no user_id column on admin_cert yet).
    let certs = handbook_store::db::in_transaction(&conn, || {
        let changed = conn.execute(
            "UPDATE admin_cert SET revoked_at = ?1 WHERE cn = ?2 AND revoked_at IS NULL",
            rusqlite::params![now, cn],
        )?;
        audit(&conn, now, &actor, "admin_cert_revoke", &cn)?;
        Ok(changed)
    })
    .unwrap_or_else(|e| fail(&e.to_string()));
    // 2. Sessions: all of them, immediately.
    let sessions = handbook_store::db::in_transaction(&conn, || {
        let killed = auth::revoke_user_sessions(&conn, &user_id)?;
        audit(&conn, now, &actor, "staff_offboard_sessions", &user_id)?;
        Ok(killed)
    })
    .unwrap_or_else(|e| fail(&e.to_string()));
    // 3. Passkeys: every live credential of the user.
    let creds = handbook_store::db::in_transaction(&conn, || {
        let mut revoked = 0;
        let rows = auth::passkey::credentials_for_user(&conn, &user_id)?;
        for (credential_id, _) in &rows {
            revoked += auth::passkey::revoke_credential(&conn, &user_id, credential_id, now)?;
        }
        audit(&conn, now, &actor, "staff_offboard_credentials", &user_id)?;
        Ok(revoked)
    })
    .unwrap_or_else(|e| fail(&e.to_string()));
    // 4. Lock (belt-and-braces; login rejects locked).
    handbook_store::db::in_transaction(&conn, || {
        auth::passkey::lock_account(&conn, &user_id, now)?;
        audit(
            &conn,
            now,
            &actor,
            "staff_offboard",
            &format!("{user_id}:{cn}"),
        )?;
        Ok(())
    })
    .unwrap_or_else(|e| fail(&e.to_string()));
    println!(
        "offboarded {user_id} ({cn}): {certs} cert(s), {sessions} session(s), {creds} credential(s) revoked; account locked"
    );
}

fn audit(
    conn: &rusqlite::Connection,
    now: i64,
    actor: &str,
    action: &str,
    reason: &str,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO audit(at, actor, action, entity, entity_id, reason)
         VALUES (?1, ?2, ?3, 'account', 0, ?4)",
        rusqlite::params![now, actor, action, reason],
    )?;
    Ok(())
}

fn fail(message: &str) -> ! {
    eprintln!("admin-staff: {message}");
    std::process::exit(1);
}

#[cfg(test)]
mod tests {
    use handbook_store::store::Store;

    fn setup() -> (tempfile::TempDir, Store, rusqlite::Connection) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path());
        store.init().unwrap();
        let conn = handbook_store::db::open(&store).unwrap();
        handbook_store::db::migrate(&conn).unwrap();
        auth::migrate_auth_tables(&conn).unwrap();
        (dir, store, conn)
    }

    #[test]
    fn invite_roundtrip_stores_hash_only() {
        let (_dir, _store, conn) = setup();
        auth::issue_admin_cert(&conn, "fp-1", "jdoe", 1000, 2000, "").unwrap();
        let token = auth::login::mint_magic_token();
        assert_eq!(token.len(), 64);
        let hash = auth::login::token_hash(&token);
        auth::passkey::store_invite(
            &conn,
            &hash,
            &auth::passkey::NewInvite {
                role: "staff",
                cn: "jdoe",
                created_by: "tester",
                now: 1000,
                ttl_secs: 7 * 86_400,
            },
        )
        .unwrap();
        // The raw token appears nowhere in the table.
        let hits: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM staff_invite WHERE token_hash = ?1",
                [&token],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(hits, 0);
        assert!(
            auth::passkey::get_invite(&conn, &hash, 1100)
                .unwrap()
                .is_some()
        );
        // Mint-gating query: live cert row exists for the CN.
        let live: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM admin_cert
                 WHERE cn = 'jdoe' AND revoked_at IS NULL AND expires_at > 1100",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(live, 1);
    }

    #[test]
    fn offboard_order_kills_cert_sessions_creds_and_locks() {
        let (_dir, _store, conn) = setup();
        let now = 1_800_000_000;
        auth::passkey::ensure_staff_account(&conn, "u-s", "staff", now).unwrap();
        auth::issue_admin_cert(&conn, "fp-1", "jdoe", now, now + 1000, "").unwrap();
        auth::create_session(
            &conn,
            "sess-1",
            &auth::NewSession {
                user_id: "u-s",
                role: auth::Role::Staff,
                customer_ref: None,
                now,
                ip: "127.0.0.1",
                user_agent: "test",
            },
        )
        .unwrap();
        conn.execute(
            "INSERT INTO webauthn_credential(credential_id, user_id, passkey_json, label, created_at, last_used_at, revoked_at)
             VALUES ('cred-1', 'u-s', '{}', '', ?1, NULL, NULL)",
            [now],
        )
        .unwrap();
        // Step 1: certs.
        let certs = conn
            .execute(
                "UPDATE admin_cert SET revoked_at = ?1 WHERE cn = 'jdoe' AND revoked_at IS NULL",
                [now],
            )
            .unwrap();
        assert_eq!(certs, 1);
        assert!(!auth::admin_cert_valid(&conn, "fp-1").unwrap());
        // Step 2: sessions.
        assert_eq!(auth::revoke_user_sessions(&conn, "u-s").unwrap(), 1);
        assert!(auth::get_session(&conn, "sess-1").unwrap().is_none());
        // Step 3: credentials (+ link disable is a no-op without rows).
        for (credential_id, _) in auth::passkey::credentials_for_user(&conn, "u-s").unwrap() {
            auth::passkey::revoke_credential(&conn, "u-s", &credential_id, now).unwrap();
        }
        assert!(
            auth::passkey::credentials_for_user(&conn, "u-s")
                .unwrap()
                .is_empty()
        );
        // Step 4: lock.
        auth::passkey::lock_account(&conn, "u-s", now).unwrap();
        let account = auth::passkey::get_account(&conn, "u-s").unwrap().unwrap();
        assert!(account.locked_at.is_some());
        // Re-run is a clean no-op (idempotent).
        assert_eq!(
            conn.execute(
                "UPDATE admin_cert SET revoked_at = ?1 WHERE cn = 'jdoe' AND revoked_at IS NULL",
                [now],
            )
            .unwrap(),
            0
        );
        assert_eq!(auth::revoke_user_sessions(&conn, "u-s").unwrap(), 0);
    }
}
