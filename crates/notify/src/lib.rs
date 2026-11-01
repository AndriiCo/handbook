//! Follower notifications, phase 1: email (phase 2 calls arrive later).
//!
//! Visitors follow an authority; when an accepted fact changes, each
//! active follower gets at most one digest email per authority per
//! hour. Targets are sealed with the KEK (`config::encrypt_value`);
//! only hashes are indexed. Double opt-in everywhere: nothing sends
//! to an unconfirmed address.

use rusqlite::{Connection, OptionalExtension};

/// Double-opt-in confirmation window (a stale confirm link dies).
pub const CONFIRM_TTL_SECS: i64 = 86400;
/// Send attempts per outbox row before it dead-letters.
pub const MAX_ATTEMPTS: i64 = 5;
/// Retry delays after failure 1..=4 (attempt 5 is the last try).
pub const BACKOFF_SECS: [i64; 4] = [60, 600, 3600, 3600];
/// Digest window: one email per subscriber per authority per hour.
pub const DIGEST_WINDOW_SECS: i64 = 3600;
pub const DAY_SECS: i64 = 86400;
/// Cursor row name for the `fact_accept` audit tail.
pub const CURSOR_FACT_ACCEPT: &str = "fact_accept";

/// Additive tables only (`CREATE TABLE IF NOT EXISTS`, like
/// `auth::migrate_auth_tables`): no older binary opens a migrated DB,
/// so a migration here never strands a rollback.
pub fn migrate(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS notify_subscription(
           id INTEGER PRIMARY KEY,
           authority_slug TEXT NOT NULL,
           email_hash TEXT NOT NULL,
           email_blob BLOB NOT NULL,
           status TEXT NOT NULL DEFAULT 'pending',
           confirm_hash TEXT,
           confirm_expires_at INTEGER,
           unsub_hash TEXT NOT NULL,
           created_at INTEGER NOT NULL,
           confirmed_at INTEGER,
           UNIQUE(authority_slug, email_hash)
         );
         CREATE TABLE IF NOT EXISTS notify_outbox(
           id INTEGER PRIMARY KEY,
           sub_id INTEGER NOT NULL REFERENCES notify_subscription(id),
           fact_id INTEGER NOT NULL,
           channel TEXT NOT NULL DEFAULT 'email',
           state TEXT NOT NULL DEFAULT 'queued',
           attempts INTEGER NOT NULL DEFAULT 0,
           not_before INTEGER NOT NULL DEFAULT 0,
           leased_at INTEGER,
           created_at INTEGER NOT NULL,
           sent_at INTEGER,
           UNIQUE(sub_id, fact_id, channel)
         );
         CREATE TABLE IF NOT EXISTS notify_cursor(
           name TEXT PRIMARY KEY,
           last_id INTEGER NOT NULL
         );
         CREATE TABLE IF NOT EXISTS notify_suppression(
           email_hash TEXT PRIMARY KEY,
           created_at INTEGER NOT NULL
         );
         CREATE INDEX IF NOT EXISTS idx_notify_outbox_due
           ON notify_outbox(state, not_before);
         CREATE INDEX IF NOT EXISTS idx_notify_outbox_sub
           ON notify_outbox(sub_id, state);",
    )
}

/// Seal a follower address: `(blob, hash)`. The blob is opaque
/// without the KEK; the hash is the only indexed form (lookup,
/// dedupe, rate limits — never the address).
pub fn seal_email(kek: &config::Kek, email: &str) -> (Vec<u8>, String) {
    let norm = email.trim().to_lowercase();
    let blob = config::encrypt_value(kek, "notify.email", 1, norm.as_bytes());
    (blob, auth::hash_str(&norm))
}

/// Open a sealed address (worker send path only).
pub fn open_email(kek: &config::Kek, blob: &[u8]) -> Result<String, String> {
    let bytes = config::decrypt_value(kek, "notify.email", 1, blob)
        .map_err(|e| format!("notify: unseal: {e}"))?;
    String::from_utf8(bytes).map_err(|_| "notify: unseal: not utf-8".to_string())
}

/// 64-hex token (same shape as magic links) and its stored hash.
/// Only the hash rests in the DB; the plaintext travels once, in the
/// confirm email's fragment link.
pub fn mint_token() -> String {
    auth::login::mint_magic_token()
}

