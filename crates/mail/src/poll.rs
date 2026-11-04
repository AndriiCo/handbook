//! Watermarked Gmail poll into immutable `email` captures.
//!
//! At-least-once + idempotent ingest: every poll walks `history.list`
//! from the stored cursor, fetches each added message's raw bytes,
//! ingests (sha256 dedup makes re-delivery free), and advances the
//! cursor only over fully processed ids. A crash or rate limit keeps
//! the old watermark — the next tick re-lists and dedup-hits.
//! Expired watermarks reseed via `getProfile` + a bounded 7-day
//! `messages.list` backfill. 401 refreshes once and retries once.

use rusqlite::{Connection, OptionalExtension};

use handbook_store::db::{NewAuthority, add_authority};
use handbook_store::store::Store;
use ingest::capture::{IngestInput, IngestOutcome, ingest_bytes};

use crate::auth_results;
use crate::client::{AddedMessage, MailClient, MailError};

/// Fallback sender authority for community/unattributed mail, created
/// on first use. Official-domain mail attributes to its own authority.
const INBOX_SLUG: &str = "mail-inbox";

/// Backfill window on watermark expiry (never a full-mailbox scan).
const BACKFILL_QUERY: &str = "newer_than:7d";

/// One [`poll`] run's knobs.
pub struct PollOpts {
    pub actor: String,
    pub now: i64,
    /// Sleeps on [`MailError::RateLimited`] before aborting the run
    /// with the watermark held (prod: 5s/20s/60s; tests: empty).
    pub backoff_secs: Vec<u64>,
    /// Cap on `messages.get` calls per run (bounds the reseed spike;
    /// the watermark holds when the cap hits mid-list).
    pub max_fetches: usize,
}

/// Outcome of one [`poll`] run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PollReport {
    /// Ids listed this run (history or backfill).
    pub listed: usize,
    /// New captures minted (`Duplicate` re-deliveries excluded).
    pub created: usize,
    /// Watermark after the run (unchanged when nothing completed).
    pub watermark: String,
    /// True when the run took the reseed path.
    pub reseeded: bool,
}

/// Poll once: refresh, walk history from the cursor, ingest, advance.
/// See the module docs for the crash/rate-limit contract.
///
/// # Errors
///
/// [`MailError::NotConfigured`] without credentials;
/// [`MailError::Api`] on refresh refusal or non-retryable Gmail
/// failures; [`MailError::Db`] on storage failure. Rate limits and
/// fetch caps return `Ok` with a partial report (the hold reason in
/// `mail_poll.last_error`), never an error — the next tick resumes.
pub fn poll(
    conn: &Connection,
    store: &Store,
    client: &mut MailClient,
    opts: &PollOpts,
) -> Result<PollReport, MailError> {
    client.refresh()?;
    ensure_row(conn)?;
    let cursor = cursor(conn)?;
    if cursor.history_id.is_empty() {
        let profile = with_refresh(client, |c| c.profile())?;
        store_cursor(conn, &profile.email, &profile.history_id, opts.now, "")?;
        return Ok(PollReport {
            listed: 0,
            created: 0,
            watermark: profile.history_id,
            reseeded: false,
        });
    }
    match walk_history(conn, store, client, opts, &cursor.history_id)? {
        WalkOutcome::Done(report) => Ok(report),
        WalkOutcome::RateLimited { fetched, created } => {
            backoff(&opts.backoff_secs);
            record_error(conn, opts.now, "rate limited; watermark held")?;
            Ok(PollReport {
                listed: fetched,
                created,
                watermark: cursor.history_id,
                reseeded: false,
            })
        }
        WalkOutcome::Expired => reseed(conn, store, client, opts),
        WalkOutcome::Capped { fetched, created } => {
            record_error(conn, opts.now, "fetch cap hit; watermark held")?;
            Ok(PollReport {
                listed: fetched,
                created,
                watermark: cursor.history_id,
                reseeded: false,
            })
        }
    }
}

struct Cursor {
    history_id: String,
}

fn ensure_row(conn: &Connection) -> Result<(), MailError> {
    conn.execute("INSERT OR IGNORE INTO mail_poll(id) VALUES (1)", [])?;
    Ok(())
}

fn cursor(conn: &Connection) -> Result<Cursor, MailError> {
    let history_id: String =
        conn.query_row("SELECT history_id FROM mail_poll WHERE id = 1", [], |row| {
            row.get(0)
        })?;
    Ok(Cursor { history_id })
}

