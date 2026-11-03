//! Fact translation into Spanish and French (SA-20 lane).
//!
//! Accepted facts are translated by Muse Spark **after** staff approval,
//! never before: [`request_after_accept`] queues one pending row per
//! supported language (called from the accept path — builder wires it),
//! [`backfill_accepted`] queues rows for already-accepted facts, and
//! [`translate_one`] renders one pending row through a [`Translator`].
//! The public read ([`public_translations`]) serves only rows whose
//! number/fee/address guard passed; anything the guard rejects stays
//! staff-visible with `guard_ok = 0`.
//!
//! Number/fee/address guard: digits, currency amounts, PO boxes and
//! street addresses are never translated. [`guard_ok`] re-extracts the
//! protected spans from both sides and requires every original span to
//! survive (digits separator-tolerant: `25,000` matches `25 000`).
//!
//! Meta call shape follows `docs/meta-api-shapes.md` (chat completions,
//! `developer` + `user`, `response_format.json_schema`, no `temperature`
//! per F1, `reasoning_effort`, `prompt_cache_key`): see
//! [`translate_body`]. Non-streaming: one fact is a few hundred tokens,
//! well under the sync budget, and there is nothing to render
//! progressively. No live keys in the worktree: tests replay recorded
//! responses through [`parse_translate_response`] and [`StubTranslator`].
//!
//! Rate/cost caps: the public read is per-IP rate-limited
//! ([`READ_PER_MINUTE`]/min); staff-triggered model calls are capped at
//! [`MAX_TRANSLATE_CALLS_PER_DAY`] per process-day plus a per-request
//! backfill ceiling ([`MAX_BACKFILL_PER_CALL`]).
//!
//! New tables arrive ONLY via `CREATE TABLE IF NOT EXISTS` (hard rule:
//! never `ALTER`/`DROP`/`RENAME`, never touch `SCHEMA_VERSION`).
//! [`migrate_translate_tables`] is idempotent and runs beside the
//! existing migrations at boot.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{LazyLock, Mutex};

use axum::{
    Json,
    extract::{ConnectInfo, Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use handbook_store::{db, store::Store};
use rusqlite::{Connection, OptionalExtension as _};
use serde_json::json;

use crate::{AppState, admin::AdminAuth, passkey, store_unavailable, unix_now};

/// Languages served (`?lang=` values, `fact_translation.lang`).
pub const SUPPORTED_LANGS: [&str; 2] = ["es", "fr"];

/// Drafting model for translations (Meta-only hard rule).
pub const TRANSLATE_MODEL: &str = "muse-spark-1.3";

/// Stable cache key for the translation prompt (one shared prefix).
pub const PROMPT_CACHE_KEY: &str = "fact-translate-v1";

/// Public reads per IP per minute (serves stored rows; never calls a model).
pub const READ_PER_MINUTE: u32 = 60;

/// Process-wide staff-triggered model-call budget per UTC day.
pub const MAX_TRANSLATE_CALLS_PER_DAY: u32 = 500;

/// Per-request backfill ceiling (default 20, hard max).
pub const MAX_BACKFILL_PER_CALL: i64 = 50;
const DEFAULT_BACKFILL_LIMIT: i64 = 20;

/// Standing translation rules (the stable `developer` prefix).
const DEVELOPER_RULES: &str = "You translate public handbook facts from English \
    into the target language. Rules: translate the prose faithfully and plainly; \
    NEVER translate, reformat, convert or drop digits, currency amounts, street \
    addresses or PO box numbers — reproduce every number character-for-character, \
    keeping the original digit grouping when possible; keep official office and \
    place names recognizable. Reply with JSON only: \
    {\"translated_statement\": \"...\"}.";

static READ_LIMITS: LazyLock<passkey::RateBuckets> = LazyLock::new(passkey::RateBuckets::default);

/// Process-wide daily model-call budget: `(utc_day, used)`.
static BUDGET: Mutex<(i64, u32)> = Mutex::new((0, 0));

/// Pure budget step (static wraps it): one call allowed per unit under
/// the daily cap; the day rolls over automatically.
fn budget_take_in(state: &mut (i64, u32), now: i64) -> bool {
    let day = now.div_euclid(86_400);
    if state.0 != day {
        state.0 = day;
        state.1 = 0;
    }
    if state.1 >= MAX_TRANSLATE_CALLS_PER_DAY {
        return false;
    }
    state.1 += 1;
    true
}

/// Take one unit of the daily model-call budget. Fail-closed when the
/// lock is poisoned.
fn budget_take(now: i64) -> bool {
    budget_take_in(&mut BUDGET.lock().unwrap_or_else(|e| e.into_inner()), now)
}

/// `?lang=` / `{lang}` validation: `Some("es" | "fr")`, else `None`.
pub fn parse_lang(raw: Option<&str>) -> Option<&'static str> {
    match raw {
        Some("es") => Some("es"),
        Some("fr") => Some("fr"),
        _ => None,
    }
}

/// Create `fact_translation` (and its index) when missing. Idempotent:
/// safe to call on every boot beside [`db::migrate`].
pub fn migrate_translate_tables(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS fact_translation(
           id INTEGER PRIMARY KEY,
           fact_id INTEGER NOT NULL REFERENCES fact(id),
           lang TEXT NOT NULL CHECK(lang IN ('es', 'fr')),
           statement TEXT NOT NULL DEFAULT '',
           model TEXT NOT NULL DEFAULT '',
           created_by TEXT NOT NULL DEFAULT '',
           created_at INTEGER NOT NULL,
           guard_ok INTEGER NOT NULL DEFAULT 0,
           UNIQUE(fact_id, lang)
         );
         CREATE INDEX IF NOT EXISTS idx_fact_translation_lookup
           ON fact_translation(fact_id, lang);",
    )
}

/// Kind of a protected span (digits, currency, addresses are never translated).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProtectedKind {
    /// Bare number (`digits` holds the ASCII digits only).
    Number,
    /// Currency amount (`symbol` is the leading/trailing sign).
    Money(char),
    /// `PO Box <digits>`.
    PoBox,
    /// `<number> <name…> <street suffix>`, kept verbatim.
    Street,
}

/// One protected span: `text` is the verbatim slice, `digits` the ASCII
/// digits it carries (empty for streets, whose names may legitimately
/// contain no digits beyond the house number — kept verbatim instead).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProtectedSpan {
    pub kind: ProtectedKind,
    pub text: String,
    pub digits: String,
}

/// Street suffixes that close an address span (case-insensitive,
/// trailing period allowed).
const STREET_SUFFIXES: [&str; 24] = [
    "street",
    "st",
    "avenue",
    "ave",
    "road",
    "rd",
    "boulevard",
    "blvd",
    "lane",
    "ln",
    "drive",
    "dr",
    "way",
    "place",
    "pl",
    "court",
    "ct",
    "circle",
    "cir",
    "plaza",
    "terrace",
    "highway",
    "hwy",
    "route",
];

/// Separator chars allowed *inside* a number run (only when a digit follows).
fn is_number_sep(c: char) -> bool {
    matches!(c, '.' | ',' | ':' | '/' | '-' | '\u{2013}' | '\u{2014}')
}

fn is_money_symbol(c: char) -> bool {
    matches!(c, '$' | '\u{20AC}' | '\u{A3}')
}

/// Maximal number runs: `(byte_start, byte_end, digits)`.
fn find_numbers(text: &str) -> Vec<(usize, usize, String)> {
    let ch: Vec<(usize, char)> = text.char_indices().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < ch.len() {
        if !ch[i].1.is_ascii_digit() {
            i += 1;
            continue;
        }
        let start = ch[i].0;
        let mut digits = String::new();
        let mut end = start;
        let mut j = i;
        while j < ch.len() {
            let (off, c) = ch[j];
            if c.is_ascii_digit() {
                digits.push(c);
                end = off + c.len_utf8();
                j += 1;
            } else if is_number_sep(c) && ch.get(j + 1).is_some_and(|(_, n)| n.is_ascii_digit()) {
                j += 1;
            } else {
                break;
            }
        }
        out.push((start, end, digits));
        i = j;
    }
    out
}

/// Byte offset just past any single space/tab after `pos`.
fn skip_one_space(text: &str, pos: usize) -> usize {
    text[pos..]
        .char_indices()
        .next()
        .filter(|(_, c)| *c == ' ' || *c == '\t')
        .map(|(off, c)| pos + off + c.len_utf8())
        .unwrap_or(pos)
}

