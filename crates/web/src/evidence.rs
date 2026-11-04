//! Considerations lane (plan §5.4): staff notes on phone calls, emails
//! and visits, staff decisions, and absences our own check found.
//! `POST /api/v1/admin/evidence` publishes one note and `…/{id}/withdraw`
//! takes it down (staff + CSRF, audited, fanned out as ids-only
//! `evidence_updated` on `authority:{slug}`); the public `/plan` read
//! carries the published notes as `evidence[]`.
//!
//! A note is context beside the facts, never a source: it never enters
//! a fact's `citations`, never changes a fact's status, and the public
//! shape names a role ("Staff reviewer"), never `created_by`. Calls go
//! out from a staff phone and are never linked to recordings, so a note
//! carries no call id, and `recorded` is refused until a consent lane
//! exists.
//!
//! Guards (every refusal is a 422 with a plain reason):
//! - call, email and visit notes anchor to an accepted fact of the
//!   office (`fact_id`); absence and decision notes anchor to a slot;
//! - a decision prefers a fact that is in conflict on the public read
//!   and rests on published call, email, visit or absence notes;
//! - the summary is 8-280 characters of staff-written plain text, with
//!   plain digits only: no format characters (Unicode category Cf: the
//!   zero-width space, soft hyphen, joiners, bidi controls) or other
//!   invisible code points (the Braille blank included), and no digit
//!   other than ASCII 0-9 (fullwidth, mathematical, superscript, circled
//!   or other-script look-alikes), any of which would split or disguise
//!   a number the browser still shows whole;
//! - contact details must already be public. Instead of
//!   `reports::mask_pii` (which masks every 7+ digit run, the office's
//!   own phone and ZIP+4 included), every 7+ digit run must occur in the
//!   office's accepted facts, the groups of a number joined across any
//!   separator a person or a look-alike could put between them
//!   ([`joins_digits`]: spaces, dots, slashes, dashes of every kind,
//!   parentheses, a comma between two digits, any non-ASCII symbol); and
//!   the only email address a summary may name is the channel's, which
//!   must itself be one the office publishes, read after folding
//!   fullwidth and small forms (`＠`, `﹫`, `．`) and with letters of any
//!   script ([`emails_in`]);
//! - the read re-checks: a note leaves `/plan` once its anchor fact, or
//!   the accepted fact that made one of its numbers or emails public, is
//!   retired.

use std::collections::BTreeSet;
use std::sync::LazyLock;

use axum::{
    Router,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Json, Response},
    routing::{get, post},
};
use handbook_store::{db, store::Store};
use regex::Regex;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::admin::AdminAuth;
use crate::approval::FactView;
use crate::{AppState, store_unavailable, unix_now};

/// Note kinds (DB CHECK).
const KINDS: &[&str] = &["call", "email", "visit", "decision", "absence"];
/// Kinds that report what the office told or showed us: they anchor
/// to an accepted fact. The other two anchor to a slot.
const FACT_KINDS: &[&str] = &["call", "email", "visit"];
const STANCES: &[&str] = &["confirms", "contradicts", "context"];
const SUMMARY_MIN: usize = 8;
const SUMMARY_MAX: usize = 280;
const MAX_CHANNEL: usize = 160;
const MAX_SLOT: usize = 64;
/// Most notes one decision may rest on.
const MAX_SUPPORTS: usize = 20;
/// Most published notes one `/plan` read serves (the newest).
const MAX_PUBLIC: i64 = 200;
/// A staff clock may run this far ahead of the server's.
const CLOCK_SKEW_SECS: i64 = 3_600;
/// Shortest digit run the contact guard checks (a local phone number).
const MIN_DIGIT_RUN: usize = 7;
/// The public byline: a role, never the staff member.
pub const PUBLIC_BY: &str = "Staff reviewer";

// ------------------------------------------------------------------ DB

/// Boot DDL on the metadata DB (`handbook.sqlite`): `CREATE TABLE IF
/// NOT EXISTS` only — no ALTER, DROP or RENAME (docs/ops/deploy.md §
/// Rails). No REFERENCES: the store runs `foreign_keys=ON` and the
/// referenced tables may be absent in some stores, so the handlers
/// check every id instead.
pub fn migrate_tables(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS evidence_note (
            id INTEGER PRIMARY KEY,
            authority_slug TEXT NOT NULL,
            kind TEXT NOT NULL
                CHECK(kind IN ('call', 'email', 'visit', 'decision', 'absence')),
            fact_id INTEGER,
            preferred_fact_id INTEGER,
            slot TEXT NOT NULL DEFAULT '',
            stance TEXT NOT NULL DEFAULT 'context'
                CHECK(stance IN ('confirms', 'contradicts', 'context')),
            occurred_at INTEGER NOT NULL,
            channel TEXT NOT NULL DEFAULT '',
            summary TEXT NOT NULL,
            recorded INTEGER NOT NULL DEFAULT 0,
            capture_id INTEGER,
            supports TEXT NOT NULL DEFAULT '[]',
            status TEXT NOT NULL DEFAULT 'published'
                CHECK(status IN ('published', 'withdrawn')),
            created_by TEXT NOT NULL,
            created_at INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS evidence_note_slug ON evidence_note(authority_slug, status);",
    )
}

/// One stored note.
#[derive(Debug, Clone)]
struct Note {
    id: i64,
    slug: String,
    kind: String,
    fact_id: Option<i64>,
    preferred_fact_id: Option<i64>,
    slot: String,
    stance: String,
    occurred_at: i64,
    channel: String,
    summary: String,
    recorded: bool,
    capture_id: Option<i64>,
    /// Note ids a decision rests on (`[]` for the other kinds).
    supports: Vec<i64>,
    status: String,
    created_by: String,
    created_at: i64,
}

const COLS: &str = "id, authority_slug, kind, fact_id, preferred_fact_id, slot, stance,
                    occurred_at, channel, summary, recorded, capture_id, supports, status,
                    created_by, created_at";

fn row_of(r: &rusqlite::Row<'_>) -> rusqlite::Result<Note> {
    let supports: String = r.get(12)?;
    Ok(Note {
        id: r.get(0)?,
        slug: r.get(1)?,
        kind: r.get(2)?,
        fact_id: r.get(3)?,
        preferred_fact_id: r.get(4)?,
        slot: r.get(5)?,
        stance: r.get(6)?,
        occurred_at: r.get(7)?,
        channel: r.get(8)?,
        summary: r.get(9)?,
        recorded: r.get::<_, i64>(10)? != 0,
        capture_id: r.get(11)?,
        supports: serde_json::from_str(&supports).unwrap_or_default(),
        status: r.get(13)?,
        created_by: r.get(14)?,
        created_at: r.get(15)?,
    })
}

impl Note {
    /// Staff shape (admin routes): every column plus the Pacific date.
    fn staff_view(&self) -> Value {
        json!({
            "id": self.id,
            "authority_slug": self.slug,
            "kind": self.kind,
            "fact_id": self.fact_id,
            "preferred_fact_id": self.preferred_fact_id,
            "slot": self.slot,
            "stance": self.stance,
            "occurred_at": self.occurred_at,
            "date": la_date(self.occurred_at),
            "channel": self.channel,
            "summary": self.summary,
            "recorded": self.recorded,
            "capture_id": self.capture_id,
            "supports": self.supports,
            "status": self.status,
            "created_by": self.created_by,
            "created_at": self.created_at,
        })
    }

    /// Public shape (`/plan` → `evidence[]`): a role byline and the
    /// day only — never `created_by`, the capture or the clock time.
    /// `supports` lists the visible notes a decision rests on.
    fn public_view(&self, supports: &[i64]) -> Value {
        json!({
            "id": self.id,
            "kind": self.kind,
            "stance": self.stance,
            "fact_id": self.fact_id,
            "preferred_fact_id": self.preferred_fact_id,
            "slot": self.slot,
            "date": la_date(self.occurred_at),
            "channel": self.channel,
            "summary": self.summary,
            "recorded": self.recorded,
            "supports": supports,
            "by": PUBLIC_BY,
        })
    }
}

fn open(state: &AppState) -> Option<Connection> {
    db::open(&Store::new(&state.store_root)).ok()
}

// ------------------------------------------------------------ dates

/// Unix seconds of the `nth` Sunday of `month` in `year`, at `hour`
/// UTC (Hinnant's days-from-civil; no date crate).
fn nth_sunday_utc(year: i64, month: i64, nth: i64, hour: i64) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * ((month + 9) % 12) + 2) / 5;
    let first = era * 146_097 + yoe * 365 + yoe / 4 - yoe / 100 + doy - 719_468;
    // 1970-01-01 was a Thursday, so `days + 4` is 0 mod 7 on Sundays.
    let sunday = first + (7 - (first + 4).rem_euclid(7)) % 7 + 7 * (nth - 1);
    sunday * 86_400 + hour * 3_600
}

/// Seconds east of UTC on the America/Los_Angeles clock at `unix` (US
/// rules since 2007: PDT from 2:00 a.m. PST on the second Sunday of
/// March to 2:00 a.m. PDT on the first Sunday of November).
fn la_offset(unix: i64) -> i64 {
    // The DST window never straddles New Year, so testing the years
    // around an average-length estimate is exact.
    let year = 1970 + unix.div_euclid(31_556_952);
    let dst = (year - 1..=year + 1)
        .any(|y| (nth_sunday_utc(y, 3, 2, 10)..nth_sunday_utc(y, 11, 1, 9)).contains(&unix));
    if dst { -7 * 3_600 } else { -8 * 3_600 }
}

/// `YYYY-MM-DD` of `unix` on the America/Los_Angeles calendar.
fn la_date(unix: i64) -> String {
    notify::iso_day(unix + la_offset(unix))
}

// ----------------------------------------------------------- guards

/// `POST /admin/evidence` body. Every field defaults, so a missing
/// one answers with a plain reason rather than a parse error; an
/// unknown one (`call_id`, `created_by`, `status`) is refused.
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct NoteBody {
    pub authority_slug: Option<String>,
    pub kind: Option<String>,
    pub fact_id: Option<i64>,
    pub preferred_fact_id: Option<i64>,
    pub slot: Option<String>,
    pub stance: Option<String>,
    /// Unix seconds.
    pub occurred_at: Option<i64>,
    pub channel: Option<String>,
    pub summary: Option<String>,
    pub capture_id: Option<i64>,
    pub supports: Option<Vec<i64>>,
    /// `true`/`1` is refused: there is no consent lane yet.
    pub recorded: Value,
}