fn store_cursor(
    conn: &Connection,
    email: &str,
    history_id: &str,
    now: i64,
    error: &str,
) -> Result<(), MailError> {
    conn.execute(
        "UPDATE mail_poll SET email = ?1, history_id = ?2, last_poll_at = ?3,
                              last_error = ?4 WHERE id = 1",
        rusqlite::params![email, history_id, now, error],
    )?;
    Ok(())
}

/// Advance the watermark, keeping the seeded mailbox email.
fn advance_watermark(conn: &Connection, history_id: &str, now: i64) -> Result<(), MailError> {
    conn.execute(
        "UPDATE mail_poll SET history_id = ?1, last_poll_at = ?2, last_error = '' WHERE id = 1",
        rusqlite::params![history_id, now],
    )?;
    Ok(())
}

fn record_error(conn: &Connection, now: i64, error: &str) -> Result<(), MailError> {
    conn.execute(
        "UPDATE mail_poll SET last_poll_at = ?1, last_error = ?2 WHERE id = 1",
        rusqlite::params![now, error],
    )?;
    Ok(())
}

/// Refresh-once-and-retry-once for any Gmail call.
fn with_refresh<T>(
    client: &mut MailClient,
    mut call: impl FnMut(&MailClient) -> Result<T, MailError>,
) -> Result<T, MailError> {
    match call(client) {
        Err(MailError::Unauthorized) => {
            client.refresh()?;
            call(client)
        }
        other => other,
    }
}

fn backoff(sleeps: &[u64]) {
    for secs in sleeps {
        std::thread::sleep(std::time::Duration::from_secs(*secs));
    }
}

enum WalkOutcome {
    Done(PollReport),
    RateLimited { fetched: usize, created: usize },
    Expired,
    Capped { fetched: usize, created: usize },
}

/// Page through `history.list` from the cursor, ingesting every added
/// id. The watermark advances to the last page's id only when every
/// listed id was fetched.
fn walk_history(
    conn: &Connection,
    store: &Store,
    client: &mut MailClient,
    opts: &PollOpts,
    cursor: &str,
) -> Result<WalkOutcome, MailError> {
    let mut page_token: Option<String> = None;
    let mut listed = 0;
    let mut created = 0;
    // Advance target, set from the last consumed page; the LIST
    // cursor stays fixed across pages (every page request carries the
    // original startHistoryId + pageToken). Deferred init: the loop
    // always assigns before any break.
    let mut watermark: String;
    loop {
        let page = match with_refresh(client, |c| c.history_list(cursor, page_token.as_deref())) {
            Ok(page) => page,
            Err(MailError::HistoryExpired) => return Ok(WalkOutcome::Expired),
            Err(MailError::RateLimited) => {
                return Ok(WalkOutcome::RateLimited {
                    fetched: listed,
                    created,
                });
            }
            Err(e) => return Err(e),
        };
        for added in &page.added {
            if listed >= opts.max_fetches {
                return Ok(WalkOutcome::Capped {
                    fetched: listed,
                    created,
                });
            }
            listed += 1;
            match fetch_one(conn, store, client, opts, added, &page.history_id)? {
                FetchOutcome::Created => created += 1,
                FetchOutcome::Duplicate => {}
                FetchOutcome::RateLimited => {
                    return Ok(WalkOutcome::RateLimited {
                        fetched: listed,
                        created,
                    });
                }
            }
        }
        watermark = page.history_id;
        page_token = page.next_page_token;
        if page_token.is_none() {
            break;
        }
    }
    advance_watermark(conn, &watermark, opts.now)?;
    Ok(WalkOutcome::Done(PollReport {
        listed,
        created,
        watermark,
        reseeded: false,
    }))
}

enum FetchOutcome {
    Created,
    Duplicate,
    RateLimited,
}

