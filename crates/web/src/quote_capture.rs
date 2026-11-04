//! Staff quotes from a captured page: pick an exact span of a capture's
//! text sidecar (`<store>/text/<id>.txt`) and propose it as a fact.
//!
//! Staff (admin listener, guard stack from `mission_admin_routers`):
//! - `GET /apostille/api/v1/admin/authorities/{slug}/captures?limit=`:
//!   the office's captures that have text, newest first.
//! - `GET /apostille/api/v1/admin/captures/{id}/text`: one capture's
//!   sidecar (lossy UTF-8), refused over [`MAX_TEXT_BYTES`].
//! - `POST /apostille/api/v1/admin/captures/{id}/quote`: propose one
//!   fact citing a span of that sidecar.
//!
//! The fact lands `proposed` with real citation offsets (UTF-8 bytes
//! into the sidecar, end exclusive), so once a reviewer accepts it in
//! /review the public read cites it like any drafted fact: the quote,
//! the capture's https source and its date. Nothing publishes here.
//! The span is the console's selection (`start`/`end`, which must
//! still slice exactly `quote`) or, for prepared quotes, the one place
//! `quote` occurs (case-sensitive, any whitespace run between its
//! words); either way it is stored trimmed, may not cut a word or number
//! in two ("$15" of "$150"), and only a quote the public read serves
//! (8-400 characters, an https source) is taken. The statement is
//! `key: value` with the value inside the quote (the accept-with-edit
//! gate, plus the number guard: "$1" never rests on "$15"). One already
//! on file from the same capture
//! (office, topic and [`db::normalize_statement`]; proposed or accepted)
//! is refused. The same statement quoted from another page is not a
//! duplicate: it is a second citation, which the guide merges into the
//! same row.
//!
//! `created_by` is the role `staff`, never the user: it is served
//! publicly as `created_by`/`drafted_by`. The staff member is named in
//! the `fact_quote` audit row only (ids and offsets, no quote text).

use std::path::{Path as FsPath, PathBuf};

use axum::{
    Router,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Json, Response},
    routing::{get, post},
};
use contracts::topics::Topic;
use handbook_agent::analyze::{MAX_QUOTE_CHARS, MIN_QUOTE_CHARS, MIN_VALUE_CHARS, normalize};
use handbook_store::{db, store::Store};
use rusqlite::{Connection, OptionalExtension as _, TransactionBehavior};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{AppState, admin::AdminAuth, store_unavailable, unix_now};

/// Captures listed per authority: default and most.
const DEFAULT_LIMIT: i64 = 50;
const MAX_LIMIT: i64 = 200;
/// Largest sidecar either route reads (4 MiB): the console shows the
/// whole text, and quoting stays inside what staff can see.
const MAX_TEXT_BYTES: u64 = 4 * 1024 * 1024;
/// Longest statement, in chars (the accept-with-edit bound).
const MAX_STATEMENT_CHARS: usize = 2000;
/// `fact.created_by` of a staff quote: a role, never a user id.
const CREATED_BY: &str = "staff";

/// Staff routes merged onto the ADMIN listener only (the builder wraps
/// them in the admin guard stack; handlers take `AdminAuth`).
pub fn admin_router(state: AppState) -> Router {
    Router::<AppState>::new()
        .route(
            "/apostille/api/v1/admin/authorities/{slug}/captures",
            get(list_captures),
        )
        .route(
            "/apostille/api/v1/admin/captures/{id}/text",
            get(capture_text),
        )
        .route("/apostille/api/v1/admin/captures/{id}/quote", post(quote))
        .with_state(state)
}

fn open(state: &AppState) -> Option<Connection> {
    db::open(&Store::new(&state.store_root)).ok()
}

fn refusal(status: StatusCode, reason: &str) -> Response {
    (status, Json(json!({"error": reason}))).into_response()
}

fn unprocessable(reason: &str) -> Response {
    refusal(StatusCode::UNPROCESSABLE_ENTITY, reason)
}

/// Unknown, rejected, or without text: the same answer for all three.
fn not_found() -> Response {
    refusal(StatusCode::NOT_FOUND, "capture not found")
}

fn too_large(bytes: u64) -> Response {
    refusal(
        StatusCode::PAYLOAD_TOO_LARGE,
        &format!("this capture's text is {bytes} bytes, over the 4 MiB this tool reads"),
    )
}

fn length_refusal(chars: usize) -> Response {
    unprocessable(&format!(
        "a quote must be {MIN_QUOTE_CHARS} to {MAX_QUOTE_CHARS} characters (this one is {chars})"
    ))
}

// ---------------------------------------------------------- captures

/// One capture as this tool reads it (rejected ones never load).
struct Capture {
    id: i64,
    authority_id: i64,
    slug: String,
    source_id: Option<i64>,
    /// The source row's URI (`None`: no source, or its row is gone).
    source_uri: Option<String>,
    kind: String,
    trust: String,
    captured_at: i64,
}

const CAPTURE_SQL: &str =
    "SELECT c.id, c.authority_id, a.slug, c.source_id, s.uri, c.kind, c.trust, c.captured_at
     FROM capture c
     JOIN authority a ON a.id = c.authority_id
     LEFT JOIN source s ON s.id = c.source_id";

fn capture_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Capture> {
    Ok(Capture {
        id: row.get(0)?,
        authority_id: row.get(1)?,
        slug: row.get(2)?,
        source_id: row.get(3)?,
        source_uri: row.get(4)?,
        kind: row.get(5)?,
        trust: row.get(6)?,
        captured_at: row.get(7)?,
    })
}

/// One capture of a real office that is not rejected, or `None` (the
/// id-0 sentinels back unattributed rows and are never quoted).
fn load_capture(conn: &Connection, id: i64) -> rusqlite::Result<Option<Capture>> {
    conn.query_row(
        &format!(
            "{CAPTURE_SQL}
             WHERE c.id = ?1 AND c.id > 0 AND a.id != 0 AND c.trust != 'rejected'"
        ),
        [id],
        capture_row,
    )
    .optional()
}

impl Capture {
    /// The https URL the public read cites this capture's quotes to
    /// ([`crate::citation`]: its own source; a source-less page or
    /// upload borrows the office's website), or `None`: no quote of it
    /// could ever publish.
    fn cite_url(&self, conn: &Connection) -> Option<String> {
        crate::citation::CitedCapture::load(conn, self.id)?.source_url(conn, self.authority_id)
    }

    /// The page staff see: the cited URL, else the source's own URI.
    fn uri(&self, conn: &Connection) -> Option<String> {
        self.cite_url(conn).or_else(|| self.source_uri.clone())
    }
}

fn sidecar_path(root: &FsPath, id: i64) -> PathBuf {
    root.join("text").join(format!("{id}.txt"))
}

/// Sidecar size in bytes, or `None` when it is missing or empty.
fn sidecar_len(root: &FsPath, id: i64) -> Option<u64> {
    std::fs::metadata(sidecar_path(root, id))
        .ok()
        .filter(|meta| meta.is_file() && meta.len() > 0)
        .map(|meta| meta.len())
}

/// A capture's sidecar bytes, or the refusal (404 missing or empty,
/// 413 over [`MAX_TEXT_BYTES`]). The `Err` response is boxed: a bare
/// `Response` trips `clippy::result_large_err`.
fn read_sidecar(root: &FsPath, id: i64) -> Result<Vec<u8>, Box<Response>> {
    let len = sidecar_len(root, id).ok_or_else(|| Box::new(not_found()))?;
    if len > MAX_TEXT_BYTES {
        return Err(Box::new(too_large(len)));
    }
    let bytes = std::fs::read(sidecar_path(root, id)).map_err(|_| Box::new(not_found()))?;
    // The file may have changed since its size was read.
    match bytes.len() as u64 {
        0 => Err(Box::new(not_found())),
        n if n > MAX_TEXT_BYTES => Err(Box::new(too_large(n))),
        _ => Ok(bytes),
    }
}