/// A body that passed the stateless guards.
#[derive(Debug)]
struct Draft {
    slug: String,
    kind: String,
    fact_id: Option<i64>,
    preferred_fact_id: Option<i64>,
    slot: String,
    stance: String,
    occurred_at: i64,
    channel: String,
    summary: String,
    capture_id: Option<i64>,
    supports: Vec<i64>,
}

/// Code points that render as nothing: every format character (Unicode
/// general category Cf: soft hyphen, zero-width space and joiners, word
/// joiner, bidi embeddings, overrides and isolates, BOM), every other
/// default-ignorable one (combining grapheme joiner, variation
/// selectors, Hangul fillers), and the Braille pattern blank (U+2800, a
/// symbol that draws as a space). Placed between digits they split a
/// number for the digit-run check while the browser still shows it
/// whole, so the contact guard refuses them outright.
static INVISIBLE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"[\p{Cf}\p{Default_Ignorable_Code_Point}\u{2800}]")
        .expect("invisible class compiles")
});

/// True when `text` writes a number the digit-run check could miss: an
/// invisible code point ([`INVISIBLE`]), or a digit other than ASCII
/// 0-9 (`char::is_numeric` but not `is_ascii_digit`: fullwidth `２`,
/// mathematical `𝟐`, superscript `²`, circled `②`, Arabic-Indic `٢`
/// and other look-alikes).
fn hides_digits(text: &str) -> bool {
    INVISIBLE.is_match(text) || text.chars().any(|c| c.is_numeric() && !c.is_ascii_digit())
}

/// Staff text as stored: whitespace runs collapse to one space; any
/// other control character refuses the field, and so does any
/// invisible or look-alike character that could disguise a number
/// ([`hides_digits`]), before the digit-run check reads it.
fn plain_text(field: &str, raw: Option<&str>) -> Result<String, String> {
    let text = raw
        .unwrap_or("")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if text.chars().any(char::is_control) {
        return Err(format!("{field} must be plain text"));
    }
    if hides_digits(&text) {
        return Err(format!(
            "{field} must write numbers in plain digits (0-9), with no invisible \
             characters or look-alike digits"
        ));
    }
    Ok(text)
}

/// A slot key: dot-separated lowercase segments (`fees.same_day`,
/// `address.office__tumwater`), at most 64 characters.
fn slot_ok(slot: &str) -> bool {
    slot.len() <= MAX_SLOT
        && slot.split('.').all(|seg| {
            seg.starts_with(|c: char| c.is_ascii_lowercase())
                && seg
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        })
}

/// The stateless guards: shape, lengths, kinds and which anchor each
/// kind carries. The office's facts are checked later, in the write.
fn check_body(body: NoteBody, now: i64) -> Result<Draft, String> {
    let kind = body.kind.unwrap_or_default();
    if !KINDS.contains(&kind.as_str()) {
        return Err("kind must be call, email, visit, decision or absence".into());
    }
    let stance = body.stance.unwrap_or_default();
    if !STANCES.contains(&stance.as_str()) {
        return Err("stance must be confirms, contradicts or context".into());
    }
    let unrecorded = match &body.recorded {
        Value::Null | Value::Bool(false) => true,
        Value::Number(n) => n.as_i64() == Some(0),
        _ => false,
    };
    if !unrecorded {
        return Err("recorded notes are refused until a consent lane exists".into());
    }
    let slug = body.authority_slug.unwrap_or_default().trim().to_string();
    if slug.is_empty() {
        return Err("authority_slug required".into());
    }
    let summary = plain_text("summary", body.summary.as_deref())?;
    if !(SUMMARY_MIN..=SUMMARY_MAX).contains(&summary.chars().count()) {
        return Err(format!(
            "summary must be {SUMMARY_MIN}-{SUMMARY_MAX} characters"
        ));
    }
    let channel = plain_text("channel", body.channel.as_deref())?;
    if channel.chars().count() > MAX_CHANNEL {
        return Err(format!("channel must be at most {MAX_CHANNEL} characters"));
    }
    let slot = body.slot.unwrap_or_default().trim().to_string();
    if !slot.is_empty() && !slot_ok(&slot) {
        return Err(format!(
            "slot must be a lowercase key like fees.same_day (at most {MAX_SLOT} characters)"
        ));
    }
    let occurred_at = body.occurred_at.unwrap_or(0);
    if !(1..=now + CLOCK_SKEW_SECS).contains(&occurred_at) {
        return Err("occurred_at must be a past time in unix seconds".into());
    }
    if body.capture_id.is_some_and(|id| id <= 0) {
        return Err("capture_id must name a capture of this office".into());
    }
    let supports = body.supports.unwrap_or_default();
    if FACT_KINDS.contains(&kind.as_str()) {
        if !body.fact_id.is_some_and(|id| id > 0) {
            return Err("a call, email or visit note anchors to an accepted fact (fact_id)".into());
        }
        if channel.is_empty() {
            return Err("a call, email or visit note names its channel".into());
        }
    } else {
        if body.fact_id.is_some() {
            return Err("an absence or decision note anchors to a slot, not a fact".into());
        }
        if slot.is_empty() {
            return Err("an absence or decision note needs a slot".into());
        }
    }
    if kind == "decision" {
        if !body.preferred_fact_id.is_some_and(|id| id > 0) {
            return Err("a decision needs a preferred_fact_id".into());
        }
        if supports.is_empty() {
            return Err("a decision needs at least one supporting note (supports)".into());
        }
        let distinct: BTreeSet<i64> = supports.iter().copied().collect();
        if supports.len() > MAX_SUPPORTS
            || distinct.len() != supports.len()
            || supports.iter().any(|id| *id <= 0)
        {
            return Err(format!(
                "supports must list 1-{MAX_SUPPORTS} distinct note ids"
            ));
        }
    } else {
        if body.preferred_fact_id.is_some() {
            return Err("only a decision names a preferred fact".into());
        }
        if !supports.is_empty() {
            return Err("only a decision lists supporting notes".into());
        }
    }
    Ok(Draft {
        slug,
        kind,
        fact_id: body.fact_id,
        preferred_fact_id: body.preferred_fact_id,
        slot,
        stance,
        occurred_at,
        channel,
        summary,
        capture_id: body.capture_id,
        supports,
    })
}

/// Characters a written number may carry between its digit groups:
/// white space, the ASCII separators `. - ( ) [ ] / \ _ | ~`, the
/// prolonged-sound marks that draw as a dash (`ー`, `ｰ`, letters by
/// category), and every non-ASCII character that is neither a letter
/// nor a digit: all Unicode dashes and minus signs, middle dots, box
/// drawing rules, fullwidth and small punctuation, combining marks and
/// symbols (`206·555·0101`, `206／555／0101`, `206─555─0101`). Letters
/// and other ASCII punctuation (`, ; : $ #`) end a number; a comma
/// joins only between two digits ([`strip_separators`]).
fn joins_digits(c: char) -> bool {
    const JOINERS: &[char] = &[
        '.', '-', '(', ')', '[', ']', '/', '\\', '_', '|', '~', '\u{30FC}', '\u{FF70}',
    ];
    c.is_whitespace() || JOINERS.contains(&c) || (!c.is_ascii() && !c.is_alphanumeric())
}

/// `text` with every number separator removed ([`joins_digits`], plus
/// a comma written between two digits), so `(360) 555-0142`,
/// `360.555.0142`, `360/555/0142` and `360,555,0142` all read
/// `3605550142`, while a list (`40220, 40234`) keeps its comma.
fn strip_separators(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let digit = |i: Option<usize>| {
        i.and_then(|i| chars.get(i))
            .is_some_and(char::is_ascii_digit)
    };
    chars
        .iter()
        .enumerate()
        .filter(|&(i, &c)| {
            let comma = c == ',' && digit(i.checked_sub(1)) && digit(Some(i + 1));
            !(joins_digits(c) || comma)
        })
        .map(|(_, &c)| c)
        .collect()
}

/// Runs of 7+ digits in `text` once separators are stripped.
fn long_digit_runs(text: &str) -> Vec<String> {
    strip_separators(text)
        .split(|c: char| !c.is_ascii_digit())
        .filter(|run| run.len() >= MIN_DIGIT_RUN)
        .map(str::to_string)
        .collect()
}

/// `text` with the compatibility forms people paste into an address
/// folded to ASCII: fullwidth ASCII (`ｊａｎｅ＠ｇｍａｉｌ．ｃｏｍ`, U+FF01-FF5E),
/// the small commercial at (`﹫`), and the small, halfwidth and
/// ideographic full stops and the one-dot leader (`。`, `｡`, `﹒`, `․`).
fn fold_compat(text: &str) -> String {
    text.chars()
        .map(|c| match c {
            '\u{FF01}'..='\u{FF5E}' => char::from_u32(c as u32 - 0xFEE0).unwrap_or(c),
            '\u{FE6B}' => '@',
            '\u{3002}' | '\u{FF61}' | '\u{FE52}' | '\u{2024}' => '.',
            c => c,
        })
        .collect()
}

/// Email addresses in `text`, lowercased, after [`fold_compat`]: a
/// local part, `@` (spaces around it allowed: `jane @ example.com`), and
/// a dotted domain whose last label holds a letter (a sentence's closing
/// dot is not part of it). Both parts take letters and digits of any
/// script (`josé@`, `@bücher.de`, a Cyrillic `а` in `gmаil.com`), so a
/// non-ASCII letter next to the `@` is part of the address, never a
/// reason to miss it.
fn emails_in(text: &str) -> Vec<String> {
    let text = fold_compat(text);
    let local = |c: char| c.is_alphanumeric() || matches!(c, '.' | '_' | '%' | '+' | '-');
    let domain = |c: char| c.is_alphanumeric() || matches!(c, '.' | '-');
    text.match_indices('@')
        .filter_map(|(at, _)| {
            let before = text[..at].trim_end();
            let start = before
                .char_indices()
                .rev()
                .find(|(_, c)| !local(*c))
                .map_or(0, |(i, c)| i + c.len_utf8());
            let after = text[at + 1..].trim_start();
            let end = after.find(|c: char| !domain(c)).unwrap_or(after.len());
            let name = &before[start..];
            let host = after[..end].trim_end_matches(['.', '-']);
            let tld = host.rsplit('.').next().unwrap_or("");
            (!name.is_empty()
                && host.contains('.')
                && !host.starts_with('.')
                && tld.chars().any(char::is_alphabetic))
            .then(|| format!("{name}@{host}").to_lowercase())
        })
        .collect()
}

