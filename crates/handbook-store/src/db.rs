//! SQLite metadata database: migrations and foundation queries.
//!
//! Fresh databases start from the `migrations/schema_baseline.sql`
//! snapshot (= v11; the 001–011 incrementals were squashed away in
//! prompt 055 after prod reached v11 — see git history). Later
//! migrations add `N+1` files and extend [`migrate`]: `012` widens
//! the `fact.topic` CHECK for `service_notice`; `013` adds authority
//! visibility (`hidden_at`) and location columns. The P2
//! customer-PII database (`p2.sqlite`) has its own baseline
//! (`migrations/p2_schema_baseline.sql`, = P2 v2) and its own
//! [`migrate_p2`].

use std::collections::BTreeSet;
use std::sync::LazyLock;

use regex::{Captures, Regex};
use rusqlite::{Connection, OptionalExtension};

use contracts::{HandbookError, Result};

use crate::store::Store;

/// Current schema version (`pragma user_version`).
pub const SCHEMA_VERSION: i32 = 13;

/// Version the squashed `schema_baseline.sql` stamps; every later
/// version is one `0NN_*.sql` step on top of it.
pub const BASELINE_VERSION: i32 = 11;

/// Current P2 customer-PII schema version (`pragma user_version` on
/// `p2.sqlite`; independent of [`SCHEMA_VERSION`]).
pub const P2_SCHEMA_VERSION: i32 = 2;

/// Voice-audio retention in days (decided 2026-10-26).
pub const VOICE_RETENTION_DAYS: i64 = 365;

/// Open (creating if missing) the store's metadata database.
///
/// Sets `journal_mode=WAL` (one writer, many readers),
/// `busy_timeout=5000` (wait out lock contention instead of failing),
/// `synchronous=NORMAL` (WAL-safe durability), and `foreign_keys=ON`.
///
/// # SAFE DELETE rule
///
/// Every connection from this function enforces foreign keys, so a
/// `DELETE` that would orphan a child row fails instead of silently
/// dangling. Consequences for all writers:
/// - New tables MUST declare real `REFERENCES` clauses for references.
/// - Deletes MUST remove (or null, where nullable) child rows first.
/// - Never work around a violation with `PRAGMA foreign_keys=OFF`.
pub fn open(store: &Store) -> Result<Connection> {
    let conn = Connection::open(store.db_path())?;
    apply_pragmas(&conn)?;
    Ok(conn)
}

/// Open (creating if missing) the P2 customer-PII database.
///
/// Same PRAGMAs as [`open`]. Cross-DB references (authority /
/// form_template / capture ids) are opaque integers SQLite cannot
/// enforce: readers must treat a missing parent as "open a task +
/// audit", never as a join assumption. PII retention is OQ-7 (open).
pub fn open_p2(store: &Store) -> Result<Connection> {
    let conn = Connection::open(store.p2_db_path())?;
    apply_pragmas(&conn)?;
    Ok(conn)
}

/// The PRAGMA set every store database gets: `journal_mode=WAL` (one
/// writer, many readers), `busy_timeout=5000` (wait out lock contention
/// instead of failing), `synchronous=NORMAL` (WAL-safe durability),
/// and `foreign_keys=ON` (deletes that would orphan a child row fail).
pub(crate) fn apply_pragmas(conn: &Connection) -> Result<()> {
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "busy_timeout", 5000)?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    Ok(())
}

/// Apply pending migrations. Idempotent.
///
/// Fresh (`user_version = 0`) databases get the v11 baseline
/// snapshot, then every later step, in one transaction, so a crash
/// or error can never leave a half-applied schema behind (a fresh
/// database is therefore exactly "v11 + the steps"). Databases at
/// [`BASELINE_VERSION`] or later take only their missing steps;
/// databases already at [`SCHEMA_VERSION`] are untouched. Anything
/// below the baseline (v1–v10) is refused loudly: the incremental
/// path was squashed away, so those databases must first open under
/// the last incremental release (≤ dc400a9) or be restored from a
/// v11 backup — silently stamping them current would corrupt the
/// version contract. A version above [`SCHEMA_VERSION`] (a newer
/// binary wrote it) is refused the same way.
pub fn migrate(conn: &Connection) -> Result<()> {
    in_transaction(conn, || {
        let mut version: i32 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        if version == 0 {
            conn.execute_batch(include_str!("../migrations/schema_baseline.sql"))?;
            conn.pragma_update(None, "user_version", BASELINE_VERSION)?;
            version = BASELINE_VERSION;
        } else if !(BASELINE_VERSION..=SCHEMA_VERSION).contains(&version) {
            return Err(version_gap(
                "handbook.sqlite",
                version,
                BASELINE_VERSION,
                SCHEMA_VERSION,
            ));
        }
        if version < 12 {
            conn.execute_batch(include_str!("../migrations/012_service_notice.sql"))?;
            conn.pragma_update(None, "user_version", 12)?;
        }
        if version < 13 {
            conn.execute_batch(include_str!("../migrations/013_authority_hidden.sql"))?;
            conn.pragma_update(None, "user_version", 13)?;
        }
        Ok(())
    })?;
    debug_assert_eq!(
        conn.query_row("PRAGMA user_version", [], |row| row.get::<_, i32>(0))?,
        SCHEMA_VERSION
    );
    Ok(())
}

/// Apply pending P2 migrations to a [`open_p2`] connection. Idempotent.
///
/// Same baseline discipline as [`migrate`]: fresh databases get the
/// P2 v2 snapshot; v1 databases are refused loudly (upgrade through
/// the last incremental release first); versioned by the p2.sqlite
/// `user_version`, independently of [`SCHEMA_VERSION`].
pub fn migrate_p2(conn: &Connection) -> Result<()> {
    in_transaction(conn, || {
        let version: i32 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        if version == 0 {
            conn.execute_batch(include_str!("../migrations/p2_schema_baseline.sql"))?;
            conn.pragma_update(None, "user_version", P2_SCHEMA_VERSION)?;
        } else if version != P2_SCHEMA_VERSION {
            return Err(version_gap(
                "p2.sqlite",
                version,
                P2_SCHEMA_VERSION,
                P2_SCHEMA_VERSION,
            ));
        }
        Ok(())
    })?;
    debug_assert_eq!(
        conn.query_row("PRAGMA user_version", [], |row| row.get::<_, i32>(0))?,
        P2_SCHEMA_VERSION
    );
    Ok(())
}

/// Loud refusal for databases stranded between the old incremental
/// versions and the squashed `baseline` (or newer than this binary's
/// `current`). Ad-hoc error via the same `ToSqlConversionFailure`
/// carrier db.rs already uses for non-SQLite failures.
fn version_gap(db: &str, version: i32, baseline: i32, current: i32) -> HandbookError {
    let remedy = if version < baseline {
        format!(
            "upgrade it through the last incremental release first \
             (≤ dc400a9 applies v{version}→v{baseline} step by step), \
             or restore from a v{baseline} backup"
        )
    } else {
        format!("it was written by a newer binary (v{version} > v{current}); upgrade this binary")
    };
    HandbookError::Db(rusqlite::Error::ToSqlConversionFailure(Box::new(
        std::io::Error::other(format!(
            "{db} is at schema v{version}, this binary baselines v{baseline} \
             and migrates to v{current}: {remedy}"
        )),
    )))
}

/// Run `f` inside one `BEGIN IMMEDIATE`/`COMMIT` transaction, rolling
/// back when `f` fails. Takes a shared `&Connection` (rusqlite uses
/// interior mutability), so callers with a shared handle still get
/// atomic multi-statement updates.
pub fn in_transaction<T>(conn: &Connection, f: impl FnOnce() -> Result<T>) -> Result<T> {
    conn.execute_batch("BEGIN IMMEDIATE")?;
    match f() {
        Ok(value) => {
            conn.execute_batch("COMMIT")?;
            Ok(value)
        }
        Err(e) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(e)
        }
    }
}

/// New-authority fields plus audit attribution.
pub struct NewAuthority<'a> {
    pub kind: &'a str,
    pub country: &'a str,
    pub region: &'a str,
    pub name: &'a str,
    pub slug: &'a str,
    pub actor: &'a str,
    pub at: i64,
}

/// Insert an authority (office / jurisdiction). Returns the new row id.
/// The authority row and its audit row commit atomically (B1).
pub fn add_authority(conn: &Connection, a: &NewAuthority<'_>) -> Result<i64> {
    in_transaction(conn, || {
        conn.execute(
            "INSERT INTO authority(kind, country, region, name, slug)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            (a.kind, a.country, a.region, a.name, a.slug),
        )?;
        let id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO audit(at, actor, action, entity, entity_id, reason)
             VALUES (?1, ?2, 'authority_add', 'authority', ?3, ?4)",
            rusqlite::params![a.at, a.actor, id, a.slug],
        )?;
        Ok(id)
    })
}

/// Why an authority's display name reads as a fragment of a page
/// rather than the name of an office (P2-1: "Land", "Fax: …", "the
/// federal government,"), or `None` for a plausible name: 6 characters
/// or fewer, a lowercase start, a trailing `:`/`,`/`;`, an e-mail
/// address, a phone or fax number, a postcode or a street line. Pack
/// validation and the staff authorities screen both apply it.
#[must_use]
pub fn authority_name_lint(name: &str) -> Option<&'static str> {
    let name = name.trim();
    if name.chars().count() <= 6 {
        return Some("6 characters or fewer");
    }
    if name.chars().next().is_some_and(char::is_lowercase) {
        return Some("starts lowercase (a sentence fragment)");
    }
    if name.ends_with([':', ',', ';']) {
        return Some("ends with ':', ',' or ';' (a list lead-in)");
    }
    let lower = name.to_lowercase();
    if lower.contains('@') || lower.contains("[email") {
        return Some("contains an email address");
    }
    let words: Vec<&str> = lower
        .split(|c: char| !c.is_alphanumeric() && c != '+')
        .filter(|w| !w.is_empty())
        .collect();
    if words
        .iter()
        .any(|w| matches!(*w, "fax" | "tel" | "telephone" | "phone"))
        || words
            .iter()
            .any(|w| w.starts_with('+') && w[1..].starts_with(|c: char| c.is_ascii_digit()))
    {
        return Some("contains a phone or fax number");
    }
    let postcode = name.split_whitespace().any(|token| {
        let (prefix, digits) = token.split_once('-').unwrap_or(("", ""));
        (1..=3).contains(&prefix.len())
            && prefix.bytes().all(|b| b.is_ascii_uppercase())
            && (4..=6).contains(&digits.len())
            && digits.bytes().all(|b| b.is_ascii_digit())
    });
    let tokens: Vec<&str> = name.split_whitespace().collect();
    let street = tokens.len() <= 3
        && tokens
            .last()
            .is_some_and(|t| t.bytes().all(|b| b.is_ascii_digit()));
    if postcode || street {
        return Some("contains a postcode or street address");
    }
    None
}

/// Authority columns [`edit_authority`] may change. Never `slug`: it is
/// the join key of every table, API route and WS topic.
pub const AUTHORITY_EDITABLE: [&str; 9] = [
    "kind",
    "country",
    "region",
    "name",
    "iso2",
    "subdivision",
    "level",
    "name_en",
    "aliases",
];

/// Set `fields` (`(column, value)`, columns from [`AUTHORITY_EDITABLE`])
/// on authority `id` where they differ, with one `authority_update`
/// audit row whose `before_ref`/`after_ref` carry the changed columns
/// (`name=Land` → `name=…`). Opens no transaction: callers already hold
/// one (a pack load) or wrap it with [`in_transaction`]. Returns the
/// changed column names (empty: nothing to do, no audit row).
pub fn edit_authority(
    conn: &Connection,
    id: i64,
    fields: &[(&str, &str)],
    actor: &str,
    at: i64,
) -> Result<Vec<String>> {
    let mut changed = Vec::new();
    let mut before = Vec::new();
    let mut after = Vec::new();
    for (column, value) in fields {
        if !AUTHORITY_EDITABLE.contains(column) {
            return Err(HandbookError::Db(rusqlite::Error::InvalidColumnName(
                (*column).to_string(),
            )));
        }
        let current: String = conn.query_row(
            &format!("SELECT {column} FROM authority WHERE id = ?1"),
            [id],
            |row| row.get(0),
        )?;
        if current == *value {
            continue;
        }
        conn.execute(
            &format!("UPDATE authority SET {column} = ?1 WHERE id = ?2"),
            rusqlite::params![value, id],
        )?;
        before.push(format!("{column}={current}"));
        after.push(format!("{column}={value}"));
        changed.push((*column).to_string());
    }
    if !changed.is_empty() {
        insert_audit(
            conn,
            at,
            actor,
            "authority_update",
            "authority",
            id,
            &before.join("\n"),
            &after.join("\n"),
            &changed.join(","),
        )?;
    }
    Ok(changed)
}

/// Hide (`hidden_at = at`) or unhide authority `id`, with one
/// `authority_hide` / `authority_unhide` audit row naming `reason`.
/// Hiding also closes the row's open and batched pack tasks (`pack:*`
/// dedup keys: checks of names and links nobody will publish) as
/// `rejected`; in-progress work is left to its owner. No transaction of
/// its own. Returns false when the row was already in that state.
pub fn set_authority_hidden(
    conn: &Connection,
    id: i64,
    hidden: bool,
    actor: &str,
    at: i64,
    reason: &str,
) -> Result<bool> {
    let was_hidden: bool = conn.query_row(
        "SELECT hidden_at IS NOT NULL FROM authority WHERE id = ?1",
        [id],
        |row| row.get(0),
    )?;
    if was_hidden == hidden {
        return Ok(false);
    }
    if hidden {
        conn.execute(
            "UPDATE authority SET hidden_at = ?1 WHERE id = ?2",
            [at, id],
        )?;
        conn.execute(
            "UPDATE task SET status = 'rejected', resolved_at = ?1,
                    resolution = 'authority hidden'
             WHERE authority_id = ?2 AND status IN ('open', 'batched')
               AND dedup_key LIKE 'pack:%'",
            [at, id],
        )?;
    } else {
        conn.execute("UPDATE authority SET hidden_at = NULL WHERE id = ?1", [id])?;
    }
    let action = if hidden {
        "authority_hide"
    } else {
        "authority_unhide"
    };
    insert_audit(conn, at, actor, action, "authority", id, "", "", reason)?;
    Ok(true)
}