/// Word spans (`[alphanumeric _' -]` runs) with byte offsets.
fn word_spans(text: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut start: Option<usize> = None;
    for (off, c) in text.char_indices() {
        if c.is_alphanumeric() || c == '_' || c == '\'' || c == '-' {
            if start.is_none() {
                start = Some(off);
            }
        } else if let Some(s) = start.take() {
            out.push((s, off));
        }
    }
    if let Some(s) = start.take() {
        out.push((s, text.len()));
    }
    out
}

/// Strip trailing sentence punctuation for suffix/name comparison.
fn clean_word(word: &str) -> &str {
    word.trim_end_matches(['.', ',', ';', ':', '!', '?'])
}

/// `PO Box <number>` spans (whole span, digits).
fn find_po_boxes(text: &str) -> Vec<(usize, usize, String)> {
    let words = word_spans(text);
    let at = |i: usize| words.get(i).map(|(s, e)| &text[*s..*e]);
    let mut out = Vec::new();
    for i in 0..words.len().saturating_sub(2) {
        if !at(i).is_some_and(|w| w.eq_ignore_ascii_case("po")) {
            continue;
        }
        if !at(i + 1).is_some_and(|w| w.eq_ignore_ascii_case("box")) {
            continue;
        }
        let after = words[i + 1].1;
        let num_start = skip_one_space(text, after);
        let rest = &text[num_start..];
        let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        if digits.is_empty() {
            continue;
        }
        out.push((words[i].0, num_start + digits.len(), digits));
    }
    out
}

/// Re-extract every protected span from `text`: currency amounts,
/// PO boxes, street addresses, then remaining bare numbers.
pub fn protected_spans(text: &str) -> Vec<ProtectedSpan> {
    let mut out: Vec<ProtectedSpan> = Vec::new();
    let mut used: Vec<(usize, usize)> = Vec::new();
    let overlaps =
        |s: usize, e: usize, used: &[(usize, usize)]| used.iter().any(|(a, b)| s < *b && *a < e);

    for (s, e, digits) in find_po_boxes(text) {
        out.push(ProtectedSpan {
            kind: ProtectedKind::PoBox,
            text: text[s..e].to_string(),
            digits,
        });
        used.push((s, e));
    }

    let numbers = find_numbers(text);
    // Money: a currency symbol adjacent to a number (spaces between the
    // symbol and the digits are allowed: `$25`, `25 €`).
    let mut money_nums: HashMap<usize, char> = HashMap::new();
    for (idx, (s, e, _)) in numbers.iter().enumerate() {
        if let Some(sym) = adjacent_symbol_before(text, *s)
            && !overlaps(*s, *e, &used)
        {
            money_nums.insert(idx, sym);
            continue;
        }
        if let Some(sym) = adjacent_symbol_after(text, *e)
            && !overlaps(*s, *e, &used)
        {
            money_nums.insert(idx, sym);
        }
    }
    for (idx, (s, e, digits)) in numbers.iter().enumerate() {
        if let Some(sym) = money_nums.get(&idx) {
            out.push(ProtectedSpan {
                kind: ProtectedKind::Money(*sym),
                text: text[*s..*e].to_string(),
                digits: digits.clone(),
            });
            used.push((*s, *e));
        }
    }

    // Streets: `<number> <name…> <suffix>` — at least one name word plus
    // a suffix word, first name word capitalized.
    let words = word_spans(text);
    for (idx, (s, _, digits)) in numbers.iter().enumerate() {
        if money_nums.contains_key(&idx) || overlaps(*s, *s + 1, &used) {
            continue;
        }
        let after: Vec<(usize, usize)> = words
            .iter()
            .copied()
            .filter(|(ws, _)| *ws >= numbers[idx].1)
            .take(4)
            .collect();
        if after.len() < 2 {
            continue;
        }
        // The span ends at the first suffix word within the window, and
        // never crosses sentence punctuation (review #19: "100 Fixture
        // Way, Olympia" used to fall back to digits only).
        let mut stop = None;
        for k in 1..after.len() {
            let prev = &text[after[k - 1].0..after[k - 1].1];
            if prev.ends_with([',', ';', '.']) {
                break;
            }
            let w = clean_word(&text[after[k].0..after[k].1]);
            if STREET_SUFFIXES.contains(&w.to_lowercase().as_str()) {
                stop = Some(k);
                break;
            }
        }
        let Some(stop) = stop else { continue };
        let after: Vec<(usize, usize)> = after[..=stop].to_vec();
        let first = clean_word(&text[after[0].0..after[0].1]);
        let first_ok = first
            .chars()
            .next()
            .is_some_and(|c| c.is_uppercase() || c.is_ascii_digit());
        if !first_ok {
            continue;
        }
        let end = after[after.len() - 1].1;
        // Swallow a trailing period ("Main St.").
        let end = if text[end..].starts_with('.') {
            end + 1
        } else {
            end
        };
        out.push(ProtectedSpan {
            kind: ProtectedKind::Street,
            text: text[*s..end].to_string(),
            digits: digits.clone(),
        });
        used.push((*s, end));
    }

    for (idx, (s, e, digits)) in numbers.iter().enumerate() {
        if money_nums.contains_key(&idx) || overlaps(*s, *e, &used) {
            continue;
        }
        out.push(ProtectedSpan {
            kind: ProtectedKind::Number,
            text: text[*s..*e].to_string(),
            digits: digits.clone(),
        });
        used.push((*s, *e));
    }
    out.sort_by_key(|span| text.find(&span.text).unwrap_or(usize::MAX));
    out
}

/// Money-symbol adjacency: first non-blank char before `pos`
/// (spaces between symbol and digits allowed: `$25`, `25 €`).
fn adjacent_symbol_before(text: &str, pos: usize) -> Option<char> {
    let sym = text[..pos]
        .chars()
        .rev()
        .find(|c| *c != ' ' && *c != '\t')?;
    is_money_symbol(sym).then_some(sym)
}

/// Money-symbol adjacency: first non-blank char at/after `pos`.
fn adjacent_symbol_after(text: &str, pos: usize) -> Option<char> {
    let sym = text[pos..].chars().find(|c| *c != ' ' && *c != '\t')?;
    is_money_symbol(sym).then_some(sym)
}

/// Whole number tokens of `hay` for the guards: the maximal runs of
/// [`find_numbers`], with a space / NBSP / narrow NBSP joining two runs
/// only as a thousands group (`25 000` reads as `25000`). Returns
/// `(byte_start, byte_end, digits)`.
fn number_tokens(hay: &str) -> Vec<(usize, usize, String)> {
    let runs = find_numbers(hay);
    let mut out: Vec<(usize, usize, String)> = Vec::new();
    for (s, e, d) in runs {
        if let Some(last) = out.last_mut() {
            let gap = &hay[last.1..s];
            let grouping = matches!(gap, " " | "\u{A0}" | "\u{202F}") && d.len() == 3;
            if grouping {
                last.1 = e;
                last.2.push_str(&d);
                continue;
            }
        }
        out.push((s, e, d));
    }
    out
}

/// Whole number tokens of `hay`, plus the five-digit head of a ZIP+4
/// (`98504-0228` grounds `98504`).
fn plain_numbers(hay: &str) -> Vec<String> {
    let mut out = Vec::new();
    for (s, e, d) in number_tokens(hay) {
        let raw = &hay[s..e];
        let zip4 = raw.len() == 10
            && raw.as_bytes()[5] == b'-'
            && raw
                .bytes()
                .enumerate()
                .all(|(i, b)| i == 5 || b.is_ascii_digit());
        if zip4 {
            out.push(raw[..5].to_string());
        }
        out.push(d);
    }
    out
}

/// True when `hay` holds a whole number token with exactly `digits` —
/// `15` never matches inside `150`, nor `4022` inside `40228` (review
/// P338 #4: the old substring scan accepted both) — or a ZIP+4 whose
/// head is `digits`.
fn contains_digit_run(hay: &str, digits: &str) -> bool {
    digits.is_empty() || plain_numbers(hay).iter().any(|d| d == digits)
}

/// True when `hay` holds `digits` as a whole token next to `sym`
/// (`$15`, `15 $`, `US$ 15`).
fn contains_money(hay: &str, sym: char, digits: &str) -> bool {
    number_tokens(hay).iter().any(|(s, e, d)| {
        d == digits
            && (adjacent_symbol_before(hay, *s) == Some(sym)
                || adjacent_symbol_after(hay, *e) == Some(sym))
    })
}