/// What the public read already prints for the office: accepted fact
/// statements and their cited quotes (`phone_script` never reaches it).
struct Published {
    /// Separator-stripped text, one statement or quote per line.
    stripped: String,
    emails: BTreeSet<String>,
}

impl Published {
    fn of(facts: &[FactView]) -> Self {
        let mut published = Published {
            stripped: String::new(),
            emails: BTreeSet::new(),
        };
        let texts = facts.iter().flat_map(|f| {
            std::iter::once(f.statement.as_str())
                .chain(f.citations.iter().map(|c| c.quote.as_str()))
        });
        for text in texts {
            published.stripped.push_str(&strip_separators(text));
            published.stripped.push('\n');
            published.emails.extend(emails_in(text));
        }
        published
    }
}

/// The first contact detail a note would publish that the office does
/// not, as a plain refusal: a 7+ digit number missing from its accepted
/// facts, a channel email it does not publish, or any other email in
/// the summary than the channel's. Runs on every write and again on
/// every public read ([`public_notes`]).
fn private_detail(
    slot: &str,
    channel: &str,
    summary: &str,
    published: &Published,
) -> Option<String> {
    for (field, text) in [("slot", slot), ("channel", channel), ("summary", summary)] {
        if let Some(run) = long_digit_runs(text)
            .into_iter()
            .find(|run| !published.stripped.contains(run.as_str()))
        {
            return Some(format!(
                "{field} has the number {run}, which is not in this office's accepted facts"
            ));
        }
    }
    let channel_emails: BTreeSet<String> = emails_in(channel).into_iter().collect();
    if let Some(email) = channel_emails
        .iter()
        .find(|e| !published.emails.contains(*e))
    {
        return Some(format!(
            "channel has {email}, which is not in this office's accepted facts"
        ));
    }
    emails_in(summary)
        .into_iter()
        .find(|e| !channel_emails.contains(e))
        .map(|email| {
            format!("summary names {email}; a summary may name only the channel's email address")
        })
}

// ----------------------------------------------------------- writes

/// The office-dependent guards and the insert, under SQLite's write
/// lock (IMMEDIATE), so a fact retired or a note withdrawn mid-request
/// cannot slip past the checks. `Ok(Err(reason))` is a refusal.
fn publish_note(
    conn: &mut Connection,
    store_root: &std::path::Path,
    draft: &Draft,
    actor: &str,
    now: i64,
) -> contracts::Result<Result<Note, String>> {
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    // A hidden row (page fragment, internal) is no public office.
    let authority_id: Option<i64> = tx
        .query_row(
            "SELECT id FROM authority WHERE slug = ?1 AND id != 0 AND hidden_at IS NULL",
            [&draft.slug],
            |r| r.get(0),
        )
        .optional()?;
    let Some(authority_id) = authority_id else {
        return Ok(Err("unknown authority".into()));
    };
    // Exactly the facts the public read serves: accepted, no
    // `phone_script`, conflicts counted among accepted siblings.
    let facts =
        crate::approval::accepted_fact_views(&tx, store_root, &draft.slug).unwrap_or_default();
    let public_fact = |id: i64| facts.iter().find(|f| f.id == id);
    if draft.fact_id.is_some_and(|id| public_fact(id).is_none()) {
        return Ok(Err(
            "fact_id must name an accepted fact of this office".into()
        ));
    }
    if let Some(id) = draft.preferred_fact_id {
        match public_fact(id) {
            None => {
                return Ok(Err(
                    "preferred_fact_id must name an accepted fact of this office".into(),
                ));
            }
            Some(fact) if !fact.conflict => {
                return Ok(Err(
                    "preferred_fact_id is not in conflict with another accepted fact".into(),
                ));
            }
            Some(_) => {}
        }
    }
    for &id in &draft.supports {
        let basis: Option<(String, String, String)> = tx
            .query_row(
                "SELECT authority_slug, kind, status FROM evidence_note WHERE id = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        let ok = basis.is_some_and(|(slug, kind, status)| {
            slug == draft.slug && kind != "decision" && status == "published"
        });
        if !ok {
            return Ok(Err(format!(
                "supports: note {id} is not a published call, email, visit or absence \
                 note of this office"
            )));
        }
    }
    if let Some(capture_id) = draft.capture_id {
        let owner: Option<i64> = tx
            .query_row(
                "SELECT authority_id FROM capture WHERE id = ?1",
                [capture_id],
                |r| r.get(0),
            )
            .optional()?;
        if owner != Some(authority_id) {
            return Ok(Err("capture_id must name a capture of this office".into()));
        }
    }
    if let Some(reason) = private_detail(
        &draft.slot,
        &draft.channel,
        &draft.summary,
        &Published::of(&facts),
    ) {
        return Ok(Err(reason));
    }
    let supports = serde_json::to_string(&draft.supports).unwrap_or_else(|_| "[]".into());
    tx.execute(
        "INSERT INTO evidence_note(authority_slug, kind, fact_id, preferred_fact_id, slot,
                                   stance, occurred_at, channel, summary, recorded,
                                   capture_id, supports, status, created_by, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 0, ?10, ?11, 'published', ?12, ?13)",
        rusqlite::params![
            draft.slug,
            draft.kind,
            draft.fact_id,
            draft.preferred_fact_id,
            draft.slot,
            draft.stance,
            draft.occurred_at,
            draft.channel,
            draft.summary,
            draft.capture_id,
            supports,
            actor,
            now,
        ],
    )?;
    let id = tx.last_insert_rowid();
    db::insert_audit(
        &tx,
        now,
        actor,
        "evidence_publish",
        "evidence_note",
        id,
        "",
        "published",
        &format!("{} {}", draft.slug, draft.kind),
    )?;
    tx.commit()?;
    Ok(Ok(Note {
        id,
        slug: draft.slug.clone(),
        kind: draft.kind.clone(),
        fact_id: draft.fact_id,
        preferred_fact_id: draft.preferred_fact_id,
        slot: draft.slot.clone(),
        stance: draft.stance.clone(),
        occurred_at: draft.occurred_at,
        channel: draft.channel.clone(),
        summary: draft.summary.clone(),
        recorded: false,
        capture_id: draft.capture_id,
        supports: draft.supports.clone(),
        status: "published".into(),
        created_by: actor.to_string(),
        created_at: now,
    }))
}

/// Withdraw one published note under the same write lock. A note a
/// published decision rests on stays until that decision is withdrawn,
/// so no public decision loses its basis.
fn withdraw_note(
    conn: &mut Connection,
    id: i64,
    reason: &str,
    actor: &str,
    now: i64,
) -> contracts::Result<Result<Note, (StatusCode, String)>> {
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let note: Option<Note> = tx
        .query_row(
            &format!("SELECT {COLS} FROM evidence_note WHERE id = ?1"),
            [id],
            row_of,
        )
        .optional()?;
    let Some(mut note) = note else {
        return Ok(Err((StatusCode::NOT_FOUND, "note not found".into())));
    };
    if note.status != "published" {
        return Ok(Err((
            StatusCode::CONFLICT,
            "note is already withdrawn".into(),
        )));
    }
    if note.kind != "decision" {
        let mut stmt = tx.prepare(&format!(
            "SELECT {COLS} FROM evidence_note
             WHERE authority_slug = ?1 AND kind = 'decision' AND status = 'published'
             ORDER BY id"
        ))?;
        let decisions = stmt
            .query_map([&note.slug], row_of)?
            .collect::<rusqlite::Result<Vec<Note>>>()?;
        drop(stmt);
        if let Some(decision) = decisions.iter().find(|d| d.supports.contains(&id)) {
            return Ok(Err((
                StatusCode::CONFLICT,
                format!(
                    "decision {} rests on this note; withdraw the decision first",
                    decision.id
                ),
            )));
        }
    }
    tx.execute(
        "UPDATE evidence_note SET status = 'withdrawn' WHERE id = ?1",
        [id],
    )?;
    db::insert_audit(
        &tx,
        now,
        actor,
        "evidence_withdraw",
        "evidence_note",
        id,
        "published",
        "withdrawn",
        reason,
    )?;
    tx.commit()?;
    note.status = "withdrawn".into();
    Ok(Ok(note))
}

// ---------------------------------------------------------- routes

fn refusal(status: StatusCode, reason: &str) -> Response {
    (status, Json(json!({"error": reason}))).into_response()
}

/// Ids only (the page re-reads `/plan`): no note text on the socket.
fn announce(state: &AppState, note: &Note, now: i64) {
    crate::ws_bus::publish_authority_event(
        &state.bus,
        &note.slug,
        "evidence_updated",
        json!({"note_id": note.id}),
        now,
    );
}