/// Hash a confirm/unsubscribe token for storage.
pub fn token_hash(token: &str) -> String {
    auth::login::token_hash(token)
}

/// Minimal target shape check (the web tier also runs its full email
/// validator; this keeps the crate safe standalone).
pub fn valid_target(email: &str) -> bool {
    let email = email.trim();
    email.len() >= 5 && email.len() <= 320 && email.contains('@') && !email.contains(' ')
}

/// Outcome of a subscribe call: what (if anything) to mail.
pub struct SubscribeOutcome {
    pub sub_id: i64,
    /// Confirm-token plaintext to mail, or `None` when already active
    /// (the caller sends nothing — silent 200 either way).
    pub confirm_token: Option<String>,
}

/// Subscribe (or resubscribe) a sealed address to one authority.
/// Idempotent per `(authority_slug, email_hash)`: active stays
/// silent, pending mints a fresh confirm token, unsubscribed
/// re-enters pending. Returns the confirm token to mail, if any.
pub fn subscribe(
    conn: &Connection,
    now: i64,
    authority_slug: &str,
    email: &str,
    kek: &config::Kek,
) -> rusqlite::Result<SubscribeOutcome> {
    sweep_expired(conn, now)?;
    let (blob, hash) = seal_email(kek, email);
    let confirm_token = mint_token();
    let row: Option<(i64, String, Option<i64>)> = conn
        .query_row(
            "SELECT id, status, confirm_expires_at FROM notify_subscription
             WHERE authority_slug = ?1 AND email_hash = ?2",
            rusqlite::params![authority_slug, hash],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    match row {
        Some((id, status, _)) if status == "active" => Ok(SubscribeOutcome {
            sub_id: id,
            confirm_token: None,
        }),
        Some((id, _, _)) => {
            conn.execute(
                "UPDATE notify_subscription
                 SET status = 'pending', email_blob = ?1, confirm_hash = ?2,
                     confirm_expires_at = ?3, confirmed_at = NULL
                 WHERE id = ?4",
                rusqlite::params![blob, token_hash(&confirm_token), now + CONFIRM_TTL_SECS, id],
            )?;
            Ok(SubscribeOutcome {
                sub_id: id,
                confirm_token: Some(confirm_token),
            })
        }
        None => {
            let unsub = mint_token();
            conn.execute(
                "INSERT INTO notify_subscription(authority_slug, email_hash, email_blob,
                     status, confirm_hash, confirm_expires_at, unsub_hash, created_at)
                 VALUES (?1, ?2, ?3, 'pending', ?4, ?5, ?6, ?7)",
                rusqlite::params![
                    authority_slug,
                    hash,
                    blob,
                    token_hash(&confirm_token),
                    now + CONFIRM_TTL_SECS,
                    token_hash(&unsub),
                    now
                ],
            )?;
            Ok(SubscribeOutcome {
                sub_id: conn.last_insert_rowid(),
                confirm_token: Some(confirm_token),
            })
        }
    }
}

/// Drop expired pending rows (unconfirmed addresses never linger).
pub fn sweep_expired(conn: &Connection, now: i64) -> rusqlite::Result<usize> {
    conn.execute(
        "DELETE FROM notify_subscription
         WHERE status = 'pending' AND confirm_expires_at IS NOT NULL
           AND confirm_expires_at <= ?1",
        [now],
    )
}