/// Every `$` amount written in `text`, in cents (`$150` → 15000,
/// `$15.00` → 1500, `$1,500` → 150000, `15,00 $` → 1500). A trailing
/// `.d`/`.dd` or `,d`/`,dd` is a decimal; any other `.`/`,` is grouping.
pub fn dollar_amounts_cents(text: &str) -> Vec<i64> {
    number_tokens(text)
        .into_iter()
        .filter(|(s, e, _)| {
            adjacent_symbol_before(text, *s) == Some('$')
                || adjacent_symbol_after(text, *e) == Some('$')
        })
        .filter_map(|(s, e, _)| cents_of(&text[s..e]))
        .collect()
}

/// One written amount in cents (`15.00`, `1,500`, `15,00`); `None` for
/// non-amounts or overflow.
fn cents_of(raw: &str) -> Option<i64> {
    let raw: String = raw
        .chars()
        .filter(|c| c.is_ascii_digit() || matches!(c, '.' | ','))
        .collect();
    let (whole, cents) = match raw.rsplit_once(['.', ',']) {
        Some((w, f)) if !f.is_empty() && f.len() <= 2 && f.bytes().all(|b| b.is_ascii_digit()) => {
            (w.to_string(), format!("{f:0<2}"))
        }
        _ => (raw.clone(), "00".to_string()),
    };
    let whole: String = whole.chars().filter(char::is_ascii_digit).collect();
    let (w, c) = (whole.parse::<i64>().ok()?, cents.parse::<i64>().ok()?);
    w.checked_mul(100)?.checked_add(c)
}

/// Re-extraction guard: every protected span of `original` must survive
/// in `translated`. Numbers and currency match on digits
/// (separator-tolerant) plus the currency symbol; PO boxes need the
/// `po box` marker plus digits; street addresses must appear verbatim.
pub fn guard_ok(original: &str, translated: &str) -> bool {
    let lower = translated.to_lowercase();
    protected_spans(original)
        .into_iter()
        .all(|span| match span.kind {
            // Whole tokens only (as production since P340): equivalent
            // time formats (8 AM ~ 8:00 a.m.) fail closed until a
            // context-aware time matcher lands — three verify rounds of a
            // clock matcher found times grounding in the wrong half of
            // the day or against count ranges.
            ProtectedKind::Number => contains_digit_run(translated, &span.digits),
            // Dollars compare by value (`$1,500` never grounds `$15.00`,
            // `15,00 $` grounds `$15.00`); other symbols by digits.
            ProtectedKind::Money('$') => match cents_of(&span.text) {
                Some(cents) => dollar_amounts_cents(translated).contains(&cents),
                None => contains_money(translated, '$', &span.digits),
            },
            ProtectedKind::Money(sym) => contains_money(translated, sym, &span.digits),
            ProtectedKind::PoBox => {
                lower.contains("po box") && contains_digit_run(translated, &span.digits)
            }
            ProtectedKind::Street => {
                let norm = |t: &str| {
                    t.split_whitespace()
                        .collect::<Vec<_>>()
                        .join(" ")
                        .to_lowercase()
                };
                norm(translated).contains(&norm(span.text.trim_end_matches('.')))
            }
        })
}

/// Chat-completions request body for one fact translation. Shape per
/// `docs/meta-api-shapes.md`: `developer` (stable rules, cacheable
/// prefix) + `user` (per-run values), `response_format.json_schema`,
/// `reasoning_effort`, `max_completion_tokens`, `prompt_cache_key`.
/// No `temperature`: Spark is tuned at its defaults (F1).
pub fn translate_body(lang: &str, statement: &str) -> serde_json::Value {
    json!({
        "model": TRANSLATE_MODEL,
        "messages": [
            {"role": "developer", "content": DEVELOPER_RULES},
            {"role": "user", "content": format!(
                "Target language: {lang}\nEnglish fact statement:\n{statement}"
            )},
        ],
        "reasoning_effort": "low",
        "max_completion_tokens": 2000,
        "response_format": {
            "type": "json_schema",
            "json_schema": {
                "name": "TranslatedFact",
                "schema": {
                    "type": "object",
                    "properties": {
                        "translated_statement": {"type": "string", "minLength": 1},
                    },
                    "required": ["translated_statement"],
                    "additionalProperties": false,
                },
            },
        },
        "prompt_cache_key": PROMPT_CACHE_KEY,
    })
}

/// Translation failure modes (terminal vs retryable is the caller's call).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TranslateError {
    /// Key missing/unusable — fail closed, never the stub.
    Auth(String),
    /// Transport or non-2xx status (short phrase only: bodies can echo
    /// request data, which holds fact text — never the key).
    Transport(String),
    /// 2xx body outside the translated-statement shape.
    BadOutput(String),
    /// Model refused or the reply truncated (`finish_reason`).
    Refused(String),
}

impl std::fmt::Display for TranslateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Auth(msg) => write!(f, "auth: {msg}"),
            Self::Transport(msg) => write!(f, "transport: {msg}"),
            Self::BadOutput(msg) => write!(f, "bad output: {msg}"),
            Self::Refused(msg) => write!(f, "refused: {msg}"),
        }
    }
}

/// Parse a chat-completion response body into the translated statement.
/// Checks `finish_reason`/`refusal` first (F10/F11): truncation and
/// refusals are `Refused`, never parsed as content.
pub fn parse_translate_response(body: &[u8]) -> Result<String, TranslateError> {
    let value: serde_json::Value = serde_json::from_slice(body)
        .map_err(|e| TranslateError::BadOutput(format!("response is not JSON: {e}")))?;
    if let Some(err) = value.get("error") {
        let msg = err
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or("unknown");
        let short: String = msg.chars().take(120).collect();
        return Err(TranslateError::Transport(format!(
            "meta api error: {}",
            short.trim()
        )));
    }
    let choice = value
        .get("choices")
        .and_then(|c| c.as_array())
        .and_then(|c| c.first())
        .ok_or_else(|| TranslateError::BadOutput("response: want choices[0]".to_string()))?;
    if let Some(reason) = choice.get("finish_reason").and_then(|r| r.as_str()) {
        if reason == "length" {
            return Err(TranslateError::Refused(
                "truncated (finish_reason = length)".to_string(),
            ));
        }
        if reason == "refusal" || reason == "content_filter" {
            return Err(TranslateError::Refused(format!("model refused ({reason})")));
        }
    }
    let message = choice.get("message").ok_or_else(|| {
        TranslateError::BadOutput("response: want choices[0].message".to_string())
    })?;
    if message.get("refusal").is_some_and(|r| !r.is_null()) {
        return Err(TranslateError::Refused(
            "model refused (message.refusal)".to_string(),
        ));
    }
    let content = message
        .get("content")
        .and_then(|c| c.as_str())
        .ok_or_else(|| {
            TranslateError::BadOutput("response: want message.content string".to_string())
        })?;
    let parsed: serde_json::Value = serde_json::from_str(content).map_err(|e| {
        TranslateError::BadOutput(format!("content is not TranslatedFact JSON: {e}"))
    })?;
    let statement = parsed
        .get("translated_statement")
        .and_then(|s| s.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            TranslateError::BadOutput("response: want translated_statement".to_string())
        })?;
    Ok(statement.to_string())
}

/// One rendered translation plus the model that wrote it.
#[derive(Debug, Clone)]
pub struct TranslateOutcome {
    pub translated: String,
    pub model: String,
}

/// Model backend for [`translate_one`]: live Meta client in prod,
/// [`StubTranslator`] in tests.
pub trait Translator {
    fn translate(&self, statement: &str, lang: &str) -> Result<TranslateOutcome, TranslateError>;
}

/// Deterministic test double: wraps the statement with a language tag
/// while keeping every protected span byte-identical, so the guard
/// passes. Synthetic only — never served publicly.
#[cfg(test)]
#[derive(Debug, Clone, Default)]
pub struct StubTranslator;

#[cfg(test)]
impl Translator for StubTranslator {
    fn translate(&self, statement: &str, lang: &str) -> Result<TranslateOutcome, TranslateError> {
        Ok(TranslateOutcome {
            translated: format!("[{lang}] {statement}"),
            model: "stub".to_string(),
        })
    }
}

/// Live Meta chat-completions client for translations (blocking ureq,
/// voice/draft precedent). Auth: Model API key only.
#[derive(Clone)]
pub struct MetaTranslateClient {
    agent: ureq::Agent,
    base_url: String,
    api_key: String,
}