/// `POST /apostille/api/v1/admin/evidence`: publish one staff note
/// (201, the staff view). Audited as `evidence_publish`.
async fn create(
    auth: AdminAuth,
    State(state): State<AppState>,
    Json(body): Json<NoteBody>,
) -> Response {
    let now = unix_now();
    let draft = match check_body(body, now) {
        Ok(draft) => draft,
        Err(reason) => return refusal(StatusCode::UNPROCESSABLE_ENTITY, &reason),
    };
    let Some(mut conn) = open(&state) else {
        return store_unavailable();
    };
    match publish_note(
        &mut conn,
        &state.store_root,
        &draft,
        &auth.user.user_id,
        now,
    ) {
        Ok(Ok(note)) => {
            announce(&state, &note, now);
            (StatusCode::CREATED, Json(note.staff_view())).into_response()
        }
        Ok(Err(reason)) => refusal(StatusCode::UNPROCESSABLE_ENTITY, &reason),
        Err(_) => store_unavailable(),
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct WithdrawBody {
    pub reason: Option<String>,
}

/// `POST /apostille/api/v1/admin/evidence/{id}/withdraw`: take one note
/// off the public read; a non-empty reason is required. Audited as
/// `evidence_withdraw`.
async fn withdraw(
    auth: AdminAuth,
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(body): Json<WithdrawBody>,
) -> Response {
    let reason = body.reason.as_deref().unwrap_or("").trim();
    if reason.is_empty() || reason.len() > 1024 {
        return refusal(
            StatusCode::UNPROCESSABLE_ENTITY,
            "reason required (1-1024 chars)",
        );
    }
    let Some(mut conn) = open(&state) else {
        return store_unavailable();
    };
    let now = unix_now();
    match withdraw_note(&mut conn, id, reason, &auth.user.user_id, now) {
        Ok(Ok(note)) => {
            announce(&state, &note, now);
            Json(note.staff_view()).into_response()
        }
        Ok(Err((status, reason))) => refusal(status, &reason),
        Err(_) => store_unavailable(),
    }
}

#[derive(Debug, Deserialize)]
pub struct ListQuery {
    pub slug: Option<String>,
    /// `published` (default), `withdrawn` or `all`.
    pub status: Option<String>,
}

/// `GET /apostille/api/v1/admin/evidence?slug=&status=`: staff list,
/// newest first, so /review can offer supports and withdrawals (the
/// admin host does not serve the public `/plan`).
async fn list(
    _auth: AdminAuth,
    State(state): State<AppState>,
    Query(q): Query<ListQuery>,
) -> Response {
    let status = q.status.as_deref().unwrap_or("published");
    if !["published", "withdrawn", "all"].contains(&status) {
        return refusal(
            StatusCode::UNPROCESSABLE_ENTITY,
            "status must be published, withdrawn or all",
        );
    }
    let Some(conn) = open(&state) else {
        return store_unavailable();
    };
    let slug = q.slug.unwrap_or_default();
    let notes: Vec<Value> = conn
        .prepare(&format!(
            "SELECT {COLS} FROM evidence_note
             WHERE (?1 = 'all' OR status = ?1) AND (?2 = '' OR authority_slug = ?2)
             ORDER BY id DESC LIMIT 200"
        ))
        .and_then(|mut stmt| {
            stmt.query_map(rusqlite::params![status, slug], row_of)?
                .collect::<rusqlite::Result<Vec<_>>>()
        })
        .unwrap_or_default()
        .iter()
        .map(Note::staff_view)
        .collect();
    Json(json!({"count": notes.len(), "notes": notes})).into_response()
}

/// Staff routes merged onto the ADMIN listener only (the builder wraps
/// them in the admin guard stack; handlers take `AdminAuth`).
pub fn admin_router(state: AppState) -> Router {
    Router::<AppState>::new()
        .route("/apostille/api/v1/admin/evidence", get(list).post(create))
        .route(
            "/apostille/api/v1/admin/evidence/{id}/withdraw",
            post(withdraw),
        )
        .with_state(state)
}

// ------------------------------------------------------- public read

/// Published notes of `slug` for the `/plan` read, oldest first, as a
/// JSON array: `[]` for an unknown slug or a store without the table.
/// Re-checked at read time like the plan blocks: a note whose fact left
/// `facts` leaves with it, a decision needs its preferred fact plus at
/// least one visible supporting note, and the contact guard
/// ([`private_detail`]) runs again against `facts`, so a note leaves as
/// soon as no accepted fact prints one of its numbers or emails (the
/// fact that made it public was retired).
pub fn public_notes(conn: &Connection, slug: &str, facts: &[FactView]) -> Value {
    let shown: BTreeSet<i64> = facts.iter().map(|f| f.id).collect();
    let published = Published::of(facts);
    let public = |n: &Note| private_detail(&n.slot, &n.channel, &n.summary, &published).is_none();
    let mut notes: Vec<Note> = conn
        .prepare(&format!(
            "SELECT {COLS} FROM evidence_note
             WHERE authority_slug = ?1 AND status = 'published'
             ORDER BY occurred_at DESC, id DESC LIMIT ?2"
        ))
        .and_then(|mut stmt| {
            stmt.query_map(rusqlite::params![slug, MAX_PUBLIC], row_of)?
                .collect::<rusqlite::Result<Vec<_>>>()
        })
        .unwrap_or_default();
    notes.reverse();
    let basis: BTreeSet<i64> = notes
        .iter()
        .filter(|n| {
            n.kind != "decision" && n.fact_id.is_none_or(|id| shown.contains(&id)) && public(n)
        })
        .map(|n| n.id)
        .collect();
    let items: Vec<Value> = notes
        .iter()
        .filter_map(|n| {
            if n.kind != "decision" {
                return basis.contains(&n.id).then(|| n.public_view(&[]));
            }
            let supports: Vec<i64> = n
                .supports
                .iter()
                .copied()
                .filter(|id| basis.contains(id))
                .collect();
            (n.preferred_fact_id.is_some_and(|id| shown.contains(&id))
                && !supports.is_empty()
                && public(n))
            .then(|| n.public_view(&supports))
        })
        .collect();
    Value::Array(items)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt as _;

    const STAFF: &str = "staff-token-evidence";
    const CUSTOMER: &str = "customer-token-evidence";
    const SLUG: &str = "usa-wa-sos";
    /// Mon 2026-11-02 09:10 PST (fixtures stay in the past).
    const NOV_02_0910_PT: i64 = 1_793_639_400;
    /// One captured page of the office: the canonical PO-box pair, a
    /// phone and an email (555-01xx and `.example` are fictional).
    const SIDECAR: &str = "Mail requests to PO Box 40220, Olympia, WA 98504-0220.\n\
                           Apostille requests go to PO Box 40234, Olympia, WA 98504-0234.\n\
                           Call (360) 555-0142 or write to apostille@sos.example.\n\
                           The fee is $15 per document.\n";

    struct Fixture {
        _dir: tempfile::TempDir,
        state: AppState,
        /// Accepted `mailing` facts that disagree (40220 vs 40234).
        box_a: i64,
        box_b: i64,
        /// Accepted `contacts` fact: the published phone and email.
        contacts: i64,
        /// Accepted `fees` fact with no sibling: never in conflict.
        fee: i64,
        /// Accepted `phone_script` fact: hidden from the public read.
        script: i64,
        /// Proposed `hours` fact.
        proposed: i64,
        /// Accepted fact and capture of another office.
        foreign: i64,
        foreign_capture: i64,
    }

    fn state_for(root: &std::path::Path) -> AppState {
        AppState {
            spa_index: root.join("index.html"),
            static_dir: root.to_path_buf(),
            store_root: root.to_path_buf(),
            bus: crate::ws_bus::Bus::new(),
            kek: None,
            admin_mtls_off: true,
            admin_allow_ips: config::loopback_allow_ips(),
            admin_spa_dir: root.to_path_buf(),
            media_terms_read: false,
            label_cfg: crate::p2_labels::LabelCfg::default(),
            voice_cfg: voice::VoiceConfig::default(),
            voice_api_key: None,
            webauthn: std::sync::Arc::new(crate::passkey::test_webauthn()),
            auth_limits: std::sync::Arc::new(crate::passkey::RateBuckets::default()),
            demo_limits: std::sync::Arc::new(crate::passkey::RateBuckets::default()),
            draft_runs: std::sync::Arc::new(crate::draft::DraftRuns::default()),
            config_cache: std::sync::Arc::new(config::CachedReader::default()),
            google_cfg: None,
            apple_cfg: None,
            meta_cfg: None,
            mailer: None,
            magic_link_base: "https://example.test/apostille/login".to_string(),
            notify_poke: std::sync::Arc::new(tokio::sync::Notify::new()),
            notify_enabled: false,
            notify_daily_cap: 3,
            notify_postal: String::new(),
            notify_limits: std::sync::Arc::new(crate::passkey::RateBuckets::default()),
            notify_base: "https://example.test/apostille".to_string(),
            meta_delete_status_base: "https://example.test/auth/meta/delete-status".to_string(),
            fedex_live: crate::fedex_live::FedexLiveState::default(),
        }
    }

    fn add_office(conn: &Connection, slug: &str) -> i64 {
        db::add_authority(
            conn,
            &db::NewAuthority {
                kind: "state",
                country: "USA",
                region: "Washington",
                name: "Washington Secretary of State",
                slug,
                actor: "test",
                at: 1,
            },
        )
        .unwrap()
    }

    fn add_capture(conn: &Connection, root: &std::path::Path, authority: i64, text: &str) -> i64 {
        conn.execute(
            "INSERT INTO capture(authority_id, captured_at, kind, path,
                                  bytes_raw, bytes_stored, sha256, trust)
             VALUES (?1, 1, 'file', 'p', 0, 0, 's', 'staff_verified')",
            [authority],
        )
        .unwrap();
        let id = conn.last_insert_rowid();
        std::fs::create_dir_all(root.join("text")).unwrap();
        std::fs::write(root.join("text").join(format!("{id}.txt")), text).unwrap();
        id
    }

    /// One fact citing `quote` in `SIDECAR` (no quote: uncited).
    fn add_fact(
        conn: &Connection,
        (authority, capture): (i64, i64),
        topic: &str,
        statement: &str,
        status: &str,
        quote: Option<&str>,
    ) -> i64 {
        let (start, end) = quote.map_or((-1, -1), |q| {
            let start = SIDECAR.find(q).expect("quote in sidecar");
            (start as i64, (start + q.len()) as i64)
        });
        conn.execute(
            "INSERT INTO fact(authority_id, capture_id, topic, statement, status,
                              created_by, created_at, citation_start, citation_end)
             VALUES (?1, ?2, ?3, ?4, ?5, 't', 1, ?6, ?7)",
            rusqlite::params![authority, capture, topic, statement, status, start, end],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    impl Fixture {
        fn new() -> Self {
            let dir = tempfile::TempDir::new().unwrap();
            let root = dir.path();
            let store = Store::new(root);
            store.init().unwrap();
            let conn = db::open(&store).unwrap();
            db::migrate(&conn).unwrap();
            auth::migrate_auth_tables(&conn).unwrap();
            config::migrate_config_tables(&conn).unwrap();
            // Boot DDL exactly as `main` runs it (this lane included).
            crate::migrate_mission_tables(&conn).unwrap();
            let office = add_office(&conn, SLUG);
            let other = add_office(&conn, "other-sos");
            conn.execute(
                "INSERT INTO source(authority_id, kind, uri)
                 VALUES (?1, 'website_official', 'https://sos.example/apostille')",
                [office],
            )
            .unwrap();
            let page = (office, add_capture(&conn, root, office, SIDECAR));
            let foreign_capture = add_capture(&conn, root, other, "The fee is $20 per document.");
            let fixture = Fixture {
                box_a: add_fact(
                    &conn,
                    page,
                    "mailing",
                    "mailing: PO Box 40220",
                    "accepted",
                    Some("Mail requests to PO Box 40220, Olympia, WA 98504-0220."),
                ),
                box_b: add_fact(
                    &conn,
                    page,
                    "mailing",
                    "mailing: PO Box 40234, Olympia, WA 98504-0234",
                    "accepted",
                    Some("Apostille requests go to PO Box 40234, Olympia, WA 98504-0234."),
                ),
                contacts: add_fact(
                    &conn,
                    page,
                    "contacts",
                    "contacts: (360) 555-0142, apostille@sos.example",
                    "accepted",
                    Some("Call (360) 555-0142 or write to apostille@sos.example."),
                ),
                fee: add_fact(
                    &conn,
                    page,
                    "fees",
                    "fees: $15 per document",
                    "accepted",
                    Some("The fee is $15 per document."),
                ),
                script: add_fact(
                    &conn,
                    page,
                    "phone_script",
                    "phone_script: Ask for the desk on 360-555-0199.",
                    "accepted",
                    None,
                ),
                proposed: add_fact(
                    &conn,
                    page,
                    "hours",
                    "hours: Monday to Friday",
                    "proposed",
                    None,
                ),
                foreign: add_fact(
                    &conn,
                    (other, foreign_capture),
                    "fees",
                    "fees: $20 per document",
                    "accepted",
                    None,
                ),
                foreign_capture,
                state: state_for(root),
                _dir: dir,
            };
            for (token, role) in [(STAFF, auth::Role::Staff), (CUSTOMER, auth::Role::Customer)] {
                auth::create_session(
                    &conn,
                    token,
                    &auth::NewSession {
                        user_id: if role == auth::Role::Staff {
                            "staff-1"
                        } else {
                            "cust-1"
                        },
                        role,
                        customer_ref: None,
                        now: unix_now(),
                        ip: "127.0.0.1",
                        user_agent: "test",
                    },
                )
                .unwrap();
            }
            fixture
        }

        fn conn(&self) -> Connection {
            db::open(&Store::new(&self.state.store_root)).unwrap()
        }

        /// The production admin stack: host guard, IP allowlist, CSRF.
        fn admin(&self) -> Router {
            crate::mission_admin_routers(&self.state)
        }

        fn public(&self) -> Router {
            crate::public_router(self.state.clone(), "static", "test-spa", "test-acme")
        }

        /// A valid call note on the 40234 box; tests override fields.
        fn call_note(&self) -> Value {
            json!({
                "authority_slug": SLUG,
                "kind": "call",
                "fact_id": self.box_b,
                "slot": "mailing.po_box",
                "stance": "confirms",
                "occurred_at": NOV_02_0910_PT,
                "channel": "Apostille desk, (360) 555-0142 option 3",
                "summary": "The desk said PO Box 40234, Olympia, WA 98504-0234 takes apostille mail.",
            })
        }

        async fn post(&self, uri: &str, body: &Value) -> (StatusCode, Value) {
            send(
                self.admin(),
                admin_request("POST", uri, Some(STAFF), true, Some(body)),
            )
            .await
        }

        async fn create(&self, body: &Value) -> (StatusCode, Value) {
            self.post("/apostille/api/v1/admin/evidence", body).await
        }

        async fn withdraw(&self, id: i64, reason: &str) -> (StatusCode, Value) {
            self.post(
                &format!("/apostille/api/v1/admin/evidence/{id}/withdraw"),
                &json!({"reason": reason}),
            )
            .await
        }

        async fn public_get(&self, uri: &str) -> (StatusCode, Value) {
            // A TEST-NET peer: the `/plan` rate bucket is per IP and
            // process-wide, so these reads never drain another suite's.
            let req = Request::builder()
                .uri(uri)
                .extension(axum::extract::ConnectInfo(
                    "192.0.2.71:443".parse::<std::net::SocketAddr>().unwrap(),
                ))
                .body(Body::empty())
                .unwrap();
            send(self.public(), req).await
        }

        async fn evidence(&self) -> Vec<Value> {
            let (status, plan) = self
                .public_get(&format!("/apostille/api/v1/authorities/{SLUG}/plan"))
                .await;
            assert_eq!(status, StatusCode::OK, "{plan}");
            plan["evidence"].as_array().expect("evidence array").clone()
        }

        fn notes(&self) -> i64 {
            self.conn()
                .query_row("SELECT COUNT(*) FROM evidence_note", [], |r| r.get(0))
                .unwrap()
        }
    }

    /// Admin request: loopback peer, `localhost` host (the dev seam),
    /// a staff/customer session cookie, and the CSRF pair when `csrf`.
    fn admin_request(
        method: &str,
        uri: &str,
        session: Option<&str>,
        csrf: bool,
        body: Option<&Value>,
    ) -> Request<Body> {
        let mut cookies = Vec::new();
        let mut req = Request::builder()
            .method(method)
            .uri(uri)
            .header("host", "localhost");
        if let Some(token) = session {
            cookies.push(format!("{}={token}", auth::ADM_COOKIE));
        }
        if csrf {
            cookies.push(format!("{}=t", auth::CSRF_COOKIE));
            req = req
                .header("origin", "http://localhost:3001")
                .header(auth::CSRF_HEADER, "t");
        }
        if !cookies.is_empty() {
            req = req.header("cookie", cookies.join("; "));
        }
        if body.is_some() {
            req = req.header("content-type", "application/json");
        }
        req.extension(axum::extract::ConnectInfo(
            "127.0.0.1:443".parse::<std::net::SocketAddr>().unwrap(),
        ))
        .body(body.map_or_else(Body::empty, |b| Body::from(b.to_string())))
        .unwrap()
    }

    async fn send(app: Router, req: Request<Body>) -> (StatusCode, Value) {
        let res = app.oneshot(req).await.unwrap();
        let status = res.status();
        let bytes = axum::body::to_bytes(res.into_body(), 1 << 20)
            .await
            .unwrap();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    /// `base` with `changes` merged over it (`null` removes a field).
    fn with(base: &Value, changes: Value) -> Value {
        let mut out = base.clone();
        for (k, v) in changes.as_object().unwrap() {
            if v.is_null() {
                out.as_object_mut().unwrap().remove(k);
            } else {
                out[k] = v.clone();
            }
        }
        out
    }

    fn keys(v: &Value) -> BTreeSet<String> {
        v.as_object().unwrap().keys().cloned().collect()
    }

    #[tokio::test]
    async fn call_note_publishes_on_plan_and_withdraws() {
        let f = Fixture::new();
        let facts_uri = format!("/apostille/api/v1/authorities/{SLUG}/facts");
        let (_, facts_before) = f.public_get(&facts_uri).await;
        let mut rx = f.state.bus.subscribe(&format!("authority:{SLUG}"));

        let (status, note) = f.create(&f.call_note()).await;
        assert_eq!(status, StatusCode::CREATED, "{note}");
        let id = note["id"].as_i64().unwrap();
        assert_eq!(note["status"], "published");
        assert_eq!(note["created_by"], "staff-1", "staff view names the author");
        assert_eq!(note["date"], "2026-11-02");
        assert_eq!(note["recorded"], false);

        // Ids only on the anonymous socket: no note text.
        let event: Value = serde_json::from_str(&rx.try_recv().unwrap()).unwrap();
        assert_eq!(event["type"], "evidence_updated");
        assert_eq!(event["topic"], format!("authority:{SLUG}"));
        assert_eq!(event["note_id"], id);
        assert!(event.get("summary").is_none());

        let audit: (String, String, String, String) = f
            .conn()
            .query_row(
                "SELECT action, actor, after_ref, reason FROM audit
                 WHERE entity = 'evidence_note' AND entity_id = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .unwrap();
        assert_eq!(
            audit,
            (
                "evidence_publish".into(),
                "staff-1".into(),
                "published".into(),
                format!("{SLUG} call")
            )
        );

        // Public shape: a role byline, never the author or the capture.
        let evidence = f.evidence().await;
        assert_eq!(evidence.len(), 1);
        let item = &evidence[0];
        assert_eq!(
            keys(item),
            BTreeSet::from(
                [
                    "by",
                    "channel",
                    "date",
                    "fact_id",
                    "id",
                    "kind",
                    "preferred_fact_id",
                    "recorded",
                    "slot",
                    "stance",
                    "summary",
                    "supports"
                ]
                .map(String::from)
            )
        );
        assert_eq!(item["id"], id);
        assert_eq!(item["by"], PUBLIC_BY);
        assert_eq!(item["kind"], "call");
        assert_eq!(item["stance"], "confirms");
        assert_eq!(item["fact_id"], f.box_b);
        assert_eq!(item["preferred_fact_id"], Value::Null);
        assert_eq!(item["slot"], "mailing.po_box");
        assert_eq!(item["date"], "2026-11-02");
        assert_eq!(item["recorded"], false);
        assert_eq!(item["supports"], json!([]));

        // Evidence never enters a fact's citations or moves its status.
        let (_, facts_after) = f.public_get(&facts_uri).await;
        assert_eq!(facts_before, facts_after);
        let status: String = f
            .conn()
            .query_row("SELECT status FROM fact WHERE id = ?1", [f.box_b], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(status, "accepted");

        // Staff list (the admin host has no `/plan`).
        let (status, listed) = send(
            f.admin(),
            admin_request(
                "GET",
                &format!("/apostille/api/v1/admin/evidence?slug={SLUG}"),
                Some(STAFF),
                false,
                None,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{listed}");
        assert_eq!(listed["count"], 1);
        assert_eq!(listed["notes"][0]["created_by"], "staff-1");

        // Withdraw: a reason is required, then the note leaves `/plan`.
        let (status, _) = f.withdraw(id, "  ").await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        let (status, withdrawn) = f.withdraw(id, "the desk called back").await;
        assert_eq!(status, StatusCode::OK, "{withdrawn}");
        assert_eq!(withdrawn["status"], "withdrawn");
        let event: Value = serde_json::from_str(&rx.try_recv().unwrap()).unwrap();
        assert_eq!(
            (event["type"].as_str(), event["note_id"].as_i64()),
            (Some("evidence_updated"), Some(id))
        );
        let reason: String = f
            .conn()
            .query_row(
                "SELECT reason FROM audit WHERE action = 'evidence_withdraw' AND entity_id = ?1",
                [id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(reason, "the desk called back");
        assert!(f.evidence().await.is_empty(), "withdrawn notes disappear");
        let (status, again) = f.withdraw(id, "twice").await;
        assert_eq!(status, StatusCode::CONFLICT, "{again}");
        let (status, _) = f.withdraw(id + 100, "no such note").await;
        assert_eq!(status, StatusCode::NOT_FOUND);

        // The staff list keeps the history.
        for (query, count) in [("", 0), ("&status=withdrawn", 1), ("&status=all", 1)] {
            let (_, listed) = send(
                f.admin(),
                admin_request(
                    "GET",
                    &format!("/apostille/api/v1/admin/evidence?slug={SLUG}{query}"),
                    Some(STAFF),
                    false,
                    None,
                ),
            )
            .await;
            assert_eq!(listed["count"], count, "{query}");
        }
    }

    #[tokio::test]
    async fn decision_settles_a_conflict_on_published_notes() {
        let f = Fixture::new();
        let (status, call) = f
            .create(&with(
                &f.call_note(),
                json!({
                    "fact_id": f.box_a,
                    "stance": "contradicts",
                    "summary": "The desk said PO Box 40220 no longer takes apostille mail.",
                }),
            ))
            .await;
        assert_eq!(status, StatusCode::CREATED, "{call}");
        let call_id = call["id"].as_i64().unwrap();
        let decision = json!({
            "authority_slug": SLUG,
            "kind": "decision",
            "preferred_fact_id": f.box_b,
            "slot": "mailing.po_box",
            "stance": "confirms",
            "occurred_at": NOV_02_0910_PT + 600,
            "channel": "",
            "summary": "Use PO Box 40234 until the office says otherwise.",
            "supports": [call_id],
        });
        for (changes, reason) in [
            (json!({"supports": []}), "at least one supporting note"),
            (json!({"supports": null}), "at least one supporting note"),
            (json!({"supports": [call_id, call_id]}), "distinct note ids"),
            (
                json!({"supports": [call_id + 50]}),
                "is not a published call",
            ),
            (
                json!({"preferred_fact_id": null}),
                "needs a preferred_fact_id",
            ),
            (json!({"preferred_fact_id": f.fee}), "not in conflict"),
            (json!({"preferred_fact_id": f.proposed}), "accepted fact"),
            (json!({"preferred_fact_id": f.foreign}), "accepted fact"),
            (json!({"fact_id": f.box_b}), "anchors to a slot"),
            (json!({"slot": null}), "needs a slot"),
        ] {
            let (status, v) = f.create(&with(&decision, changes.clone())).await;
            assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{changes}: {v}");
            assert!(
                v["error"].as_str().unwrap().contains(reason),
                "{changes}: {v}"
            );
        }
        let (status, settled) = f.create(&decision).await;
        assert_eq!(status, StatusCode::CREATED, "{settled}");
        let decision_id = settled["id"].as_i64().unwrap();
        // A decision rests on evidence, never on another decision.
        let (status, v) = f
            .create(&with(&decision, json!({"supports": [decision_id]})))
            .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{v}");

        let evidence = f.evidence().await;
        assert_eq!(evidence.len(), 2);
        assert_eq!(evidence[1]["id"], decision_id);
        assert_eq!(evidence[1]["kind"], "decision");
        assert_eq!(evidence[1]["preferred_fact_id"], f.box_b);
        assert_eq!(evidence[1]["fact_id"], Value::Null);
        assert_eq!(evidence[1]["supports"], json!([call_id]));

        // The supporting call stays while the decision rests on it.
        let (status, v) = f.withdraw(call_id, "wrong box").await;
        assert_eq!(status, StatusCode::CONFLICT, "{v}");
        assert!(
            v["error"]
                .as_str()
                .unwrap()
                .contains(&format!("decision {decision_id}"))
        );
        assert_eq!(
            f.withdraw(decision_id, "office to confirm").await.0,
            StatusCode::OK
        );
        assert_eq!(f.withdraw(call_id, "wrong box").await.0, StatusCode::OK);
        assert!(f.evidence().await.is_empty());

        // Supports must be published notes of this office: the withdrawn
        // call no longer counts, and neither does another office's note.
        let not_published = "is not a published call, email, visit or absence note of this office";
        let (status, v) = f.create(&decision).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{v}");
        assert!(v["error"].as_str().unwrap().contains(not_published), "{v}");
        let (status, elsewhere) = f
            .create(&json!({
                "authority_slug": "other-sos",
                "kind": "absence",
                "slot": "fees.same_day",
                "stance": "context",
                "occurred_at": NOV_02_0910_PT,
                "channel": "",
                "summary": "Neither page names a same-day fee.",
            }))
            .await;
        assert_eq!(status, StatusCode::CREATED, "{elsewhere}");
        let (status, v) = f
            .create(&with(&decision, json!({"supports": [elsewhere["id"]]})))
            .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{v}");
        assert!(v["error"].as_str().unwrap().contains(not_published), "{v}");
        // A published absence of this office is a basis again.
        let (status, absence) = f
            .create(&json!({
                "authority_slug": SLUG,
                "kind": "absence",
                "slot": "mailing.po_box",
                "stance": "context",
                "occurred_at": NOV_02_0910_PT,
                "channel": "",
                "summary": "Neither mailing page says which box is current.",
            }))
            .await;
        assert_eq!(status, StatusCode::CREATED, "{absence}");
        let (status, v) = f
            .create(&with(&decision, json!({"supports": [absence["id"]]})))
            .await;
        assert_eq!(status, StatusCode::CREATED, "{v}");
    }

    #[tokio::test]
    async fn anchor_and_shape_guards_refuse_with_plain_reasons() {
        let f = Fixture::new();
        let base = f.call_note();
        let absence = json!({
            "authority_slug": SLUG,
            "kind": "absence",
            "slot": "fees.same_day",
            "stance": "context",
            "occurred_at": NOV_02_0910_PT,
            "channel": "FAQ and fee pages",
            "summary": "Neither page names a same-day fee.",
        });
        let now = unix_now();
        for (body, reason) in [
            (with(&base, json!({"kind": "fax"})), "kind must be"),
            (with(&base, json!({"kind": null})), "kind must be"),
            (with(&base, json!({"stance": "maybe"})), "stance must be"),
            (
                with(&base, json!({"authority_slug": " "})),
                "authority_slug",
            ),
            (
                with(&base, json!({"authority_slug": "no-such-office"})),
                "unknown authority",
            ),
            (
                with(&base, json!({"fact_id": null})),
                "anchors to an accepted fact",
            ),
            (
                with(&base, json!({"fact_id": f.proposed})),
                "accepted fact of this office",
            ),
            (
                with(&base, json!({"fact_id": f.foreign})),
                "accepted fact of this office",
            ),
            (
                with(&base, json!({"fact_id": f.script})),
                "accepted fact of this office",
            ),
            (
                with(&base, json!({"fact_id": 9_999})),
                "accepted fact of this office",
            ),
            (
                with(&base, json!({"preferred_fact_id": f.box_a})),
                "only a decision",
            ),
            (
                with(&base, json!({"supports": [1]})),
                "only a decision lists",
            ),
            (with(&base, json!({"slot": "Fees.Same"})), "slot must be"),
            (with(&base, json!({"slot": "fees..same"})), "slot must be"),
            (with(&base, json!({"occurred_at": 0})), "occurred_at"),
            (with(&base, json!({"occurred_at": null})), "occurred_at"),
            (
                with(&base, json!({"occurred_at": now + 2 * 86_400})),
                "occurred_at",
            ),
            (
                with(&base, json!({"occurred_at": now * 1_000})),
                "occurred_at",
            ),
            (with(&base, json!({"channel": ""})), "names its channel"),
            (
                with(&base, json!({"channel": "x".repeat(161)})),
                "channel must be",
            ),
            (
                with(&base, json!({"capture_id": f.foreign_capture})),
                "capture_id",
            ),
            (with(&base, json!({"capture_id": 0})), "capture_id"),
            (
                with(&absence, json!({"fact_id": f.fee})),
                "anchors to a slot",
            ),
            (with(&absence, json!({"slot": ""})), "needs a slot"),
        ] {
            let (status, v) = f.create(&body).await;
            assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}: {v}");
            assert!(v["error"].as_str().unwrap().contains(reason), "{body}: {v}");
        }
        // No call id and no author from the body: unknown fields refuse.
        for field in ["call_id", "created_by", "status"] {
            let (status, _) = f.create(&with(&base, json!({field: "x"}))).await;
            assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{field}");
        }
        assert_eq!(f.notes(), 0, "no refusal stored a row");
        let (status, v) = f.create(&absence).await;
        assert_eq!(status, StatusCode::CREATED, "{v}");
        assert_eq!(v["fact_id"], Value::Null);
    }

    #[tokio::test]
    async fn summary_and_contact_guards_keep_private_details_out() {
        let f = Fixture::new();
        let base = f.call_note();
        let email = with(
            &base,
            json!({
                "kind": "email",
                "fact_id": f.contacts,
                "channel": "apostille@sos.example",
                "summary": "Reply from apostille@sos.example confirms the 40234 box.",
            }),
        );
        let summary = |text: &str| with(&base, json!({"summary": text}));
        for (body, reason) in [
            (summary("Too few"), "8-280 characters"),
            (summary(&"x".repeat(281)), "8-280 characters"),
            (summary("   \n  "), "8-280 characters"),
            (summary("Desk said \u{7} the box."), "plain text"),
            (
                summary("Call back on 206-555-0101 for updates."),
                "number 2065550101",
            ),
            (
                summary("Asked at 360-555-0199 for the desk."),
                "number 3605550199",
            ),
            (summary("Office ref 2026-11-04 noted."), "number 20261104"),
            // A private number split by an invisible character, or
            // written in look-alike digits, still shows whole in the
            // browser: refused before the digit-run check.
            (
                summary("Call back on 206\u{200B}555\u{200B}0101 for updates."),
                "plain digits (0-9)",
            ),
            (
                summary("Call back on 206\u{2060}555\u{2060}0101 for updates."),
                "plain digits (0-9)",
            ),
            (
                summary("Call back on 206\u{AD}555\u{AD}0101 for updates."),
                "plain digits (0-9)",
            ),
            (
                summary("Call back on 206555\u{FE0F}0101 for updates."),
                "plain digits (0-9)",
            ),
            (
                summary("Desk said \u{202E}xob eht\u{202C} is open."),
                "plain digits (0-9)",
            ),
            (
                summary("Call back on \u{FF12}\u{FF10}\u{FF16}-555-0101 for updates."),
                "plain digits (0-9)",
            ),
            (
                summary("Call back on \u{1D7D0}\u{1D7CE}\u{1D7D4} 555 0101 for updates."),
                "plain digits (0-9)",
            ),
            (
                summary("Call back on \u{0662}\u{0660}\u{0666} 555 0101 for updates."),
                "plain digits (0-9)",
            ),
            (
                summary("Call back on \u{00B2}06 555 0101 for updates."),
                "plain digits (0-9)",
            ),
            (
                with(&base, json!({"channel": "Desk line\u{200B} option 3"})),
                "channel must write numbers in plain digits",
            ),
            // Middle dots and fullwidth or small dashes join the groups.
            (
                summary("Call back on 206\u{B7}555\u{B7}0101 for updates."),
                "number 2065550101",
            ),
            (
                summary("Call back on 206\u{FF0D}555\u{FE63}0101 for updates."),
                "number 2065550101",
            ),
            // Slashes (spaced or not), commas between digits, underscores
            // and non-ASCII symbols join the groups too.
            (
                summary("Called back 206/555/0101 for updates."),
                "number 2065550101",
            ),
            (
                summary("Called back 206 / 555 / 0101 for updates."),
                "number 2065550101",
            ),
            (
                summary("Called back 206,555,0101 for updates."),
                "number 2065550101",
            ),
            (
                summary("Called back 206_555_0101 for updates."),
                "number 2065550101",
            ),
            (
                summary("Called back 206\u{2043}555\u{2043}0101 for updates."),
                "number 2065550101",
            ),
            (
                summary("Called back 206\u{30FC}555\u{30FC}0101 for updates."),
                "number 2065550101",
            ),
            (
                summary("Called back 206\u{2500}555\u{2500}0101 for updates."),
                "number 2065550101",
            ),
            (
                summary("Office line 0221/12 34 56 per the clerk."),
                "number 0221123456",
            ),
            // The Braille blank draws as a space: refused outright.
            (
                summary("Called back 206\u{2800}555\u{2800}0101 for updates."),
                "plain digits (0-9)",
            ),
            (
                summary("They said to write to apostille@sos.example instead."),
                "only the channel's email",
            ),
            (
                with(&base, json!({"channel": "Direct line 206-555-0101"})),
                "channel has the number",
            ),
            (
                with(&email, json!({"channel": "jane.doe@sos.example"})),
                "channel has jane.doe@sos.example",
            ),
            (
                with(
                    &email,
                    json!({"summary": "Reply copied Jane.Doe@sos.example on it."}),
                ),
                "summary names jane.doe@sos.example",
            ),
            // Look-alike at signs and stops fold to ASCII; letters of any
            // script and a spaced `@` are still an address.
            (
                with(
                    &email,
                    json!({"summary": "Reply copied jane.doe\u{FF20}sos.example on it."}),
                ),
                "summary names jane.doe@sos.example",
            ),
            (
                with(
                    &email,
                    json!({"summary": "Reply copied jane.doe\u{FE6B}sos.example on it."}),
                ),
                "summary names jane.doe@sos.example",
            ),
            (
                with(
                    &email,
                    json!({"summary": "Reply copied jane.doe@sos\u{FF0E}example on it."}),
                ),
                "summary names jane.doe@sos.example",
            ),
            (
                with(
                    &email,
                    json!({"summary": "Reply copied jos\u{E9}@sos.example on it."}),
                ),
                "summary names jos\u{E9}@sos.example",
            ),
            (
                with(
                    &email,
                    json!({"summary": "Reply copied jane@s\u{043E}s.example on it."}),
                ),
                "summary names jane@s\u{043E}s.example",
            ),
            (
                with(
                    &email,
                    json!({"summary": "Reply copied jane.doe @ sos.example on it."}),
                ),
                "summary names jane.doe@sos.example",
            ),
            (
                with(&email, json!({"channel": "jane.doe\u{FF20}sos.example"})),
                "channel has jane.doe@sos.example",
            ),
            (with(&base, json!({"recorded": true})), "consent lane"),
            (with(&base, json!({"recorded": 1})), "consent lane"),
            (with(&base, json!({"recorded": "no"})), "consent lane"),
        ] {
            let (status, v) = f.create(&body).await;
            assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}: {v}");
            assert!(v["error"].as_str().unwrap().contains(reason), "{body}: {v}");
        }
        assert_eq!(f.notes(), 0);
        for (body, stored) in [
            (summary("Eight ch"), "Eight ch"),
            (summary(&"y".repeat(280)), &*"y".repeat(280)),
            (
                summary("Desk\n  confirmed\tthe box."),
                "Desk confirmed the box.",
            ),
            // The published phone in any spelling, and the ZIP+4 that
            // only the 40220 fact's quote prints.
            (
                summary("Rang 360.555.0142, then (360) 555 0142 again."),
                "Rang 360.555.0142, then (360) 555 0142 again.",
            ),
            (
                summary("Old address PO Box 40220, Olympia, WA 98504-0220 still listed."),
                "Old address PO Box 40220, Olympia, WA 98504-0220 still listed.",
            ),
            (
                email.clone(),
                "Reply from apostille@sos.example confirms the 40234 box.",
            ),
            // The channel's own address in a fullwidth spelling, and a
            // list of short numbers.
            (
                with(
                    &email,
                    json!({"summary": "Reply from apostille\u{FF20}sos.example confirms it."}),
                ),
                "Reply from apostille\u{FF20}sos.example confirms it.",
            ),
            (
                summary("The pages list boxes 40220, 40234 for mail."),
                "The pages list boxes 40220, 40234 for mail.",
            ),
            (
                with(&base, json!({"recorded": false})),
                base["summary"].as_str().unwrap(),
            ),
            (
                with(&base, json!({"recorded": 0})),
                base["summary"].as_str().unwrap(),
            ),
        ] {
            let (status, v) = f.create(&body).await;
            assert_eq!(status, StatusCode::CREATED, "{body}: {v}");
            assert_eq!(v["summary"], stored);
        }
    }

    #[tokio::test]
    async fn routes_need_a_staff_session_and_the_csrf_pair() {
        let f = Fixture::new();
        let body = f.call_note();
        let uri = "/apostille/api/v1/admin/evidence";
        let (status, v) = send(
            f.admin(),
            admin_request("POST", uri, Some(STAFF), false, Some(&body)),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "no CSRF pair: {v}");
        assert_eq!(v["code"], crate::admin_ip::CODE_CSRF_FAILED);
        let (status, _) = send(
            f.admin(),
            admin_request("POST", uri, None, true, Some(&body)),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "no session");
        let (status, v) = send(
            f.admin(),
            admin_request("POST", uri, Some(CUSTOMER), true, Some(&body)),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "customer session: {v}");
        assert_eq!(v["code"], crate::admin_ip::CODE_STAFF_ROLE_REQUIRED);
        let (status, _) = send(f.admin(), admin_request("GET", uri, None, false, None)).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        let mut wrong_host = admin_request("POST", uri, Some(STAFF), true, Some(&body));
        wrong_host
            .headers_mut()
            .insert("host", "evil.test".parse().unwrap());
        assert!(send(f.admin(), wrong_host).await.0.is_client_error());
        assert_eq!(f.notes(), 0);

        let (status, note) = f.create(&body).await;
        assert_eq!(status, StatusCode::CREATED);
        let withdraw = format!("{uri}/{}/withdraw", note["id"]);
        let reason = json!({"reason": "test"});
        let (status, _) = send(
            f.admin(),
            admin_request("POST", &withdraw, Some(STAFF), false, Some(&reason)),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "withdraw needs CSRF too");
        let (status, _) = send(
            f.admin(),
            admin_request("POST", &withdraw, Some(CUSTOMER), true, Some(&reason)),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(
            f.evidence().await.len(),
            1,
            "refused withdrawals change nothing"
        );

        // The public host never serves the staff routes.
        let (status, _) = f.public_get(uri).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn plan_read_rechecks_anchors_and_is_empty_for_unknown_slugs() {
        let f = Fixture::new();
        let (status, plan) = f
            .public_get("/apostille/api/v1/authorities/no-such-office/plan")
            .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(plan["evidence"], json!([]));
        assert!(f.evidence().await.is_empty());
        // A bare store (no boot DDL) still reads as no evidence.
        let bare = Connection::open_in_memory().unwrap();
        assert_eq!(public_notes(&bare, SLUG, &[]), json!([]));

        let (status, call) = f
            .create(&with(
                &f.call_note(),
                json!({
                    "fact_id": f.box_a,
                    "stance": "contradicts",
                    "summary": "The desk said the 40220 box no longer takes apostille mail.",
                }),
            ))
            .await;
        assert_eq!(status, StatusCode::CREATED, "{call}");
        let (status, absence) = f
            .create(&json!({
                "authority_slug": SLUG,
                "kind": "absence",
                "slot": "fees.same_day",
                "stance": "context",
                "occurred_at": NOV_02_0910_PT - 86_400,
                "channel": "FAQ and fee pages",
                "summary": "Neither page names a same-day fee.",
            }))
            .await;
        assert_eq!(status, StatusCode::CREATED, "{absence}");
        let (status, decision) = f
            .create(&json!({
                "authority_slug": SLUG,
                "kind": "decision",
                "preferred_fact_id": f.box_b,
                "slot": "mailing.po_box",
                "stance": "confirms",
                "occurred_at": NOV_02_0910_PT + 60,
                "channel": "",
                "summary": "Use PO Box 40234 until the office says otherwise.",
                "supports": [call["id"]],
            }))
            .await;
        assert_eq!(status, StatusCode::CREATED, "{decision}");
        let ids =
            |evidence: Vec<Value>| -> Value { evidence.iter().map(|n| n["id"].clone()).collect() };
        // Oldest first.
        assert_eq!(
            ids(f.evidence().await),
            json!([absence["id"], call["id"], decision["id"]])
        );
        // The preferred fact leaves the public read: so does the decision.
        let retire = |id: i64| {
            f.conn()
                .execute("UPDATE fact SET status = 'superseded' WHERE id = ?1", [id])
                .unwrap()
        };
        retire(f.box_b);
        assert_eq!(ids(f.evidence().await), json!([absence["id"], call["id"]]));
        // The anchored fact leaves: so does the call; the absence stays.
        retire(f.box_a);
        assert_eq!(ids(f.evidence().await), json!([absence["id"]]));
    }

    #[tokio::test]
    async fn plan_read_drops_a_note_once_its_contact_details_are_no_longer_public() {
        let f = Fixture::new();
        let retire = |id: i64| {
            f.conn()
                .execute("UPDATE fact SET status = 'superseded' WHERE id = ?1", [id])
                .unwrap()
        };
        let ids =
            |evidence: Vec<Value>| -> Value { evidence.iter().map(|n| n["id"].clone()).collect() };
        // Notes on the fee fact whose channel and summary carry the phone
        // and email only the contacts fact publishes, and one that names
        // the 40234 ZIP+4 only box_b prints.
        let (status, call) = f
            .create(&with(
                &f.call_note(),
                json!({
                    "fact_id": f.fee,
                    "slot": "fees.per_document",
                    "channel": "Apostille desk, (360) 555-0142",
                    "summary": "The desk confirmed the $15 fee per document.",
                }),
            ))
            .await;
        assert_eq!(status, StatusCode::CREATED, "{call}");
        let (status, email) = f
            .create(&with(
                &f.call_note(),
                json!({
                    "kind": "email",
                    "fact_id": f.fee,
                    "slot": "fees.per_document",
                    "channel": "apostille@sos.example",
                    "summary": "Reply from apostille@sos.example confirms the fee.",
                }),
            ))
            .await;
        assert_eq!(status, StatusCode::CREATED, "{email}");
        let (status, zip) = f
            .create(&with(
                &f.call_note(),
                json!({"fact_id": f.fee, "channel": "Apostille desk"}),
            ))
            .await;
        assert_eq!(status, StatusCode::CREATED, "{zip}");
        assert_eq!(
            ids(f.evidence().await),
            json!([call["id"], email["id"], zip["id"]])
        );
        // The ZIP+4 leaves the accepted facts with box_b: so does the
        // note that names it, though it is anchored to the fee.
        retire(f.box_b);
        assert_eq!(ids(f.evidence().await), json!([call["id"], email["id"]]));
        // The contacts fact was a private line accepted by mistake: once
        // it is retired, neither its number nor its email stays public.
        retire(f.contacts);
        assert_eq!(ids(f.evidence().await), json!([]));
        // The staff list still has them, for a withdrawal.
        let (status, listed) = send(
            f.admin(),
            admin_request(
                "GET",
                &format!("/apostille/api/v1/admin/evidence?slug={SLUG}"),
                Some(STAFF),
                false,
                None,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(listed["count"], 3);
    }

    #[test]
    fn pacific_dates_follow_us_daylight_time() {
        for (unix, day) in [
            (1_793_812_200, "2026-11-04"), // 09:10 PST
            (1_793_863_800, "2026-11-04"), // 23:30 PST (Nov 5 in UTC)
            (1_782_891_000, "2026-07-01"), // 00:30 PDT (PST would say Jun 30)
            (1_772_955_000, "2026-03-07"), // 23:30 PST, before DST starts
            (1_773_041_400, "2026-03-09"), // 00:30 PDT, after it starts
            (1_793_518_200, "2026-11-01"), // 00:30 PDT, before DST ends
            (1_793_604_600, "2026-11-01"), // 23:30 PST, after it ends
        ] {
            assert_eq!(la_date(unix), day, "{unix}");
        }
        // Switch instants: 10:00 UTC on the second Sunday of March,
        // 09:00 UTC on the first Sunday of November.
        for (start, end) in [
            (1_710_064_800, 1_730_624_400), // 2024
            (1_772_964_000, 1_793_523_600), // 2026
            (1_805_018_400, 1_825_578_000), // 2027
        ] {
            assert_eq!(la_offset(start - 1), -8 * 3_600);
            assert_eq!(la_offset(start), -7 * 3_600);
            assert_eq!(la_offset(end - 1), -7 * 3_600);
            assert_eq!(la_offset(end), -8 * 3_600);
        }
    }

    #[test]
    fn digit_runs_and_emails_read_like_people_write_them() {
        assert_eq!(long_digit_runs("(360) 555-0142"), ["3605550142"]);
        assert_eq!(
            long_digit_runs("360.555.0142 or 360\u{2011}555\u{2011}0142"),
            ["3605550142", "3605550142"]
        );
        assert_eq!(long_digit_runs("WA 98504-0234"), ["985040234"]);
        assert!(long_digit_runs("PO Box 40234, $15, 1-10 documents, 11/04").is_empty());
        // A comma ends a number only before a space: lists stay apart,
        // `206,555,0101` and `1,234,567` read whole. A slash joins, so a
        // full numeric date reads as a number, as an ISO date already did.
        assert!(long_digit_runs("Boxes 40220, 40234 and $1,500").is_empty());
        assert_eq!(long_digit_runs("1,234,567 requests"), ["1234567"]);
        assert_eq!(long_digit_runs("Called 11/04/2026"), ["11042026"]);
        // Middle dots, fullwidth stops and parentheses, and every
        // Unicode dash separate groups the way an ASCII dash does.
        for written in [
            "360\u{B7}555\u{B7}0142",
            "360\u{FF0E}555\u{FF0E}0142",
            "\u{FF08}360\u{FF09} 555\u{2013}0142",
            "360\u{FE63}555\u{FF0D}0142",
            "360\u{2E3A}555\u{2012}0142",
            // Slashes, commas between digits, underscores, pipes, and any
            // non-ASCII symbol or mark: hyphen bullet, box drawing, the
            // Braille blank, prolonged-sound marks, a combining low line.
            "360/555/0142",
            "360 / 555 / 0142",
            "360,555,0142",
            "360_555_0142",
            "360|555|0142",
            "360\u{FF0F}555\u{FF3F}0142",
            "360\u{2043}555\u{2043}0142",
            "360\u{2500}555\u{2501}0142",
            "360\u{2800}555\u{2800}0142",
            "360\u{30FC}555\u{FF70}0142",
            "3\u{0332}60555\u{0332}0142",
        ] {
            assert_eq!(long_digit_runs(written), ["3605550142"], "{written}");
        }
        assert_eq!(long_digit_runs("Office line 0221/12 34 56"), ["0221123456"]);
        // Invisible code points and non-ASCII digits are refused before
        // the run check; ordinary prose and ASCII digits are not.
        for hidden in [
            "206\u{200B}555",
            "a\u{200D}b",
            "\u{FEFF}lead",
            "x\u{2066}y\u{2069}",
            "soft\u{AD}hyphen",
            "fill\u{3164}er",
            "206\u{2800}555",
            "\u{FF15}",
            "\u{1D7DC}",
            "\u{2463}",
            "\u{0969}",
            "\u{00B9}",
        ] {
            assert!(hides_digits(hidden), "{hidden:?}");
        }
        for plain in [
            "Call (360) 555-0142, option 3.",
            "Café hours: 9 a.m. – 4 p.m., Mon–Fri",
            "PO Box 40234 · Olympia",
            "Ünïcode letters are fine",
        ] {
            assert!(!hides_digits(plain), "{plain}");
        }
        assert_eq!(
            emails_in("Write to Apostille@SOS.example. Or a@b"),
            ["apostille@sos.example"]
        );
        assert_eq!(
            emails_in("(x.y+z@mail.sos.example)"),
            ["x.y+z@mail.sos.example"]
        );
        assert!(emails_in("@sos.example and name@ and a@@b.c").is_empty());
        // Fullwidth and small forms fold to ASCII; letters of any script
        // belong to the address; spaces may sit around the `@`.
        for (written, read) in [
            ("jane.doe\u{FF20}example.com", "jane.doe@example.com"),
            ("jane.doe\u{FE6B}example.com", "jane.doe@example.com"),
            ("jane.doe@example\u{FF0E}com", "jane.doe@example.com"),
            ("jane.doe@example\u{3002}com", "jane.doe@example.com"),
            (
                "\u{FF4A}\u{FF41}\u{FF4E}\u{FF45}@example.com",
                "jane@example.com",
            ),
            ("Jos\u{E9}@Example.com", "jos\u{E9}@example.com"),
            ("ren\u{E9}e.doe@example.com", "ren\u{E9}e.doe@example.com"),
            ("jane@b\u{FC}cher.example", "jane@b\u{FC}cher.example"),
            ("jane@ex\u{0430}mple.com", "jane@ex\u{0430}mple.com"),
            ("write jane.doe @ example.com today", "jane.doe@example.com"),
        ] {
            assert_eq!(emails_in(written), [read], "{written}");
        }
        // An `@` that is no address: a time, a price, a bare number host.
        assert!(emails_in("meet @ 3pm, fee @ $15, arrived @ 9.30").is_empty());
        assert!(slot_ok("fees.same_day") && slot_ok("address.office__tumwater"));
        for bad in ["Fees", "fees.", ".fees", "fe-es", "fees.Same"] {
            assert!(!slot_ok(bad), "{bad}");
        }
    }

    #[test]
    fn contract_seeds_carry_the_evidence_shape() {
        let seed: Value =
            serde_json::from_str(include_str!("../../../docs/contracts/plan.json")).unwrap();
        let note = Note {
            id: 7,
            slug: SLUG.into(),
            kind: "call".into(),
            fact_id: Some(203),
            preferred_fact_id: None,
            slot: "mailing.po_box".into(),
            stance: "confirms".into(),
            occurred_at: NOV_02_0910_PT,
            channel: "Apostille desk".into(),
            summary: "SYNTHETIC FIXTURE".into(),
            recorded: false,
            capture_id: Some(3),
            supports: vec![],
            status: "published".into(),
            created_by: "staff-1".into(),
            created_at: NOV_02_0910_PT,
        };
        let served = note.public_view(&[]);
        for item in seed["evidence"].as_array().unwrap() {
            assert_eq!(keys(item), keys(&served), "{item}");
            assert_eq!(item["by"], PUBLIC_BY);
        }
        let empty: Value =
            serde_json::from_str(include_str!("../../../docs/contracts/plan-empty.json")).unwrap();
        assert_eq!(empty["evidence"], json!([]));
        let events: Value =
            serde_json::from_str(include_str!("../../../docs/contracts/ws-events.json")).unwrap();
        let seeded: BTreeSet<&str> = events["authority"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|e| e["type"].as_str())
            .collect();
        for kind in crate::ws_bus::AUTHORITY_KINDS {
            assert!(seeded.contains(kind), "ws-events.json seeds {kind}");
        }
        assert!(crate::ws_bus::AUTHORITY_KINDS.contains(&"evidence_updated"));
    }
}