/// Delete voice-audio captures older than `VOICE_RETENTION_DAYS`.
///
/// For each expired `call_session.audio_capture_id`: removes the blob file
/// and its capture row, nulls the session link, and writes an `audit` row.
/// Transcripts, annotations, and consent records are never touched.
/// All database deletes run in one transaction. A stored path that does
/// not resolve inside the store root is skipped entirely (neither file
/// nor row is touched). Returns the number of audio captures deleted.
/// `now` is unix seconds (a parameter so tests don't depend on the clock).
pub fn retention_sweep(conn: &Connection, store: &Store, now: i64, actor: &str) -> Result<usize> {
    let cutoff = now.saturating_sub(VOICE_RETENTION_DAYS * 86_400);
    let expired: Vec<(i64, String)> = conn
        .prepare(
            "SELECT c.id, c.path FROM capture c
             JOIN call_session s ON s.audio_capture_id = c.id
             WHERE c.kind = 'audio' AND c.captured_at < ?1",
        )?
        .query_map([cutoff], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    // Canonicalize once: symlinked parents (e.g. macOS `/var` ->
    // `/private/var`) must compare against the resolved root.
    let canonical_root = store
        .root()
        .canonicalize()
        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;

    in_transaction(conn, || {
        let mut deleted = 0;
        for (id, path) in &expired {
            let full = store.root().join(path);
            // Traversal guard: the stored path is untrusted input. When
            // it does not resolve inside the store root, skip BOTH the
            // file and the row (fail closed; counted as skipped, not
            // deleted), so a malicious row can never delete outside.
            if !parent_inside_root(&canonical_root, &full) {
                continue;
            }
            // The row is the record; a missing file is already swept enough.
            if full.exists() {
                std::fs::remove_file(&full)
                    .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
            }
            conn.execute("DELETE FROM capture WHERE id = ?1", [id])?;
            conn.execute(
                "UPDATE call_session SET audio_capture_id = NULL
                 WHERE audio_capture_id = ?1",
                [id],
            )?;
            conn.execute(
                "INSERT INTO audit(at, actor, action, entity, entity_id, reason)
                 VALUES (?1, ?2, 'retention_delete', 'capture', ?3,
                         'voice audio older than 365 days')",
                rusqlite::params![now, actor, id],
            )?;
            deleted += 1;
        }
        Ok(deleted)
    })
}

/// Per-table counts from [`erase_customer_data`] (audit reason).
#[derive(Debug, Default, PartialEq, Eq)]
pub struct EraseReport {
    pub submissions: usize,
    pub shipments: usize,
    pub labels: usize,
    pub label_defaults: usize,
    pub captures: usize,
    pub blobs: usize,
    /// Customer orders erased (web-owned tables, when present).
    pub orders: usize,
}

/// Hard-erase one customer's handbook data (data-deletion
/// requests, spec §11.9): form submissions (incl. values_json
/// PII), shipments + labels (addresses, tracking, payloads),
/// label defaults, and the captures + blob files those rows
/// point at (signature PNGs, form PDFs, label PDFs). `hb` is the
/// metadata DB (captures, audit), `p2` the customer-PII DB;
/// children die before parents (shipment FK) and blob files get
/// the same traversal guard as [`retention_sweep`]. Voice rows are NOT
/// customer data (staff-initiated, third-party callees) and
/// audit rows are append-only — both stay, documented in the
/// privacy policy. One `customer_erased` audit row records the
/// counts under the (now-dangling, opaque) customer ref.
pub fn erase_customer_data(
    hb: &Connection,
    p2: &Connection,
    store: &Store,
    customer_ref: &str,
    now: i64,
) -> Result<EraseReport> {
    // PII rows first (P2 DB): a crash between the two transactions
    // leaves orphan captures, never orphaned PII rows pointing at
    // missing blobs the readers cannot handle.
    let report = in_transaction(p2, || {
        let mut report = EraseReport::default();
        let submission_ids: Vec<i64> = p2
            .prepare("SELECT id FROM form_submission WHERE customer_ref = ?1")?
            .query_map([customer_ref], |row| row.get(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut capture_ids: Vec<i64> = Vec::new();
        for id in &submission_ids {
            let (sig, pdf): (Option<i64>, Option<i64>) = p2.query_row(
                "SELECT signature_capture_id, pdf_capture_id FROM form_submission WHERE id = ?1",
                [id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?;
            capture_ids.extend(sig);
            capture_ids.extend(pdf);
        }
        // Web-owned order tables (created by the web crate at boot, so
        // guarded): their foreign keys point at shipment and
        // form_submission, so they go first or the deletes below fail.
        let has = |name: &str| -> rusqlite::Result<bool> {
            Ok(p2
                .query_row(
                    "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1",
                    [name],
                    |_| Ok(()),
                )
                .optional()?
                .is_some())
        };
        if has("customer_order")? {
            // Money outlives the erasure (re-review #35): an open or
            // captured, unrefunded payment is copied, without the
            // customer ref or any address, into an additive ledger so
            // staff can still refund it or expire its session.
            if has("order_payment")? {
                p2.execute_batch(
                    "CREATE TABLE IF NOT EXISTS erased_payment (
                        order_id TEXT PRIMARY KEY,
                        session_id TEXT NOT NULL,
                        payment_intent TEXT,
                        amount_cents INTEGER NOT NULL,
                        currency TEXT NOT NULL,
                        livemode INTEGER NOT NULL,
                        status TEXT NOT NULL,
                        paid_at INTEGER,
                        erased_at INTEGER NOT NULL
                    );",
                )?;
                p2.execute(
                    "INSERT OR REPLACE INTO erased_payment(order_id, session_id, payment_intent, amount_cents,
                         currency, livemode, status, paid_at, erased_at)
                     SELECT p.order_id, p.session_id, p.payment_intent, p.amount_cents, p.currency,
                            p.livemode, p.status, p.paid_at, ?2
                     FROM order_payment p JOIN customer_order o ON o.id = p.order_id
                     WHERE o.customer_ref = ?1
                       AND (p.status = 'open' OR (p.status = 'paid' AND p.refund_id IS NULL))",
                    rusqlite::params![customer_ref, now],
                )?;
            }
            for (table, col) in [
                ("order_event", "order_id"),
                ("order_shipment", "order_id"),
                ("order_payment", "order_id"),
            ] {
                if has(table)? {
                    p2.execute(
                        &format!(
                            "DELETE FROM {table} WHERE {col} IN
                             (SELECT id FROM customer_order WHERE customer_ref = ?1)"
                        ),
                        [customer_ref],
                    )?;
                }
            }
            report.orders += p2.execute(
                "DELETE FROM customer_order WHERE customer_ref = ?1",
                [customer_ref],
            )?;
        }
        let mut fill_paths: Vec<String> = Vec::new();
        if has("form_fill")? {
            fill_paths = p2
                .prepare("SELECT pdf_path FROM form_fill WHERE customer_ref = ?1")?
                .query_map([customer_ref], |row| row.get(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            p2.execute(
                "DELETE FROM form_fill WHERE customer_ref = ?1",
                [customer_ref],
            )?;
        }
        for rel in &fill_paths {
            // Filled official forms carry the customer's values.
            if rel.starts_with("fills/") && !rel.contains("..") {
                let _ = std::fs::remove_file(store.root().join(rel));
            }
        }
        if !submission_ids.is_empty() {
            let placeholders = submission_ids
                .iter()
                .map(|_| "?")
                .collect::<Vec<_>>()
                .join(",");
            let shipment_ids: Vec<i64> = p2
                .prepare(&format!(
                    "SELECT id FROM shipment WHERE submission_id IN ({placeholders})"
                ))?
                .query_map(rusqlite::params_from_iter(submission_ids.iter()), |row| {
                    row.get(0)
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            for id in &shipment_ids {
                let mut stmt = p2.prepare(
                    "SELECT label_capture_id FROM shipping_label WHERE shipment_id = ?1",
                )?;
                let rows = stmt.query_map([id], |row| row.get::<_, Option<i64>>(0))?;
                for row in rows {
                    capture_ids.extend(row?);
                }
                report.labels +=
                    p2.execute("DELETE FROM shipping_label WHERE shipment_id = ?1", [id])?;
                report.shipments += p2.execute("DELETE FROM shipment WHERE id = ?1", [id])?;
            }
            for id in &submission_ids {
                report.submissions +=
                    p2.execute("DELETE FROM form_submission WHERE id = ?1", [id])?;
            }
        }
        report.label_defaults += p2.execute(
            "DELETE FROM profile_label_defaults WHERE customer_ref = ?1",
            [customer_ref],
        )?;
        capture_ids.sort_unstable();
        capture_ids.dedup();
        let mut paths: Vec<(i64, String)> = Vec::new();
        for id in &capture_ids {
            let path: Option<String> = hb
                .query_row("SELECT path FROM capture WHERE id = ?1", [id], |row| {
                    row.get(0)
                })
                .optional()?;
            if let Some(path) = path {
                paths.push((*id, path));
            }
        }
        let canonical_root = store
            .root()
            .canonicalize()
            .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
        for (id, path) in &paths {
            // Never touch the id-0 sentinel (squashed 005): shared
            // infrastructure backing unattributed references.
            if *id <= 0 {
                continue;
            }
            let full = store.root().join(path);
            if !parent_inside_root(&canonical_root, &full) {
                continue;
            }
            if full.exists() {
                std::fs::remove_file(&full)
                    .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
                report.blobs += 1;
            }
            report.captures += hb.execute("DELETE FROM capture WHERE id = ?1", [id])?;
        }
        Ok(report)
    })?;
    // One audit row (metadata DB) under the now-dangling ref.
    hb.execute(
        "INSERT INTO audit(at, actor, action, entity, entity_id, reason)
         VALUES (?1, ?2, 'customer_erased', 'customer', 0, ?3)",
        rusqlite::params![
            now,
            customer_ref,
            format!(
                "submissions={} shipments={} labels={} defaults={} captures={} blobs={}",
                report.submissions,
                report.shipments,
                report.labels,
                report.label_defaults,
                report.captures,
                report.blobs
            ),
        ],
    )?;
    Ok(report)
}

/// True when `candidate`'s parent directory resolves (via the
/// filesystem) to a location inside `canonical_root`. Fail-closed: any
/// canonicalization failure reports "not inside".
fn parent_inside_root(canonical_root: &std::path::Path, candidate: &std::path::Path) -> bool {
    candidate
        .parent()
        .unwrap_or(candidate)
        .canonicalize()
        .is_ok_and(|parent| parent.starts_with(canonical_root))
}

/// Insert or replace the per-authority analysis watermark row (S2 lane).
pub fn upsert_watermark(
    conn: &Connection,
    slug: &str,
    at: i64,
    prompt_version: &str,
) -> Result<()> {
    conn.execute(
        "INSERT INTO watermark(authority_slug, last_run_at, last_prompt_version)
         VALUES (?1, ?2, ?3)
         ON CONFLICT(authority_slug) DO UPDATE SET
           last_run_at = excluded.last_run_at,
           last_prompt_version = excluded.last_prompt_version",
        rusqlite::params![slug, at, prompt_version],
    )?;
    Ok(())
}

/// Read back `(last_run_at, last_prompt_version)` for one authority, if any.
pub fn get_watermark(conn: &Connection, slug: &str) -> Result<Option<(i64, String)>> {
    match conn.query_row(
        "SELECT last_run_at, last_prompt_version FROM watermark
         WHERE authority_slug = ?1",
        [slug],
        |row| Ok((row.get(0)?, row.get(1)?)),
    ) {
        Ok(found) => Ok(Some(found)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// Highest analyzed capture id for one authority (0 = none yet).
/// The Option-B scan floor: captures at or below this id are done.
pub fn get_capture_watermark(conn: &Connection, slug: &str) -> Result<i64> {
    match conn.query_row(
        "SELECT last_capture_id FROM watermark WHERE authority_slug = ?1",
        [slug],
        |row| row.get(0),
    ) {
        Ok(id) => Ok(id),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(0),
        Err(e) => Err(e.into()),
    }
}

/// One row for [`insert_proposed_fact`].
pub struct NewProposedFact<'a> {
    pub authority_id: i64,
    pub capture_id: i64,
    pub topic: &'a str,
    pub statement: &'a str,
    pub created_by: &'a str,
    pub created_at: i64,
    /// Validated citation offsets into the capture sidecar, or `None`
    /// when uncited (stored as -1/-1).
    pub citation: Option<(u32, u32)>,
    /// Quarantined model summary (`fact.reasoning`; '' when absent).
    pub reasoning: &'a str,
}

/// Insert ONE `fact` row with `status='proposed'`. Returns the new row id.
///
/// Column mapping: `authority_id`, `capture_id`, `topic`, `statement`,
/// `created_by`, `created_at` pass through; `status` is always
/// `'proposed'`. The caller folds `field` into `statement`; citation
/// offsets index the fact's own capture sidecar (`None` → -1/-1).
pub fn insert_proposed_fact(conn: &Connection, f: &NewProposedFact<'_>) -> Result<i64> {
    let (start, end) = match f.citation {
        Some((s, e)) => (i64::from(s), i64::from(e)),
        None => (-1, -1),
    };
    conn.execute(
        "INSERT INTO fact(authority_id, capture_id, topic, statement,
                           status, created_by, created_at,
                           citation_start, citation_end, reasoning)
         VALUES (?1, ?2, ?3, ?4, 'proposed', ?5, ?6, ?7, ?8, ?9)",
        rusqlite::params![
            f.authority_id,
            f.capture_id,
            f.topic,
            f.statement,
            f.created_by,
            f.created_at,
            start,
            end,
            f.reasoning,
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Normalize a statement for suppression matching (§11.3): trim,
/// collapse internal whitespace, lowercase. One shared helper, used
/// identically by the analyzer, reject-with-suppress, and standalone
/// rule create.
pub fn normalize_statement(s: &str) -> String {
    s.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

// One field-level conflict rule (plan §5.1). Its JS twin is
// `web/src/lib/public-ui/compare.js`; both run the golden cases in
// `docs/contracts/conflict-cases.json`, so a change here lands there.

/// Topics whose facts never conflict: `service_notice` facts are
/// dated announcements that stack (newest first in the dossier
/// banner), so a second notice is news, not a disagreement.
pub const NON_CONFLICTING_TOPICS: &[&str] = &["service_notice"];

/// Topics that share one USPS identity. Two facts here that both name
/// a PO box are compared on box and ZIP whatever their keys or topic,
/// so a real box dispute cannot hide behind ad-hoc keys or the
/// `po_box`/`mailing` split. Courier street facts (no box) never are.
pub const USPS_TOPICS: &[&str] = &["mailing", "po_box"];

/// The statement grammar, exactly the guide's `parseStatement`
/// (`/^([a-z][a-z0-9_]*):\s+(.+)$/s`), with JavaScript's `\s` spelled
/// out so both twins split the same strings.
static STATEMENT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
        r"(?s)^([a-z][a-z0-9_]*):",
        r"[\t\n\x0B\x0C\r \x{A0}\x{1680}\x{2000}-\x{200A}\x{2028}\x{2029}\x{202F}\x{205F}\x{3000}\x{FEFF}]+",
        r"(.+)$",
    ))
    .expect("statement grammar compiles")
});

/// Split `{field}: {value}` into its key and value; anything else is
/// unkeyed, `(None, s)`. An instance suffix (`weekday__tumwater`) is
/// part of the key.
pub fn split_statement(s: &str) -> (Option<&str>, &str) {
    match STATEMENT.captures(s) {
        Some(c) => (
            c.get(1).map(|m| m.as_str()),
            c.get(2).map_or(s, |m| m.as_str()),
        ),
        None => (None, s),
    }
}

/// True when two statements of one topic describe the same field:
/// equal keys (instance suffix included), or either side unkeyed (a
/// legacy pin is compared with everything in its topic).
pub fn same_field(a: &str, b: &str) -> bool {
    !other_fields(split_statement(a).0, split_statement(b).0)
}

/// Weekday names and abbreviations; the first three letters name the day.
const DAY_NAMES: &str = "monday|tuesday|wednesday|thursday|friday|saturday|sunday\
                         |mon|tues|tue|weds|wed|thurs|thur|thu|fri|sat|sun";

/// Month names and abbreviations; the first three letters name the month.
const MONTH_NAMES: &str = "january|february|march|april|may|june|july|august\
                           |september|october|november|december\
                           |jan|feb|mar|apr|jun|jul|aug|sept|sep|oct|nov|dec";

const WEEKDAYS: [&str; 7] = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"];

const MONTHS: [&str; 12] = [
    "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
];

/// One token pattern over the [`token_text`] haystack, compiled once.
/// Every pattern is plain ASCII regex syntax the JS twin shares as is.
fn token_re(pattern: &str) -> Regex {
    Regex::new(pattern).expect("token pattern compiles")
}

static BOX_RE: LazyLock<Regex> = LazyLock::new(|| {
    token_re(r"\b(?:p ?\.? ?o ?\.?|post office) ?box ?(?::|#|no\.?|number)? ?([0-9]+)\b")
});
static AMT_RE: LazyLock<Regex> =
    LazyLock::new(|| token_re(r"\$ ?([0-9]{1,3}(?:,[0-9]{3})+|[0-9]+)(?:\.([0-9]{1,2}))?"));
static TEL_RE: LazyLock<Regex> = LazyLock::new(|| {
    token_re(
        r"(?:^|[^0-9])((?:\+?1[ .-]?)?(?:\(([0-9]{3})\) ?|([0-9]{3})[ .-]?)([0-9]{3})[ .-]?([0-9]{4}))\b",
    )
});
static DATE_MDY_RE: LazyLock<Regex> = LazyLock::new(|| {
    token_re(&format!(
        r"\b({MONTH_NAMES})\.? ([0-9]{{1,2}})(?:st|nd|rd|th)?,? ([0-9]{{4}})\b"
    ))
});
static DATE_DMY_RE: LazyLock<Regex> = LazyLock::new(|| {
    token_re(&format!(
        r"\b([0-9]{{1,2}})(?:st|nd|rd|th)? ({MONTH_NAMES})\.?,? ([0-9]{{4}})\b"
    ))
});
static DATE_NUM_RE: LazyLock<Regex> =
    LazyLock::new(|| token_re(r"\b([0-9]{1,2})[/]([0-9]{1,2})[/]([0-9]{4})\b"));
static DATE_ISO_RE: LazyLock<Regex> =
    LazyLock::new(|| token_re(r"\b([0-9]{4})-([0-9]{1,2})-([0-9]{1,2})\b"));
static ZIP_RE: LazyLock<Regex> = LazyLock::new(|| token_re(r"\b([0-9]{5})(?:-([0-9]{4}))?\b"));
static TIME_AMPM_RE: LazyLock<Regex> =
    LazyLock::new(|| token_re(r"\b([0-9]{1,2})(?::([0-9]{2}))? ?([ap])\.? ?m\b\.?"));
static TIME_NOON_RE: LazyLock<Regex> = LazyLock::new(|| token_re(r"\b(?:12 )?noon\b"));
static TIME_24H_RE: LazyLock<Regex> = LazyLock::new(|| token_re(r"\b([0-9]{1,2}):([0-9]{2})\b"));
static DAYS_RE: LazyLock<Regex> = LazyLock::new(|| {
    token_re(r"\b([0-9]{1,3})(?: ?(?:-|to) ?([0-9]{1,3}))? (?:business|working) days?\b")
});
static DOW_RANGE_RE: LazyLock<Regex> = LazyLock::new(|| {
    token_re(&format!(
        r"\b({DAY_NAMES})s?\b\.? ?(?:-|to|through|thru) ?({DAY_NAMES})s?\b\.?"
    ))
});
static DOW_RE: LazyLock<Regex> = LazyLock::new(|| token_re(&format!(r"\b({DAY_NAMES})s?\b\.?")));

/// The token haystack: lowercased, every dash variant folded to `-`,
/// every other non-ASCII or non-printing character (NBSP, newline) to
/// one space, runs collapsed and trimmed. Pure ASCII, so Rust byte
/// offsets and JS UTF-16 offsets agree.
fn token_text(v: &str) -> String {
    let mut out = String::with_capacity(v.len());
    for c in v.to_lowercase().chars() {
        let c = match c {
            '\u{2010}'..='\u{2014}' | '\u{2212}' => '-',
            c if c.is_ascii_graphic() => c,
            _ => ' ',
        };
        if c != ' ' || !(out.is_empty() || out.ends_with(' ')) {
            out.push(c);
        }
    }
    if out.ends_with(' ') {
        out.pop();
    }
    out
}

/// Read every match of `re` in `text`, then blank each match's `body`
/// group to spaces, so later kinds never re-read it (a box number is
/// not also a ZIP). `text` is ASCII, so blanking keeps every offset.
fn take(text: &mut String, re: &Regex, body: usize, mut read: impl FnMut(&Captures<'_>)) {
    let spans: Vec<(usize, usize)> = re
        .captures_iter(text.as_str())
        .filter_map(|c| {
            read(&c);
            c.get(body).map(|m| (m.start(), m.end()))
        })
        .collect();
    for (start, end) in spans {
        text.replace_range(start..end, &" ".repeat(end - start));
    }
}

/// `"007"` → `"7"`, `"000"` → `"0"`.
fn trim_zeros(digits: &str) -> &str {
    let trimmed = digits.trim_start_matches('0');
    if trimmed.is_empty() { "0" } else { trimmed }
}

/// `date:YYYY-MM-DD` for a calendar-shaped month/day, else nothing.
fn date_token(year: &str, month: u32, day: u32) -> Option<String> {
    ((1..=12).contains(&month) && (1..=31).contains(&day))
        .then(|| format!("date:{year}-{month:02}-{day:02}"))
}

/// Month number from a [`MONTH_NAMES`] match.
fn month_number(name: &str) -> u32 {
    MONTHS
        .iter()
        .position(|m| name.starts_with(m))
        .map_or(0, |i| i as u32 + 1)
}

/// Weekday index (Monday = 0) from a [`DAY_NAMES`] match.
fn weekday(name: &str) -> usize {
    WEEKDAYS
        .iter()
        .position(|d| name.starts_with(d))
        .unwrap_or(0)
}

/// Typed tokens of one statement value: the conflict rule's evidence,
/// each `kind:value`.
/// - `box:40228`: PO boxes ("PO Box", "P.O. Box", "PO BOX", "Post Office Box");
/// - `amt:1500`: dollar amounts in cents (`$15` = `$15.00`);
/// - `tel:3607250377`: 10-digit phone numbers in any common format;
/// - `date:2026-11-11`: full dates ("November 11, 2026", "11/11/2026", ISO);
/// - `zip:98501`; a ZIP+4 yields `zip:98504-0228` and also its bare
///   `zip:98504`, so under the identity rule `98504` agrees with
///   `98504-0228` while two different +4 codes still disagree;
/// - `time:16:00`: clock times with a.m./p.m., noon, or 24h; a bare
///   `h:mm` with no a.m./p.m. and an hour of 1-12 written without a
///   leading zero reads either way, as `time:03:30|15:30` (`3:30` is
///   03:30 or 15:30, `12:30` is 00:30 or 12:30; see [`times_agree`]);
/// - `days:7-10`: business- or working-day counts and ranges;
/// - `dow:mon`: weekdays, ranges expanded (`Mon-Thu` = mon..thu).
///
/// Kinds read in that order, each blanking what it took (see [`take`]).
pub fn value_tokens(v: &str) -> BTreeSet<String> {
    let mut text = token_text(v);
    let mut out = BTreeSet::new();
    take(&mut text, &BOX_RE, 0, |c| {
        out.insert(format!("box:{}", &c[1]));
    });
    take(&mut text, &AMT_RE, 0, |c| {
        let dollars: String = c[1].chars().filter(|ch| *ch != ',').collect();
        let cents = c.get(2).map_or("", |m| m.as_str());
        out.insert(format!(
            "amt:{}",
            trim_zeros(&format!("{dollars}{cents:0<2}"))
        ));
    });
    take(&mut text, &TEL_RE, 1, |c| {
        let area = c.get(2).or_else(|| c.get(3)).map_or("", |m| m.as_str());
        out.insert(format!("tel:{area}{}{}", &c[4], &c[5]));
    });
    let num = |c: &Captures<'_>, i: usize| c[i].parse::<u32>().unwrap_or(0);
    take(&mut text, &DATE_MDY_RE, 0, |c| {
        out.extend(date_token(&c[3], month_number(&c[1]), num(c, 2)));
    });
    take(&mut text, &DATE_DMY_RE, 0, |c| {
        out.extend(date_token(&c[3], month_number(&c[2]), num(c, 1)));
    });
    take(&mut text, &DATE_NUM_RE, 0, |c| {
        out.extend(date_token(&c[3], num(c, 1), num(c, 2)));
    });
    take(&mut text, &DATE_ISO_RE, 0, |c| {
        out.extend(date_token(&c[1], num(c, 2), num(c, 3)));
    });
    take(&mut text, &ZIP_RE, 0, |c| {
        out.insert(format!("zip:{}", &c[1]));
        if let Some(plus4) = c.get(2) {
            out.insert(format!("zip:{}-{}", &c[1], plus4.as_str()));
        }
    });
    take(&mut text, &TIME_AMPM_RE, 0, |c| {
        let (hour, minute) = (num(c, 1), c.get(2).map_or(0, |_| num(c, 2)));
        if (1..=12).contains(&hour) && minute < 60 {
            let hour = hour % 12 + if &c[3] == "p" { 12 } else { 0 };
            out.insert(format!("time:{hour:02}:{minute:02}"));
        }
    });
    take(&mut text, &TIME_NOON_RE, 0, |_| {
        out.insert("time:12:00".to_string());
    });
    take(&mut text, &TIME_24H_RE, 0, |c| {
        let (hour, minute) = (num(c, 1), num(c, 2));
        if (1..=12).contains(&hour) && minute < 60 && !c[1].starts_with('0') {
            let am = hour % 12;
            out.insert(format!(
                "time:{am:02}:{minute:02}|{:02}:{minute:02}",
                am + 12
            ));
        } else if hour < 24 && minute < 60 {
            out.insert(format!("time:{hour:02}:{minute:02}"));
        }
    });
    take(&mut text, &DAYS_RE, 0, |c| {
        out.insert(match c.get(2) {
            Some(_) => format!("days:{}-{}", num(c, 1), num(c, 2)),
            None => format!("days:{}", num(c, 1)),
        });
    });
    take(&mut text, &DOW_RANGE_RE, 0, |c| {
        let (from, to) = (weekday(&c[1]), weekday(&c[2]));
        for step in 0..=(to + 7 - from) % 7 {
            out.insert(format!("dow:{}", WEEKDAYS[(from + step) % 7]));
        }
    });
    take(&mut text, &DOW_RE, 0, |c| {
        out.insert(format!("dow:{}", WEEKDAYS[weekday(&c[1])]));
    });
    out
}

/// Token kinds, and how two sides' sets of one kind agree: identities
/// (box, ZIP, phone) when either set contains the other, so a less
/// specific value agrees; measures (amount, time, day count, date,
/// weekday) only when the sets are equal (times once each bare clock
/// time takes the reading the other side names, [`times_agree`]).
const TOKEN_KINDS: [(&str, bool); 8] = [
    ("box", true),
    ("zip", true),
    ("tel", true),
    ("amt", false),
    ("time", false),
    ("days", false),
    ("date", false),
    ("dow", false),
];

fn of_kind<'a>(tokens: &'a BTreeSet<String>, kind: &str) -> BTreeSet<&'a str> {
    tokens
        .iter()
        .filter(|t| t.split_once(':').is_some_and(|(k, _)| k == kind))
        .map(String::as_str)
        .collect()
}

/// Two identity sets disagree when both are present and neither
/// contains the other.
fn identities_disagree(a: &BTreeSet<&str>, b: &BTreeSet<&str>) -> bool {
    !a.is_empty() && !b.is_empty() && !a.is_subset(b) && !b.is_subset(a)
}

/// True when two ZIP strings agree under the conflict rule's identity
/// test ([`value_tokens`]): `98504` agrees with `98504-0228`, and two
/// different +4 codes of one ZIP disagree. A side with no ZIP agrees.
/// Orders' PO-box dispute check uses it, so labels and `/facts` read
/// one ZIP the same way.
pub fn zips_agree(a: &str, b: &str) -> bool {
    let (tokens_a, tokens_b) = (value_tokens(a), value_tokens(b));
    !identities_disagree(&of_kind(&tokens_a, "zip"), &of_kind(&tokens_b, "zip"))
}

/// Token-free values compare as text: [`normalize_statement`] after
/// folding punctuation. Letters, digits and `$` stay; `.`, `:` and `-`
/// stay only between two digits (`$15.00`, `4:30`, `98504-0228`);
/// other periods and apostrophes vanish (`P.O.` = `PO`); anything
/// else breaks words.
fn fold_text(v: &str) -> String {
    let chars: Vec<char> = v
        .to_lowercase()
        .chars()
        .map(|c| match c {
            '\u{2010}'..='\u{2014}' | '\u{2212}' => '-',
            c => c,
        })
        .collect();
    let digit = |i: Option<usize>| {
        i.and_then(|i| chars.get(i))
            .is_some_and(char::is_ascii_digit)
    };
    let mut out = String::with_capacity(v.len());
    for (i, &c) in chars.iter().enumerate() {
        if c.is_alphanumeric()
            || c == '$'
            || (matches!(c, '.' | ':' | '-') && digit(i.checked_sub(1)) && digit(Some(i + 1)))
        {
            out.push(c);
        } else if !matches!(c, '.' | '\'' | '\u{2019}') {
            out.push(' ');
        }
    }
    normalize_statement(&out)
}

/// One side's time tokens with each bare clock time
/// (`time:03:30|15:30`) read as the reading the other side names: its
/// p.m. reading if the other side has it, else its a.m. one, else as
/// written (so it agrees only with the same bare time).
fn read_times<'a>(own: &BTreeSet<&'a str>, other: &BTreeSet<&'a str>) -> BTreeSet<String> {
    own.iter()
        .map(|token| {
            let readings = token
                .strip_prefix("time:")
                .and_then(|v| v.split_once('|'))
                .filter(|_| !other.contains(token));
            let Some((am, pm)) = readings else {
                return (*token).to_string();
            };
            [pm, am]
                .iter()
                .map(|reading| format!("time:{reading}"))
                .find(|reading| other.contains(reading.as_str()))
                .unwrap_or_else(|| (*token).to_string())
        })
        .collect()
}

/// Two sides' time sets agree when they are equal once each bare clock
/// time takes the reading the other side names ([`read_times`]): `3:30`
/// agrees with `3:30 p.m.` and with `03:30`, and `8:00-4:30` with `8
/// a.m. to 4:30 p.m.`, while `3:30` still disagrees with `4:30 p.m.`.
fn times_agree(a: &BTreeSet<&str>, b: &BTreeSet<&str>) -> bool {
    read_times(a, b) == read_times(b, a)
}

/// The value rule over each side's [`value_tokens`]: every token kind
/// both sides carry must agree (see [`TOKEN_KINDS`]); sides that share
/// no kind compare as folded text.
fn values_disagree(
    a: &str,
    tokens_a: &BTreeSet<String>,
    b: &str,
    tokens_b: &BTreeSet<String>,
) -> bool {
    let mut shared = false;
    for (kind, identity) in TOKEN_KINDS {
        let (set_a, set_b) = (of_kind(tokens_a, kind), of_kind(tokens_b, kind));
        if set_a.is_empty() || set_b.is_empty() {
            continue;
        }
        shared = true;
        let agree = if identity {
            !identities_disagree(&set_a, &set_b)
        } else if kind == "time" {
            times_agree(&set_a, &set_b)
        } else {
            set_a == set_b
        };
        if !agree {
            return true;
        }
    }
    !shared && fold_text(a) != fold_text(b)
}

/// Both keyed with different keys: different fields.
fn other_fields(key_a: Option<&str>, key_b: Option<&str>) -> bool {
    matches!((key_a, key_b), (Some(a), Some(b)) if a != b)
}

/// True when two statements of ONE topic disagree: the same field
/// ([`same_field`]) with values that disagree (every shared token
/// kind must agree, else folded text must match). Different fields
/// never disagree. Callers comparing live facts use
/// [`facts_disagree`], which adds the topic rules.
pub fn statements_disagree(a: &str, b: &str) -> bool {
    let ((key_a, value_a), (key_b, value_b)) = (split_statement(a), split_statement(b));
    !other_fields(key_a, key_b)
        && values_disagree(
            value_a,
            &value_tokens(value_a),
            value_b,
            &value_tokens(value_b),
        )
}

/// The USPS identity rule: both sides name a PO box, and their box
/// sets differ or their ZIP sets disagree.
fn boxes_disagree(tokens_a: &BTreeSet<String>, tokens_b: &BTreeSet<String>) -> bool {
    let (box_a, box_b) = (of_kind(tokens_a, "box"), of_kind(tokens_b, "box"));
    if box_a.is_empty() || box_b.is_empty() {
        return false;
    }
    box_a != box_b || identities_disagree(&of_kind(tokens_a, "zip"), &of_kind(tokens_b, "zip"))
}

/// The conflict rule for two live facts of one authority.
/// [`NON_CONFLICTING_TOPICS`] never conflict; two [`USPS_TOPICS`] facts
/// that both name a PO box disagree on box or ZIP whatever their keys
/// or topic; otherwise only facts of one topic compare, by
/// [`statements_disagree`]. Each side is tokenized at most once.
pub fn facts_disagree(topic_a: &str, a: &str, topic_b: &str, b: &str) -> bool {
    if NON_CONFLICTING_TOPICS.contains(&topic_a) || NON_CONFLICTING_TOPICS.contains(&topic_b) {
        return false;
    }
    let usps = USPS_TOPICS.contains(&topic_a) && USPS_TOPICS.contains(&topic_b);
    let ((key_a, value_a), (key_b, value_b)) = (split_statement(a), split_statement(b));
    let one_field = topic_a == topic_b && !other_fields(key_a, key_b);
    if !usps && !one_field {
        return false;
    }
    let (tokens_a, tokens_b) = (value_tokens(value_a), value_tokens(value_b));
    (usps && boxes_disagree(&tokens_a, &tokens_b))
        || (one_field && values_disagree(value_a, &tokens_a, value_b, &tokens_b))
}

/// The topics whose live facts one fact is compared with under
/// [`facts_disagree`]: its own, plus every [`USPS_TOPICS`] topic when
/// it is one and names a PO box.
pub fn compared_topics<'a>(topic: &'a str, statement: &str) -> Vec<&'a str> {
    let names_box = || {
        value_tokens(split_statement(statement).1)
            .iter()
            .any(|t| t.starts_with("box:"))
    };
    if USPS_TOPICS.contains(&topic) && names_box() {
        USPS_TOPICS.to_vec()
    } else {
        vec![topic]
    }
}

/// A suppression-check hit: path A (rejected-memory) or B (rule id).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuppressionHit {
    RejectedMemory,
    Rule(i64),
}

/// Check one candidate against suppression (§11.3): path A
/// (normalized match on a `rejected` fact, same authority+topic),
/// then path B (live `suppress_rule`). Pure reads.
pub fn check_suppression(
    conn: &Connection,
    authority_id: i64,
    topic: &str,
    statement_norm: &str,
) -> Result<Option<SuppressionHit>> {
    let mut stmt = conn.prepare(
        "SELECT statement FROM fact
         WHERE authority_id = ?1 AND topic = ?2 AND status = 'rejected'",
    )?;
    let rows = stmt.query_map(rusqlite::params![authority_id, topic], |row| {
        row.get::<_, String>(0)
    })?;
    for statement in rows {
        if normalize_statement(&statement?) == statement_norm {
            return Ok(Some(SuppressionHit::RejectedMemory));
        }
    }
    let rule: Option<i64> = rusqlite::OptionalExtension::optional(conn.query_row(
        "SELECT id FROM suppress_rule
         WHERE revoked_at IS NULL AND statement_norm = ?1
           AND (authority_id IS NULL OR authority_id = ?2)
           AND (topic IS NULL OR topic = ?3)
         ORDER BY id LIMIT 1",
        rusqlite::params![statement_norm, authority_id, topic],
        |row| row.get(0),
    ))?;
    Ok(rule.map(SuppressionHit::Rule))
}

/// Log one analyzer skip (§11.3). Returns the new `suppression` row id.
#[allow(clippy::too_many_arguments)]
pub fn insert_suppression(
    conn: &Connection,
    capture_id: i64,
    authority_id: i64,
    topic: &str,
    statement: &str,
    statement_norm: &str,
    hit: SuppressionHit,
    created_at: i64,
) -> Result<i64> {
    let (path, rule_id) = match hit {
        SuppressionHit::RejectedMemory => ("A", None),
        SuppressionHit::Rule(id) => ("B", Some(id)),
    };
    conn.execute(
        "INSERT INTO suppression(capture_id, authority_id, topic, statement,
                                  statement_norm, rule_id, path, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        rusqlite::params![
            capture_id,
            authority_id,
            topic,
            statement,
            statement_norm,
            rule_id,
            path,
            created_at,
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Append one fully-specified `audit` row. Returns the new row id.
#[allow(clippy::too_many_arguments)]
pub fn insert_audit(
    conn: &Connection,
    at: i64,
    actor: &str,
    action: &str,
    entity: &str,
    entity_id: i64,
    before_ref: &str,
    after_ref: &str,
    reason: &str,
) -> Result<i64> {
    conn.execute(
        "INSERT INTO audit(at, actor, action, entity, entity_id,
                           before_ref, after_ref, reason)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        rusqlite::params![
            at, actor, action, entity, entity_id, before_ref, after_ref, reason
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Append one `action='analysis_run'` audit row for a finished analysis run.
/// `authority_id` is the run's `entity_id` (`entity='authority'`); `reason`
/// carries the proposed/rejected counts plus token usage.
pub fn insert_analysis_audit(
    conn: &Connection,
    at: i64,
    actor: &str,
    authority_id: i64,
    reason: &str,
) -> Result<i64> {
    conn.execute(
        "INSERT INTO audit(at, actor, action, entity, entity_id, reason)
         VALUES (?1, ?2, 'analysis_run', 'authority', ?3, ?4)",
        rusqlite::params![at, actor, authority_id, reason],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Hash a DEMO session secret for storage (H4: the table held
/// plaintext tokens, so a DB/file leak hijacked every demo session).
/// Tokens are 256-bit random hex, so plain SHA-256 suffices — the
/// same rationale as the magic-link hashes in `auth`.
fn hash_demo_token(token: &str) -> String {
    use sha2::Digest as _;
    let mut hasher = sha2::Sha256::new();
    hasher.update(token.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// Insert a DEMO session row (migration 003). Demo-only: no password is
/// checked anywhere; real auth (passkey/OAuth) replaces this table.
/// Stores only the token hash — the raw token travels in the cookie
/// and is never persisted (H4). Rows written before hashing return
/// no match (fail closed; demo sessions re-mint on next login).
pub fn create_demo_session(
    conn: &Connection,
    token: &str,
    customer_ref: &str,
    created_at: i64,
    expires_at: i64,
) -> Result<()> {
    conn.execute(
        "INSERT INTO demo_session(token, customer_ref, created_at, expires_at)
         VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params![hash_demo_token(token), customer_ref, created_at, expires_at],
    )?;
    Ok(())
}

/// Look up a DEMO session by token. Returns `(customer_ref, expires_at)`;
/// callers must reject expired rows (`now >= expires_at`).
pub fn get_demo_session(conn: &Connection, token: &str) -> Result<Option<(String, i64)>> {
    // Fully-qualified trait call: avoids touching the import block.
    // `?` lifts the rusqlite error into `contracts::Result`.
    Ok(rusqlite::OptionalExtension::optional(conn.query_row(
        "SELECT customer_ref, expires_at FROM demo_session WHERE token = ?1",
        [hash_demo_token(token)],
        |row| Ok((row.get(0)?, row.get(1)?)),
    ))?)
}

/// Delete DEMO sessions with `expires_at <= now`. Returns rows deleted.
pub fn delete_expired_demo_sessions(conn: &Connection, now: i64) -> Result<usize> {
    let deleted = conn.execute("DELETE FROM demo_session WHERE expires_at <= ?1", [now])?;
    Ok(deleted)
}

/// Saved per-customer label choices (P2 `profile_label_defaults`).
/// Spellings match `labels::{PrintFormat,PaperSize}::as_str`; readers
/// must treat unknown values as "use the default", never as an error.
pub struct LabelDefaults {
    pub option_a_format: String,
    pub option_b_paper: String,
    pub updated_at: i64,
}

/// Read saved label choices, if any.
pub fn get_label_defaults(conn: &Connection, customer_ref: &str) -> Result<Option<LabelDefaults>> {
    Ok(rusqlite::OptionalExtension::optional(conn.query_row(
        "SELECT option_a_format, option_b_paper, updated_at
         FROM profile_label_defaults WHERE customer_ref = ?1",
        [customer_ref],
        |row| {
            Ok(LabelDefaults {
                option_a_format: row.get(0)?,
                option_b_paper: row.get(1)?,
                updated_at: row.get(2)?,
            })
        },
    ))?)
}

/// Insert or replace saved label choices.
pub fn upsert_label_defaults(
    conn: &Connection,
    customer_ref: &str,
    option_a_format: &str,
    option_b_paper: &str,
    now: i64,
) -> Result<()> {
    conn.execute(
        "INSERT INTO profile_label_defaults(customer_ref, option_a_format, option_b_paper, updated_at)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(customer_ref) DO UPDATE SET
           option_a_format = excluded.option_a_format,
           option_b_paper = excluded.option_b_paper,
           updated_at = excluded.updated_at",
        rusqlite::params![customer_ref, option_a_format, option_b_paper, now],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn migrated() -> (tempfile::TempDir, Store, Connection) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path());
        store.init().unwrap();
        let conn = open(&store).unwrap();
        migrate(&conn).unwrap();
        (dir, store, conn)
    }

    #[test]
    fn baseline_has_source_fetch_state() {
        use rusqlite::OptionalExtension as _;
        let (_dir, _store, conn) = migrated();
        conn.execute(
            "INSERT INTO source(authority_id, kind, uri)
             VALUES (0, 'website_official', 'https://example.com/')",
            [],
        )
        .unwrap();
        let (cadence, etag, last_modified, content_hash, simhash, checked): (
            i64,
            String,
            String,
            String,
            i64,
            i64,
        ) = conn
            .query_row(
                "SELECT poll_cadence_sec, etag, last_modified, content_hash,
                        simhash, last_checked_at FROM source",
                [],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!((cadence, simhash, checked), (86400, 0, 0));
        assert!(etag.is_empty() && last_modified.is_empty() && content_hash.is_empty());
        let index: Option<String> = conn
            .query_row(
                "SELECT name FROM sqlite_master WHERE type = 'index' AND name = 'idx_source_due'",
                [],
                |row| row.get(0),
            )
            .optional()
            .unwrap();
        assert_eq!(index.as_deref(), Some("idx_source_due"));
    }

    #[test]
    fn baseline_has_mail_poll_cursor() {
        let (_dir, _store, conn) = migrated();
        conn.execute("INSERT INTO mail_poll(id) VALUES (1)", [])
            .unwrap();
        let (email, history_id, polled, error): (String, String, i64, String) = conn
            .query_row(
                "SELECT email, history_id, last_poll_at, last_error FROM mail_poll WHERE id = 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert!(email.is_empty() && history_id.is_empty() && error.is_empty());
        assert_eq!(polled, 0);
        // Singleton: a second row is refused.
        assert!(
            conn.execute("INSERT INTO mail_poll(id) VALUES (2)", [])
                .is_err()
        );
    }

    #[test]
    fn baseline_has_suppression_and_citation_home() {
        let (_dir, _store, conn) = migrated();
        for table in ["suppress_rule", "suppression"] {
            let found: String = conn
                .query_row(
                    "SELECT name FROM sqlite_master WHERE type = 'table' AND name = ?1",
                    [table],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(found, table);
        }
        let cols: Vec<String> = conn
            .prepare("PRAGMA table_info(fact)")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        for col in ["citation_start", "citation_end", "reasoning"] {
            assert!(cols.contains(&col.to_string()), "fact.{col} missing");
        }
        // Insert stores offsets + reasoning; None citation → -1/-1.
        let id = insert_proposed_fact(
            &conn,
            &NewProposedFact {
                authority_id: 0,
                capture_id: 0,
                topic: "address",
                statement: "po_box: PO Box 1",
                created_by: "test",
                created_at: 0,
                citation: Some((3, 11)),
                reasoning: "why",
            },
        )
        .unwrap();
        let (start, end, reasoning): (i64, i64, String) = conn
            .query_row(
                "SELECT citation_start, citation_end, reasoning FROM fact WHERE id = ?1",
                [id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!((start, end), (3, 11));
        assert_eq!(reasoning, "why");
        let bare = insert_proposed_fact(
            &conn,
            &NewProposedFact {
                authority_id: 0,
                capture_id: 0,
                topic: "address",
                statement: "s",
                created_by: "test",
                created_at: 0,
                citation: None,
                reasoning: "",
            },
        )
        .unwrap();
        let (start, end): (i64, i64) = conn
            .query_row(
                "SELECT citation_start, citation_end FROM fact WHERE id = ?1",
                [bare],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!((start, end), (-1, -1));
    }

    #[test]
    fn suppression_check_paths_and_scopes() {
        let (_dir, _store, conn) = migrated();
        let auth = add_authority(
            &conn,
            &NewAuthority {
                kind: "office",
                country: "Test",
                region: "",
                name: "T",
                slug: "t-sos",
                actor: "test",
                at: 0,
            },
        )
        .unwrap();
        // Path A: exact-norm match on a rejected row, same topic.
        conn.execute(
            "INSERT INTO fact(authority_id, capture_id, topic, statement, status, created_at)
             VALUES (?1, 0, 'address', 'PO Box  1', 'rejected', 0)",
            [auth],
        )
        .unwrap();
        assert_eq!(
            check_suppression(&conn, auth, "address", "po box 1").unwrap(),
            Some(SuppressionHit::RejectedMemory)
        );
        // Other topic: no hit.
        assert_eq!(
            check_suppression(&conn, auth, "hours", "po box 1").unwrap(),
            None
        );
        // Path B: global rule, any topic.
        conn.execute(
            "INSERT INTO suppress_rule(statement_norm, statement, created_at)
             VALUES ('fee $9', 'Fee $9', 0)",
            [],
        )
        .unwrap();
        assert_eq!(
            check_suppression(&conn, auth, "fees", "fee $9").unwrap(),
            Some(SuppressionHit::Rule(1))
        );
        // Revoked rules never match.
        conn.execute("UPDATE suppress_rule SET revoked_at = 1 WHERE id = 1", [])
            .unwrap();
        assert_eq!(
            check_suppression(&conn, auth, "fees", "fee $9").unwrap(),
            None
        );
        // Authority-scoped rule does not leak across authorities.
        conn.execute(
            "INSERT INTO suppress_rule(authority_id, topic, statement_norm, statement, created_at)
             VALUES (?1, 'fees', 'fee $8', 'Fee $8', 0)",
            [auth],
        )
        .unwrap();
        assert_eq!(
            check_suppression(&conn, auth + 999, "fees", "fee $8").unwrap(),
            None
        );
        assert!(matches!(
            check_suppression(&conn, auth, "fees", "fee $8").unwrap(),
            Some(SuppressionHit::Rule(_))
        ));
    }

    #[test]
    fn normalize_statement_folds_whitespace_and_case() {
        assert_eq!(normalize_statement("  PO   Box 40220. "), "po box 40220.");
        assert_eq!(normalize_statement("Fee\t$8\nMonday"), "fee $8 monday");
    }

    #[test]
    fn split_statement_keeps_the_guide_grammar() {
        assert_eq!(
            split_statement("per_document: $15"),
            (Some("per_document"), "$15")
        );
        assert_eq!(
            split_statement("weekday__tumwater: Mon-Thu"),
            (Some("weekday__tumwater"), "Mon-Thu")
        );
        for unkeyed in ["PO Box 40220", "UPPER: not a key", "field:nospace", ": v"] {
            assert_eq!(split_statement(unkeyed), (None, unkeyed));
        }
        // JavaScript's `\s`: NBSP splits, NEL does not.
        assert_eq!(
            split_statement("field:\u{a0}no-break space"),
            (Some("field"), "no-break space")
        );
        assert_eq!(split_statement("field:\u{85}x"), (None, "field:\u{85}x"));
        assert!(same_field("fee: $1", "fee: $2"));
        assert!(!same_field("fee: $1", "expedite: $2"));
        assert!(same_field("Fee is $1", "expedite: $2"));
    }

    fn tokens(v: &str) -> Vec<String> {
        value_tokens(v).into_iter().collect()
    }

    #[test]
    fn value_tokens_reads_each_kind_in_canonical_form() {
        assert_eq!(
            tokens("PO Box 40228, P.O. Box 7, Post Office Box #12"),
            ["box:12", "box:40228", "box:7"]
        );
        assert_eq!(
            tokens("$15, $15.00, $1,000.5 and $0.50"),
            ["amt:100050", "amt:1500", "amt:50"]
        );
        for phone in [
            "(360) 555-0100",
            "360.555.0100",
            "+1 360 555 0100",
            "1-360-555-0100",
            "3605550100",
        ] {
            assert_eq!(tokens(phone), ["tel:3605550100"], "{phone}");
        }
        // A ZIP+4 also yields its bare ZIP (the identity subset rule
        // then lets `98504` agree with `98504-0228`).
        assert_eq!(
            tokens("Olympia, WA 98504-0228; Tumwater 98501"),
            ["zip:98501", "zip:98504", "zip:98504-0228"]
        );
        assert_eq!(tokens("4:00 p.m., 4 PM, 16:00"), ["time:16:00"]);
        assert_eq!(
            tokens("12 noon, 12 a.m., 9:30am"),
            ["time:00:00", "time:09:30", "time:12:00"]
        );
        // A bare 12-hour clock time reads either way; a leading zero or
        // an hour past 12 is a 24-hour time.
        assert_eq!(
            tokens("8:00-4:30, 12:15, 08:15, 16:30"),
            [
                "time:00:15|12:15",
                "time:04:30|16:30",
                "time:08:00|20:00",
                "time:08:15",
                "time:16:30"
            ]
        );
        assert_eq!(
            tokens("7 to 10 business days, 2-3 working days, 1 business day"),
            ["days:1", "days:2-3", "days:7-10"]
        );
        for date in [
            "November 11, 2026",
            "Nov. 11th 2026",
            "11 November 2026",
            "11/11/2026",
            "2026-11-11",
        ] {
            assert_eq!(tokens(date), ["date:2026-11-11"], "{date}");
        }
        assert_eq!(
            tokens("Mon.-Thu."),
            ["dow:mon", "dow:thu", "dow:tue", "dow:wed"]
        );
        assert_eq!(tokens("Fridays"), ["dow:fri"]);
        assert_eq!(
            tokens("Fri-Mon"),
            ["dow:fri", "dow:mon", "dow:sat", "dow:sun"]
        );
        // A box number is never re-read as a ZIP; prose yields nothing.
        assert_eq!(tokens("PO Box 40228"), ["box:40228"]);
        assert!(tokens("Secretary of State").is_empty());
    }

    #[test]
    fn facts_disagree_compares_fields_not_topics() {
        // Different fields of one topic, then one field's two values.
        assert!(!facts_disagree(
            "fees",
            "per_document: $15",
            "fees",
            "expedite: $100"
        ));
        assert!(facts_disagree(
            "fees",
            "per_document: $15",
            "fees",
            "per_document: $20"
        ));
        // The USPS rule crosses keys and the po_box/mailing split.
        assert!(facts_disagree(
            "po_box",
            "po_box: PO Box 40220",
            "mailing",
            "mailing_address: PO Box 40234"
        ));
        assert!(!facts_disagree(
            "service_notice",
            "PO Box 40220",
            "service_notice",
            "PO Box 40234"
        ));
        // A bare ZIP agrees with its own ZIP+4, on one key and across
        // the USPS topics; two different +4 codes still disagree.
        assert!(!facts_disagree(
            "address",
            "office__olympia: Olympia, WA 98504",
            "address",
            "office__olympia: Olympia, WA 98504-0228"
        ));
        assert!(!facts_disagree(
            "po_box",
            "po_box: PO Box 40220, Olympia, WA 98504",
            "mailing",
            "apostille_mailing_address: PO Box 40220 Olympia, WA 98504-0220"
        ));
        assert!(facts_disagree(
            "address",
            "office__olympia: Olympia, WA 98504-0228",
            "address",
            "office__olympia: Olympia, WA 98504-0234"
        ));
        assert!(facts_disagree(
            "po_box",
            "po_box: PO Box 40220, Olympia, WA 98504-0220",
            "mailing",
            "apostille_mailing_address: PO Box 40220, Olympia, WA 98504-0234"
        ));
        assert_eq!(
            compared_topics("po_box", "po_box: PO Box 40220"),
            ["mailing", "po_box"]
        );
        assert_eq!(
            compared_topics("mailing", "courier: 243 Example Rd"),
            ["mailing"]
        );
        assert_eq!(compared_topics("address", "PO Box 40220"), ["address"]);
    }

    #[test]
    fn migrate_is_idempotent() {
        let (_dir, _store, conn) = migrated();
        migrate(&conn).unwrap();
        let version: i32 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, SCHEMA_VERSION);
    }

    #[test]
    fn add_authority_roundtrip() {
        let (_dir, _store, conn) = migrated();
        let id = add_authority(
            &conn,
            &NewAuthority {
                kind: "state",
                country: "USA",
                region: "Washington",
                name: "WA Secretary of State",
                slug: "usa-wa-sos",
                actor: "test",
                at: 1_800_000_000,
            },
        )
        .unwrap();
        let slug: String = conn
            .query_row("SELECT slug FROM authority WHERE id = ?1", [id], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(slug, "usa-wa-sos");
    }

    #[test]
    fn task_dedup_key_rejects_live_duplicates() {
        let (_dir, _store, conn) = migrated();
        let authority = add_authority(
            &conn,
            &NewAuthority {
                kind: "state",
                country: "USA",
                region: "WA",
                name: "WA",
                slug: "usa-wa",
                actor: "test",
                at: 1_800_000_000,
            },
        )
        .unwrap();
        let insert = || {
            conn.execute(
                "INSERT INTO task(authority_id, kind, title, status, dedup_key, created_at)
                 VALUES (?1, 'validate_fact', 't', 'open', 'same-key', 0)",
                [authority],
            )
        };
        insert().unwrap();
        insert().unwrap_err();
        // Exactly one live row with this key exists.
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM task WHERE dedup_key = 'same-key'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn retention_sweep_deletes_only_expired_audio() {
        let (_dir, store, conn) = migrated();
        let authority = add_authority(
            &conn,
            &NewAuthority {
                kind: "state",
                country: "USA",
                region: "WA",
                name: "WA",
                slug: "usa-wa",
                actor: "test",
                at: 1_800_000_000,
            },
        )
        .unwrap();
        conn.execute(
            "INSERT INTO channel(authority_id, visibility, kind, value, purpose)
             VALUES (?1, 'private', 'phone', '+1', 'our_validation')",
            [authority],
        )
        .unwrap();
        let channel = conn.last_insert_rowid();

        let now = 1_800_000_000;
        let old = now - 366 * 86_400;
        // Old audio (expired), old transcript (kept), fresh audio (kept).
        let audio_old = store.write_blob(b"old audio").unwrap();
        let audio_new = store.write_blob(b"new audio").unwrap();
        let transcript = store.write_blob(b"old transcript").unwrap();
        for (kind, sha, at) in [
            ("audio", audio_old.as_str(), old),
            ("transcript", transcript.as_str(), old),
            ("audio", audio_new.as_str(), now),
        ] {
            conn.execute(
                "INSERT INTO capture(authority_id, captured_at, kind, path,
                                     bytes_raw, bytes_stored, sha256, trust)
                 VALUES (?1, ?2, ?3, ?4, 0, 0, ?5, 'staff_verified')",
                rusqlite::params![
                    authority,
                    at,
                    kind,
                    store
                        .blob_path(sha)
                        .unwrap()
                        .strip_prefix(store.root())
                        .unwrap()
                        .to_str()
                        .unwrap(),
                    sha,
                ],
            )
            .unwrap();
        }
        let (audio_id, transcript_id, fresh_id): (i64, i64, i64) = conn
            .query_row(
                "SELECT
                   MAX(CASE WHEN kind = 'audio' AND captured_at = ?1 THEN id END),
                   MAX(CASE WHEN kind = 'transcript' THEN id END),
                   MAX(CASE WHEN kind = 'audio' AND captured_at = ?2 THEN id END)
                 FROM capture",
                rusqlite::params![old, now],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        conn.execute(
            "INSERT INTO call_session(authority_id, channel_id, consent,
                                      audio_capture_id, transcript_capture_id)
             VALUES (?1, ?2, 'granted', ?3, ?4)",
            rusqlite::params![authority, channel, audio_id, transcript_id],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO call_session(authority_id, channel_id, consent,
                                      audio_capture_id)
             VALUES (?1, ?2, 'granted', ?3)",
            rusqlite::params![authority, channel, fresh_id],
        )
        .unwrap();

        let deleted = retention_sweep(&conn, &store, now, "test").unwrap();
        assert_eq!(deleted, 1);

        // Expired audio: row gone, file gone, session link nulled.
        let audio_rows: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM capture WHERE id = ?1",
                [audio_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(audio_rows, 0);
        assert!(!store.blob_path(&audio_old).unwrap().exists());
        let link: Option<i64> = conn
            .query_row(
                "SELECT audio_capture_id FROM call_session
                 WHERE transcript_capture_id = ?1",
                [transcript_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(link, None);

        // Transcript and fresh audio untouched.
        for id in [transcript_id, fresh_id] {
            let rows: i64 = conn
                .query_row("SELECT COUNT(*) FROM capture WHERE id = ?1", [id], |row| {
                    row.get(0)
                })
                .unwrap();
            assert_eq!(rows, 1);
        }
        assert!(store.blob_path(&audio_new).unwrap().exists());

        // One audit row explains the deletion.
        let audits: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM audit
                 WHERE action = 'retention_delete' AND entity_id = ?1",
                [audio_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(audits, 1);
    }

    #[test]
    fn erase_customer_data_removes_pii_and_blobs() {
        let (_dir, store, hb) = migrated();
        let p2 = open_p2(&store).unwrap();
        migrate_p2(&p2).unwrap();
        let authority = add_authority(
            &hb,
            &NewAuthority {
                kind: "state",
                country: "USA",
                region: "WA",
                name: "WA",
                slug: "usa-wa",
                actor: "test",
                at: 1000,
            },
        )
        .unwrap();
        // Two captures with real blob files (signature PNG + form PDF).
        let mut cap_ids: Vec<(i64, String)> = Vec::new();
        for (kind, bytes) in [
            ("photo", b"sig-png".as_slice()),
            ("file", b"form-pdf".as_slice()),
        ] {
            let sha = store.write_blob(bytes).unwrap();
            let rel = store
                .blob_path(&sha)
                .unwrap()
                .strip_prefix(store.root())
                .unwrap()
                .to_str()
                .unwrap()
                .to_string();
            hb.execute(
                "INSERT INTO capture(authority_id, captured_at, kind, path,
                                     bytes_raw, bytes_stored, sha256, trust)
                 VALUES (?1, 1000, ?2, ?3, 0, 0, ?4, 'official')",
                rusqlite::params![authority, kind, rel, sha],
            )
            .unwrap();
            cap_ids.push((hb.last_insert_rowid(), sha));
        }
        let seeded: Vec<(i64, String)> = hb
            .prepare("SELECT id, path FROM capture")
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        // Two seeded rows plus the id-0 sentinel every DB ships.
        assert_eq!(seeded.len(), 3, "seeded rows: {seeded:?}");
        // One submission (u-1, both captures) + one stranger's (u-2).
        p2.execute(
            "INSERT INTO form_submission(form_template_id, authority_id, customer_ref,
                 values_json, signature_capture_id, pdf_capture_id, status, created_at)
             VALUES (1, ?1, 'u-1', '{\"name\":\"Ann\"}', ?2, ?3, 'submitted', 1000)",
            rusqlite::params![authority, cap_ids[0].0, cap_ids[1].0],
        )
        .unwrap();
        let sub1 = p2.last_insert_rowid();
        p2.execute(
            "INSERT INTO form_submission(form_template_id, authority_id, customer_ref,
                 values_json, status, created_at)
             VALUES (1, ?1, 'u-2', '{}', 'draft', 1000)",
            [authority],
        )
        .unwrap();
        p2.execute(
            "INSERT INTO shipment(submission_id, authority_id, status, from_json, to_json,
                 created_at, updated_at)
             VALUES (?1, ?2, 'ready', '{\"name\":\"Ann\"}', '{}', 1000, 1000)",
            rusqlite::params![sub1, authority],
        )
        .unwrap();
        let ship1 = p2.last_insert_rowid();
        p2.execute(
            "INSERT INTO shipping_label(shipment_id, direction, carrier, service, tracking_no,
                 created_at)
             VALUES (?1, 'outbound', 'usps', 'priority', '9400', 1000)",
            [ship1],
        )
        .unwrap();
        p2.execute(
            "INSERT INTO profile_label_defaults(customer_ref, updated_at) VALUES ('u-1', 1000)",
            [],
        )
        .unwrap();
        p2.execute(
            "INSERT INTO profile_label_defaults(customer_ref, updated_at) VALUES ('u-2', 1000)",
            [],
        )
        .unwrap();

        // A paid, unrefunded order (re-review #35): its payment must
        // survive the erasure in the de-identified ledger.
        p2.execute_batch(
            "CREATE TABLE IF NOT EXISTS customer_order (id TEXT PRIMARY KEY, customer_ref TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS order_payment (order_id TEXT PRIMARY KEY, session_id TEXT NOT NULL,
                 checkout_url TEXT NOT NULL, amount_cents INTEGER NOT NULL, currency TEXT NOT NULL,
                 livemode INTEGER NOT NULL, status TEXT NOT NULL, payment_intent TEXT,
                 created_at INTEGER NOT NULL, paid_at INTEGER, refund_id TEXT);
             INSERT INTO customer_order VALUES ('ord_a', 'u-1'), ('ord_b', 'u-2');
             INSERT INTO order_payment VALUES ('ord_a', 'cs_a', 'u', 4900, 'usd', 0, 'paid', 'pi_a', 1000, 1100, NULL);
             INSERT INTO order_payment VALUES ('ord_b', 'cs_b', 'u', 4900, 'usd', 0, 'paid', 'pi_b', 1000, 1100, NULL);",
        )
        .unwrap();
        let report = erase_customer_data(&hb, &p2, &store, "u-1", 2000).unwrap();
        let kept: (String, String, i64, i64) = p2
            .query_row(
                "SELECT order_id, payment_intent, amount_cents, erased_at FROM erased_payment",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .unwrap();
        assert_eq!(kept, ("ord_a".to_string(), "pi_a".to_string(), 4900, 2000));
        let left: i64 = p2
            .query_row("SELECT COUNT(*) FROM order_payment", [], |r| r.get(0))
            .unwrap();
        assert_eq!(left, 1, "the stranger's payment stays, u-1's is erased");
        assert_eq!(
            report,
            EraseReport {
                submissions: 1,
                shipments: 1,
                labels: 1,
                label_defaults: 1,
                captures: 2,
                blobs: 2,
                orders: 1,
            }
        );
        // PII rows gone; the stranger's rows survive.
        for (table, col) in [
            ("form_submission", "customer_ref"),
            ("profile_label_defaults", "customer_ref"),
        ] {
            let gone: i64 = p2
                .query_row(
                    &format!("SELECT COUNT(*) FROM {table} WHERE {col} = 'u-1'"),
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(gone, 0, "{table}");
            let kept: i64 = p2
                .query_row(
                    &format!("SELECT COUNT(*) FROM {table} WHERE {col} = 'u-2'"),
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(kept, 1, "{table}");
        }
        let labels: i64 = p2
            .query_row("SELECT COUNT(*) FROM shipping_label", [], |row| row.get(0))
            .unwrap();
        assert_eq!(labels, 0);
        let remaining: Vec<(i64, String, String)> = hb
            .prepare("SELECT id, kind, path FROM capture")
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        // Only the id-0 sentinel survives.
        assert_eq!(remaining, vec![(0, "file".to_string(), String::new())]);
        for (_, sha) in &cap_ids {
            assert!(!store.blob_path(sha).unwrap().exists());
        }
        // One audit row explains the erasure.
        let reason: String = hb
            .query_row(
                "SELECT reason FROM audit WHERE action = 'customer_erased'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(reason.contains("submissions=1"), "{reason}");
    }

    fn audit_trigger_count(conn: &Connection) -> i64 {
        conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master
             WHERE type = 'trigger' AND tbl_name = 'audit'",
            [],
            |row| row.get(0),
        )
        .unwrap()
    }

    #[test]
    fn open_sets_pragmas() {
        let (_dir, _store, conn) = migrated();
        let journal: String = conn
            .query_row("PRAGMA journal_mode", [], |row| row.get(0))
            .unwrap();
        assert_eq!(journal.to_lowercase(), "wal");
        let busy: i32 = conn
            .query_row("PRAGMA busy_timeout", [], |row| row.get(0))
            .unwrap();
        assert_eq!(busy, 5000);
        let synchronous: i32 = conn
            .query_row("PRAGMA synchronous", [], |row| row.get(0))
            .unwrap();
        assert_eq!(synchronous, 1); // NORMAL
        let foreign_keys: i32 = conn
            .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
            .unwrap();
        assert_eq!(foreign_keys, 1); // ON
    }

    #[test]
    fn migrate_fresh_db_applies_baseline() {
        let (_dir, _store, conn) = migrated();
        let version: i32 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, SCHEMA_VERSION);
        assert_eq!(audit_trigger_count(&conn), 2);
    }

    #[test]
    fn migrate_refuses_stale_v1_db() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path());
        store.init().unwrap();
        let conn = open(&store).unwrap();
        // A stranded v1 database: the version stamp alone triggers the
        // refusal — the old incremental path is gone.
        conn.execute("CREATE TABLE probe(id INTEGER PRIMARY KEY)", [])
            .unwrap();
        conn.execute("INSERT INTO probe(id) VALUES (1)", [])
            .unwrap();
        conn.pragma_update(None, "user_version", 1).unwrap();

        let err = migrate(&conn).unwrap_err().to_string();
        assert!(err.contains("schema v1"), "unexpected error: {err}");
        assert!(err.contains("incremental"), "unexpected error: {err}");

        // Refusal touches nothing: version and rows intact.
        let version: i32 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, 1);
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM probe", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn audit_is_append_only() {
        let (_dir, _store, conn) = migrated();
        conn.execute(
            "INSERT INTO audit(at, actor, action, entity, entity_id)
             VALUES (1, 'test', 'accept', 'fact', 7)",
            [],
        )
        .unwrap();
        conn.execute("UPDATE audit SET actor = 'evil' WHERE id = 1", [])
            .unwrap_err();
        conn.execute("DELETE FROM audit WHERE id = 1", [])
            .unwrap_err();
        // INSERT still works and the row is intact.
        conn.execute(
            "INSERT INTO audit(at, actor, action, entity, entity_id)
             VALUES (2, 'test', 'reject', 'fact', 8)",
            [],
        )
        .unwrap();
        let (rows, actor): (i64, String) = conn
            .query_row(
                "SELECT COUNT(*), MAX(CASE WHEN id = 1 THEN actor END) FROM audit",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!((rows, actor.as_str()), (2, "test"));
    }

    #[test]
    fn retention_sweep_skips_paths_outside_root() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().join("store"));
        store.init().unwrap();
        let conn = open(&store).unwrap();
        migrate(&conn).unwrap();

        // A sentinel file OUTSIDE the store root that a malicious
        // capture row tries to aim the sweep at.
        let sentinel = dir.path().join("sentinel.txt");
        std::fs::write(&sentinel, b"do not touch").unwrap();

        let authority = add_authority(
            &conn,
            &NewAuthority {
                kind: "state",
                country: "USA",
                region: "WA",
                name: "WA",
                slug: "usa-wa",
                actor: "test",
                at: 1_800_000_000,
            },
        )
        .unwrap();
        conn.execute(
            "INSERT INTO channel(authority_id, visibility, kind, value, purpose)
             VALUES (?1, 'private', 'phone', '+1', 'our_validation')",
            [authority],
        )
        .unwrap();
        let channel = conn.last_insert_rowid();

        let now = 1_800_000_000;
        let old = now - 366 * 86_400;
        // One legitimate expired blob plus one malicious row.
        let legit = store.write_blob(b"legit audio").unwrap();
        let legit_rel = store
            .blob_path(&legit)
            .unwrap()
            .strip_prefix(store.root())
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        for path in [legit_rel.as_str(), "../sentinel.txt"] {
            conn.execute(
                "INSERT INTO capture(authority_id, captured_at, kind, path,
                                     bytes_raw, bytes_stored, sha256, trust)
                 VALUES (?1, ?2, 'audio', ?3, 0, 0, 'x', 'staff_verified')",
                rusqlite::params![authority, old, path],
            )
            .unwrap();
            let capture = conn.last_insert_rowid();
            conn.execute(
                "INSERT INTO call_session(authority_id, channel_id, consent,
                                          audio_capture_id)
                 VALUES (?1, ?2, 'granted', ?3)",
                rusqlite::params![authority, channel, capture],
            )
            .unwrap();
        }

        let deleted = retention_sweep(&conn, &store, now, "test").unwrap();
        assert_eq!(deleted, 1);

        // Nothing outside the root was touched: sentinel intact ...
        assert_eq!(std::fs::read(&sentinel).unwrap(), b"do not touch");
        // ... and the malicious row survives (file AND row skipped).
        let evil_rows: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM capture WHERE path = '../sentinel.txt'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(evil_rows, 1);
        let evil_audits: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM audit WHERE action = 'retention_delete'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(evil_audits, 1);
        // The legitimate capture is fully swept.
        assert!(!store.blob_path(&legit).unwrap().exists());
    }

    #[test]
    fn retention_sweep_deletes_missing_file_row_skips_absolute_path() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().join("store"));
        store.init().unwrap();
        let conn = open(&store).unwrap();
        migrate(&conn).unwrap();

        let sentinel = dir.path().join("sentinel.txt");
        std::fs::write(&sentinel, b"do not touch").unwrap();

        let authority = add_authority(
            &conn,
            &NewAuthority {
                kind: "state",
                country: "USA",
                region: "WA",
                name: "WA",
                slug: "usa-wa",
                actor: "test",
                at: 1_800_000_000,
            },
        )
        .unwrap();
        conn.execute(
            "INSERT INTO channel(authority_id, visibility, kind, value, purpose)
             VALUES (?1, 'private', 'phone', '+1', 'our_validation')",
            [authority],
        )
        .unwrap();
        let channel = conn.last_insert_rowid();

        let now = 1_800_000_000;
        let old = now - 366 * 86_400;
        // A blob whose file vanished but whose parent dirs remain (the
        // guard needs a resolvable parent), plus an absolute path
        // outside the root (`Path::join` discards the root for
        // absolute joins — the guard must still catch it).
        let ghost = store.write_blob(b"ghost audio").unwrap();
        let ghost_full = store.blob_path(&ghost).unwrap();
        std::fs::remove_file(&ghost_full).unwrap();
        let ghost_rel = ghost_full
            .strip_prefix(store.root())
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        let absolute = sentinel.to_str().unwrap().to_string();
        for path in [ghost_rel.as_str(), absolute.as_str()] {
            conn.execute(
                "INSERT INTO capture(authority_id, captured_at, kind, path,
                                     bytes_raw, bytes_stored, sha256, trust)
                 VALUES (?1, ?2, 'audio', ?3, 0, 0, 'x', 'staff_verified')",
                rusqlite::params![authority, old, path],
            )
            .unwrap();
            let capture = conn.last_insert_rowid();
            conn.execute(
                "INSERT INTO call_session(authority_id, channel_id, consent,
                                          audio_capture_id)
                 VALUES (?1, ?2, 'granted', ?3)",
                rusqlite::params![authority, channel, capture],
            )
            .unwrap();
        }

        let deleted = retention_sweep(&conn, &store, now, "test").unwrap();
        assert_eq!(deleted, 1);

        // Missing file: row swept anyway ("already swept enough").
        let ghost_rows: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM capture WHERE path = ?1",
                [&ghost_rel],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(ghost_rows, 0);
        // Absolute path: skipped, sentinel intact.
        assert_eq!(std::fs::read(&sentinel).unwrap(), b"do not touch");
        let abs_rows: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM capture WHERE path = ?1",
                [&absolute],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(abs_rows, 1);
    }

    #[test]
    fn migrate_baseline_failure_rolls_back_cleanly() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path());
        store.init().unwrap();
        let conn = open(&store).unwrap();
        // A fresh (v0) database with a conflicting table name: the
        // baseline must fail, and the failure must roll back cleanly —
        // no partial schema, version still 0.
        conn.execute("CREATE TABLE authority(id INTEGER PRIMARY KEY)", [])
            .unwrap();

        migrate(&conn).unwrap_err();

        let version: i32 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, 0);
        let tables: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master
                 WHERE type = 'table' AND name != 'authority'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(tables, 0);
    }

    #[test]
    fn watermark_upsert_get_roundtrip() {
        let (_dir, _store, conn) = migrated();
        assert_eq!(get_watermark(&conn, "usa-wa").unwrap(), None);
        upsert_watermark(&conn, "usa-wa", 100, "aa-prompt-v1").unwrap();
        assert_eq!(
            get_watermark(&conn, "usa-wa").unwrap(),
            Some((100, "aa-prompt-v1".to_string()))
        );
        // Upsert replaces the row for the same authority.
        upsert_watermark(&conn, "usa-wa", 200, "aa-prompt-v2").unwrap();
        assert_eq!(
            get_watermark(&conn, "usa-wa").unwrap(),
            Some((200, "aa-prompt-v2".to_string()))
        );
        // Other authorities are independent rows.
        upsert_watermark(&conn, "usa-ca", 300, "aa-prompt-v1").unwrap();
        assert_eq!(
            get_watermark(&conn, "usa-ca").unwrap(),
            Some((300, "aa-prompt-v1".to_string()))
        );
    }

    #[test]
    fn demo_session_roundtrip() {
        let (_dir, _store, conn) = migrated();
        create_demo_session(&conn, "tok-1", "demo-1", 1000, 2000).unwrap();
        assert_eq!(
            get_demo_session(&conn, "tok-1").unwrap(),
            Some(("demo-1".to_string(), 2000))
        );
        assert_eq!(get_demo_session(&conn, "bogus").unwrap(), None);
    }

    #[test]
    fn demo_session_stores_hash_not_plaintext() {
        // H4: a DB/file leak must not hand out live credentials.
        let (_dir, _store, conn) = migrated();
        create_demo_session(&conn, "tok-1", "demo-1", 1000, 2000).unwrap();
        let stored: String = conn
            .query_row("SELECT token FROM demo_session", [], |row| row.get(0))
            .unwrap();
        assert_ne!(stored, "tok-1");
        assert_eq!(stored.len(), 64);
        // Lookup still resolves the raw token from the cookie.
        assert_eq!(
            get_demo_session(&conn, "tok-1").unwrap(),
            Some(("demo-1".to_string(), 2000))
        );
    }

    #[test]
    fn demo_session_expiry_sweep() {
        let (_dir, _store, conn) = migrated();
        create_demo_session(&conn, "old", "demo-1", 0, 999).unwrap();
        create_demo_session(&conn, "edge", "demo-1", 0, 1000).unwrap();
        create_demo_session(&conn, "live", "demo-2", 0, 1001).unwrap();
        assert_eq!(delete_expired_demo_sessions(&conn, 1000).unwrap(), 2);
        assert_eq!(get_demo_session(&conn, "old").unwrap(), None);
        assert_eq!(get_demo_session(&conn, "edge").unwrap(), None);
        assert_eq!(
            get_demo_session(&conn, "live").unwrap(),
            Some(("demo-2".to_string(), 1001))
        );
    }

    #[test]
    fn migrate_refuses_stale_mid_version_db() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path());
        store.init().unwrap();
        let conn = open(&store).unwrap();
        // v8: the last prod schema before the squash. Same refusal as
        // v1 — any version below the baseline must upgrade through
        // the last incremental release first.
        conn.pragma_update(None, "user_version", 8).unwrap();

        let err = migrate(&conn).unwrap_err().to_string();
        assert!(err.contains("schema v8"), "unexpected error: {err}");
        assert!(err.contains("v8→v11"), "unexpected error: {err}");

        let version: i32 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, 8);
    }

    /// A v11 database exactly as prod holds it: the squashed baseline
    /// alone, stamped 11 (no 012 step).
    fn v11_database() -> (tempfile::TempDir, Store, Connection) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path());
        store.init().unwrap();
        let conn = open(&store).unwrap();
        conn.execute_batch(include_str!("../migrations/schema_baseline.sql"))
            .unwrap();
        conn.pragma_update(None, "user_version", BASELINE_VERSION)
            .unwrap();
        (dir, store, conn)
    }

    type FactRow = (
        i64,
        i64,
        i64,
        String,
        String,
        String,
        String,
        i64,
        i64,
        i64,
        String,
    );

    fn fact_rows(conn: &Connection) -> Vec<FactRow> {
        conn.prepare(
            "SELECT id, authority_id, capture_id, topic, statement, status, created_by,
                    created_at, citation_start, citation_end, reasoning
             FROM fact ORDER BY id",
        )
        .unwrap()
        .query_map([], |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
                r.get(6)?,
                r.get(7)?,
                r.get(8)?,
                r.get(9)?,
                r.get(10)?,
            ))
        })
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
    }

    fn fk_list(conn: &Connection, table: &str) -> Vec<(String, String, String)> {
        conn.prepare(&format!(
            "SELECT \"table\", \"from\", \"to\" FROM pragma_foreign_key_list('{table}')
             ORDER BY \"from\""
        ))
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
    }

    fn insert_fact(
        conn: &Connection,
        authority: i64,
        capture: i64,
        topic: &str,
    ) -> rusqlite::Result<usize> {
        conn.execute(
            "INSERT INTO fact(authority_id, capture_id, topic, statement, status, created_at)
             VALUES (?1, ?2, ?3, 'notice: closed', 'proposed', 9)",
            rusqlite::params![authority, capture, topic],
        )
    }

    #[test]
    fn migrate_v11_to_v12_keeps_every_row_and_fk() {
        let (_dir, store, conn) = v11_database();
        let authority = add_authority(
            &conn,
            &NewAuthority {
                kind: "state",
                country: "USA",
                region: "Washington",
                name: "WA Secretary of State",
                slug: "usa-wa-sos",
                actor: "test",
                at: 1_800_000_000,
            },
        )
        .unwrap();
        conn.execute(
            "INSERT INTO source(authority_id, kind, uri) VALUES (?1, 'website_official',
             'https://www.sos.wa.gov/')",
            [authority],
        )
        .unwrap();
        let source = conn.last_insert_rowid();
        let sha = store.write_blob(b"v11 capture").unwrap();
        conn.execute(
            "INSERT INTO capture(authority_id, source_id, captured_at, kind, path,
                                 bytes_raw, bytes_stored, sha256, trust)
             VALUES (?1, ?2, 5, 'warc', 'x', 11, 11, ?3, 'official')",
            rusqlite::params![authority, source, sha],
        )
        .unwrap();
        let capture = conn.last_insert_rowid();
        for (topic, status, start, end, reasoning) in [
            ("fees", "accepted", 0, 12, "fee row"),
            ("hours", "proposed", 3, 20, ""),
            ("po_box", "superseded", -1, -1, "old box"),
            ("dropbox", "rejected", 1, 9, "SUPERSEDES #0"),
        ] {
            conn.execute(
                "INSERT INTO fact(authority_id, capture_id, topic, statement, status,
                                  created_by, created_at, citation_start, citation_end,
                                  reasoning)
                 VALUES (?1, ?2, ?3, ?4, ?5, 'muse-spark-1.3 via test', 7, ?6, ?7, ?8)",
                rusqlite::params![
                    authority,
                    capture,
                    topic,
                    format!("{topic}: value"),
                    status,
                    start,
                    end,
                    reasoning
                ],
            )
            .unwrap();
        }
        // An unattributed fact on the id-0 sentinel capture survives too.
        conn.execute(
            "INSERT INTO fact(authority_id, capture_id, topic, statement, status, created_at)
             VALUES (?1, 0, 'address', 'address: x', 'proposed', 8)",
            [authority],
        )
        .unwrap();
        let ship_to: i64 = conn
            .query_row("SELECT MIN(id) FROM fact", [], |r| r.get(0))
            .unwrap();
        conn.execute(
            "UPDATE authority SET ship_to_fact_id = ?1 WHERE id = ?2",
            [ship_to, authority],
        )
        .unwrap();
        // A boot-DDL child table (web `fact_translation` shape) whose
        // REFERENCES fact(id) must survive the rebuild.
        conn.execute_batch(
            "CREATE TABLE fact_translation(
               id INTEGER PRIMARY KEY,
               fact_id INTEGER NOT NULL REFERENCES fact(id),
               lang TEXT NOT NULL, statement TEXT NOT NULL DEFAULT '')",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO fact_translation(fact_id, lang, statement) VALUES (?1, 'es', 'tarifa')",
            [ship_to],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO audit(at, actor, action, entity, entity_id)
             VALUES (1, 'test', 'fact_accept', 'fact', ?1)",
            [ship_to],
        )
        .unwrap();
        // v11 refuses the new topic.
        insert_fact(&conn, authority, capture, "service_notice")
            .expect_err("v11 CHECK must reject service_notice");

        let facts_before = fact_rows(&conn);
        let fact_fks_before = fk_list(&conn, "fact");
        let authority_fks_before = fk_list(&conn, "authority");
        let audits_before: i64 = conn
            .query_row("SELECT COUNT(*) FROM audit", [], |r| r.get(0))
            .unwrap();

        migrate(&conn).unwrap();

        let version: i32 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(version, SCHEMA_VERSION);
        assert_eq!(fact_rows(&conn), facts_before);
        assert_eq!(fk_list(&conn, "fact"), fact_fks_before);
        assert_eq!(fk_list(&conn, "authority"), authority_fks_before);
        assert_eq!(
            fk_list(&conn, "fact_translation"),
            vec![("fact".to_string(), "fact_id".to_string(), "id".to_string())]
        );
        let ship_after: i64 = conn
            .query_row(
                "SELECT ship_to_fact_id FROM authority WHERE id = ?1",
                [authority],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(ship_after, ship_to);
        let translated: String = conn
            .query_row(
                "SELECT statement FROM fact_translation WHERE fact_id = ?1",
                [ship_to],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(translated, "tarifa");
        let audits_after: i64 = conn
            .query_row("SELECT COUNT(*) FROM audit", [], |r| r.get(0))
            .unwrap();
        assert_eq!(audits_after, audits_before);
        assert_eq!(audit_trigger_count(&conn), 2);
        let dangling: i64 = conn
            .query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(dangling, 0);
        let integrity: String = conn
            .query_row("PRAGMA integrity_check", [], |r| r.get(0))
            .unwrap();
        assert_eq!(integrity, "ok");
        let index: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'index'
                 AND name = 'idx_fact_lookup' AND tbl_name = 'fact'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(index, 1);
        let leftovers: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE name LIKE 'fact_new%'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(leftovers, 0);

        // The widened CHECK takes the new topic and still refuses junk;
        // FKs are still enforced in both directions.
        insert_fact(&conn, authority, capture, "service_notice").unwrap();
        insert_fact(&conn, authority, capture, "bitcoin").expect_err("unknown topic");
        insert_fact(&conn, 9_999, capture, "fees").expect_err("dangling authority");
        conn.execute("DELETE FROM fact WHERE id = ?1", [ship_to])
            .expect_err("authority.ship_to_fact_id still guards its fact");

        // Re-running is a no-op.
        migrate(&conn).unwrap();
        assert_eq!(fact_rows(&conn).len(), facts_before.len() + 1);
    }

    fn schema_of(conn: &Connection) -> Vec<(String, String, String)> {
        conn.prepare(
            "SELECT type, name, COALESCE(sql, '') FROM sqlite_master
             WHERE name NOT LIKE 'sqlite_%' ORDER BY type, name",
        )
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
    }

    #[test]
    fn fresh_schema_equals_v11_plus_the_step() {
        let (_fresh_dir, _fresh_store, fresh) = migrated();
        let (_old_dir, _old_store, upgraded) = v11_database();
        migrate(&upgraded).unwrap();
        assert_eq!(schema_of(&fresh), schema_of(&upgraded));
        let version: i32 = fresh
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(version, SCHEMA_VERSION);
    }

    #[test]
    fn migrate_refuses_every_version_outside_baseline_to_current() {
        for stale in [1, 10, SCHEMA_VERSION + 1, 99] {
            let dir = tempfile::tempdir().unwrap();
            let store = Store::new(dir.path());
            store.init().unwrap();
            let conn = open(&store).unwrap();
            conn.pragma_update(None, "user_version", stale).unwrap();
            let err = migrate(&conn).unwrap_err().to_string();
            assert!(err.contains(&format!("schema v{stale}")), "{err}");
            let remedy = if stale < BASELINE_VERSION {
                "incremental"
            } else {
                "newer binary"
            };
            assert!(err.contains(remedy), "{stale}: {err}");
            let version: i32 = conn
                .query_row("PRAGMA user_version", [], |r| r.get(0))
                .unwrap();
            assert_eq!(version, stale, "refusal must not restamp");
            let tables: i64 = conn
                .query_row("SELECT COUNT(*) FROM sqlite_master", [], |r| r.get(0))
                .unwrap();
            assert_eq!(tables, 0, "refusal must not touch the schema");
        }
    }

    #[test]
    fn constraints_reject_bad_values_and_dangling_refs() {
        let (_dir, _store, conn) = migrated();
        let authority = add_authority(
            &conn,
            &NewAuthority {
                kind: "state",
                country: "USA",
                region: "WA",
                name: "WA",
                slug: "usa-wa",
                actor: "test",
                at: 1_800_000_000,
            },
        )
        .unwrap();
        // Bad CHECK value on a controlled vocabulary.
        conn.execute(
            "INSERT INTO capture(authority_id, captured_at, kind, path,
                                 bytes_raw, bytes_stored, sha256, trust)
             VALUES (?1, 0, 'file', 'x', 0, 0, 's', 'bogus')",
            [authority],
        )
        .expect_err("bad trust value must be rejected");
        conn.execute(
            "INSERT INTO capture(authority_id, captured_at, kind, path,
                                 bytes_raw, bytes_stored, sha256, trust)
             VALUES (?1, 0, 'file', 'x', 0, 0, 's', 'official')",
            [authority],
        )
        .unwrap();
        let capture = conn.last_insert_rowid();
        // Bad fact.topic (not one of the 25 TOPICS.md strings).
        conn.execute(
            "INSERT INTO fact(authority_id, capture_id, topic, statement,
                              status, created_at)
             VALUES (?1, ?2, 'bitcoin', 's', 'proposed', 0)",
            rusqlite::params![authority, capture],
        )
        .expect_err("bad fact.topic must be rejected");
        // Dangling foreign key.
        conn.execute(
            "INSERT INTO capture(authority_id, captured_at, kind, path,
                                 bytes_raw, bytes_stored, sha256, trust)
             VALUES (99999, 0, 'file', 'x', 0, 0, 's', 'official')",
            [],
        )
        .expect_err("dangling capture.authority_id must be rejected");
    }

    #[test]
    fn audit_stays_append_only_after_constraint_rebuild() {
        let (_dir, _store, conn) = migrated();
        conn.execute(
            "INSERT INTO audit(at, actor, action, entity, entity_id)
             VALUES (1, 'test', 'accept', 'fact', 1)",
            [],
        )
        .unwrap();
        conn.execute("UPDATE audit SET actor = 'evil' WHERE id = 1", [])
            .expect_err("audit UPDATE must fail after the 005 rebuild");
        assert_eq!(audit_trigger_count(&conn), 2);
    }

    #[test]
    fn config_tables_roundtrip_and_constraints() {
        let (_dir, _store, conn) = migrated();
        conn.execute(
            "INSERT INTO config(key, type, value_enc, enc_version, updated_at, updated_by)
             VALUES ('app.mode', 'text', x'0102', 1, 100, 'test')",
            [],
        )
        .unwrap();
        let (type_, enc_version): (String, i64) = conn
            .query_row(
                "SELECT type, enc_version FROM config WHERE key = 'app.mode'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!((type_.as_str(), enc_version), ("text", 1));
        // Unknown key type violates the CHECK.
        conn.execute(
            "INSERT INTO config(key, type, value_enc, enc_version, updated_at, updated_by)
             VALUES ('bad', 'bogus', x'00', 1, 100, 'test')",
            [],
        )
        .expect_err("bad config.type must be rejected");
        // STRICT table: TEXT is not storable in an INTEGER column.
        conn.execute(
            "INSERT INTO config(key, type, value_enc, enc_version, updated_at, updated_by)
             VALUES ('strict', 'text', x'00', 'one', 100, 'test')",
            [],
        )
        .expect_err("STRICT must reject TEXT in enc_version");
        conn.execute(
            "INSERT INTO config_kek(version, cred_path, created_at, retired_at)
             VALUES (1, '/etc/credstore.encrypted/handbook-kek-v1.cred', 100, NULL)",
            [],
        )
        .unwrap();
        let retired: Option<i64> = conn
            .query_row(
                "SELECT retired_at FROM config_kek WHERE version = 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(retired, None);
    }

    #[test]
    fn form_template_active_unique_and_constraints() {
        let (_dir, _store, conn) = migrated();
        let authority = add_authority(
            &conn,
            &NewAuthority {
                kind: "state",
                country: "USA",
                region: "WA",
                name: "WA",
                slug: "usa-wa",
                actor: "test",
                at: 1_800_000_000,
            },
        )
        .unwrap();
        let insert = |authority: i64, code: &str, status: &str| {
            conn.execute(
                "INSERT INTO form_template(authority_id, code, title, version,
                                           status, schema_json, created_at)
                 VALUES (?1, ?2, 'WA request', 1, ?3, '{}', 0)",
                rusqlite::params![authority, code, status],
            )
        };
        insert(authority, "wa-apostille-request", "active").unwrap();
        // Exactly one active template per (authority, code).
        insert(authority, "wa-apostille-request", "active")
            .expect_err("second active template must be rejected");
        // Non-active rows with the same code are fine (partial index).
        insert(authority, "wa-apostille-request", "superseded").unwrap();
        // Unknown status violates the CHECK.
        insert(authority, "wa-other", "bogus").expect_err("bad status must be rejected");
        // Dangling authority violates the FK.
        insert(99999, "wa-orphan", "draft").expect_err("dangling authority must be rejected");
        // Spec defaults landed on the first row.
        let (replica, drawn, mode, cadence): (String, String, String, i64) = conn
            .query_row(
                "SELECT accepts_replica, accepts_drawn_signature, signature_mode,
                        verify_cadence_days
                 FROM form_template WHERE code = 'wa-apostille-request' AND status = 'active'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(
            (replica.as_str(), drawn.as_str(), mode.as_str(), cadence),
            ("unknown", "no", "wet_ink", 90)
        );
    }

    #[test]
    fn authority_p2_columns_default_and_checked() {
        let (_dir, _store, conn) = migrated();
        let authority = add_authority(
            &conn,
            &NewAuthority {
                kind: "state",
                country: "USA",
                region: "WA",
                name: "WA",
                slug: "usa-wa",
                actor: "test",
                at: 1_800_000_000,
            },
        )
        .unwrap();
        let (replica_default, ship_to): (String, Option<i64>) = conn
            .query_row(
                "SELECT replica_default, ship_to_fact_id FROM authority WHERE id = ?1",
                [authority],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(replica_default, "unknown");
        assert_eq!(ship_to, None);
        conn.execute(
            "UPDATE authority SET replica_default = 'bogus' WHERE id = ?1",
            [authority],
        )
        .expect_err("bad replica_default must be rejected");
    }

    #[test]
    fn fact_topic_check_matches_frozen_enum() {
        let (_dir, store, conn) = migrated();
        let authority = add_authority(
            &conn,
            &NewAuthority {
                kind: "state",
                country: "USA",
                region: "WA",
                name: "WA",
                slug: "usa-wa",
                actor: "test",
                at: 1_800_000_000,
            },
        )
        .unwrap();
        let sha = store.write_blob(b"freeze proof").unwrap();
        conn.execute(
            "INSERT INTO capture(authority_id, captured_at, kind, path,
                                 bytes_raw, bytes_stored, sha256, trust)
             VALUES (?1, 0, 'file', 'x', 0, 0, ?2, 'official')",
            rusqlite::params![authority, sha],
        )
        .unwrap();
        let capture = conn.last_insert_rowid();
        // Every frozen topic string is storable; the CHECK and the enum
        // agree exactly (rejections are covered by contracts' parse test
        // plus constraints_reject_bad_values_and_dangling_refs).
        for topic in contracts::topics::Topic::ALL {
            conn.execute(
                "INSERT INTO fact(authority_id, capture_id, topic, statement,
                                  status, created_at)
                 VALUES (?1, ?2, ?3, 's', 'proposed', 0)",
                rusqlite::params![authority, capture, topic.as_str()],
            )
            .unwrap();
        }
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM fact", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, contracts::topics::Topic::ALL.len() as i64);
    }

    #[test]
    fn baseline_seeds_no_facts_or_fixtures() {
        let (_dir, _store, conn) = migrated();
        // The baseline inserts no facts: fixtures can only enter via
        // test helpers (crate::fixtures, cfg(test)) or real ingest.
        let facts: i64 = conn
            .query_row("SELECT COUNT(*) FROM fact", [], |row| row.get(0))
            .unwrap();
        assert_eq!(facts, 0);
        // The only seeded rows are the id-0 sentinels from migration 005.
        let slugs: Vec<String> = conn
            .prepare("SELECT slug FROM authority ORDER BY id")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        assert_eq!(slugs, ["sentinel-unattributed-005"]);
        // And no seeded text anywhere carries the canonical fixture values.
        let mut leaks = 0;
        for (table, column) in [
            ("authority", "name"),
            ("authority", "slug"),
            ("capture", "path"),
            ("capture", "sha256"),
            ("capture", "notes"),
            ("source", "uri"),
            ("source", "notes"),
            ("domain", "name"),
            ("domain", "notes"),
        ] {
            let hits: i64 = conn
                .query_row(
                    &format!(
                        "SELECT COUNT(*) FROM {table} \
                         WHERE {column} LIKE '%40220%' OR {column} LIKE '%40234%'"
                    ),
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            leaks += hits;
        }
        assert_eq!(leaks, 0, "fixture values leaked into seeded rows");
    }

    fn migrated_p2() -> (tempfile::TempDir, Store, Connection) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path());
        store.init().unwrap();
        let conn = open_p2(&store).unwrap();
        migrate_p2(&conn).unwrap();
        (dir, store, conn)
    }

    #[test]
    fn migrate_p2_is_idempotent() {
        let (_dir, _store, conn) = migrated_p2();
        migrate_p2(&conn).unwrap();
        let version: i32 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, P2_SCHEMA_VERSION);
    }

    #[test]
    fn open_p2_sets_pragmas() {
        let (_dir, _store, conn) = migrated_p2();
        let journal: String = conn
            .query_row("PRAGMA journal_mode", [], |row| row.get(0))
            .unwrap();
        assert_eq!(journal.to_lowercase(), "wal");
        let busy: i32 = conn
            .query_row("PRAGMA busy_timeout", [], |row| row.get(0))
            .unwrap();
        assert_eq!(busy, 5000);
        let foreign_keys: i32 = conn
            .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
            .unwrap();
        assert_eq!(foreign_keys, 1); // ON
    }

    #[test]
    fn p2_tables_roundtrip_and_constraints() {
        let (_dir, _store, conn) = migrated_p2();
        conn.execute(
            "INSERT INTO form_submission(form_template_id, authority_id, customer_ref,
                                        values_json, status, created_at)
             VALUES (7, 1, 'cust-1', '{\"fee\": 30}', 'submitted', 100)",
            [],
        )
        .unwrap();
        let submission = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO shipment(submission_id, authority_id, status, created_at, updated_at)
             VALUES (?1, 1, 'labels_pending', 100, 100)",
            [submission],
        )
        .unwrap();
        let shipment = conn.last_insert_rowid();
        for direction in ["outbound", "return"] {
            conn.execute(
                "INSERT INTO shipping_label(shipment_id, direction, carrier, service,
                                           tracking_no, status, created_at)
                 VALUES (?1, ?2, 'usps', 'PRIORITY', 'track-1', 'issued', 100)",
                rusqlite::params![shipment, direction],
            )
            .unwrap();
        }
        // The tracking index is non-unique per the spec sketch: duplicate
        // tracking numbers insert fine (reissued labels share numbers).
        let labels: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM shipping_label WHERE tracking_no = 'track-1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(labels, 2);
        let index: String = conn
            .query_row(
                "SELECT name FROM sqlite_master
                 WHERE type = 'index' AND name = 'idx_label_tracking'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(index, "idx_label_tracking");
        // Unknown direction violates the CHECK.
        conn.execute(
            "INSERT INTO shipping_label(shipment_id, direction, carrier, service, created_at)
             VALUES (?1, 'sideways', 'usps', 'PRIORITY', 100)",
            [shipment],
        )
        .expect_err("bad direction must be rejected");
        // Dangling in-file parent violates the FK (opaque cross-DB ids
        // like form_template_id/authorities have no FK by design).
        conn.execute(
            "INSERT INTO shipment(submission_id, authority_id, status, created_at, updated_at)
             VALUES (99999, 1, 'draft', 100, 100)",
            [],
        )
        .expect_err("dangling submission_id must be rejected");
    }

    #[test]
    fn migrate_p2_refuses_stale_v1_db() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path());
        store.init().unwrap();
        let conn = open_p2(&store).unwrap();
        // A stranded P2 v1 database: same loud refusal as the main
        // schema — upgrade through the last incremental release.
        conn.pragma_update(None, "user_version", 1).unwrap();

        let err = migrate_p2(&conn).unwrap_err().to_string();
        assert!(err.contains("p2.sqlite"), "unexpected error: {err}");
        assert!(err.contains("schema v1"), "unexpected error: {err}");

        let version: i32 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, 1);
    }

    #[test]
    fn p2_baseline_carries_label_purchase_shape() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path());
        store.init().unwrap();
        let conn = open_p2(&store).unwrap();
        migrate_p2(&conn).unwrap();
        conn.execute(
            "INSERT INTO form_submission(form_template_id, authority_id, customer_ref, status, created_at)
             VALUES (7, 1, 'cust-1', 'submitted', 100)",
            [],
        )
        .unwrap();
        let submission = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO shipment(submission_id, authority_id, status, created_at, updated_at)
             VALUES (?1, 1, 'ready', 100, 100)",
            [submission],
        )
        .unwrap();
        let shipment = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO shipping_label(shipment_id, direction, carrier, service,
                                       tracking_no, status, created_at)
             VALUES (?1, 'outbound', 'usps', 'PRIORITY', 'track-1', 'issued', 100)",
            [shipment],
        )
        .unwrap();
        // Fresh rows default to prepaid 4x6 PDFs (squashed p2_002 shape).
        let (option, format, sha, content): (String, String, String, String) = conn
            .query_row(
                "SELECT purchase_option, label_format, label_sha256, content_type
                 FROM shipping_label WHERE shipment_id = ?1",
                [shipment],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(option, "prepaid");
        assert_eq!(format, "four_by_six");
        assert_eq!(sha, "");
        assert_eq!(content, "application/pdf");
        conn.execute(
            "INSERT INTO shipping_label(shipment_id, direction, carrier, service, created_at,
                                       purchase_option)
             VALUES (?1, 'return', 'usps', 'PRIORITY', 100, 'telegraph')",
            [shipment],
        )
        .expect_err("bad purchase_option must be rejected");
    }

    #[test]
    fn label_defaults_roundtrip_per_customer() {
        let (_dir, _store, conn) = migrated_p2();
        assert!(get_label_defaults(&conn, "cust-1").unwrap().is_none());
        upsert_label_defaults(&conn, "cust-1", "window_envelope", "letter", 100).unwrap();
        let got = get_label_defaults(&conn, "cust-1").unwrap().unwrap();
        assert_eq!(got.option_a_format, "window_envelope");
        assert_eq!(got.option_b_paper, "letter");
        assert_eq!(got.updated_at, 100);
        upsert_label_defaults(&conn, "cust-1", "a4", "a4", 200).unwrap();
        let got = get_label_defaults(&conn, "cust-1").unwrap().unwrap();
        assert_eq!(got.option_a_format, "a4");
        assert_eq!(got.updated_at, 200);
        assert!(get_label_defaults(&conn, "cust-2").unwrap().is_none());
    }

    /// A v12 store exactly as prod holds it before 013: baseline + 012.
    fn v12_database() -> (tempfile::TempDir, Store, Connection) {
        let (dir, store, conn) = v11_database();
        conn.execute_batch(include_str!("../migrations/012_service_notice.sql"))
            .unwrap();
        conn.pragma_update(None, "user_version", 12).unwrap();
        (dir, store, conn)
    }

    fn hidden_at(conn: &Connection, slug: &str) -> Option<i64> {
        conn.query_row(
            "SELECT hidden_at FROM authority WHERE slug = ?1",
            [slug],
            |r| r.get(0),
        )
        .unwrap()
    }

    #[test]
    fn migrate_v12_to_v13_hides_the_mail_inbox_and_keeps_rows() {
        let (_dir, _store, conn) = v12_database();
        for (slug, name) in [("usa-wa-sos", "WA SoS"), ("mail-inbox", "Mail inbox")] {
            add_authority(
                &conn,
                &NewAuthority {
                    kind: "office",
                    country: "",
                    region: "",
                    name,
                    slug,
                    actor: "test",
                    at: 1,
                },
            )
            .unwrap();
        }
        let ids_before: Vec<(i64, String)> = conn
            .prepare("SELECT id, slug FROM authority ORDER BY id")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();

        migrate(&conn).unwrap();

        let version: i32 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(version, 13);
        let ids_after: Vec<(i64, String)> = conn
            .prepare("SELECT id, slug FROM authority ORDER BY id")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(ids_after, ids_before);
        assert!(hidden_at(&conn, "mail-inbox").is_some());
        assert_eq!(hidden_at(&conn, "usa-wa-sos"), None);
        let hides: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM audit WHERE action = 'authority_hide'
                 AND actor = 'migration-013'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(hides, 1);
        // `level` is a closed list; the path table exists for pack loads.
        conn.execute(
            "UPDATE authority SET level = 'federal' WHERE slug = 'usa-wa-sos'",
            [],
        )
        .unwrap();
        conn.execute(
            "UPDATE authority SET level = 'galactic' WHERE slug = 'usa-wa-sos'",
            [],
        )
        .expect_err("unknown level");
        conn.execute(
            "INSERT INTO authority_path(path, authority_slug, is_primary)
             VALUES ('usa/washington', 'usa-wa-sos', 1)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO authority_scope(authority_id, doc_class, issued_in, source_uri)
             SELECT id, 'vital_record', 'US-WA', 'https://example.gov/' FROM authority
             WHERE slug = 'usa-wa-sos'",
            [],
        )
        .unwrap();
        // Re-running is a no-op (no second audit row).
        migrate(&conn).unwrap();
        let hides_again: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM audit WHERE action = 'authority_hide'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(hides_again, 1);
    }

    #[test]
    fn edit_and_hide_write_one_audit_row_each_and_close_pack_tasks() {
        let (_dir, _store, conn) = migrated();
        let id = add_authority(
            &conn,
            &NewAuthority {
                kind: "country",
                country: "Germany",
                region: "",
                name: "Land",
                slug: "de-ca-1",
                actor: "test",
                at: 1,
            },
        )
        .unwrap();
        for (key, status) in [
            ("pack:DE:obs:1", "open"),
            ("pack:DE:step:x", "in_progress"),
            ("call:de", "open"),
        ] {
            conn.execute(
                "INSERT INTO task(authority_id, kind, title, status, dedup_key, created_at)
                 VALUES (?1, 'validate_fact', 't', ?2, ?3, 1)",
                rusqlite::params![id, status, key],
            )
            .unwrap();
        }
        let changed = edit_authority(
            &conn,
            id,
            &[
                ("name", "Germany — Competent Authority (Art. 6)"),
                ("kind", "country"),
            ],
            "pack:DE",
            2,
        )
        .unwrap();
        assert_eq!(changed, vec!["name".to_string()]);
        assert!(
            edit_authority(
                &conn,
                id,
                &[("name", "Germany — Competent Authority (Art. 6)")],
                "x",
                3
            )
            .unwrap()
            .is_empty()
        );
        edit_authority(&conn, id, &[("slug", "x")], "x", 3).expect_err("slug is not editable");
        let (before, after): (String, String) = conn
            .query_row(
                "SELECT before_ref, after_ref FROM audit WHERE action = 'authority_update'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(before, "name=Land");
        assert_eq!(after, "name=Germany — Competent Authority (Art. 6)");

        assert!(set_authority_hidden(&conn, id, true, "pack:DE", 4, "fragment").unwrap());
        assert!(!set_authority_hidden(&conn, id, true, "pack:DE", 5, "again").unwrap());
        assert_eq!(hidden_at(&conn, "de-ca-1"), Some(4));
        let statuses: Vec<(String, String)> = conn
            .prepare("SELECT dedup_key, status FROM task ORDER BY id")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(
            statuses,
            [
                ("pack:DE:obs:1".to_string(), "rejected".to_string()),
                ("pack:DE:step:x".to_string(), "in_progress".to_string()),
                ("call:de".to_string(), "open".to_string()),
            ]
        );
        assert!(set_authority_hidden(&conn, id, false, "staff:1", 6, "real office").unwrap());
        assert_eq!(hidden_at(&conn, "de-ca-1"), None);
        let actions: Vec<String> = conn
            .prepare("SELECT action FROM audit WHERE entity_id = ?1 ORDER BY id")
            .unwrap()
            .query_map([id], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(
            actions,
            [
                "authority_add",
                "authority_update",
                "authority_hide",
                "authority_unhide"
            ]
        );
    }
}