impl MetaTranslateClient {
    /// Build a client; an empty key fails closed at call time.
    pub fn new(base_url: String, api_key: String) -> Self {
        let provider = rustls::crypto::aws_lc_rs::default_provider();
        let _ = rustls::crypto::CryptoProvider::install_default(provider);
        let config = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_global(Some(std::time::Duration::from_secs(90)))
            .build();
        Self {
            agent: ureq::Agent::new_with_config(config),
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key,
        }
    }
}

impl std::fmt::Debug for MetaTranslateClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MetaTranslateClient")
            .field("base_url", &self.base_url)
            .field("api_key", &"<redacted>")
            .finish()
    }
}

impl Translator for MetaTranslateClient {
    fn translate(&self, statement: &str, lang: &str) -> Result<TranslateOutcome, TranslateError> {
        if self.api_key.is_empty() {
            return Err(TranslateError::Auth(
                "meta api key not configured".to_string(),
            ));
        }
        let url = format!("{}/chat/completions", self.base_url);
        let response = self
            .agent
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .send_json(translate_body(lang, statement))
            .map_err(|e| TranslateError::Transport(e.to_string()))?;
        let status = response.status().as_u16();
        let body = response
            .into_body()
            .read_to_vec()
            .map_err(|e| TranslateError::Transport(e.to_string()))?;
        if status == 401 {
            return Err(TranslateError::Auth("meta api: unauthorized".to_string()));
        }
        if !(200..300).contains(&status) {
            let text = String::from_utf8_lossy(&body);
            let short: String = text.chars().take(120).collect();
            return Err(TranslateError::Transport(format!(
                "meta api status {status}: {}",
                short.trim()
            )));
        }
        Ok(TranslateOutcome {
            translated: parse_translate_response(&body)?,
            model: TRANSLATE_MODEL.to_string(),
        })
    }
}

/// Live client: `$META_API_KEY` first (staging one-shots), else the
/// admin-sealed `meta.api_key` (draft.rs `live_client` precedent).
/// Fails closed — never the stub.
fn live_client(
    conn: &rusqlite::Connection,
    kek: Option<&config::Kek>,
) -> Result<MetaTranslateClient, String> {
    if let Ok(key) = std::env::var("META_API_KEY")
        && !key.is_empty()
    {
        return Ok(MetaTranslateClient::new(
            handbook_agent::model::META_BASE_URL.to_string(),
            key,
        ));
    }
    let Some(kek) = kek else {
        return Err("meta api key not configured".to_string());
    };
    let (cfg, _warnings) =
        config::RuntimeConfig::load(conn, kek).map_err(|e| format!("config: {e}"))?;
    let Some(key) = cfg.meta_api_key else {
        return Err("meta api key not configured".to_string());
    };
    if key.as_str().is_empty() {
        return Err("meta api key not configured".to_string());
    }
    let base = if cfg.meta_base_url.is_empty() {
        handbook_agent::model::META_BASE_URL.to_string()
    } else {
        cfg.meta_base_url
    };
    Ok(MetaTranslateClient::new(base, key.as_str().to_string()))
}

/// One stored translation row.
#[derive(Debug, Clone, serde::Serialize)]
pub struct TranslationView {
    pub fact_id: i64,
    pub lang: String,
    pub statement: String,
    pub model: String,
    pub guard_ok: bool,
    pub created_at: i64,
}

/// Queue pending rows for every supported language after an accept.
/// Idempotent (`INSERT OR IGNORE`): safe to call once per accept.
pub fn request_after_accept(conn: &Connection, fact_id: i64, now: i64) -> rusqlite::Result<usize> {
    let mut queued = 0;
    for lang in SUPPORTED_LANGS {
        queued += conn.execute(
            "INSERT OR IGNORE INTO fact_translation(fact_id, lang, created_at)
             VALUES (?1, ?2, ?3)",
            rusqlite::params![fact_id, lang, now],
        )?;
    }
    Ok(queued)
}

/// Queue pending rows for every accepted fact still missing a
/// translation row. Returns the number of rows queued.
pub fn backfill_accepted(
    conn: &Connection,
    lang: Option<&str>,
    now: i64,
) -> rusqlite::Result<usize> {
    let langs: Vec<&str> = match lang {
        Some(l) => {
            if !SUPPORTED_LANGS.contains(&l) {
                return Ok(0);
            }
            vec![l]
        }
        None => SUPPORTED_LANGS.to_vec(),
    };
    let ids: Vec<i64> = conn
        .prepare("SELECT id FROM fact WHERE status = 'accepted'")?
        .query_map([], |row| row.get(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut queued = 0;
    for id in ids {
        for l in &langs {
            queued += conn.execute(
                "INSERT OR IGNORE INTO fact_translation(fact_id, lang, created_at)
                 VALUES (?1, ?2, ?3)",
                rusqlite::params![id, l, now],
            )?;
        }
    }
    Ok(queued)
}

/// Translate one pending row synchronously (open a fresh connection:
/// `Connection` is `!Send`, so this runs inside `spawn_blocking`).
/// Only accepted facts translate; anything else is a conflict-style
/// refusal. Guard failures store with `guard_ok = 0` and are never served.
pub fn translate_one(
    store_root: &std::path::Path,
    fact_id: i64,
    lang: &str,
    translator: &dyn Translator,
    now: i64,
    actor: &str,
) -> Result<TranslationView, TranslateError> {
    let store = Store::new(store_root);
    let conn = db::open(&store).map_err(|e| TranslateError::Transport(e.to_string()))?;
    migrate_translate_tables(&conn).map_err(|e| TranslateError::Transport(e.to_string()))?;
    let (statement, status): (String, String) = conn
        .query_row(
            "SELECT statement, status FROM fact WHERE id = ?1",
            [fact_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|e| TranslateError::Transport(e.to_string()))?
        .ok_or_else(|| TranslateError::BadOutput("fact not found".to_string()))?;
    if status != "accepted" {
        return Err(TranslateError::BadOutput(
            "fact is not accepted".to_string(),
        ));
    }
    let outcome = translator.translate(&statement, lang)?;
    let ok = guard_ok(&statement, &outcome.translated);
    conn.execute(
        "INSERT INTO fact_translation(fact_id, lang, statement, model, created_by, created_at, guard_ok)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(fact_id, lang) DO UPDATE SET
           statement = excluded.statement, model = excluded.model,
           created_by = excluded.created_by, created_at = excluded.created_at,
           guard_ok = excluded.guard_ok",
        rusqlite::params![
            fact_id,
            lang,
            outcome.translated,
            outcome.model,
            actor,
            now,
            i64::from(ok),
        ],
    )
    .map_err(|e| TranslateError::Transport(e.to_string()))?;
    Ok(TranslationView {
        fact_id,
        lang: lang.to_string(),
        statement: outcome.translated,
        model: outcome.model,
        guard_ok: ok,
        created_at: now,
    })
}

/// Guard-passing translations for one authority slug + language.
/// Serves the public `?lang` dossier: anonymous, rate-limited, model-free.
fn translations_for_slug(
    conn: &Connection,
    slug: &str,
    lang: &str,
) -> rusqlite::Result<Option<Vec<TranslationView>>> {
    let authority_id: Option<i64> = conn
        .query_row("SELECT id FROM authority WHERE slug = ?1", [slug], |row| {
            row.get(0)
        })
        .optional()?;
    let Some(authority_id) = authority_id else {
        return Ok(None);
    };
    let mut stmt = conn.prepare(
        "SELECT t.fact_id, t.statement, t.model, t.created_at
         FROM fact_translation t JOIN fact f ON f.id = t.fact_id
         WHERE f.authority_id = ?1 AND t.lang = ?2
           AND t.guard_ok = 1 AND t.statement != ''
         ORDER BY t.fact_id LIMIT 200",
    )?;
    let rows = stmt
        .query_map(rusqlite::params![authority_id, lang], |row| {
            Ok(TranslationView {
                fact_id: row.get(0)?,
                lang: lang.to_string(),
                statement: row.get(1)?,
                model: row.get(2)?,
                guard_ok: true,
                created_at: row.get(3)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(Some(rows))
}

fn open(state: &AppState) -> Option<Connection> {
    db::open(&Store::new(&state.store_root)).ok()
}

#[derive(Debug, serde::Deserialize)]
pub struct LangParams {
    pub lang: Option<String>,
}

/// Public machine translations for one dossier:
/// `GET /api/v1/authorities/{slug}/translations?lang=es`.
/// Anonymous and rate-limited; serves stored guard-passing rows only —
/// this endpoint never calls a model (zero marginal cost).
pub async fn public_translations(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Path(slug): Path<String>,
    Query(params): Query<LangParams>,
) -> Response {
    let Some(lang) = parse_lang(params.lang.as_deref()) else {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"error": "lang must be es|fr"})),
        )
            .into_response();
    };
    let ip = passkey::client_ip(addr);
    if !READ_LIMITS.check(&format!("tr:{ip}"), unix_now(), READ_PER_MINUTE, 60) {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            Json(json!({"error": "too many translation reads — wait a minute"})),
        )
            .into_response();
    }
    let Some(conn) = open(&state) else {
        return store_unavailable();
    };
    migrate_translate_tables(&conn).ok();
    match translations_for_slug(&conn, &slug, lang) {
        Ok(Some(rows)) => {
            let count = rows.len();
            (
                StatusCode::OK,
                Json(
                    json!({"authority": slug, "lang": lang, "translations": rows, "count": count}),
                ),
            )
                .into_response()
        }
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "unknown authority"})),
        )
            .into_response(),
        Err(_) => store_unavailable(),
    }
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TranslateFactBody {
    pub lang: Option<String>,
}