#[derive(Debug, Deserialize)]
struct ListParams {
    /// 1-200, default 50.
    limit: Option<i64>,
}

/// `GET /apostille/api/v1/admin/authorities/{slug}/captures`: the
/// office's captures with a non-empty sidecar, newest first (rejected
/// ones never). `bytes` is the sidecar's size; `uri` the page a
/// citation names ([`Capture::uri`]).
async fn list_captures(
    _auth: AdminAuth,
    State(state): State<AppState>,
    Path(slug): Path<String>,
    Query(params): Query<ListParams>,
) -> Response {
    let limit = params.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT) as usize;
    let Some(conn) = open(&state) else {
        return store_unavailable();
    };
    let authority: Option<i64> = match conn
        .query_row(
            "SELECT id FROM authority WHERE slug = ?1 AND id != 0",
            [&slug],
            |row| row.get(0),
        )
        .optional()
    {
        Ok(found) => found,
        Err(_) => return store_unavailable(),
    };
    let Some(authority_id) = authority else {
        return refusal(StatusCode::NOT_FOUND, "unknown authority");
    };
    let listed = (|| -> rusqlite::Result<Vec<Value>> {
        let mut stmt = conn.prepare(&format!(
            "{CAPTURE_SQL}
             WHERE c.authority_id = ?1 AND c.id > 0 AND c.trust != 'rejected'
             ORDER BY c.captured_at DESC, c.id DESC"
        ))?;
        let mut captures = Vec::new();
        for row in stmt.query_map([authority_id], capture_row)? {
            let capture = row?;
            let Some(bytes) = sidecar_len(&state.store_root, capture.id) else {
                continue;
            };
            captures.push(json!({
                "id": capture.id,
                "source_id": capture.source_id,
                "uri": capture.uri(&conn),
                "kind": capture.kind,
                "trust": capture.trust,
                "captured_at": capture.captured_at,
                "bytes": bytes,
            }));
            if captures.len() == limit {
                break;
            }
        }
        Ok(captures)
    })();
    match listed {
        Ok(captures) => Json(json!({"slug": slug, "captures": captures})).into_response(),
        Err(_) => store_unavailable(),
    }
}

/// `GET /apostille/api/v1/admin/captures/{id}/text`: the whole sidecar
/// (lossy UTF-8) with the capture's page and date.
async fn capture_text(
    _auth: AdminAuth,
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Response {
    let Some(conn) = open(&state) else {
        return store_unavailable();
    };
    let capture = match load_capture(&conn, id) {
        Ok(Some(capture)) => capture,
        Ok(None) => return not_found(),
        Err(_) => return store_unavailable(),
    };
    let bytes = match read_sidecar(&state.store_root, id) {
        Ok(bytes) => bytes,
        Err(refused) => return *refused,
    };
    Json(json!({
        "id": capture.id,
        "slug": capture.slug,
        "uri": capture.uri(&conn),
        "kind": capture.kind,
        "trust": capture.trust,
        "captured_at": capture.captured_at,
        "text": String::from_utf8_lossy(&bytes),
    }))
    .into_response()
}

// ------------------------------------------------------------- spans

/// Why a quote has no single span in the text.
#[derive(Debug, PartialEq, Eq)]
enum Miss {
    /// `start`/`end` do not mark a span inside the text.
    OutOfBounds,
    /// `start` or `end` splits a character.
    Split,
    /// The selection no longer slices exactly the quote.
    Stale,
    /// A prepared quote occurs nowhere.
    NotFound,
    /// A prepared quote occurs this many times.
    Ambiguous(usize),
    /// A prepared quote can never pass the length gate (no words, or
    /// its words alone run over it): its trimmed length in chars.
    Length(usize),
    /// The selection starts or ends inside a word or number.
    CutsToken,
}

/// The trimmed byte span of `text` that `quote` cites: the console's
/// selection (`offsets`, end exclusive), which must slice exactly
/// `quote`, else the one place `quote` occurs (prepared quotes).
fn span_of(
    text: &str,
    quote: &str,
    offsets: Option<(usize, usize)>,
) -> Result<(usize, usize), Miss> {
    let (start, end) = match offsets {
        Some((start, end)) => {
            if start >= end || end > text.len() {
                return Err(Miss::OutOfBounds);
            }
            if !text.is_char_boundary(start) || !text.is_char_boundary(end) {
                return Err(Miss::Split);
            }
            if text[start..end] != *quote {
                return Err(Miss::Stale);
            }
            (start, end)
        }
        None => {
            // The stored span holds every word and at least one space
            // between each pair, so it is never shorter than this.
            let words: Vec<&str> = quote.split_whitespace().collect();
            let floor = words.iter().map(|w| w.chars().count()).sum::<usize>()
                + words.len().saturating_sub(1);
            if words.is_empty() || floor > MAX_QUOTE_CHARS {
                return Err(Miss::Length(quote.trim().chars().count()));
            }
            match occurrences(text, &words).as_slice() {
                [] => return Err(Miss::NotFound),
                [one] => *one,
                many => return Err(Miss::Ambiguous(many.len())),
            }
        }
    };
    let span = &text[start..end];
    let start = start + (span.len() - span.trim_start().len());
    let end = start + span.trim().len();
    if cuts_token(text, start) || cuts_token(text, end) {
        return Err(Miss::CutsToken);
    }
    Ok((start, end))
}

/// True when a span edge at byte `at` splits a word or number: a letter
/// or digit on both sides, or a number running on through a `.` or `,`
/// ("$15" of "$15.50", "$1" of "$1,000"). Glued page text stays quotable
/// ("Pay by check." of "Pay by check.Office").
fn cuts_token(text: &str, at: usize) -> bool {
    let mut before = text[..at].chars().rev();
    let mut after = text[at..].chars();
    let (prev, next) = (before.next(), after.next());
    let alnum = |c: Option<char>| c.is_some_and(char::is_alphanumeric);
    let digit = |c: Option<char>| c.is_some_and(|c| c.is_ascii_digit());
    let separator = |c: Option<char>| matches!(c, Some('.' | ','));
    (alnum(prev) && alnum(next))
        || (digit(prev) && separator(next) && digit(after.next()))
        || (separator(prev) && digit(next) && digit(before.next()))
}

/// Every span where `words` occur in order, each pair separated by one
/// or more whitespace chars, case-sensitive: from the first byte of the
/// first word to the end of the last. Overlapping spans each count, so
/// a count never understates the ambiguity; a hit that cuts a word or
/// number in two is not an occurrence.
fn occurrences(text: &str, words: &[&str]) -> Vec<(usize, usize)> {
    let Some(first) = words.first() else {
        return Vec::new();
    };
    let mut found = Vec::new();
    let mut from = 0;
    while let Some(at) = text[from..].find(first) {
        let start = from + at;
        if let Some(end) = words_end(text, start, words)
            && !cuts_token(text, start)
            && !cuts_token(text, end)
        {
            found.push((start, end));
        }
        from = start + text[start..].chars().next().map_or(1, char::len_utf8);
    }
    found
}

/// Where `words` end when they match at `start`, or `None`.
fn words_end(text: &str, start: usize, words: &[&str]) -> Option<usize> {
    let mut at = start;
    for (i, word) in words.iter().enumerate() {
        if i > 0 {
            let rest = &text[at..];
            let gap = rest.len() - rest.trim_start().len();
            if gap == 0 {
                return None;
            }
            at += gap;
        }
        if !text[at..].starts_with(word) {
            return None;
        }
        at += word.len();
    }
    Some(at)
}

fn miss_refusal(miss: Miss) -> Response {
    match miss {
        Miss::OutOfBounds => {
            unprocessable("start and end must mark a span inside this capture's text")
        }
        Miss::Split => unprocessable("start and end must fall on character boundaries"),
        Miss::Stale => refusal(
            StatusCode::CONFLICT,
            "the selected text no longer matches this capture; reload it",
        ),
        Miss::NotFound => unprocessable("quote not found in this capture's text"),
        Miss::Ambiguous(count) => (
            StatusCode::CONFLICT,
            Json(json!({
                "error": format!(
                    "quote appears {count} times in this capture; select the one you mean in the text"
                ),
                "count": count,
            })),
        )
            .into_response(),
        Miss::Length(chars) => length_refusal(chars),
        Miss::CutsToken => {
            unprocessable("the quote cuts a word or number in two: select the whole of it")
        }
    }
}

/// Why `statement` (trimmed) cannot rest on `quote`, or `None`: it is
/// `key: value` ([`crate::approval::edited_value`]) with a value of at
/// least [`MIN_VALUE_CHARS`] inside the quote, under the same
/// normalization as the accept-with-edit gate. Containment alone would
/// let "$1" rest on "$15", so every number, amount, PO box and street
/// in the value must also stand whole in the quote
/// ([`crate::translate::guard_ok`]).
fn statement_refusal(statement: &str, quote: &str) -> Option<&'static str> {
    let chars = statement.chars().count();
    if chars == 0 || chars > MAX_STATEMENT_CHARS {
        return Some("statement must be 1-2000 chars");
    }
    let Some(value) = crate::approval::edited_value(statement) else {
        return Some("statement must be 'key: value'");
    };
    if value.trim().chars().count() < MIN_VALUE_CHARS {
        return Some("the value after the key must be at least 2 characters");
    }
    let grounded =
        normalize(quote).contains(&normalize(value)) && crate::translate::guard_ok(value, quote);
    (!grounded).then_some("edited value not in citation quote")
}