/// Fetch one message's raw bytes and ingest as an `email` capture.
/// Idempotent: byte-identical re-delivery dedup-hits on
/// `(authority_id, sha256)` and only refreshes nothing.
fn fetch_one(
    conn: &Connection,
    store: &Store,
    client: &mut MailClient,
    opts: &PollOpts,
    added: &AddedMessage,
    history_id: &str,
) -> Result<FetchOutcome, MailError> {
    let raw = match with_refresh(client, |c| c.message_raw(&added.id)) {
        Ok(raw) => raw,
        Err(MailError::RateLimited) => return Ok(FetchOutcome::RateLimited),
        Err(e) => return Err(e),
    };
    let parsed = crate::mime::parse(&raw.raw);
    // Authentication evidence first: it decides the trust below.
    let evidence = auth_results::parse(&parsed.auth_results);
    let sender = resolve_sender(conn, &parsed, &evidence, opts)?;
    let source_id =
        find_or_create_source(conn, sender.authority_id, sender.source_kind, &sender.addr)?;
    let sidecar = format!(
        "From: {}\nTo: {}\nDate: {}\nSubject: {}\n\n{}",
        parsed.from, parsed.to, parsed.date, parsed.subject, parsed.body_text
    );
    let origin = format!("gmail:{}", raw.id);
    let outcome = ingest_bytes(
        conn,
        store,
        &IngestInput {
            authority_id: sender.authority_id,
            source_id: Some(source_id),
            kind: "email",
            trust: sender.trust,
            bytes: &raw.raw,
            text: &sidecar,
            captured_at: opts.now,
            actor: &opts.actor,
            origin: &origin,
        },
    )?;
    match outcome {
        IngestOutcome::Created { capture_id } => {
            let dkim_all: Vec<serde_json::Value> = evidence
                .dkim_all
                .iter()
                .map(|sig| serde_json::json!({"result": sig.result, "domain": sig.domain}))
                .collect();
            let authenticity = serde_json::json!({
                "gmail_id": raw.id,
                "thread_id": raw.thread_id,
                "history_id": history_id,
                "internal_date_ms": raw.internal_date_ms,
                "gmail_labels": raw.labels,
                "received_via": "gmail-api",
                "auth_results": {
                    "dkim": evidence.dkim,
                    "dkim_domain": evidence.dkim_domain,
                    "dkim_all": dkim_all,
                    "spf": evidence.spf,
                    "spf_mailfrom": evidence.spf_mailfrom,
                    "dmarc": evidence.dmarc,
                    "from_domain": evidence.from_domain,
                },
                "sender": sender.addr,
                "verified_by": sender.verified_by,
            });
            conn.execute(
                "UPDATE capture SET authenticity = ?1 WHERE id = ?2",
                rusqlite::params![authenticity.to_string(), capture_id],
            )?;
            link_outreach_reply(conn, capture_id, &parsed)?;
            Ok(FetchOutcome::Created)
        }
        IngestOutcome::Duplicate { .. } => Ok(FetchOutcome::Duplicate),
    }
}

/// First-source threading: an office's reply names our outreach
/// `Message-ID` in In-Reply-To or References. Link the outreach email
/// it answers to this capture (the newest reply wins, so an early
/// auto-reply never hides the office's real answer) and mark it replied.
/// A store without the (web-owned, additive) `outreach_email` table
/// links nothing. Returns the linked email id.
pub fn link_outreach_reply(
    conn: &Connection,
    capture_id: i64,
    parsed: &crate::mime::ParsedMail,
) -> rusqlite::Result<Option<i64>> {
    let has_table = conn
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'outreach_email'",
            [],
            |_| Ok(()),
        )
        .optional()?
        .is_some();
    if !has_table {
        return Ok(None);
    }
    let mut ids = crate::mime::message_ids(&parsed.in_reply_to);
    ids.extend(crate::mime::message_ids(&parsed.references));
    for id in ids {
        let email: Option<i64> = conn
            .query_row(
                "SELECT id FROM outreach_email
                 WHERE message_id = ?1 AND status IN ('sent', 'replied')
                 ORDER BY id LIMIT 1",
                [&id],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(email) = email {
            conn.execute(
                "UPDATE outreach_email SET status = 'replied', reply_capture_id = ?1
                 WHERE id = ?2",
                rusqlite::params![capture_id, email],
            )?;
            return Ok(Some(email));
        }
    }
    Ok(None)
}

struct Sender {
    addr: String,
    authority_id: i64,
    trust: &'static str,
    source_kind: &'static str,
    /// `dmarc` / `dkim` when Gmail's stamp authenticates the From
    /// domain, else `none`.
    verified_by: &'static str,
}