/// Staff: translate one accepted fact now (`POST /facts/{id}/translate`).
/// Synchronous model call inside `spawn_blocking`; capped by the daily
/// budget. Guard failures store with `guard_ok = 0` (200 + body, never
/// served publicly).
pub async fn translate_fact(
    _auth: AdminAuth,
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(body): Json<TranslateFactBody>,
) -> Response {
    let lang = parse_lang(body.lang.as_deref()).unwrap_or("es");
    let now = unix_now();
    if !budget_take(now) {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            Json(json!({"error": "daily translation budget spent"})),
        )
            .into_response();
    }
    let Some(conn) = open(&state) else {
        return store_unavailable();
    };
    if migrate_translate_tables(&conn).is_err() {
        return store_unavailable();
    }
    let client = match live_client(&conn, state.kek.as_deref()) {
        Ok(client) => client,
        Err(msg) => {
            return (StatusCode::SERVICE_UNAVAILABLE, Json(json!({"error": msg}))).into_response();
        }
    };
    drop(conn);
    let store_root = state.store_root.clone();
    let lang_owned = lang.to_string();
    let outcome = tokio::task::spawn_blocking(move || {
        translate_one(&store_root, id, &lang_owned, &client, now, "staff")
    })
    .await;
    match outcome {
        Ok(Ok(view)) => (StatusCode::OK, Json(json!(view))).into_response(),
        Ok(Err(TranslateError::BadOutput(msg))) if msg == "fact not found" => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "fact not found"})),
        )
            .into_response(),
        Ok(Err(TranslateError::BadOutput(msg))) if msg == "fact is not accepted" => (
            StatusCode::CONFLICT,
            Json(json!({"error": "fact is not accepted"})),
        )
            .into_response(),
        Ok(Err(e)) => (
            StatusCode::BAD_GATEWAY,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
        Err(_) => store_unavailable(),
    }
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BackfillBody {
    pub lang: Option<String>,
    pub limit: Option<i64>,
}

/// Staff: queue + translate accepted facts missing a translation
/// (`POST /translations/backfill`). Queues everything missing, renders
/// at most `limit` (default 20, max 50) oldest-first, each call drawing
/// from the daily budget.
pub async fn backfill_translations(
    _auth: AdminAuth,
    State(state): State<AppState>,
    Json(body): Json<BackfillBody>,
) -> Response {
    if let Some(l) = body.lang.as_deref()
        && parse_lang(Some(l)).is_none()
    {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"error": "lang must be es|fr"})),
        )
            .into_response();
    }
    let limit = body
        .limit
        .unwrap_or(DEFAULT_BACKFILL_LIMIT)
        .clamp(1, MAX_BACKFILL_PER_CALL);
    let now = unix_now();
    let Some(conn) = open(&state) else {
        return store_unavailable();
    };
    if migrate_translate_tables(&conn).is_err() {
        return store_unavailable();
    }
    let lang_filter = body.lang.clone();
    let queued = backfill_accepted(&conn, lang_filter.as_deref(), now).unwrap_or(0);
    let pending: Vec<(i64, String)> = {
        conn.prepare(
            "SELECT fact_id, lang FROM fact_translation
             WHERE statement = '' ORDER BY fact_id LIMIT ?1",
        )
        .and_then(|mut stmt| {
            stmt.query_map([limit], |row| Ok((row.get(0)?, row.get(1)?)))?
                .collect::<rusqlite::Result<Vec<_>>>()
        })
        .unwrap_or_default()
    };
    let client = match live_client(&conn, state.kek.as_deref()) {
        Ok(client) => client,
        Err(msg) => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({"error": msg, "queued": queued})),
            )
                .into_response();
        }
    };
    drop(conn);
    let store_root = state.store_root.clone();
    let rendered = tokio::task::spawn_blocking(move || {
        let mut done = 0;
        let mut guard_failed = 0;
        for (fact_id, lang) in pending {
            if !budget_take(unix_now()) {
                break;
            }
            if let Ok(view) = translate_one(&store_root, fact_id, &lang, &client, now, "staff") {
                done += 1;
                if !view.guard_ok {
                    guard_failed += 1;
                }
            }
        }
        (done, guard_failed)
    })
    .await
    .unwrap_or((0, 0));
    (
        StatusCode::OK,
        Json(json!({"queued": queued, "rendered": rendered.0, "guard_failed": rendered.1})),
    )
        .into_response()
}

/// Staff: translation coverage (`GET /translations/status?lang=&fact_id=`).
#[derive(Debug, serde::Deserialize)]
pub struct StatusParams {
    pub lang: Option<String>,
    pub fact_id: Option<i64>,
}