// ------------------------------------------------------------- quote

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)] // m3: mass-assignment hygiene on mutating DTOs
struct QuoteBody {
    quote: String,
    /// UTF-8 byte offsets into the sidecar, end exclusive: both or
    /// neither (neither: the server finds `quote` in the text).
    start: Option<u32>,
    end: Option<u32>,
    topic: String,
    statement: String,
}

/// One checked proposal, ready to insert.
struct Proposal<'a> {
    capture: &'a Capture,
    start: u32,
    end: u32,
    topic: &'a str,
    statement: &'a str,
}

/// What [`propose`] did.
enum Proposed {
    Created(i64),
    /// The statement is already on file as this fact.
    OnFile {
        fact_id: i64,
        status: String,
    },
}

/// Insert the proposed fact and its `fact_quote` audit row, unless this
/// capture already gave the office the statement under the topic
/// (proposed or accepted; accepted reported first). Another capture's
/// copy is corroboration, not a duplicate. `BEGIN IMMEDIATE` serializes
/// concurrent quotes, so the check-then-insert cannot duplicate.
fn propose(
    conn: &mut Connection,
    proposal: &Proposal<'_>,
    actor: &str,
    now: i64,
) -> contracts::Result<Proposed> {
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let norm = db::normalize_statement(proposal.statement);
    let on_file = {
        let mut stmt = tx.prepare(
            "SELECT id, statement, status FROM fact
             WHERE authority_id = ?1 AND topic = ?2 AND capture_id = ?3
               AND status IN ('proposed', 'accepted')
             ORDER BY status = 'accepted' DESC, id",
        )?;
        let rows = stmt.query_map(
            rusqlite::params![
                proposal.capture.authority_id,
                proposal.topic,
                proposal.capture.id
            ],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )?;
        let mut on_file = None;
        for row in rows {
            let (fact_id, statement, status) = row?;
            if db::normalize_statement(&statement) == norm {
                on_file = Some(Proposed::OnFile { fact_id, status });
                break;
            }
        }
        on_file
    };
    if let Some(on_file) = on_file {
        return Ok(on_file);
    }
    let fact_id = db::insert_proposed_fact(
        &tx,
        &db::NewProposedFact {
            authority_id: proposal.capture.authority_id,
            capture_id: proposal.capture.id,
            topic: proposal.topic,
            statement: proposal.statement,
            created_by: CREATED_BY,
            created_at: now,
            citation: Some((proposal.start, proposal.end)),
            reasoning: "",
        },
    )?;
    db::insert_audit(
        &tx,
        now,
        actor,
        "fact_quote",
        "fact",
        fact_id,
        "",
        "proposed",
        &format!(
            "capture_id={} start={} end={} topic={}",
            proposal.capture.id, proposal.start, proposal.end, proposal.topic
        ),
    )?;
    tx.commit()?;
    Ok(Proposed::Created(fact_id))
}