/// Authority + trust from the sender domain: official domains
/// attribute to their own authority, and earn `official` trust only
/// when Gmail's stamp authenticates that domain (DMARC pass for it,
/// or an aligned passing DKIM signature); unauthenticated mail from
/// an official domain keeps its authority but caps at `community`.
/// Everything else lands in the inbox as `community` (proposable,
/// never directly promotable); known-fake domains capture as
/// `rejected` (kept as evidence, never analyzed).
fn resolve_sender(
    conn: &Connection,
    parsed: &crate::mime::ParsedMail,
    evidence: &auth_results::AuthResults,
    opts: &PollOpts,
) -> Result<Sender, MailError> {
    let addr = crate::mime::sender_addr(&parsed.from);
    let domain = crate::mime::sender_domain(&addr);
    let verified_by = domain
        .as_deref()
        .and_then(|d| evidence.verified_for(d))
        .unwrap_or("none");
    let row: Option<(String, Option<i64>)> = match &domain {
        Some(name) => conn
            .query_row(
                "SELECT status, authority_id FROM domain WHERE name = ?1",
                [name],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?,
        None => None,
    };
    let inbox = || inbox_authority(conn, opts);
    let (authority_id, trust, source_kind) = match row {
        Some((status, auth)) if status == "official" => {
            let authority_id = match auth {
                Some(auth) => auth,
                None => inbox()?,
            };
            if verified_by == "none" {
                (authority_id, "community", "email_customer")
            } else {
                (authority_id, "official", "email_official")
            }
        }
        Some((status, _)) if status == "fake" => (inbox()?, "rejected", "email_customer"),
        Some((_, Some(auth))) => (auth, "community", "email_customer"),
        _ => (inbox()?, "community", "email_customer"),
    };
    Ok(Sender {
        addr,
        authority_id,
        trust,
        source_kind,
        verified_by,
    })
}

fn inbox_authority(conn: &Connection, opts: &PollOpts) -> Result<i64, MailError> {
    if let Some(id) = conn
        .query_row(
            "SELECT id FROM authority WHERE slug = ?1",
            [INBOX_SLUG],
            |row| row.get::<_, i64>(0),
        )
        .optional()?
    {
        return Ok(id);
    }
    Ok(add_authority(
        conn,
        &NewAuthority {
            kind: "office",
            country: "",
            region: "",
            name: "Mail inbox",
            slug: INBOX_SLUG,
            actor: &opts.actor,
            at: opts.now,
        },
    )?)
}

fn find_or_create_source(
    conn: &Connection,
    authority_id: i64,
    kind: &str,
    uri: &str,
) -> Result<i64, MailError> {
    if let Some(id) = conn
        .query_row(
            "SELECT id FROM source WHERE authority_id = ?1 AND uri = ?2",
            rusqlite::params![authority_id, uri],
            |row| row.get::<_, i64>(0),
        )
        .optional()?
    {
        return Ok(id);
    }
    conn.execute(
        "INSERT INTO source(authority_id, kind, uri) VALUES (?1, ?2, ?3)",
        rusqlite::params![authority_id, kind, uri],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Watermark-expired reseed: fresh profile, bounded 7-day backfill,
/// then re-profile so mail arriving mid-backfill is not skipped (it
/// re-delivers instead; dedup absorbs it).
fn reseed(
    conn: &Connection,
    store: &Store,
    client: &mut MailClient,
    opts: &PollOpts,
) -> Result<PollReport, MailError> {
    let profile = with_refresh(client, |c| c.profile())?;
    let mut page_token: Option<String> = None;
    let mut listed = 0;
    let mut created = 0;
    loop {
        let page = match with_refresh(client, |c| {
            c.messages_list(BACKFILL_QUERY, page_token.as_deref())
        }) {
            Ok(page) => page,
            Err(MailError::RateLimited) => {
                backoff(&opts.backoff_secs);
                record_error(conn, opts.now, "rate limited during reseed; watermark held")?;
                let held = cursor(conn)?.history_id;
                return Ok(PollReport {
                    listed,
                    created,
                    watermark: held,
                    reseeded: true,
                });
            }
            Err(e) => return Err(e),
        };
        let mut capped = false;
        for added in &page.ids {
            if listed >= opts.max_fetches {
                capped = true;
                break;
            }
            listed += 1;
            match fetch_one(conn, store, client, opts, added, &profile.history_id)? {
                FetchOutcome::Created => created += 1,
                FetchOutcome::Duplicate => {}
                FetchOutcome::RateLimited => {
                    backoff(&opts.backoff_secs);
                    record_error(conn, opts.now, "rate limited during reseed; watermark held")?;
                    let held = cursor(conn)?.history_id;
                    return Ok(PollReport {
                        listed,
                        created,
                        watermark: held,
                        reseeded: true,
                    });
                }
            }
        }
        page_token = page.next_page_token;
        if page_token.is_none() || capped {
            break;
        }
    }
    // Re-profile AFTER the backfill: anything that arrived mid-run
    // re-delivers on the next tick instead of being skipped.
    let fresh = with_refresh(client, |c| c.profile())?;
    store_cursor(conn, &fresh.email, &fresh.history_id, opts.now, "")?;
    Ok(PollReport {
        listed,
        created,
        watermark: fresh.history_id,
        reseeded: true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use handbook_store::db::{migrate, open};
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;

    const NOW: i64 = 1_800_000_000;
    const TOKEN_OK: &str = r#"{"access_token":"tok-1","expires_in":3599,"token_type":"Bearer"}"#;

    /// Office-closure notice with a forged-future `Date:` (sender-
    /// claimed) and a google auth stamp. internalDate rules ordering.
    const NOTICE: &str = "From: ops@example.gov\r\nTo: box@example.com\r\nDate: Sat, 01 Jan 2033 00:00:00 +0000\r\nSubject: Office closure tomorrow\r\nAuthentication-Results: mx.google.com; dkim=pass header.d=example.gov; spf=pass smtp.mailfrom=ops@example.gov; dmarc=pass header.from=example.gov\r\n\r\nThe Olympia office is closed tomorrow.";

    #[test]
    fn replies_link_to_the_outreach_email_they_answer() {
        let (_dir, _store, conn) = migrated();
        let reply = crate::mime::parse(
            b"From: ops@example.gov\r\nIn-Reply-To: <outreach-4-ab@andrii.ai>\r\n\r\nYes.",
        );
        // No outreach table on this store: nothing to link.
        assert_eq!(link_outreach_reply(&conn, 9, &reply).unwrap(), None);
        conn.execute_batch(
            "CREATE TABLE outreach_email (id INTEGER PRIMARY KEY, message_id TEXT NOT NULL,
                 status TEXT NOT NULL, reply_capture_id INTEGER);
             INSERT INTO outreach_email VALUES (4, '<outreach-4-ab@andrii.ai>', 'sent', NULL);
             INSERT INTO outreach_email VALUES (5, '<outreach-5-cd@andrii.ai>', 'draft', NULL);",
        )
        .unwrap();
        assert_eq!(link_outreach_reply(&conn, 9, &reply).unwrap(), Some(4));
        // A later reply (the real answer after an auto-reply) wins.
        assert_eq!(link_outreach_reply(&conn, 10, &reply).unwrap(), Some(4));
        let (status, capture): (String, i64) = conn
            .query_row(
                "SELECT status, reply_capture_id FROM outreach_email WHERE id = 4",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!((status.as_str(), capture), ("replied", 10));
        // An unsent draft is never matched.
        let stray = crate::mime::parse(
            b"From: x@example.gov\r\nReferences: <outreach-5-cd@andrii.ai>\r\n\r\nHi.",
        );
        assert_eq!(link_outreach_reply(&conn, 11, &stray).unwrap(), None);
    }

    fn migrated() -> (tempfile::TempDir, Store, Connection) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path());
        store.init().unwrap();
        let conn = open(&store).unwrap();
        migrate(&conn).unwrap();
        (dir, store, conn)
    }

    fn opts() -> PollOpts {
        PollOpts {
            actor: "test".to_string(),
            now: NOW,
            backoff_secs: vec![],
            max_fetches: 200,
        }
    }

    /// Serve canned exchanges in order; both token + Gmail paths share
    /// the base. An unexpected extra call fails (connection refused).
    fn stub(replies: Vec<(u16, String)>) -> (String, std::thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
        let handle = std::thread::spawn(move || {
            for (status, reply) in replies {
                let (stream, _) = listener.accept().unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut head = String::new();
                reader.read_line(&mut head).unwrap();
                let mut len = 0usize;
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    if line.trim_end().is_empty() {
                        break;
                    }
                    let lower = line.to_ascii_lowercase();
                    if let Some(n) = lower
                        .strip_prefix("content-length:")
                        .and_then(|v| v.trim().parse().ok())
                    {
                        len = n;
                    }
                }
                let mut body = vec![0u8; len];
                reader.read_exact(&mut body).unwrap();
                let mut stream = reader.into_inner();
                let text = format!(
                    "HTTP/1.1 {status} OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}",
                    reply.len()
                );
                stream.write_all(text.as_bytes()).unwrap();
            }
        });
        (base, handle)
    }

    fn test_client(base: &str) -> MailClient {
        MailClient::new(base, &format!("{base}/token"), "cid", "csec", "rtok")
    }

    fn raw_message_json(id: &str) -> String {
        raw_message_json_of(id, NOTICE)
    }

    fn raw_message_json_of(id: &str, raw: &str) -> String {
        use base64::Engine as _;
        let encoded = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(raw.as_bytes());
        format!(
            r#"{{"id":"{id}","threadId":"t-{id}","labelIds":["INBOX"],"internalDate":"1729000000000","raw":"{encoded}"}}"#
        )
    }

    fn history_page(ids: &[&str], history_id: &str) -> String {
        let records: Vec<String> = ids
            .iter()
            .map(|id| {
                format!(
                    r#"{{"messagesAdded":[{{"message":{{"id":"{id}","threadId":"t-{id}"}}}}]}}"#
                )
            })
            .collect();
        format!(
            r#"{{"history":[{}],"historyId":"{history_id}"}}"#,
            records.join(",")
        )
    }

    fn seed_domain(conn: &Connection, authority_id: i64) {
        conn.execute(
            "INSERT INTO domain(name, authority_id, status) VALUES ('example.gov', ?1, 'official')",
            [authority_id],
        )
        .unwrap();
    }

    fn seed_authority(conn: &Connection, slug: &str) -> i64 {
        add_authority(
            conn,
            &NewAuthority {
                kind: "office",
                country: "Test",
                region: "",
                name: "Test Office",
                slug,
                actor: "test",
                at: NOW,
            },
        )
        .unwrap()
    }

    fn watermark(conn: &Connection) -> (String, String, String) {
        conn.query_row(
            "SELECT email, history_id, last_error FROM mail_poll WHERE id = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap()
    }

    #[test]
    fn first_run_seeds_watermark_without_fetching() {
        let (_dir, store, conn) = migrated();
        let (base, server) = stub(vec![
            (200, TOKEN_OK.to_string()),
            (
                200,
                r#"{"emailAddress":"box@example.com","historyId":"100"}"#.to_string(),
            ),
        ]);
        let mut client = test_client(&base);
        let report = poll(&conn, &store, &mut client, &opts()).expect("seed");
        assert_eq!(report.listed, 0);
        assert_eq!(report.watermark, "100");
        assert!(!report.reseeded);
        assert_eq!(watermark(&conn).1, "100");
        server.join().unwrap();
    }

    #[test]
    fn poll_ingests_official_mail_and_advances() {
        let (_dir, store, conn) = migrated();
        let auth = seed_authority(&conn, "t-sos");
        seed_domain(&conn, auth);
        conn.execute(
            "INSERT INTO mail_poll(id, email, history_id) VALUES (1, 'box@example.com', '100')",
            [],
        )
        .unwrap();
        let (base, server) = stub(vec![
            (200, TOKEN_OK.to_string()),
            (200, history_page(&["m1"], "101")),
            (200, raw_message_json("m1")),
        ]);
        let mut client = test_client(&base);
        let report = poll(&conn, &store, &mut client, &opts()).expect("poll");
        assert_eq!((report.listed, report.created), (1, 1));
        assert_eq!(report.watermark, "101");
        // Capture: official trust, office authority, sidecar, evidence.
        let (kind, trust, authority_id, authenticity): (String, String, i64, String) = conn
            .query_row(
                "SELECT kind, trust, authority_id, authenticity FROM capture WHERE id > 0",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(kind, "email");
        assert_eq!(trust, "official");
        assert_eq!(authority_id, auth);
        let evidence: serde_json::Value = serde_json::from_str(&authenticity).unwrap();
        assert_eq!(evidence["gmail_id"], "m1");
        assert_eq!(evidence["internal_date_ms"], 1_729_000_000_000i64);
        assert_eq!(evidence["auth_results"]["dkim"], "pass");
        assert_eq!(evidence["auth_results"]["dmarc"], "pass");
        assert_eq!(evidence["verified_by"], "dmarc");
        let sidecar: String = conn
            .query_row(
                "SELECT body FROM capture_fts WHERE capture_id = (SELECT id FROM capture WHERE id > 0)",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(sidecar.contains("Office closure tomorrow"), "{sidecar}");
        assert!(sidecar.contains("closed tomorrow"), "{sidecar}");
        server.join().unwrap();
    }

    #[test]
    fn unknown_sender_lands_in_inbox_as_community() {
        let (_dir, store, conn) = migrated();
        seed_authority(&conn, "t-sos");
        // No domain row: example.gov is unknown here.
        conn.execute(
            "INSERT INTO mail_poll(id, email, history_id) VALUES (1, 'box@example.com', '100')",
            [],
        )
        .unwrap();
        let (base, server) = stub(vec![
            (200, TOKEN_OK.to_string()),
            (200, history_page(&["m1"], "101")),
            (200, raw_message_json("m1")),
        ]);
        let mut client = test_client(&base);
        poll(&conn, &store, &mut client, &opts()).expect("poll");
        let (trust, slug): (String, String) = conn
            .query_row(
                "SELECT c.trust, a.slug FROM capture c JOIN authority a ON a.id = c.authority_id
                 WHERE c.id > 0",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(trust, "community");
        assert_eq!(slug, "mail-inbox");
        server.join().unwrap();
    }

    /// Poll one message from an official-domain store; returns the
    /// capture's trust, authority slug, and authenticity evidence.
    fn poll_official_store(raw: &str) -> (String, String, serde_json::Value) {
        let (_dir, store, conn) = migrated();
        let auth = seed_authority(&conn, "t-sos");
        seed_domain(&conn, auth);
        conn.execute(
            "INSERT INTO mail_poll(id, email, history_id) VALUES (1, 'box@example.com', '100')",
            [],
        )
        .unwrap();
        let (base, server) = stub(vec![
            (200, TOKEN_OK.to_string()),
            (200, history_page(&["m1"], "101")),
            (200, raw_message_json_of("m1", raw)),
        ]);
        let mut client = test_client(&base);
        let report = poll(&conn, &store, &mut client, &opts()).expect("poll");
        assert_eq!(report.created, 1);
        server.join().unwrap();
        let (trust, slug, authenticity): (String, String, String) = conn
            .query_row(
                "SELECT c.trust, a.slug, c.authenticity FROM capture c
                 JOIN authority a ON a.id = c.authority_id WHERE c.id > 0",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        (trust, slug, serde_json::from_str(&authenticity).unwrap())
    }

    #[test]
    fn official_trust_needs_authentication_of_the_from_domain() {
        let notice = |from: &str, stamp: &str| {
            format!(
                "From: {from}\r\nTo: box@example.com\r\nSubject: Fee change\r\n{stamp}\r\nThe fee is now $90."
            )
        };
        // DMARC fail: the mail is kept on the office, capped at community.
        let (trust, slug, evidence) = poll_official_store(&notice(
            "ops@example.gov",
            "Authentication-Results: mx.google.com; dkim=fail header.d=example.gov; spf=softfail smtp.mailfrom=ops@example.gov; dmarc=fail header.from=example.gov\r\n",
        ));
        assert_eq!((trust.as_str(), slug.as_str()), ("community", "t-sos"));
        assert_eq!(evidence["verified_by"], "none");
        assert_eq!(evidence["auth_results"]["dmarc"], "fail");
        // No Gmail stamp at all (or only a forwarder's): community.
        let (trust, _, evidence) = poll_official_store(&notice(
            "ops@example.gov",
            "Authentication-Results: mx.evil.test; dmarc=pass header.from=example.gov\r\n",
        ));
        assert_eq!(trust, "community");
        assert_eq!(evidence["auth_results"]["dmarc"], "none");
        // The display name quotes the official address; the mailbox
        // (and the domain DMARC passed for) is the attacker's.
        let (trust, slug, evidence) = poll_official_store(&notice(
            "\"x <ops@example.gov>\" <a@evil.test>",
            "Authentication-Results: mx.google.com; dkim=pass header.i=@evil.test; spf=pass smtp.mailfrom=a@evil.test; dmarc=pass header.from=evil.test\r\n",
        ));
        assert_eq!((trust.as_str(), slug.as_str()), ("community", "mail-inbox"));
        assert_eq!(evidence["sender"], "a@evil.test");
        // Aligned DKIM without a DMARC record (Gmail's `header.i` shape).
        let (trust, slug, evidence) = poll_official_store(&notice(
            "Ops <ops@example.gov>",
            "Authentication-Results: mx.google.com; dkim=pass header.i=@example.gov header.s=s1 header.b=Ab; spf=pass smtp.mailfrom=ops@example.gov; dmarc=none header.from=example.gov\r\n",
        ));
        assert_eq!((trust.as_str(), slug.as_str()), ("official", "t-sos"));
        assert_eq!(evidence["verified_by"], "dkim");
        assert_eq!(
            evidence["auth_results"]["dkim_all"][0]["domain"],
            "example.gov"
        );
    }

    #[test]
    fn expired_watermark_reseeds_with_backfill() {
        let (_dir, store, conn) = migrated();
        seed_authority(&conn, "t-sos");
        conn.execute(
            "INSERT INTO mail_poll(id, email, history_id) VALUES (1, 'box@example.com', '1')",
            [],
        )
        .unwrap();
        let (base, server) = stub(vec![
            (200, TOKEN_OK.to_string()),
            (
                404,
                r#"{"error":{"errors":[{"reason":"notFound"}]}}"#.to_string(),
            ),
            (
                200,
                r#"{"emailAddress":"box@example.com","historyId":"200"}"#.to_string(),
            ),
            (
                200,
                r#"{"messages":[{"id":"m9","threadId":"t-m9"}]}"#.to_string(),
            ),
            (200, raw_message_json("m9")),
            (
                200,
                r#"{"emailAddress":"box@example.com","historyId":"201"}"#.to_string(),
            ),
        ]);
        let mut client = test_client(&base);
        let report = poll(&conn, &store, &mut client, &opts()).expect("reseed");
        assert!(report.reseeded);
        assert_eq!((report.listed, report.created), (1, 1));
        assert_eq!(report.watermark, "201");
        assert_eq!(watermark(&conn).1, "201");
        server.join().unwrap();
    }

    #[test]
    fn unauthorized_refreshes_once_then_retries() {
        let (_dir, store, conn) = migrated();
        seed_authority(&conn, "t-sos");
        conn.execute(
            "INSERT INTO mail_poll(id, email, history_id) VALUES (1, 'box@example.com', '100')",
            [],
        )
        .unwrap();
        let (base, server) = stub(vec![
            (200, TOKEN_OK.to_string()),
            (
                401,
                r#"{"error":{"errors":[{"reason":"authError"}]}}"#.to_string(),
            ),
            (200, TOKEN_OK.to_string()),
            (200, history_page(&[], "102")),
        ]);
        let mut client = test_client(&base);
        let report = poll(&conn, &store, &mut client, &opts()).expect("retry wins");
        assert_eq!(report.watermark, "102");
        server.join().unwrap();
    }

    #[test]
    fn second_unauthorized_is_a_hard_error_and_holds() {
        let (_dir, store, conn) = migrated();
        seed_authority(&conn, "t-sos");
        conn.execute(
            "INSERT INTO mail_poll(id, email, history_id) VALUES (1, 'box@example.com', '100')",
            [],
        )
        .unwrap();
        let (base, server) = stub(vec![
            (200, TOKEN_OK.to_string()),
            (401, r#"{"error":{}}"#.to_string()),
            (200, TOKEN_OK.to_string()),
            (401, r#"{"error":{}}"#.to_string()),
        ]);
        let mut client = test_client(&base);
        let err = poll(&conn, &store, &mut client, &opts()).expect_err("hard error");
        assert_eq!(err, MailError::Unauthorized);
        assert_eq!(watermark(&conn).1, "100");
        server.join().unwrap();
    }

    #[test]
    fn rate_limit_returns_partial_and_holds() {
        let (_dir, store, conn) = migrated();
        seed_authority(&conn, "t-sos");
        conn.execute(
            "INSERT INTO mail_poll(id, email, history_id) VALUES (1, 'box@example.com', '100')",
            [],
        )
        .unwrap();
        let (base, server) = stub(vec![
            (200, TOKEN_OK.to_string()),
            (429, r#"{"error":{}}"#.to_string()),
        ]);
        let mut client = test_client(&base);
        let report = poll(&conn, &store, &mut client, &opts()).expect("partial ok");
        assert_eq!(report.watermark, "100");
        assert_eq!(report.created, 0);
        let (_, held, error) = watermark(&conn);
        assert_eq!(held, "100");
        assert!(error.contains("rate limited"), "{error}");
        server.join().unwrap();
    }

    #[test]
    fn redelivery_dedups_without_new_capture() {
        let (_dir, store, conn) = migrated();
        let auth = seed_authority(&conn, "t-sos");
        seed_domain(&conn, auth);
        conn.execute(
            "INSERT INTO mail_poll(id, email, history_id) VALUES (1, 'box@example.com', '100')",
            [],
        )
        .unwrap();
        let script = || {
            stub(vec![
                (200, TOKEN_OK.to_string()),
                (200, history_page(&["m1"], "101")),
                (200, raw_message_json("m1")),
            ])
        };
        let (base, server) = script();
        let mut client = test_client(&base);
        let first = poll(&conn, &store, &mut client, &opts()).expect("first");
        assert_eq!(first.created, 1);
        server.join().unwrap();
        // Same id re-listed (cursor held externally): dedup-hit, no write.
        conn.execute("UPDATE mail_poll SET history_id = '100' WHERE id = 1", [])
            .unwrap();
        let (base2, server2) = script();
        let mut client2 = test_client(&base2);
        let second = poll(&conn, &store, &mut client2, &opts()).expect("second");
        assert_eq!((second.listed, second.created), (1, 0));
        let captures: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM capture WHERE authority_id = ?1",
                [auth],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(captures, 1);
        server2.join().unwrap();
    }
}