/// Confirm a double-opt-in token. Single-use: the hash clears on
/// success, so a leaked confirm link dies after one click.
pub fn confirm(
    conn: &Connection,
    now: i64,
    token: &str,
) -> rusqlite::Result<Option<(i64, String)>> {
    let hash = token_hash(token);
    let row: Option<(i64, String)> = conn
        .query_row(
            "SELECT id, authority_slug FROM notify_subscription
             WHERE status = 'pending' AND confirm_hash = ?1
               AND confirm_expires_at IS NOT NULL AND confirm_expires_at > ?2",
            rusqlite::params![hash, now],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    if let Some((id, slug)) = &row {
        conn.execute(
            "UPDATE notify_subscription
             SET status = 'active', confirm_hash = NULL,
                 confirm_expires_at = NULL, confirmed_at = ?1
             WHERE id = ?2",
            rusqlite::params![now, id],
        )?;
        return Ok(Some((*id, slug.clone())));
    }
    Ok(None)
}

/// Unsubscribe by long-lived token (one-click URL or page form).
/// Idempotent: a known token reports done even on repeat; only a
/// truly unknown token reports missing.
pub fn unsubscribe(conn: &Connection, token: &str) -> rusqlite::Result<bool> {
    let hash = token_hash(token);
    let known: bool = conn
        .query_row(
            "SELECT 1 FROM notify_subscription WHERE unsub_hash = ?1",
            [&hash],
            |_| Ok(()),
        )
        .optional()?
        .is_some();
    if known {
        conn.execute(
            "UPDATE notify_subscription SET status = 'unsubscribed'
             WHERE unsub_hash = ?1",
            [&hash],
        )?;
    }
    Ok(known)
}

/// Rotate the one-click unsubscribe token (each digest mints a
/// fresh one, so a leaked footer link dies at the next send).
/// Returns the plaintext for the URL; only the hash rests.
pub fn rotate_unsub(conn: &Connection, sub_id: i64) -> rusqlite::Result<String> {
    let token = mint_token();
    conn.execute(
        "UPDATE notify_subscription SET unsub_hash = ?1 WHERE id = ?2",
        rusqlite::params![token_hash(&token), sub_id],
    )?;
    Ok(token)
}

/// Global suppression (bounces, complaints, abuse): blocked from
/// subscribing and skipped by the worker. Never logs the address.
pub fn suppress(conn: &Connection, now: i64, email_hash: &str) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT OR IGNORE INTO notify_suppression(email_hash, created_at) VALUES (?1, ?2)",
        rusqlite::params![email_hash, now],
    )?;
    Ok(())
}