pub async fn translation_status(
    _auth: AdminAuth,
    State(state): State<AppState>,
    Query(params): Query<StatusParams>,
) -> Response {
    if let Some(l) = params.lang.as_deref()
        && parse_lang(Some(l)).is_none()
    {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"error": "lang must be es|fr"})),
        )
            .into_response();
    }
    let Some(conn) = open(&state) else {
        return store_unavailable();
    };
    if migrate_translate_tables(&conn).is_err() {
        return store_unavailable();
    }
    let mut sql = "SELECT fact_id, lang, statement, model, guard_ok, created_at
                   FROM fact_translation WHERE 1 = 1"
        .to_string();
    if params.lang.is_some() {
        sql.push_str(" AND lang = ?1");
    }
    if params.fact_id.is_some() {
        sql.push_str(" AND fact_id = ?2");
    }
    sql.push_str(" ORDER BY fact_id, lang LIMIT 200");
    let rows: Vec<TranslationView> = (|| {
        let mut stmt = conn.prepare(&sql)?;
        let map = |row: &rusqlite::Row<'_>| {
            Ok(TranslationView {
                fact_id: row.get(0)?,
                lang: row.get(1)?,
                statement: row.get(2)?,
                model: row.get(3)?,
                guard_ok: row.get::<_, i64>(4)? != 0,
                created_at: row.get(5)?,
            })
        };
        match (params.lang.as_deref(), params.fact_id) {
            (Some(l), Some(id)) => stmt
                .query_map(rusqlite::params![l, id], map)?
                .collect::<rusqlite::Result<Vec<_>>>(),
            (Some(l), None) => stmt
                .query_map(rusqlite::params![l], map)?
                .collect::<rusqlite::Result<Vec<_>>>(),
            (None, Some(id)) => stmt
                .query_map(rusqlite::params![id], map)?
                .collect::<rusqlite::Result<Vec<_>>>(),
            (None, None) => stmt
                .query_map([], map)?
                .collect::<rusqlite::Result<Vec<_>>>(),
        }
    })()
    .unwrap_or_default();
    let count = rows.len();
    (
        StatusCode::OK,
        Json(json!({"translations": rows, "count": count})),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn street_spans_end_at_the_suffix_and_are_checked() {
        assert!(guard_ok(
            "Visit 100 Fixture Way, Olympia.",
            "Walk-in service is at 100 Fixture Way, Olympia, WA 98501."
        ));
        assert!(!guard_ok(
            "Visit 100 Invented Way, Olympia.",
            "Walk-in service is at 100 Fixture Way, Olympia, WA 98501."
        ));
    }

    #[test]
    fn numbers_match_whole_tokens_only() {
        assert!(contains_digit_run(
            "You pay $150 for same-day service",
            "150"
        ));
        assert!(
            !contains_digit_run("You pay $150 for same-day service", "15"),
            "15 is not 150"
        );
        assert!(!contains_digit_run("Mail to PO Box 40228", "4022"));
        assert!(contains_digit_run("open 8:00 AM to 4:30 PM", "800"));
        assert!(
            !contains_digit_run("open 8:00 AM to 4:30 PM", "8"),
            "8 is not 8:00 (clock times match only through guard_ok)"
        );
        assert!(
            contains_digit_run("25 000 personas", "25000"),
            "space thousands still join"
        );
        assert!(contains_money("cuesta 15 $ por documento", '$', "15"));
        assert!(
            !contains_money("$150 fee; 15 days", '$', "15"),
            "the 15 here is not money"
        );
        assert!(guard_ok("The fee is $15.", "La tarifa es de $15."));
        assert!(!guard_ok("The fee is $15.", "La tarifa es de $150."));
    }

    #[test]
    fn zip_plus_four_grounds_its_head_and_times_match_as_written() {
        // Review P346 #3: the ZIP+4 head grounds the 5-digit ZIP.
        assert!(guard_ok(
            "Olympia, WA 98504",
            "PO Box 40228, Olympia, WA 98504-0228"
        ));
        // Times ground when written the same way (whole tokens).
        assert!(guard_ok(
            "Open 8:00 AM to 4:30 PM",
            "Open Monday through Friday, 8:00 a.m. to 4:30 p.m."
        ));
        assert!(guard_ok("Mon-Fri 9-5", "Phones Mon-Fri 9-5"));
        // Equivalent formats fail closed for now (a block or translation
        // is dropped, never wrong): three verify rounds of a clock
        // matcher grounded times in the wrong half of the day.
        assert!(!guard_ok("Open 8 AM to 5 PM", "8:00 a.m. to 5:00 p.m."));
        assert!(!guard_ok(
            "Walk-ins close at 4:30 PM.",
            "Cierran a las 16:30."
        ));
        // Still refused: a different half hour, a different box, a longer number.
        assert!(!guard_ok("8:00 AM to 4 PM", "8:00-4:30"));
        assert!(!guard_ok("PO Box 4022", "PO Box 40228"));
        assert!(!guard_ok("Fee $15", "Fee $150"));
    }

    #[test]
    fn counts_never_ground_against_times_dates_or_phones() {
        // Re-review of P349 #0: a bare count is not an on-the-hour time.
        assert!(!guard_ok(
            "Get it back in 8 days.",
            "Open 8:00 AM to 4:30 PM"
        ));
        assert!(!guard_ok("Bring 8 copies.", "Open 8:00 a.m."));
        assert!(!guard_ok(
            "Mail it to PO Box 8.",
            "PO Box 4000; open 8:00 AM"
        ));
        assert!(!guard_ok(
            "Closes at 12:00 on Fridays.",
            "Limit 12 documents per request."
        ));
        assert!(!guard_ok(
            "Allow 3 business days. Open 8:00 a.m. to 3:00 p.m.",
            "Prevea 5 días hábiles. Abierto de 8:00 a 3:00."
        ));
        assert!(!guard_ok(
            "Allow 10 business days. Open 7:00 a.m.",
            "Prevea 7 días hábiles. Abierto a las 10:00."
        ));
        // #1: dates, phone numbers and count ranges are not split.
        assert!(!guard_ok(
            "Allow 7 business days.",
            "As of 7/1/2024 requests take 10 business days."
        ));
        assert!(!guard_ok(
            "Allow 1 business day.",
            "Effective 1/1/2025 the fee is $15."
        ));
        assert!(!guard_ok("Processing takes 725 days", "Call 360-725-0377"));
        assert!(!guard_ok(
            "Allow 5 business days.",
            "Hours 9-5, processing 15 business days"
        ));
        assert!(!guard_ok(
            "Processing takes 3 business days.",
            "Processing takes 3-5 business days."
        ));
        // Re-review of P351 #0: a count range followed by its unit is no
        // hour range.
        assert!(!guard_ok(
            "Walk in until 5 PM.",
            "Processing takes 3-5 business days."
        ));
        assert!(!guard_ok(
            "Allow 3-5 business days. Walk-ins until 3 PM.",
            "Prevea 3-5 días hábiles. Sin cita hasta las 5 PM."
        ));
        assert!(!guard_ok(
            "Closes at 12:00 on Fridays.",
            "Allow 10-12 business days."
        ));
        assert!(!guard_ok("Open until 6 PM.", "Allow 6-8+ weeks."));
        assert!(!guard_ok(
            "Processing takes 3-5 business days.",
            "Abierto de 3 a.m. a 5 p.m."
        ));
        // #1: am/pm is kept: the other half of the day never grounds.
        assert!(!guard_ok(
            "Open until 8 PM",
            "Open Monday to Friday, 8:00 AM to 4:30 PM."
        ));
        assert!(!guard_ok(
            "Walk-ins close at 3:00 PM.",
            "Las visitas cierran a las 3 a.m."
        ));
        // Verify of P352: never across halves of the day.
        assert!(!guard_ok(
            "Walk-ins accepted until 8 PM.",
            "Se aceptan visitas hasta las 8:00 a. m."
        ));
        assert!(!guard_ok(
            "The counter closes at 12 PM on Fridays.",
            "La ventanilla cierra a las 12:00 a. m. los viernes."
        ));
        assert!(!guard_ok(
            "The counter is open 8am-3pm Mon-Fri.",
            "Abierto de 8:00 p. m. a 3:00 a. m."
        ));
        assert!(!guard_ok(
            "Pickup 3:00-4:00pm.",
            "Recogida de 3:00 a 4:00 a. m."
        ));
        assert!(!guard_ok(
            "The counter closes at 12:00 PM on Fridays.",
            "La ventanilla cierra a las 00:00 los viernes."
        ));
        assert!(!guard_ok(
            "The counter opens at 8 PM",
            "Counter hours are Mon-Fri 8-11:30 for filing and pickup."
        ));
        assert!(!guard_ok(
            "Walk-ins start at 8 PM",
            "walk-ins 8-9am Mon-Thu"
        ));
        assert!(!guard_ok("Phones open at 8 PM", "Mon-Fri 8-5"));
        assert!(!guard_ok("Intake until 1 AM", "hours 9:30-1:00 intake"));
        assert!(!guard_ok(
            "Open until 8 PM",
            "Office hours: Monday - Friday, 8:00-4:30"
        ));
        assert!(!guard_ok(
            "Pick up documents from 3 AM, Monday to Friday.",
            "pickup 3:00-4:00pm (pet docs same-day)"
        ));
        // Verify of P353: dashes in counts, ages, prices and dates.
        assert!(!guard_ok(
            "Walk-ins close at 3 PM; expect a 10-15% surcharge.",
            "Las visitas cierran a las 9 a. m.; prevea un recargo de 10-15 %."
        ));
        assert!(!guard_ok("Walk-ins until 3 PM", "Courier fees run $10-15."));
        assert!(!guard_ok("Closes at 12 PM", "Closed November 11-12."));
        assert!(!guard_ok("Phones close at 5:00", "Children ages 2-5."));
        assert!(!guard_ok(
            "Office hours 8:00-4:30. Walk-in apostille service ends at 4:30 PM.",
            "Horario 8:00-4:30. El servicio de apostilla sin cita termina a las 3:30 p. m."
        ));
        assert!(!guard_ok(
            "Counter hours 9-5.",
            "Horario de ventanilla: de 5 p. m. a 9 a. m."
        ));
        // A faithful translation of a time still passes.
        assert!(guard_ok(
            "Allow 3 business days. Open 8:00 a.m. to 3:00 p.m.",
            "Prevea 3 días hábiles. Abierto de 8:00 a 3:00."
        ));
    }

    #[test]
    fn dollars_compare_by_value() {
        // Review P346 #22: digit strings let $1,500 ground $15.00.
        assert!(!guard_ok("The fee is $1,500.", "The fee is $15.00."));
        assert!(guard_ok("The fee is $15.", "La tarifa es de 15,00 $."));
        assert!(guard_ok("The fee is $1,500.", "Les frais sont de 1 500 $."));
        assert!(guard_ok("The fee is $15.00.", "The fee is $15."));
    }

    #[test]
    fn dollar_amounts_parse_to_exact_cents() {
        assert_eq!(
            dollar_amounts_cents("You pay $150 for same-day service"),
            vec![15000]
        );
        assert_eq!(
            dollar_amounts_cents("$15.00 per document, $1,500 max, 8:00 AM"),
            vec![1500, 150000]
        );
        assert_eq!(dollar_amounts_cents("fee 15 $"), vec![1500]);
        assert!(dollar_amounts_cents("Open 8:00 AM, PO Box 4000").is_empty());
    }

    // Recorded Meta chat-completion envelopes (replay fixtures: no key,
    // no network — the parser proves the shape, the stub proves the flow).

    /// Successful Spanish translation (digits + PO box preserved).
    const REPLAY_ES_OK: &str = r#"{"id":"chatcmpl-tr-es-1","object":"chat.completion",
      "created":1761000000,"model":"muse-spark-1.3",
      "choices":[{"index":0,"finish_reason":"stop","message":{"role":"assistant",
      "content":"{\"translated_statement\": \"Cuota de presentación: $25.00, pagadera en PO Box 40220.\"}"}}],
      "usage":{"prompt_tokens":120,"completion_tokens":60,"total_tokens":180}}"#;

    /// Successful French translation (grouping separator reformatted).
    const REPLAY_FR_OK: &str = r#"{"id":"chatcmpl-tr-fr-1","object":"chat.completion",
      "created":1761000000,"model":"muse-spark-1.3",
      "choices":[{"index":0,"finish_reason":"stop","message":{"role":"assistant",
      "content":"{\"translated_statement\": \"Frais de dépôt : 25 000 copies à 123 Main Street.\"}"}}],
      "usage":{"prompt_tokens":120,"completion_tokens":60,"total_tokens":180}}"#;

    /// Truncated reply: must surface as `Refused`, never as content.
    const REPLAY_TRUNCATED: &str = r#"{"id":"chatcmpl-tr-x","object":"chat.completion",
      "choices":[{"index":0,"finish_reason":"length","message":{"role":"assistant",
      "content":"{\"translated_statement\": \"Cuota de"}}]}"#;

    /// Explicit refusal: must surface as `Refused`.
    const REPLAY_REFUSAL: &str = r#"{"id":"chatcmpl-tr-x","object":"chat.completion",
      "choices":[{"index":0,"finish_reason":"refusal","message":{"role":"assistant",
      "content":null,"refusal":"I cannot translate this."}}]}"#;

    #[test]
    fn body_follows_meta_shape_without_temperature() {
        let body = translate_body("es", "Filing fee: $25.00.");
        assert_eq!(body["model"], "muse-spark-1.3");
        assert!(
            body.get("temperature").is_none(),
            "F1: leave temperature unset"
        );
        assert_eq!(body["reasoning_effort"], "low");
        assert_eq!(body["max_completion_tokens"], 2000);
        assert_eq!(body["prompt_cache_key"], "fact-translate-v1");
        let messages = body["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0]["role"], "developer");
        assert_eq!(messages[1]["role"], "user");
        // Stable prefix carries no per-run values (cacheable).
        assert!(!messages[0]["content"].as_str().unwrap().contains("$25.00"));
        assert!(messages[1]["content"].as_str().unwrap().contains("$25.00"));
        assert_eq!(body["response_format"]["type"], "json_schema");
        assert_eq!(
            body["response_format"]["json_schema"]["name"],
            "TranslatedFact"
        );
    }

    #[test]
    fn parse_replay_es_ok_preserves_fee_and_pobox() {
        let out = parse_translate_response(REPLAY_ES_OK.as_bytes()).unwrap();
        assert!(out.contains("$25.00"), "fee amount verbatim: {out}");
        assert!(out.contains("40220"), "PO box digits verbatim: {out}");
        assert!(
            guard_ok("Filing fee: $25.00, payable to PO Box 40220.", &out),
            "replay ES passes the guard: {out}"
        );
    }

    #[test]
    fn parse_replay_fr_ok_tolerates_grouping_separator() {
        let out = parse_translate_response(REPLAY_FR_OK.as_bytes()).unwrap();
        assert!(
            guard_ok("Filing fee 25,000 copies at 123 Main Street.", &out),
            "replay FR passes the guard: {out}"
        );
    }

    #[test]
    fn parse_replay_truncation_and_refusal_are_refused() {
        assert!(matches!(
            parse_translate_response(REPLAY_TRUNCATED.as_bytes()),
            Err(TranslateError::Refused(_))
        ));
        assert!(matches!(
            parse_translate_response(REPLAY_REFUSAL.as_bytes()),
            Err(TranslateError::Refused(_))
        ));
    }

    #[test]
    fn parse_replay_error_envelope_and_garbage() {
        let envelope = r#"{"error":{"message":"overloaded","type":"server_error"}}"#;
        assert!(matches!(
            parse_translate_response(envelope.as_bytes()),
            Err(TranslateError::Transport(_))
        ));
        assert!(matches!(
            parse_translate_response(b"not json"),
            Err(TranslateError::BadOutput(_))
        ));
        let empty = r#"{"choices":[]}"#;
        assert!(matches!(
            parse_translate_response(empty.as_bytes()),
            Err(TranslateError::BadOutput(_))
        ));
    }

    #[test]
    fn guard_digits_fees_addresses() {
        // Bare digits survive; changed digits fail.
        assert!(guard_ok(
            "Open 9 to 5 weekdays.",
            "Abierto de 9 a 5 entre semana."
        ));
        assert!(!guard_ok(
            "Open 9 to 5 weekdays.",
            "Abierto de 9 a 6 entre semana."
        ));
        // Fees: symbol + amount both required.
        assert!(guard_ok("Fee: $25.00.", "Cuota: $25.00."));
        assert!(!guard_ok("Fee: $25.00.", "Cuota: $26.00."));
        assert!(!guard_ok("Fee: $25.00.", "Cuota: 25.00."));
        // Grouping separators may be reformatted, digits never.
        assert!(guard_ok(
            "Serves 25,000 offices.",
            "Atiende a 25 000 oficinas."
        ));
        assert!(!guard_ok(
            "Serves 25,000 offices.",
            "Atiende a 25 001 oficinas."
        ));
        // PO boxes: marker + digits.
        assert!(guard_ok("Mail to PO Box 40220.", "Correo a PO Box 40220."));
        assert!(!guard_ok(
            "Mail to PO Box 40220.",
            "Correo a la casilla 40220."
        ));
        assert!(!guard_ok("Mail to PO Box 40220.", "Correo a PO Box 40221."));
        // Street addresses: verbatim or nothing.
        assert!(guard_ok(
            "Office at 123 Main Street.",
            "Oficina en 123 Main Street."
        ));
        assert!(!guard_ok(
            "Office at 123 Main Street.",
            "Oficina en 123 Calle Principal."
        ));
        // Prose without protected spans always passes.
        assert!(guard_ok("Bring identification.", "Traiga identificación."));
    }

    #[test]
    fn guard_reextraction_translated_covers_original() {
        let original = "Fee $25.00 at 123 Main Street, PO Box 40220, open 9 to 5.";
        let translated = "Cuota $25.00 en 123 Main Street, PO Box 40220, abierto de 9 a 5.";
        assert!(guard_ok(original, translated));
        // Re-extraction: every original digit run reappears in the
        // translation's own extraction.
        let want: Vec<String> = protected_spans(original)
            .iter()
            .flat_map(|s| {
                if s.digits.is_empty() {
                    vec![s.text.clone()]
                } else {
                    vec![s.digits.clone()]
                }
            })
            .collect();
        let have_digits: String = protected_spans(translated)
            .iter()
            .map(|s| s.digits.clone())
            .collect::<Vec<_>>()
            .join("|");
        for w in &want {
            if w.chars().all(|c| c.is_ascii_digit()) {
                assert!(have_digits.contains(w), "digits {w} re-extracted");
            } else {
                assert!(translated.contains(w), "street {w} re-extracted");
            }
        }
    }

    #[test]
    fn lang_and_budget_units() {
        assert_eq!(parse_lang(Some("es")), Some("es"));
        assert_eq!(parse_lang(Some("fr")), Some("fr"));
        assert_eq!(parse_lang(Some("de")), None);
        assert_eq!(parse_lang(None), None);
        let mut state = (0, 0);
        assert!(budget_take_in(&mut state, 86_400));
        assert!(budget_take_in(&mut state, 86_400 + 1));
        // Day rollover resets the count.
        assert!(budget_take_in(&mut state, 2 * 86_400));
        assert_eq!(state.1, 1);
    }

    fn mem_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "foreign_keys", "ON").unwrap();
        conn.execute_batch(include_str!(
            "../../handbook-store/migrations/schema_baseline.sql"
        ))
        .unwrap();
        migrate_translate_tables(&conn).unwrap();
        conn
    }

    fn seed_accepted_fact(conn: &Connection, statement: &str) -> i64 {
        let authority: i64 = conn
            .query_row(
                "INSERT INTO authority(kind, country, region, name, slug)
                 VALUES ('state', 'USA', 'TS', 'Test SOS', 'test-sos')
                 RETURNING id",
                [],
                |row| row.get(0),
            )
            .unwrap();
        conn.execute(
            "INSERT INTO capture(authority_id, captured_at, kind, path,
                                 bytes_raw, bytes_stored, sha256, trust)
             VALUES (?1, 1761000000, 'file', 'p', 0, 0, 's', 'staff_verified')",
            [authority],
        )
        .unwrap();
        let cap = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO fact(authority_id, capture_id, topic, statement,
                              status, created_by, created_at,
                              citation_start, citation_end)
             VALUES (?1, ?2, 'fees', ?3, 'accepted', 't', 1, -1, -1)",
            rusqlite::params![authority, cap, statement],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    #[test]
    fn migrate_is_idempotent_and_queues_after_accept() {
        let conn = mem_db();
        migrate_translate_tables(&conn).unwrap();
        let id = seed_accepted_fact(&conn, "Fee: $25.00.");
        assert_eq!(request_after_accept(&conn, id, 100).unwrap(), 2);
        // Second accept queues nothing new (INSERT OR IGNORE).
        assert_eq!(request_after_accept(&conn, id, 100).unwrap(), 0);
        // Backfill finds nothing missing now.
        assert_eq!(backfill_accepted(&conn, None, 100).unwrap(), 0);
    }

    #[test]
    fn backfill_only_accepted_and_only_missing() {
        let conn = mem_db();
        let accepted = seed_accepted_fact(&conn, "Fee: $25.00.");
        let proposed: i64 = conn
            .query_row(
                "INSERT INTO fact(authority_id, capture_id, topic, statement,
                                  status, created_by, created_at,
                                  citation_start, citation_end)
                 SELECT authority_id, capture_id, 'fees', 'Draft.', 'proposed',
                        't', 1, -1, -1 FROM fact WHERE id = ?1 RETURNING id",
                [accepted],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(backfill_accepted(&conn, Some("es"), 100).unwrap(), 1);
        assert_eq!(backfill_accepted(&conn, Some("es"), 100).unwrap(), 0);
        assert_eq!(backfill_accepted(&conn, Some("de"), 100).unwrap(), 0);
        // Proposed facts never queue.
        assert_eq!(request_after_accept(&conn, proposed, 100).unwrap(), 2);
    }

    /// Translator double that corrupts digits (guard must catch it).
    struct CorruptingTranslator;

    impl Translator for CorruptingTranslator {
        fn translate(
            &self,
            _statement: &str,
            _lang: &str,
        ) -> Result<TranslateOutcome, TranslateError> {
            Ok(TranslateOutcome {
                translated: "Cuota: $26.00.".to_string(),
                model: "corrupt".to_string(),
            })
        }
    }

    #[test]
    fn translate_one_serves_only_guard_passing() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::new(tmp.path());
        store.init().unwrap();
        let conn = db::open(&store).unwrap();
        db::migrate(&conn).unwrap();
        migrate_translate_tables(&conn).unwrap();
        let id = seed_accepted_fact(&conn, "Fee: $25.00.");
        drop(conn);

        let view = translate_one(tmp.path(), id, "es", &StubTranslator, 200, "t").unwrap();
        assert!(view.guard_ok);
        assert!(view.statement.contains("$25.00"));

        let conn = db::open(&store).unwrap();
        let rows = translations_for_slug(&conn, "test-sos", "es")
            .unwrap()
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].fact_id, id);

        // Guard-failing render stores guard_ok = 0 and is never served.
        let bad = translate_one(tmp.path(), id, "fr", &CorruptingTranslator, 201, "t").unwrap();
        assert!(!bad.guard_ok);
        let rows = translations_for_slug(&conn, "test-sos", "fr")
            .unwrap()
            .unwrap();
        assert!(rows.is_empty(), "guard-failing rows never serve");
        assert!(
            translations_for_slug(&conn, "nope", "es")
                .unwrap()
                .is_none()
        );
    }

    use axum::{
        Router,
        body::Body,
        http::{Request, StatusCode},
        routing::get,
    };
    use tower::ServiceExt as _;

    fn test_state(store_root: &std::path::Path) -> AppState {
        AppState {
            spa_index: std::path::PathBuf::from("test-spa/index.html"),
            static_dir: std::path::PathBuf::from("static"),
            store_root: store_root.to_path_buf(),
            bus: crate::ws_bus::Bus::new(),
            kek: Some(std::sync::Arc::new(config::Kek::generate())),
            admin_mtls_off: true,
            admin_allow_ips: config::loopback_allow_ips(),
            admin_spa_dir: store_root.join("admin-spa"),
            media_terms_read: false,
            label_cfg: crate::p2_labels::LabelCfg::default(),
            voice_cfg: voice::VoiceConfig::default(),
            voice_api_key: None,
            webauthn: std::sync::Arc::new(passkey::test_webauthn()),
            auth_limits: std::sync::Arc::new(passkey::RateBuckets::default()),
            demo_limits: std::sync::Arc::new(passkey::RateBuckets::default()),
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
            notify_limits: std::sync::Arc::new(passkey::RateBuckets::default()),
            notify_base: "https://example.test/apostille".to_string(),
            meta_delete_status_base: "https://example.test/auth/meta/delete-status".to_string(),
            fedex_live: crate::fedex_live::FedexLiveState::default(),
        }
    }

    fn public_router(state: &AppState) -> Router {
        Router::new()
            .route(
                "/api/v1/authorities/{slug}/translations",
                get(public_translations),
            )
            .with_state(state.clone())
    }

    async fn json_of(res: axum::response::Response) -> serde_json::Value {
        let bytes = axum::body::to_bytes(res.into_body(), 65536).await.unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    #[tokio::test]
    async fn public_translations_round_trip() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::new(tmp.path());
        store.init().unwrap();
        let conn = db::open(&store).unwrap();
        db::migrate(&conn).unwrap();
        migrate_translate_tables(&conn).unwrap();
        let id = seed_accepted_fact(&conn, "Fee: $25.00 at 123 Main Street.");
        drop(conn);
        translate_one(tmp.path(), id, "es", &StubTranslator, 300, "t").unwrap();
        let state = test_state(tmp.path());

        let addr: SocketAddr = "127.0.0.1:443".parse().unwrap();
        let res = public_router(&state)
            .oneshot(
                Request::builder()
                    .uri("/api/v1/authorities/test-sos/translations?lang=es")
                    .extension(ConnectInfo(addr))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = json_of(res).await;
        assert_eq!(body["count"], 1);
        assert_eq!(body["translations"][0]["fact_id"], id);
        assert!(
            body["translations"][0]["statement"]
                .as_str()
                .unwrap()
                .contains("$25.00")
        );

        // Missing lang is 422; unknown slug is 404.
        for (uri, status) in [
            (
                "/api/v1/authorities/test-sos/translations",
                StatusCode::UNPROCESSABLE_ENTITY,
            ),
            (
                "/api/v1/authorities/test-sos/translations?lang=de",
                StatusCode::UNPROCESSABLE_ENTITY,
            ),
            (
                "/api/v1/authorities/nope/translations?lang=es",
                StatusCode::NOT_FOUND,
            ),
        ] {
            let res = public_router(&state)
                .oneshot(
                    Request::builder()
                        .uri(uri)
                        .extension(ConnectInfo(addr))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(res.status(), status, "{uri}");
        }
    }

    #[test]
    fn translate_one_refuses_non_accepted_and_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::new(tmp.path());
        store.init().unwrap();
        let conn = db::open(&store).unwrap();
        db::migrate(&conn).unwrap();
        migrate_translate_tables(&conn).unwrap();
        let id = seed_accepted_fact(&conn, "Fee: $25.00.");
        conn.execute("UPDATE fact SET status = 'proposed' WHERE id = ?1", [id])
            .unwrap();
        drop(conn);
        assert!(matches!(
            translate_one(tmp.path(), id, "es", &StubTranslator, 200, "t"),
            Err(TranslateError::BadOutput(_))
        ));
        assert!(matches!(
            translate_one(tmp.path(), 999_999, "es", &StubTranslator, 200, "t"),
            Err(TranslateError::BadOutput(_))
        ));
    }
}