/// `POST /apostille/api/v1/admin/captures/{id}/quote`: propose one fact
/// citing a span of the capture's text (201). Audited as `fact_quote`
/// and announced as `proposal.created` on the staff `proposals` topic.
async fn quote(
    auth: AdminAuth,
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(body): Json<QuoteBody>,
) -> Response {
    let offsets = match (body.start, body.end) {
        (Some(start), Some(end)) => Some((start as usize, end as usize)),
        (None, None) => None,
        _ => return unprocessable("send start and end together, or neither"),
    };
    let Some(mut conn) = open(&state) else {
        return store_unavailable();
    };
    let capture = match load_capture(&conn, id) {
        Ok(Some(capture)) => capture,
        Ok(None) => return not_found(),
        Err(_) => return store_unavailable(),
    };
    let bytes = match read_sidecar(&state.store_root, id) {
        Ok(bytes) => bytes,
        Err(refused) => return *refused,
    };
    // A quote of this capture could never be served, so accept would
    // refuse it: say so now instead of queueing a dead proposal.
    if capture.cite_url(&conn).is_none() {
        return unprocessable("this capture has no https source page to cite");
    }
    // Offsets index the raw bytes; a lossy decode would shift them.
    let Ok(text) = std::str::from_utf8(&bytes) else {
        return unprocessable(
            "this capture's text is not valid UTF-8, so no span of it can be cited",
        );
    };
    let (start, end) = match span_of(text, &body.quote, offsets) {
        Ok(span) => span,
        Err(miss) => return miss_refusal(miss),
    };
    let quote = &text[start..end];
    let chars = quote.chars().count();
    if !(MIN_QUOTE_CHARS..=MAX_QUOTE_CHARS).contains(&chars) {
        return length_refusal(chars);
    }
    let topic = match Topic::parse(&body.topic) {
        Ok(Topic::PhoneScript) => {
            return unprocessable("phone_script stays staff-only: quote into another topic");
        }
        Ok(topic) => topic,
        Err(_) => return unprocessable("unknown handbook topic"),
    };
    let statement = body.statement.trim();
    if let Some(reason) = statement_refusal(statement, quote) {
        return unprocessable(reason);
    }
    // The sidecar is at most 4 MiB, so its offsets fit a u32.
    let proposal = Proposal {
        capture: &capture,
        start: start as u32,
        end: end as u32,
        topic: topic.as_str(),
        statement,
    };
    let now = unix_now();
    let fact_id = match propose(&mut conn, &proposal, &auth.user.user_id, now) {
        Ok(Proposed::Created(fact_id)) => fact_id,
        Ok(Proposed::OnFile { fact_id, status }) => {
            return (
                StatusCode::CONFLICT,
                Json(json!({
                    "error": "this statement is already on file from this capture",
                    "fact_id": fact_id,
                    "status": status,
                })),
            )
                .into_response();
        }
        Err(_) => return store_unavailable(),
    };
    // Same staff-only fan-out as every new proposal, so /review updates
    // live; ids only on the socket.
    state.bus.publish(
        "proposals",
        json!({"type": "proposal.created", "topic": "proposals",
               "fact_id": fact_id, "authority": capture.slug, "at": now})
        .to_string(),
    );
    (
        StatusCode::CREATED,
        Json(json!({
            "fact_id": fact_id,
            "status": "proposed",
            "capture_id": capture.id,
            "start": proposal.start,
            "end": proposal.end,
            "quote": quote,
            "topic": topic.as_str(),
            "statement": statement,
            "slug": capture.slug,
        })),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt as _;

    const STAFF: &str = "staff-token-quote";
    const CUSTOMER: &str = "customer-token-quote";
    const SLUG: &str = "usa-wa-sos";
    const SITE: &str = "https://www.sos.wa.gov/corporations-charities/apostilles";
    /// One captured page: a fee, "Monday through Friday" twice (an
    /// ambiguous prepared quote) and a PO box broken over two lines.
    const PAGE: &str = "Apostilles and Certificates\n\
                        The fee is $15 per document.\n\
                        Walk-in service: Monday through Friday, 8 a.m. to 4 p.m.\n\
                        Mail service: Monday through Friday, in the order received.\n\
                        Mail requests to PO Box 40234,\n  Olympia, WA 98504-0234.\n";
    const FEE: &str = "The fee is $15 per document.";

    struct Fixture {
        _dir: tempfile::TempDir,
        state: AppState,
        office: i64,
        /// The office's https `website_official` source.
        source: i64,
        /// `PAGE`, fetched from `source` at unix second 100.
        page: i64,
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
            fakes: config::Modes::default(),
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

    impl Fixture {
        fn new() -> Self {
            let dir = tempfile::TempDir::new().unwrap();
            let store = Store::new(dir.path());
            store.init().unwrap();
            let conn = db::open(&store).unwrap();
            db::migrate(&conn).unwrap();
            auth::migrate_auth_tables(&conn).unwrap();
            config::migrate_config_tables(&conn).unwrap();
            // Boot DDL exactly as `main` runs it.
            crate::migrate_mission_tables(&conn).unwrap();
            for (token, user_id, role) in [
                (STAFF, "staff-1", auth::Role::Staff),
                (CUSTOMER, "cust-1", auth::Role::Customer),
            ] {
                auth::create_session(
                    &conn,
                    token,
                    &auth::NewSession {
                        user_id,
                        role,
                        customer_ref: None,
                        now: unix_now(),
                        ip: "127.0.0.1",
                        user_agent: "test",
                    },
                )
                .unwrap();
            }
            let office = add_office(&conn, SLUG);
            let state = state_for(dir.path());
            let mut fixture = Fixture {
                _dir: dir,
                state,
                office,
                source: 0,
                page: 0,
            };
            fixture.source = fixture.add_source("website_official", SITE);
            fixture.page = fixture.capture(
                Some(fixture.source),
                "warc",
                "official",
                100,
                Some(PAGE.as_bytes()),
            );
            fixture
        }

        fn conn(&self) -> Connection {
            db::open(&Store::new(&self.state.store_root)).unwrap()
        }

        fn add_source(&self, kind: &str, uri: &str) -> i64 {
            let conn = self.conn();
            conn.execute(
                "INSERT INTO source(authority_id, kind, uri) VALUES (?1, ?2, ?3)",
                rusqlite::params![self.office, kind, uri],
            )
            .unwrap();
            conn.last_insert_rowid()
        }

        /// One capture of the office, with `text` as its sidecar.
        fn capture(
            &self,
            source: Option<i64>,
            kind: &str,
            trust: &str,
            captured_at: i64,
            text: Option<&[u8]>,
        ) -> i64 {
            self.capture_of(self.office, source, kind, trust, captured_at, text)
        }

        fn capture_of(
            &self,
            authority: i64,
            source: Option<i64>,
            kind: &str,
            trust: &str,
            captured_at: i64,
            text: Option<&[u8]>,
        ) -> i64 {
            let conn = self.conn();
            conn.execute(
                "INSERT INTO capture(authority_id, source_id, captured_at, kind, path,
                                      bytes_raw, bytes_stored, sha256, trust)
                 VALUES (?1, ?2, ?3, ?4, 'p', 0, 0, 's', ?5)",
                rusqlite::params![authority, source, captured_at, kind, trust],
            )
            .unwrap();
            let id = conn.last_insert_rowid();
            if let Some(text) = text {
                let dir = self.state.store_root.join("text");
                std::fs::create_dir_all(&dir).unwrap();
                std::fs::write(dir.join(format!("{id}.txt")), text).unwrap();
            }
            id
        }

        fn facts(&self) -> i64 {
            self.conn()
                .query_row("SELECT COUNT(*) FROM fact", [], |row| row.get(0))
                .unwrap()
        }

        /// The production admin stack: host guard, IP allowlist, CSRF,
        /// and the approval routes beside the lane routers.
        fn admin(&self) -> Router {
            crate::admin::router(self.state.clone())
                .merge(crate::mission_admin_routers(&self.state))
        }

        fn public(&self) -> Router {
            crate::public_router(self.state.clone(), "static", "test-spa", "test-acme")
        }

        async fn get(&self, uri: &str) -> (StatusCode, Value) {
            send(self.admin(), request("GET", uri, Some(STAFF), false, None)).await
        }

        async fn post(&self, uri: &str, body: &Value) -> (StatusCode, Value) {
            send(
                self.admin(),
                request("POST", uri, Some(STAFF), true, Some(body)),
            )
            .await
        }

        async fn quote(&self, capture: i64, body: Value) -> (StatusCode, Value) {
            self.post(
                &format!("/apostille/api/v1/admin/captures/{capture}/quote"),
                &body,
            )
            .await
        }

        async fn text(&self, capture: i64) -> (StatusCode, Value) {
            self.get(&format!("/apostille/api/v1/admin/captures/{capture}/text"))
                .await
        }

        /// The anonymous `/facts` read of the office.
        async fn public_facts(&self) -> Vec<Value> {
            let req = Request::builder()
                .uri(format!("/apostille/api/v1/authorities/{SLUG}/facts"))
                .extension(axum::extract::ConnectInfo(
                    "192.0.2.88:443".parse::<std::net::SocketAddr>().unwrap(),
                ))
                .body(Body::empty())
                .unwrap();
            let (status, body) = send(self.public(), req).await;
            assert_eq!(status, StatusCode::OK, "{body}");
            body["facts"].as_array().expect("facts array").clone()
        }
    }

    /// Admin request: loopback peer, `localhost` host (the dev seam),
    /// a session cookie, and the CSRF pair when `csrf`.
    fn request(
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
        let bytes = axum::body::to_bytes(res.into_body(), 16 << 20)
            .await
            .unwrap();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    /// The console's body: `quote` selected where it first occurs.
    fn selected(text: &str, quote: &str, topic: &str, statement: &str) -> Value {
        let start = text.find(quote).expect("quote in text");
        json!({
            "quote": quote,
            "start": start,
            "end": start + quote.len(),
            "topic": topic,
            "statement": statement,
        })
    }

    /// A prepared quote: no offsets, the server finds it.
    fn prepared(quote: &str, topic: &str, statement: &str) -> Value {
        json!({"quote": quote, "topic": topic, "statement": statement})
    }

    #[tokio::test]
    async fn selection_proposes_a_cited_fact_that_publishes_once_accepted() {
        let f = Fixture::new();
        let mut rx = f.state.bus.subscribe("proposals");
        let start = PAGE.find(FEE).unwrap();
        let end = start + FEE.len();

        let (status, made) = f
            .quote(f.page, selected(PAGE, FEE, "fees", "per_document: $15"))
            .await;
        assert_eq!(status, StatusCode::CREATED, "{made}");
        let fact_id = made["fact_id"].as_i64().unwrap();
        assert_eq!(
            made,
            json!({
                "fact_id": fact_id,
                "status": "proposed",
                "capture_id": f.page,
                "start": start,
                "end": end,
                "quote": FEE,
                "topic": "fees",
                "statement": "per_document: $15",
                "slug": SLUG,
            })
        );

        // Proposed with real offsets and the role byline, never a user.
        let row: (String, String, i64, i64, i64, String, String) = f
            .conn()
            .query_row(
                "SELECT status, created_by, capture_id, citation_start, citation_end,
                        statement, reasoning
                 FROM fact WHERE id = ?1",
                [fact_id],
                |r| {
                    Ok((
                        r.get(0)?,
                        r.get(1)?,
                        r.get(2)?,
                        r.get(3)?,
                        r.get(4)?,
                        r.get(5)?,
                        r.get(6)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(
            row,
            (
                "proposed".into(),
                "staff".into(),
                f.page,
                start as i64,
                end as i64,
                "per_document: $15".into(),
                String::new(),
            )
        );

        // The audit row names the staff member: ids and offsets only.
        let audit: (String, String, i64, String, String, String) = f
            .conn()
            .query_row(
                "SELECT actor, entity, entity_id, before_ref, after_ref, reason
                 FROM audit WHERE action = 'fact_quote'",
                [],
                |r| {
                    Ok((
                        r.get(0)?,
                        r.get(1)?,
                        r.get(2)?,
                        r.get(3)?,
                        r.get(4)?,
                        r.get(5)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(
            audit,
            (
                "staff-1".into(),
                "fact".into(),
                fact_id,
                String::new(),
                "proposed".into(),
                format!("capture_id={} start={start} end={end} topic=fees", f.page),
            )
        );

        // Staff-only fan-out, ids only.
        let event: Value = serde_json::from_str(&rx.try_recv().unwrap()).unwrap();
        assert_eq!(event["type"], "proposal.created");
        assert_eq!(event["topic"], "proposals");
        assert_eq!(event["fact_id"], fact_id);
        assert_eq!(event["authority"], SLUG);
        assert!(event["at"].as_i64().unwrap() > 0, "{event}");
        assert!(event.get("quote").is_none(), "{event}");

        // Nothing public until a reviewer accepts it on the real route.
        assert!(f.public_facts().await.is_empty());
        let (status, accepted) = f
            .post(
                &format!("/apostille/api/v1/facts/{fact_id}/accept"),
                &json!({}),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{accepted}");
        assert_eq!(accepted["status"], "accepted");
        let facts = f.public_facts().await;
        assert_eq!(facts.len(), 1, "{facts:?}");
        let fact = &facts[0];
        assert_eq!(fact["id"], fact_id);
        assert_eq!(fact["statement"], "per_document: $15");
        assert_eq!(fact["created_by"], "staff");
        assert_eq!(fact["drafted_by"], "staff");
        let citation = &fact["citations"][0];
        assert_eq!(citation["quote"], FEE);
        assert_eq!(citation["source_url"], SITE);
        assert_eq!(citation["capture_id"], f.page);
        assert_eq!(citation["kind"], "web");

        // Quoting it again finds the published fact.
        let (status, again) = f
            .quote(f.page, prepared(FEE, "fees", "per_document: $15"))
            .await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(
            again,
            json!({
                "error": "this statement is already on file from this capture",
                "fact_id": fact_id,
                "status": "accepted",
            })
        );
        assert_eq!(f.facts(), 1);
    }

    #[tokio::test]
    async fn prepared_quote_matches_across_a_line_break() {
        let f = Fixture::new();
        let mut rx = f.state.bus.subscribe("proposals");
        let (status, made) = f
            .quote(
                f.page,
                prepared(
                    "PO Box 40234, Olympia, WA 98504-0234.",
                    "po_box",
                    "po_box: PO Box 40234, Olympia, WA 98504-0234",
                ),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "{made}");
        let start = PAGE.find("PO Box 40234").unwrap();
        let end = PAGE.find("98504-0234.").unwrap() + "98504-0234.".len();
        assert_eq!(made["start"], start);
        assert_eq!(made["end"], end);
        assert_eq!(made["quote"], "PO Box 40234,\n  Olympia, WA 98504-0234.");
        assert_eq!(made["quote"], PAGE[start..end]);
        let fact_id = made["fact_id"].as_i64().unwrap();
        let stored: (i64, i64, String) = f
            .conn()
            .query_row(
                "SELECT citation_start, citation_end, created_by FROM fact WHERE id = ?1",
                [fact_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(stored, (start as i64, end as i64, "staff".into()));
        let event: Value = serde_json::from_str(&rx.try_recv().unwrap()).unwrap();
        assert_eq!(event["fact_id"], fact_id);
    }

    /// Accents, dashes and curly quotes before the quote: offsets are
    /// UTF-8 bytes, never chars.
    #[tokio::test]
    async fn multibyte_text_offsets_are_utf8_bytes() {
        let f = Fixture::new();
        let text = "Café — Olympia’s office.\nThe fee is €20 per apostille.\n";
        let id = f.capture(
            Some(f.source),
            "warc",
            "official",
            200,
            Some(text.as_bytes()),
        );
        let quote = "The fee is €20 per apostille.";
        let start = text.find(quote).unwrap();
        let chars_before = text.chars().take_while(|c| *c != '\n').count() + 1;
        assert_ne!(start, chars_before, "the fixture needs multibyte text");

        let (status, made) = f
            .quote(id, prepared(quote, "fees", "per_apostille: €20"))
            .await;
        assert_eq!(status, StatusCode::CREATED, "{made}");
        assert_eq!(made["start"], start);
        assert_eq!(made["end"], start + quote.len());
        assert_eq!(made["quote"], quote);

        // The console's selection of another span is in bytes too.
        let office = "Olympia’s office.";
        let (status, made) = f
            .quote(
                id,
                selected(text, office, "address", "office: Olympia’s office"),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "{made}");
        assert_eq!(made["start"], text.find(office).unwrap());
        assert_eq!(made["quote"], office);

        // Byte 4 sits inside "é".
        assert!(!text.is_char_boundary(4));
        let (status, refused) = f
            .quote(
                id,
                json!({"quote": "afé — Olympia", "start": 4, "end": 20,
                       "topic": "address", "statement": "office: Olympia"}),
            )
            .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(
            refused["error"],
            "start and end must fall on character boundaries"
        );
        assert_eq!(f.facts(), 2);
    }

    #[tokio::test]
    async fn prepared_quote_must_occur_exactly_once() {
        let f = Fixture::new();
        // Whitespace in the quote is flexible; the count is the page's.
        let (status, refused) = f
            .quote(
                f.page,
                prepared(
                    "Monday  through\tFriday,",
                    "hours",
                    "weekday: Monday through Friday",
                ),
            )
            .await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(
            refused,
            json!({
                "error": "quote appears 2 times in this capture; select the one you mean in the text",
                "count": 2,
            })
        );
        for missing in [
            "Tuesday through Thursday",
            // Case-sensitive.
            "the fee is $15 per document.",
            // Words must be apart in the text, not glued.
            "The fee is $15perdocument.",
        ] {
            let (status, refused) = f
                .quote(f.page, prepared(missing, "fees", "per_document: $15"))
                .await;
            assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{missing}");
            assert_eq!(
                refused["error"], "quote not found in this capture's text",
                "{missing}"
            );
        }
        assert_eq!(f.facts(), 0);
    }

    #[tokio::test]
    async fn selection_must_still_slice_its_quote() {
        let f = Fixture::new();
        let start = PAGE.find(FEE).unwrap();
        let end = start + FEE.len();
        let body = |quote: &str, start: Option<usize>, end: Option<usize>| {
            let mut body =
                json!({"quote": quote, "topic": "fees", "statement": "per_document: $15"});
            if let Some(start) = start {
                body["start"] = json!(start);
            }
            if let Some(end) = end {
                body["end"] = json!(end);
            }
            body
        };
        // The page under the selection says something else now.
        let (status, refused) = f
            .quote(
                f.page,
                body("The fee is $20 per document.", Some(start), Some(end)),
            )
            .await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(
            refused,
            json!({"error": "the selected text no longer matches this capture; reload it"})
        );
        for (start, end) in [(Some(start), None), (None, Some(end))] {
            let (status, refused) = f.quote(f.page, body(FEE, start, end)).await;
            assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
            assert_eq!(refused["error"], "send start and end together, or neither");
        }
        for (start, end) in [(end, start), (start, start), (start, PAGE.len() + 1)] {
            let (status, refused) = f.quote(f.page, body(FEE, Some(start), Some(end))).await;
            assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{start}..{end}");
            assert_eq!(
                refused["error"],
                "start and end must mark a span inside this capture's text"
            );
        }
        // A negative offset is no u32: the body does not parse.
        let mut negative = body(FEE, Some(start), Some(end));
        negative["start"] = json!(-1);
        let (status, _) = f.quote(f.page, negative).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(f.facts(), 0);
    }

    #[tokio::test]
    async fn selection_is_trimmed_to_its_text() {
        let f = Fixture::new();
        let padded = format!("\n{FEE}\n");
        let start = PAGE.find(&padded).unwrap();
        let (status, made) = f
            .quote(
                f.page,
                json!({"quote": padded, "start": start, "end": start + padded.len(),
                       "topic": "fees", "statement": "per_document: $15"}),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "{made}");
        assert_eq!(made["quote"], FEE);
        assert_eq!(made["start"], start + 1);
        assert_eq!(made["end"], start + 1 + FEE.len());
        let stored: (i64, i64) = f
            .conn()
            .query_row(
                "SELECT citation_start, citation_end FROM fact WHERE id = ?1",
                [made["fact_id"].as_i64().unwrap()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(stored, ((start + 1) as i64, (start + 1 + FEE.len()) as i64));

        // A selection of whitespace alone trims to nothing.
        let gap = PAGE.find(",\n  Olympia").unwrap() + 1;
        let (status, refused) = f
            .quote(
                f.page,
                json!({"quote": "\n  ", "start": gap, "end": gap + 3,
                       "topic": "po_box", "statement": "po_box: 40234"}),
            )
            .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(
            refused["error"],
            "a quote must be 8 to 400 characters (this one is 0)"
        );
    }

    /// Only quotes the public read serves (8-400 chars) are taken.
    #[tokio::test]
    async fn quote_length_matches_the_public_citation() {
        let f = Fixture::new();
        let refused_at = |status: StatusCode, refused: &Value, chars: usize| {
            assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{refused}");
            assert_eq!(
                refused["error"],
                format!("a quote must be 8 to 400 characters (this one is {chars})")
            );
        };
        let (status, refused) = f
            .quote(f.page, selected(PAGE, "The fee", "fees", "fee: fee"))
            .await;
        refused_at(status, &refused, 7);
        let (status, made) = f
            .quote(f.page, selected(PAGE, "document", "fees", "per: document"))
            .await;
        assert_eq!(status, StatusCode::CREATED, "{made}");

        let long = f.capture(
            Some(f.source),
            "warc",
            "official",
            200,
            Some(format!("{}\n", "a".repeat(401)).as_bytes()),
        );
        let (status, refused) = f
            .quote(
                long,
                json!({"quote": "a".repeat(401), "start": 0, "end": 401,
                       "topic": "fees", "statement": "aa: aa"}),
            )
            .await;
        refused_at(status, &refused, 401);
        // A prepared quote whose words alone run over is refused unsearched.
        let (status, refused) = f
            .quote(long, prepared(&"a".repeat(401), "fees", "aa: aa"))
            .await;
        refused_at(status, &refused, 401);
        let (status, refused) = f.quote(long, prepared(" \n\t ", "fees", "aa: aa")).await;
        refused_at(status, &refused, 0);
        let fits = f.capture(
            Some(f.source),
            "warc",
            "official",
            300,
            Some(format!("{}\n", "a".repeat(400)).as_bytes()),
        );
        let (status, made) = f
            .quote(
                fits,
                json!({"quote": "a".repeat(400), "start": 0, "end": 400,
                       "topic": "fees", "statement": "aa: aa"}),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "{made}");
        assert_eq!(made["quote"].as_str().unwrap().chars().count(), 400);
    }

    #[tokio::test]
    async fn statement_and_topic_refusals() {
        let f = Fixture::new();
        let over = format!("per_document: $15 {}", "x".repeat(2000));
        let cases = [
            (
                "fees",
                "per_document: $20",
                "edited value not in citation quote",
            ),
            ("fees", FEE, "statement must be 'key: value'"),
            ("fees", "Fee: $15", "statement must be 'key: value'"),
            ("fees", "fee:$15", "statement must be 'key: value'"),
            (
                "fees",
                "fee: 5",
                "the value after the key must be at least 2 characters",
            ),
            ("fees", "  ", "statement must be 1-2000 chars"),
            ("fees", over.as_str(), "statement must be 1-2000 chars"),
            ("fee", "per_document: $15", "unknown handbook topic"),
            (
                "phone_script",
                "per_document: $15",
                "phone_script stays staff-only: quote into another topic",
            ),
        ];
        for (topic, statement, error) in cases {
            let (status, refused) = f.quote(f.page, selected(PAGE, FEE, topic, statement)).await;
            assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{statement}");
            assert_eq!(refused["error"], error, "{statement}");
        }
        assert_eq!(f.facts(), 0);
        // The value check normalizes like the validator (case, spacing);
        // `field__instance` keys are keys.
        let (status, made) = f
            .quote(
                f.page,
                selected(
                    PAGE,
                    FEE,
                    "fees",
                    "per_document__counter:  $15 PER   document",
                ),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "{made}");
        assert_eq!(
            made["statement"],
            "per_document__counter:  $15 PER   document"
        );
    }

    #[tokio::test]
    async fn statement_already_on_file_is_a_conflict() {
        let f = Fixture::new();
        let (status, made) = f
            .quote(f.page, selected(PAGE, FEE, "fees", "per_document: $15"))
            .await;
        assert_eq!(status, StatusCode::CREATED, "{made}");
        let proposed = made["fact_id"].as_i64().unwrap();
        // Same statement under the shared normalization (spacing).
        let (status, refused) = f
            .quote(f.page, prepared(FEE, "fees", "  per_document:   $15 "))
            .await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(
            refused,
            json!({
                "error": "this statement is already on file from this capture",
                "fact_id": proposed,
                "status": "proposed",
            })
        );
        // An accepted twin is reported first.
        let conn = f.conn();
        conn.execute(
            "INSERT INTO fact(authority_id, capture_id, topic, statement, status,
                              created_by, created_at)
             VALUES (?1, ?2, 'fees', 'per_document: $15', 'accepted', 't', 1)",
            [f.office, f.page],
        )
        .unwrap();
        let accepted = conn.last_insert_rowid();
        let (status, refused) = f
            .quote(f.page, selected(PAGE, FEE, "fees", "per_document: $15"))
            .await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(refused["fact_id"], accepted);
        assert_eq!(refused["status"], "accepted");
        // Rejected facts are not on file; another topic is another slot.
        conn.execute(
            "UPDATE fact SET status = 'rejected' WHERE id IN (?1, ?2)",
            [proposed, accepted],
        )
        .unwrap();
        let (status, made) = f
            .quote(f.page, selected(PAGE, FEE, "fees", "per_document: $15"))
            .await;
        assert_eq!(status, StatusCode::CREATED, "{made}");
        let (status, made) = f
            .quote(f.page, selected(PAGE, FEE, "payment", "per_document: $15"))
            .await;
        assert_eq!(status, StatusCode::CREATED, "{made}");
        assert_eq!(f.facts(), 4);
    }

    #[tokio::test]
    async fn a_quote_or_value_may_not_cut_a_word_or_number() {
        let f = Fixture::new();
        // Prepared: a hit inside a longer token is no occurrence.
        for quote in [
            "The fee is $1",
            "Mail requests to PO Box 4023",
            "he fee is $15 per",
        ] {
            let value = quote.split_whitespace().last().unwrap();
            let (status, refused) = f
                .quote(f.page, prepared(quote, "fees", &format!("fee: {value}")))
                .await;
            assert_eq!(
                status,
                StatusCode::UNPROCESSABLE_ENTITY,
                "{quote}: {refused}"
            );
            assert_eq!(refused["error"], "quote not found in this capture's text");
        }
        // Selected: the same cut says so.
        let (status, refused) = f
            .quote(
                f.page,
                selected(PAGE, "The fee is $1", "fees", "fee: The fee"),
            )
            .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{refused}");
        assert_eq!(
            refused["error"],
            "the quote cuts a word or number in two: select the whole of it"
        );
        // A whole quote with a cut value: the number guard refuses it.
        for statement in ["per_document: $1", "per_document: The fee is $1"] {
            let (status, refused) = f
                .quote(f.page, selected(PAGE, FEE, "fees", statement))
                .await;
            assert_eq!(
                status,
                StatusCode::UNPROCESSABLE_ENTITY,
                "{statement}: {refused}"
            );
            assert_eq!(refused["error"], "edited value not in citation quote");
        }
        let (status, refused) = f
            .quote(
                f.page,
                prepared(
                    "Mail requests to PO Box 40234,",
                    "mailing",
                    "po_box: PO Box 4023",
                ),
            )
            .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{refused}");
        assert_eq!(refused["error"], "edited value not in citation quote");
        assert_eq!(f.facts(), 0);
    }

    #[test]
    fn cuts_token_reads_both_sides_of_the_edge() {
        let cases = [
            ("$150", 3, true),              // "$15" of "$150"
            ("$15.50 fee", 3, true),        // "$15" of "$15.50"
            ("$15.50 fee", 4, true),        // "50 fee" of "$15.50 fee"
            ("$1,000", 2, true),            // "$1" of "$1,000"
            ("Coffee is", 3, true),         // "fee is" of "Coffee is"
            ("$15 fee", 3, false),          // whole token
            ("the fee is $15.", 14, false), // a sentence's period
            ("by check.Office", 9, false),  // glued text
            ("*243 Israel", 1, false),      // after a marker
            ("end.", 4, false),             // end of the text
            ("start", 0, false),            // start of the text
        ];
        for (text, at, cut) in cases {
            assert_eq!(cuts_token(text, at), cut, "{text:?} at {at}");
        }
    }

    #[tokio::test]
    async fn same_statement_from_another_page_is_corroboration() {
        let f = Fixture::new();
        let (status, first) = f
            .quote(f.page, selected(PAGE, FEE, "fees", "per_document: $15"))
            .await;
        assert_eq!(status, StatusCode::CREATED, "{first}");
        // A second official page of the office prints the same fee.
        let other = f.add_source("website_official", "https://www.sos.wa.gov/fees");
        let page2 = f.capture(Some(other), "warc", "official", 300, Some(FEE.as_bytes()));
        let (status, second) = f
            .quote(page2, prepared(FEE, "fees", "per_document: $15"))
            .await;
        assert_eq!(status, StatusCode::CREATED, "{second}");
        assert_ne!(second["fact_id"], first["fact_id"]);
        assert_eq!(second["capture_id"], page2);
        // The same page again is still a duplicate.
        let (status, again) = f
            .quote(page2, prepared(FEE, "fees", "per_document: $15"))
            .await;
        assert_eq!(status, StatusCode::CONFLICT, "{again}");
        assert_eq!(again["fact_id"], second["fact_id"]);
        assert_eq!(f.facts(), 2);
    }

    #[tokio::test]
    async fn rejected_or_textless_captures_are_not_found() {
        let f = Fixture::new();
        let rejected = f.capture(
            Some(f.source),
            "warc",
            "rejected",
            200,
            Some(PAGE.as_bytes()),
        );
        let missing = f.capture(Some(f.source), "warc", "official", 200, None);
        let empty = f.capture(Some(f.source), "warc", "official", 200, Some(b""));
        // Capture 0 and authority 0 are the unattributed sentinels.
        let unattributed = f.capture_of(0, None, "warc", "official", 200, Some(PAGE.as_bytes()));
        for id in [rejected, missing, empty, unattributed, 9_999, 0] {
            let (status, body) = f.text(id).await;
            assert_eq!(status, StatusCode::NOT_FOUND, "{id}");
            assert_eq!(body, json!({"error": "capture not found"}));
            let (status, body) = f
                .quote(id, prepared(FEE, "fees", "per_document: $15"))
                .await;
            assert_eq!(status, StatusCode::NOT_FOUND, "{id}");
            assert_eq!(body, json!({"error": "capture not found"}));
        }
        assert_eq!(f.facts(), 0);
    }

    #[tokio::test]
    async fn quotes_need_a_citable_utf8_capture() {
        let f = Fixture::new();
        // An office email reads fine, but the public read never cites mail.
        let mailbox = f.add_source("email_official", "mailto:apostille@sos.example");
        let email = f.capture(
            Some(mailbox),
            "email",
            "official",
            200,
            Some(PAGE.as_bytes()),
        );
        let (status, body) = f.text(email).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["uri"], "mailto:apostille@sos.example");
        let (status, refused) = f
            .quote(email, selected(PAGE, FEE, "fees", "per_document: $15"))
            .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(
            refused["error"],
            "this capture has no https source page to cite"
        );

        // A source-less page borrows the office website, as the public
        // read does.
        let unsourced = f.capture(None, "warc", "official", 200, Some(PAGE.as_bytes()));
        let (status, body) = f.text(unsourced).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["uri"], SITE);
        let (status, made) = f
            .quote(unsourced, selected(PAGE, FEE, "fees", "per_document: $15"))
            .await;
        assert_eq!(status, StatusCode::CREATED, "{made}");

        // Text that is not UTF-8 is shown lossy but never cited: its
        // offsets would not index the bytes.
        let mut bytes = b"Hours \xff\n".to_vec();
        bytes.extend_from_slice(FEE.as_bytes());
        let broken = f.capture(Some(f.source), "warc", "official", 200, Some(&bytes));
        let (status, body) = f.text(broken).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["text"], format!("Hours \u{fffd}\n{FEE}"));
        let (status, refused) = f
            .quote(broken, prepared(FEE, "fees", "per_document: $16"))
            .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(
            refused["error"],
            "this capture's text is not valid UTF-8, so no span of it can be cited"
        );
        assert_eq!(f.facts(), 1);
    }

    #[tokio::test]
    async fn routes_need_a_staff_session_and_csrf() {
        let f = Fixture::new();
        let quote_uri = format!("/apostille/api/v1/admin/captures/{}/quote", f.page);
        let body = selected(PAGE, FEE, "fees", "per_document: $15");
        // No session: AdminAuth refuses.
        let (status, _) = send(
            f.admin(),
            request("POST", &quote_uri, None, true, Some(&body)),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        // A customer session is no staff session.
        let (status, _) = send(
            f.admin(),
            request("POST", &quote_uri, Some(CUSTOMER), true, Some(&body)),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        // No CSRF pair: the guard refuses before the handler runs.
        let (status, _) = send(
            f.admin(),
            request("POST", &quote_uri, Some(STAFF), false, Some(&body)),
        )
        .await;
        assert!(!status.is_success(), "{status}");
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(f.facts(), 0);
        for uri in [
            format!("/apostille/api/v1/admin/authorities/{SLUG}/captures"),
            format!("/apostille/api/v1/admin/captures/{}/text", f.page),
        ] {
            let (status, _) = send(f.admin(), request("GET", &uri, None, false, None)).await;
            assert_eq!(status, StatusCode::UNAUTHORIZED, "{uri}");
            let (status, _) =
                send(f.admin(), request("GET", &uri, Some(CUSTOMER), false, None)).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{uri}");
            // The public listener never serves them.
            let req = Request::builder()
                .uri(uri.as_str())
                .body(Body::empty())
                .unwrap();
            let (status, _) = send(f.public(), req).await;
            assert_eq!(status, StatusCode::NOT_FOUND, "{uri}");
        }
    }

    #[tokio::test]
    async fn captures_list_is_newest_first_with_text_only() {
        let f = Fixture::new();
        let newer = f.capture(
            Some(f.source),
            "warc",
            "official",
            300,
            Some(b"Newer page."),
        );
        let tie = f.capture(
            Some(f.source),
            "warc",
            "staff_verified",
            300,
            Some(b"Same second, later id."),
        );
        let upload = f.capture(None, "file", "staff_verified", 200, Some(b"Uploaded form."));
        f.capture(Some(f.source), "warc", "rejected", 400, Some(b"Rejected."));
        f.capture(Some(f.source), "warc", "official", 400, None);
        f.capture(Some(f.source), "warc", "official", 400, Some(b""));
        let other = add_office(&f.conn(), "other-sos");
        f.capture_of(
            other,
            None,
            "warc",
            "official",
            500,
            Some(b"Another office."),
        );

        let uri = format!("/apostille/api/v1/admin/authorities/{SLUG}/captures");
        let (status, body) = f.get(&uri).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["slug"], SLUG);
        let rows = body["captures"].as_array().unwrap();
        let ids: Vec<i64> = rows.iter().map(|r| r["id"].as_i64().unwrap()).collect();
        assert_eq!(ids, vec![tie, newer, upload, f.page]);
        assert_eq!(
            rows[3],
            json!({
                "id": f.page,
                "source_id": f.source,
                "uri": SITE,
                "kind": "warc",
                "trust": "official",
                "captured_at": 100,
                "bytes": PAGE.len(),
            })
        );
        // A source-less upload names the page its quotes cite.
        assert_eq!(rows[2]["source_id"], Value::Null);
        assert_eq!(rows[2]["uri"], SITE);
        assert_eq!(rows[2]["bytes"], "Uploaded form.".len());

        for (limit, want) in [("1", vec![tie]), ("0", vec![tie]), ("2", vec![tie, newer])] {
            let (status, body) = f.get(&format!("{uri}?limit={limit}")).await;
            assert_eq!(status, StatusCode::OK, "{body}");
            let ids: Vec<i64> = body["captures"]
                .as_array()
                .unwrap()
                .iter()
                .map(|r| r["id"].as_i64().unwrap())
                .collect();
            assert_eq!(ids, want, "limit={limit}");
        }
        let (status, body) = f
            .get("/apostille/api/v1/admin/authorities/nope/captures")
            .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body, json!({"error": "unknown authority"}));
    }

    #[tokio::test]
    async fn capture_text_serves_the_sidecar_up_to_4_mib() {
        let f = Fixture::new();
        let (status, body) = f.text(f.page).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(
            body,
            json!({
                "id": f.page,
                "slug": SLUG,
                "uri": SITE,
                "kind": "warc",
                "trust": "official",
                "captured_at": 100,
                "text": PAGE,
            })
        );
        let limit = 4 * 1024 * 1024;
        let edge = f.capture(
            Some(f.source),
            "warc",
            "official",
            200,
            Some(&vec![b'a'; limit]),
        );
        let (status, body) = f.text(edge).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["text"].as_str().unwrap().len(), limit);
        let huge = f.capture(
            Some(f.source),
            "warc",
            "official",
            200,
            Some(&vec![b'a'; limit + 1]),
        );
        let (status, body) = f.text(huge).await;
        assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
        assert!(body["error"].as_str().unwrap().contains("4 MiB"), "{body}");
        // Quoting stays inside what staff can see.
        let (status, _) = f
            .quote(huge, prepared("aaaaaaaaaa", "fees", "aa: aaaaaaaa"))
            .await;
        assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
        assert_eq!(f.facts(), 0);
    }

    #[test]
    fn spans_follow_whitespace_runs_and_count_every_start() {
        // Overlapping matches each count.
        assert_eq!(span_of("a a a", "a a", None), Err(Miss::Ambiguous(2)));
        // Tabs, newlines and no-break spaces all separate words.
        let text = "Fee:\t$15\u{a0}per\n\ndocument.";
        assert_eq!(
            span_of(text, "$15 per document.", None),
            Ok((5, text.len()))
        );
        assert_eq!(
            span_of(text, " $15\tper  document. ", None),
            Ok((5, text.len()))
        );
        assert_eq!(span_of(text, "\n \t", None), Err(Miss::Length(0)));
        // A selection is trimmed; whitespace alone trims to nothing.
        assert_eq!(span_of("  ab  ", "  ab  ", Some((0, 6))), Ok((2, 4)));
        assert_eq!(span_of("  ab  ", "  ", Some((0, 2))), Ok((2, 2)));
        assert_eq!(span_of("ab", "ab", Some((0, 3))), Err(Miss::OutOfBounds));
        assert_eq!(span_of("ab", "b", Some((0, 1))), Err(Miss::Stale));
        assert_eq!(span_of("é", "é", Some((1, 2))), Err(Miss::Split));
    }
}