pub fn is_suppressed(conn: &Connection, email_hash: &str) -> rusqlite::Result<bool> {
    Ok(conn
        .query_row(
            "SELECT 1 FROM notify_suppression WHERE email_hash = ?1",
            [email_hash],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

/// Tail the `fact_accept` audit rows: one outbox row per active
/// follower of the fact's authority (`INSERT OR IGNORE` on
/// `UNIQUE(sub, fact, channel)` — replays and double pokes collapse).
/// The cursor advances past every scanned audit id, even when nobody
/// follows that authority.
pub fn enqueue_from_audit(conn: &Connection, now: i64) -> rusqlite::Result<usize> {
    let cursor: i64 = conn
        .query_row(
            "SELECT last_id FROM notify_cursor WHERE name = ?1",
            [CURSOR_FACT_ACCEPT],
            |row| row.get(0),
        )
        .optional()?
        .unwrap_or(0);
    let mut stmt = conn.prepare(
        "SELECT a.id, a.entity_id FROM audit a
         WHERE a.action = 'fact_accept' AND a.id > ?1 ORDER BY a.id",
    )?;
    let ids: Vec<(i64, i64)> = stmt
        .query_map([cursor], |row| Ok((row.get(0)?, row.get(1)?)))?
        .filter_map(|r| r.ok())
        .collect();
    drop(stmt);
    let mut inserted = 0usize;
    let mut max_id = cursor;
    for (audit_id, fact_id) in ids {
        max_id = max_id.max(audit_id);
        let slug: Option<String> = conn
            .query_row(
                "SELECT s.slug FROM fact f JOIN authority s ON s.id = f.authority_id
                 WHERE f.id = ?1",
                [fact_id],
                |row| row.get(0),
            )
            .optional()?;
        let Some(slug) = slug else { continue };
        inserted += conn.execute(
            "INSERT OR IGNORE INTO notify_outbox(sub_id, fact_id, channel, state, created_at)
             SELECT s.id, ?1, 'email', 'queued', ?2 FROM notify_subscription s
             WHERE s.authority_slug = ?3 AND s.status = 'active'",
            rusqlite::params![fact_id, now, slug],
        )?;
    }
    conn.execute(
        "INSERT INTO notify_cursor(name, last_id) VALUES (?1, ?2)
         ON CONFLICT(name) DO UPDATE SET last_id = excluded.last_id",
        rusqlite::params![CURSOR_FACT_ACCEPT, max_id],
    )?;
    Ok(inserted)
}

/// One leased fact inside a digest.
pub struct DigestFact {
    pub fact_id: i64,
    pub topic: String,
    pub statement: String,
    pub authority_slug: String,
    pub authority_name: String,
    pub quote: String,
    pub source_url: String,
    pub captured_day: String,
    /// Newest previously-accepted statement on the same
    /// authority+topic (the "before"); empty on first acceptance.
    pub previous: String,
}

/// One digest email: all leased rows for one subscriber on one
/// authority, with the sealed address to open at send time.
pub struct DigestGroup {
    pub sub_id: i64,
    pub authority_slug: String,
    pub authority_name: String,
    pub email_blob: Vec<u8>,
    pub row_ids: Vec<i64>,
    /// Parallel to `row_ids`: the fact each leased row watches.
    pub fact_ids: Vec<i64>,
    pub facts: Vec<DigestFact>,
}

/// Lease due rows and group them into digests. Skips: unconfirmed or
/// suppressed followers, rows over the daily cap, and any
/// (subscriber, authority) with a digest inside the hour window.
/// Single-writer (the worker loop owns it); routes never touch the
/// outbox.
pub fn lease_due_groups(
    conn: &Connection,
    now: i64,
    daily_cap: u64,
    limit: usize,
) -> rusqlite::Result<Vec<DigestGroup>> {
    let rows: Vec<(i64, i64, i64)> = conn
        .prepare(
            "SELECT o.id, o.sub_id, o.fact_id FROM notify_outbox o
             JOIN notify_subscription s ON s.id = o.sub_id
             WHERE o.state = 'queued' AND o.channel = 'email'
               AND o.not_before <= ?1 AND o.attempts < ?2
               AND s.status = 'active'
               AND NOT EXISTS (SELECT 1 FROM notify_suppression x
                               WHERE x.email_hash = s.email_hash)
               AND (SELECT COUNT(*) FROM notify_outbox s2
                    WHERE s2.sub_id = o.sub_id AND s2.state = 'sent'
                      AND s2.sent_at > ?3) < ?4
               AND NOT EXISTS (SELECT 1 FROM notify_outbox s3
                    WHERE s3.sub_id = o.sub_id AND s3.state = 'sent'
                      AND s3.sent_at > ?5
                      AND EXISTS (SELECT 1 FROM fact f
                                  JOIN authority a ON a.id = f.authority_id
                                  WHERE f.id = s3.fact_id AND a.slug = s.authority_slug))
             ORDER BY o.id LIMIT ?6",
        )?
        .query_map(
            rusqlite::params![
                now,
                MAX_ATTEMPTS,
                now - DAY_SECS,
                daily_cap as i64,
                now - DIGEST_WINDOW_SECS,
                limit as i64
            ],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?
        .filter_map(|r| r.ok())
        .collect();
    let mut groups: Vec<DigestGroup> = Vec::new();
    for (row_id, sub_id, fact_id) in rows {
        conn.execute(
            "UPDATE notify_outbox SET state = 'leased', leased_at = ?1
             WHERE id = ?2 AND state = 'queued'",
            rusqlite::params![now, row_id],
        )?;
        if conn.changes() == 0 {
            continue;
        }
        let (slug, name, blob): (String, String, Vec<u8>) = conn.query_row(
            "SELECT s.authority_slug, a.name, s.email_blob
             FROM notify_subscription s JOIN authority a ON a.slug = s.authority_slug
             WHERE s.id = ?1",
            [sub_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
        match groups.iter_mut().find(|g| g.sub_id == sub_id) {
            Some(group) => {
                group.row_ids.push(row_id);
                group.fact_ids.push(fact_id);
            }
            None => groups.push(DigestGroup {
                sub_id,
                authority_slug: slug,
                authority_name: name,
                email_blob: blob,
                row_ids: vec![row_id],
                fact_ids: vec![fact_id],
                facts: Vec::new(),
            }),
        }
    }
    Ok(groups)
}

/// One `fact` row joined to its authority (the `load_fact`
/// select: statement, topic, authority/capture ids, citation span,
/// row id, authority slug and name).
type FactHead = (String, String, i64, i64, i64, i64, i64, String, String);

/// Load one fact's digest content: statement, topic, quote slice,
/// https source (capture's own, else the official website), capture
/// day, and the previous accepted statement on the topic.
pub fn load_fact(
    conn: &Connection,
    store_root: &std::path::Path,
    fact_id: i64,
) -> rusqlite::Result<Option<DigestFact>> {
    let row: Option<FactHead> = conn
        .query_row(
            "SELECT f.statement, f.topic, f.authority_id, f.capture_id,
                    f.citation_start, f.citation_end, f.id, a.slug, a.name
             FROM fact f JOIN authority a ON a.id = f.authority_id
             WHERE f.id = ?1",
            [fact_id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                    row.get(7)?,
                    row.get(8)?,
                ))
            },
        )
        .optional()?;
    let Some((statement, topic, authority_id, capture_id, start, end, _, slug, name)) = row else {
        return Ok(None);
    };
    let mut quote = String::new();
    if capture_id > 0
        && start >= 0
        && end > start
        && let Ok(bytes) = std::fs::read(store_root.join("text").join(format!("{capture_id}.txt")))
    {
        let (s, e) = (start as usize, end as usize);
        if e <= bytes.len() {
            let slice = String::from_utf8_lossy(&bytes[s..e]).to_string();
            if (8..=400).contains(&slice.chars().count()) {
                quote = slice;
            }
        }
    }
    let source_url: String = conn
        .query_row(
            "SELECT so.uri FROM capture c JOIN source so ON so.id = c.source_id
             WHERE c.id = ?1",
            [capture_id],
            |row| row.get(0),
        )
        .optional()?
        .unwrap_or_default();
    let source_url = if source_url.starts_with("https://") {
        source_url
    } else {
        conn.query_row(
            "SELECT uri FROM source WHERE authority_id = ?1 AND kind = 'website_official'
             ORDER BY id LIMIT 1",
            [authority_id],
            |row| row.get(0),
        )
        .optional()?
        .unwrap_or_default()
    };
    let captured_at: i64 = conn
        .query_row(
            "SELECT captured_at FROM capture WHERE id = ?1",
            [capture_id],
            |row| row.get(0),
        )
        .optional()?
        .unwrap_or(0);
    let previous: String = conn
        .query_row(
            "SELECT statement FROM fact
             WHERE authority_id = ?1 AND topic = ?2 AND status = 'accepted' AND id < ?3
             ORDER BY id DESC LIMIT 1",
            rusqlite::params![authority_id, topic, fact_id],
            |row| row.get(0),
        )
        .optional()?
        .unwrap_or_default();
    Ok(Some(DigestFact {
        fact_id,
        topic,
        statement,
        authority_slug: slug,
        authority_name: name,
        quote,
        source_url,
        captured_day: iso_day(captured_at),
        previous,
    }))
}

/// Unix seconds → `YYYY-MM-DD` (civil-from-days, no date crate).
pub fn iso_day(unix: i64) -> String {
    let days = unix.div_euclid(86_400) + 719_468;
    let era = days.div_euclid(146_097);
    let doe = days.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let mut y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    y += i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

/// Mark leased rows sent.
pub fn mark_sent(conn: &Connection, now: i64, row_ids: &[i64]) -> rusqlite::Result<()> {
    for id in row_ids {
        conn.execute(
            "UPDATE notify_outbox SET state = 'sent', sent_at = ?1 WHERE id = ?2",
            rusqlite::params![now, id],
        )?;
    }
    Ok(())
}

/// Mark leased rows failed: back off 1m/10m/1h, dead-letter past
/// five attempts. Returns rows dead-lettered (for the audit log).
pub fn mark_failed(conn: &Connection, now: i64, row_ids: &[i64]) -> rusqlite::Result<usize> {
    let mut dead = 0usize;
    for id in row_ids {
        let attempts: i64 = conn.query_row(
            "SELECT attempts FROM notify_outbox WHERE id = ?1",
            [id],
            |row| row.get(0),
        )?;
        let attempts = attempts + 1;
        if attempts >= MAX_ATTEMPTS {
            conn.execute(
                "UPDATE notify_outbox SET state = 'dead', attempts = ?1 WHERE id = ?2",
                rusqlite::params![attempts, id],
            )?;
            dead += 1;
        } else {
            let wait = BACKOFF_SECS[(attempts as usize - 1).min(BACKOFF_SECS.len() - 1)];
            conn.execute(
                "UPDATE notify_outbox SET state = 'queued', attempts = ?1, not_before = ?2
                 WHERE id = ?3",
                rusqlite::params![attempts, now + wait, id],
            )?;
        }
    }
    Ok(dead)
}

/// Build one digest email: before → after per fact, quote, source
/// and date, dossier link, one-click unsubscribe, postal footer.
/// Pure (unit-tested); the worker only fills the envelope.
pub fn build_digest(
    facts: &[DigestFact],
    dossier_base: &str,
    unsub_url: &str,
    postal_address: &str,
) -> (String, String) {
    let first = &facts[0];
    let subject = if facts.len() == 1 {
        format!("Apostille update: {}", first.authority_name)
    } else {
        format!(
            "Apostille update: {} ({} changes)",
            first.authority_name,
            facts.len()
        )
    };
    let mut body = String::new();
    for fact in facts {
        body.push_str(&format!("{}: {}\n", fact.topic, fact.statement));
        if !fact.previous.is_empty() {
            body.push_str(&format!("was: {}\n", fact.previous));
        }
        if !fact.quote.is_empty() {
            body.push_str(&format!("quote: \"{}\"\n", fact.quote));
        }
        if !fact.source_url.is_empty() {
            body.push_str(&format!(
                "source: {} (captured {})\n",
                fact.source_url, fact.captured_day
            ));
        }
        body.push_str(&format!(
            "details: {}/apostille/authority/{}\n\n",
            dossier_base.trim_end_matches('/'),
            fact.authority_slug
        ));
    }
    body.push_str(&format!(
        "---\nYou follow {} at the Apostille Handbook.\nUnsubscribe in one click: {}\n{}\n",
        first.authority_name, unsub_url, postal_address
    ));
    (subject, body)
}

/// Build the double-opt-in confirm email. The link carries the token
/// in the fragment (never to the server in a GET); the confirm page
/// POSTs it as JSON.
pub fn build_confirm(
    authority_name: &str,
    confirm_url: &str,
    postal_address: &str,
) -> (String, String) {
    (
        "Confirm your Apostille Handbook updates".to_string(),
        format!(
            "You asked for email updates when {authority_name} facts change.\n\nConfirm (link dies in 24 hours): {confirm_url}\n\nDidn't ask for this? Ignore this mail — nothing sends until you confirm.\n\n---\n{postal_address}\n"
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn memory() -> Connection {
        let conn = Connection::open_in_memory().expect("memory db");
        migrate(&conn).expect("migrate");
        conn
    }

    fn kek() -> config::Kek {
        config::Kek::generate()
    }

    fn seed_authority(conn: &Connection) -> (i64, i64) {
        conn.execute(
            "CREATE TABLE IF NOT EXISTS authority(id INTEGER PRIMARY KEY, slug TEXT, name TEXT)",
            [],
        )
        .unwrap();
        conn.execute(
            "CREATE TABLE IF NOT EXISTS fact(id INTEGER PRIMARY KEY, authority_id INTEGER,
             topic TEXT, statement TEXT, status TEXT, capture_id INTEGER,
             citation_start INTEGER, citation_end INTEGER)",
            [],
        )
        .unwrap();
        conn.execute(
            "CREATE TABLE IF NOT EXISTS audit(id INTEGER PRIMARY KEY, action TEXT, entity_id INTEGER)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO authority(id, slug, name) VALUES (1, 'vietnam', 'Viet Nam')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO fact(id, authority_id, topic, statement, status, capture_id,
                              citation_start, citation_end)
             VALUES (7, 1, 'fees', '-eight dollars-', 'accepted', 0, -1, -1)",
            [],
        )
        .unwrap();
        (1, 7)
    }

    #[test]
    fn seal_round_trip_and_hash_index() {
        let kek = kek();
        let (blob, hash) = seal_email(&kek, "  Staff@Example.com ");
        // Sealed blob carries no plaintext; only the hash is indexed.
        assert!(!blob.windows(5).any(|w| w == b"staff"));
        assert_eq!(open_email(&kek, &blob).expect("open"), "staff@example.com");
        assert_eq!(hash, auth::hash_str("staff@example.com"));
    }

    #[test]
    fn double_opt_in_gates_sends() {
        let conn = memory();
        let kek = kek();
        seed_authority(&conn);
        // Subscribe → pending with a confirm token.
        let out = subscribe(&conn, 100, "vietnam", "a@example.com", &kek).expect("sub");
        assert!(out.confirm_token.is_some());
        // Nothing enqueues while pending (the cursor still advances:
        // pre-confirmation accepts are not backfilled).
        conn.execute(
            "INSERT INTO audit(id, action, entity_id) VALUES (1, 'fact_accept', 7)",
            [],
        )
        .unwrap();
        assert_eq!(enqueue_from_audit(&conn, 100).expect("enqueue"), 0);
        // Confirm → active → later accepts enqueue.
        let token = out.confirm_token.unwrap();
        let confirmed = confirm(&conn, 200, &token).expect("confirm");
        assert_eq!(confirmed, Some((out.sub_id, "vietnam".to_string())));
        conn.execute(
            "INSERT INTO audit(id, action, entity_id) VALUES (2, 'fact_accept', 7)",
            [],
        )
        .unwrap();
        assert_eq!(enqueue_from_audit(&conn, 200).expect("enqueue"), 1);
        // Confirm tokens are single-use.
        assert_eq!(confirm(&conn, 201, &token).expect("replay"), None);
    }

    #[test]
    fn enqueue_is_idempotent_per_sub_fact_channel() {
        let conn = memory();
        let kek = kek();
        seed_authority(&conn);
        let out = subscribe(&conn, 100, "vietnam", "a@example.com", &kek).expect("sub");
        confirm(&conn, 101, &out.confirm_token.unwrap()).expect("confirm");
        conn.execute(
            "INSERT INTO audit(id, action, entity_id) VALUES (1, 'fact_accept', 7)",
            [],
        )
        .unwrap();
        assert_eq!(enqueue_from_audit(&conn, 100).expect("first"), 1);
        // Replay, double poke, second worker tick: still one row.
        assert_eq!(enqueue_from_audit(&conn, 101).expect("replay"), 0);
        assert_eq!(enqueue_from_audit(&conn, 102).expect("replay"), 0);
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM notify_outbox", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 1);
    }

    #[test]
    fn unsubscribe_stops_sends_and_is_idempotent() {
        let conn = memory();
        let kek = kek();
        seed_authority(&conn);
        let (blob, hash) = seal_email(&kek, "a@example.com");
        let unsub = mint_token();
        conn.execute(
            "INSERT INTO notify_subscription(authority_slug, email_hash, email_blob,
                 status, unsub_hash, created_at, confirmed_at)
             VALUES ('vietnam', ?1, ?2, 'active', ?3, 100, 101)",
            rusqlite::params![hash, blob, token_hash(&unsub)],
        )
        .unwrap();
        assert!(unsubscribe(&conn, &unsub).expect("unsub"));
        assert!(unsubscribe(&conn, &unsub).expect("idempotent"));
        assert!(!unsubscribe(&conn, "bogus").expect("unknown"));
        let status: String = conn
            .query_row(
                "SELECT status FROM notify_subscription WHERE email_hash = ?1",
                [hash],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(status, "unsubscribed");
        // No rows enqueue for the unsubscribed follower.
        conn.execute(
            "INSERT INTO audit(id, action, entity_id) VALUES (1, 'fact_accept', 7)",
            [],
        )
        .unwrap();
        assert_eq!(enqueue_from_audit(&conn, 200).expect("enqueue"), 0);
    }

    #[test]
    fn suppression_blocks_subscribe_and_send() {
        let conn = memory();
        let kek = kek();
        seed_authority(&conn);
        let (_, hash) = seal_email(&kek, "a@example.com");
        suppress(&conn, 100, &hash).expect("suppress");
        assert!(is_suppressed(&conn, &hash).expect("check"));
        // A suppressed follower still subscribes (always-200 shape) but
        // never receives: enqueue skips them.
        let out = subscribe(&conn, 100, "vietnam", "a@example.com", &kek).expect("sub");
        confirm(&conn, 101, &out.confirm_token.unwrap()).expect("confirm");
        conn.execute(
            "INSERT INTO audit(id, action, entity_id) VALUES (1, 'fact_accept', 7)",
            [],
        )
        .unwrap();
        assert_eq!(enqueue_from_audit(&conn, 200).expect("enqueue"), 1);
        let groups = lease_due_groups(&conn, 300, 10, 10).expect("lease");
        assert!(groups.is_empty(), "suppressed follower gets no digest");
    }

    #[test]
    fn backoff_dead_letters_after_five_attempts() {
        let conn = memory();
        let kek = kek();
        seed_authority(&conn);
        let (blob, hash) = seal_email(&kek, "a@example.com");
        conn.execute(
            "INSERT INTO notify_subscription(id, authority_slug, email_hash, email_blob,
                 status, unsub_hash, created_at) VALUES (1, 'vietnam', ?1, ?2, 'active', 'u', 100)",
            rusqlite::params![hash, blob],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO notify_outbox(id, sub_id, fact_id, channel, state, created_at)
             VALUES (1, 1, 7, 'email', 'leased', 100)",
            [],
        )
        .unwrap();
        // 1m, 10m, 1h, then dead on the fifth failure.
        let waits = [60, 600, 3600, 3600];
        let mut now = 1000;
        for (i, wait) in waits.iter().enumerate() {
            assert_eq!(mark_failed(&conn, now, &[1]).expect("fail"), 0);
            let (state, not_before): (String, i64) = conn
                .query_row(
                    "SELECT state, not_before FROM notify_outbox WHERE id = 1",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .unwrap();
            assert_eq!(
                (state.as_str(), not_before - now),
                ("queued", *wait),
                "attempt {}",
                i + 1
            );
            now = not_before;
        }
        assert_eq!(mark_failed(&conn, now, &[1]).expect("dead"), 1);
        let state: String = conn
            .query_row("SELECT state FROM notify_outbox WHERE id = 1", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(state, "dead");
    }

    #[test]
    fn digest_window_defers_second_send_within_hour() {
        let conn = memory();
        let kek = kek();
        seed_authority(&conn);
        let (blob, hash) = seal_email(&kek, "a@example.com");
        conn.execute(
            "INSERT INTO notify_subscription(id, authority_slug, email_hash, email_blob,
                 status, unsub_hash, created_at) VALUES (1, 'vietnam', ?1, ?2, 'active', 'u', 100)",
            rusqlite::params![hash, blob],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO fact(id, authority_id, topic, statement, status, capture_id,
                              citation_start, citation_end)
             VALUES (8, 1, 'fees', '-nine dollars-', 'accepted', 0, -1, -1),
                    (9, 1, 'hours', '-nine to five-', 'accepted', 0, -1, -1)",
            [],
        )
        .unwrap();
        for (id, fact) in [(1, 7), (2, 8)] {
            conn.execute(
                "INSERT INTO notify_outbox(id, sub_id, fact_id, channel, state, created_at)
                 VALUES (?1, 1, ?2, 'email', 'queued', 100)",
                [id, fact],
            )
            .unwrap();
        }
        // Both rows lease together (one digest), then send.
        let groups = lease_due_groups(&conn, 1000, 10, 10).expect("lease");
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].row_ids.len(), 2);
        mark_sent(&conn, 1000, &groups[0].row_ids).expect("sent");
        // A fresh row inside the hour window does not lease.
        conn.execute(
            "INSERT INTO notify_outbox(id, sub_id, fact_id, channel, state, created_at)
             VALUES (3, 1, 9, 'email', 'queued', 1001)",
            [],
        )
        .unwrap();
        assert!(
            lease_due_groups(&conn, 2000, 10, 10)
                .expect("lease")
                .is_empty()
        );
        // Past the hour it leases again.
        assert_eq!(
            lease_due_groups(&conn, 4601, 10, 10).expect("lease").len(),
            1
        );
    }

    #[test]
    fn digest_body_carries_before_after_quote_and_footer() {
        let facts = vec![DigestFact {
            fact_id: 7,
            topic: "fees".to_string(),
            statement: "eight dollars".to_string(),
            authority_slug: "vietnam".to_string(),
            authority_name: "Viet Nam".to_string(),
            quote: "fee eight dollars".to_string(),
            source_url: "https://example.test/fee".to_string(),
            captured_day: "2026-10-30".to_string(),
            previous: "seven dollars".to_string(),
        }];
        let (subject, body) = build_digest(
            &facts,
            "https://example.test",
            "https://example.test/apostille/notify/unsubscribe?t=abc",
            "Example Co, 1 Test Way",
        );
        assert!(subject.contains("Viet Nam"));
        for want in [
            "was: seven dollars",
            "quote: \"fee eight dollars\"",
            "https://example.test/fee",
            "2026-10-30",
            "https://example.test/apostille/authority/vietnam",
            "unsubscribe?t=abc",
            "Example Co, 1 Test Way",
        ] {
            assert!(body.contains(want), "missing {want:?}");
        }
    }
}
