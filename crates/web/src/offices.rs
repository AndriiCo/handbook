//! `OfficeSetV1`: Spark structures accepted address/hours/walk-in/
//! appointment/po_box facts into offices; `address_ok` + `hours_ok`
//! guards; staff accept; `offices_accepted` on `authority:{slug}`.
//!
//! Owner: lane SA-23 ground (`docs/lanes/sa-23.md`). O1 first cut
//! landed by the builder (P327) while the lane session could not start;
//! the lane owns this file from here.
//!
//! The model only proposes structure. Every address line, PO box, day
//! and time it returns must be written in the accepted fact it cites,
//! or the guard drops that piece; a timezone is kept only when a cited
//! fact names it (or staff confirm it at accept); directions links are
//! built here from the kept address, never by the model.

use std::collections::HashMap;
use std::sync::Arc;

use axum::{
    Router,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Json, Response},
    routing::{get, post},
};
use handbook_agent::model::ChatJsonRequest;
use handbook_store::db;
use rusqlite::{Connection, OptionalExtension};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::admin::AdminAuth;
use crate::plan::{Check, Grounding};
use crate::{AppState, store_unavailable, unix_now};

pub const OFFICES_MODEL: &str = "muse-spark-1.3";
const PROMPT: &str = include_str!("../prompts/offices-v1.md");
const PER_RUN_MARKER: &str = "<!-- PER-RUN -->";
const DAYS: &[&str] = &["mon", "tue", "wed", "thu", "fri", "sat", "sun"];
const DAY_NAMES: &[(&str, &str)] = &[
    ("mon", "monday"),
    ("tue", "tuesday"),
    ("wed", "wednesday"),
    ("thu", "thursday"),
    ("fri", "friday"),
    ("sat", "saturday"),
    ("sun", "sunday"),
];
/// Topics an office piece may rest on.
const OFFICE_TOPICS: &[&str] = &[
    "address",
    "hours",
    "walkin",
    "appointment",
    "po_box",
    "mailing",
    "holiday_hours",
    "dropbox",
    "parking",
    "contacts",
];
/// Timezone words a cited fact must contain for each IANA zone.
const ZONES: &[(&str, &[&str])] = &[
    ("America/Los_Angeles", &["pacific", " pt", " pst", " pdt"]),
    ("America/Denver", &["mountain", " mt", " mst", " mdt"]),
    ("America/Chicago", &["central", " ct", " cst", " cdt"]),
    ("America/New_York", &["eastern", " et", " est", " edt"]),
    ("Asia/Ho_Chi_Minh", &["ict", "indochina", "gmt+7", "utc+7"]),
    ("Europe/Amsterdam", &["cet", "cest", "central european"]),
];

// ------------------------------------------------------------- guards

fn norm(s: &str) -> String {
    s.to_lowercase()
        .replace(['.', ','], " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// `needle` written in `hay` on token boundaries (`" 100 "`, not the
/// `100` inside `1000`; review #51).
fn written_in(needle: &str, hay: &str) -> bool {
    let n = norm(needle);
    !n.is_empty() && format!(" {} ", norm(hay)).contains(&format!(" {n} "))
}

/// Sentence-ish clauses of a fact (`.`/`;` + space, newlines), so a
/// day and its hours must be written together (review #22).
fn clauses(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut cur = String::new();
    for (i, c) in chars.iter().enumerate() {
        // A period ends a clause when a new sentence follows (space,
        // then an uppercase letter or the end) — so "8 a.m. to 4 p.m."
        // stays one clause but "4:30 p.m. Closed Saturday" splits.
        let sentence_end = *c == '.' && {
            let rest: String = chars[i + 1..].iter().collect();
            let trimmed = rest.trim_start();
            rest.starts_with(char::is_whitespace)
                && (trimmed.is_empty() || trimmed.starts_with(char::is_uppercase))
        };
        if *c == '\n' || *c == ';' || sentence_end {
            if !cur.trim().is_empty() {
                out.push(cur.trim().to_lowercase());
            }
            cur.clear();
        } else {
            cur.push(*c);
        }
    }
    if !cur.trim().is_empty() {
        out.push(cur.trim().to_lowercase());
    }
    out
}

/// `HH:MM` 24 h → minutes.
fn minutes(t: &str) -> Option<u32> {
    let (h, m) = t.split_once(':')?;
    let (h, m): (u32, u32) = (h.parse().ok()?, m.parse().ok()?);
    // Lazy: an absurd model value must fail the check, not overflow.
    (h < 24 && m < 60 && t.len() == 5).then(|| h * 60 + m)
}

/// The ways a time may be written in source text (`16:30` → `4:30`,
/// `16:30`; `08:00` → `8:00`, `8 a`, `8am`, …).
fn time_forms(t: &str) -> Vec<String> {
    let Some(total) = minutes(t) else {
        return vec![];
    };
    let (h, m) = (total / 60, total % 60);
    let h12 = if h % 12 == 0 { 12 } else { h % 12 };
    let ap = if h < 12 { "a" } else { "p" };
    let mut out = vec![
        format!("{h12}:{m:02}"),
        format!("{h:02}:{m:02}"),
        format!("{h}:{m:02}"),
    ];
    if m == 0 {
        out.extend([
            format!("{h12} {ap}"),
            format!("{h12}{ap}"),
            format!("{h12} o'clock"),
        ]);
        if h == 12 {
            out.push("noon".to_string());
        }
    }
    out
}

/// True when `t` is written in `text` with a consistent am/pm marker
/// (an afternoon close written as `4:30` must not sit next to `a.m.`).
pub(crate) fn time_written(t: &str, text: &str) -> bool {
    let lower = text.to_lowercase();
    let Some(total) = minutes(t) else {
        return false;
    };
    time_forms(t).iter().any(|form| {
        lower.match_indices(form.as_str()).any(|(i, _)| {
            // Token boundaries: not the 8:00 in 18:00, not the noon in
            // afternoon (review #21).
            let before = lower[..i].chars().next_back();
            if before.is_some_and(|c| c.is_ascii_digit() || c == ':' || c.is_alphabetic()) {
                return false;
            }
            let next = lower[i + form.len()..].chars().next();
            if form.ends_with(char::is_numeric)
                && next.is_some_and(|c| c.is_ascii_digit() || c == ':')
            {
                return false;
            }
            if form == "noon" && next.is_some_and(char::is_alphabetic) {
                return false;
            }
            let after: String = lower[i + form.len()..].chars().take(6).collect();
            let after = after.trim_start().replace('.', "");
            let says_am = after.starts_with("am")
                || after.starts_with('a') && form.ends_with(char::is_numeric);
            let says_pm = after.starts_with("pm")
                || after.starts_with('p') && form.ends_with(char::is_numeric);
            !(total >= 12 * 60 && says_am) && !(total < 12 * 60 && says_pm)
        })
    })
}

/// True when every day in `days` is written in `text` (names, or a
/// weekday range for Monday–Friday sets).
fn days_written(days: &[String], text: &str) -> bool {
    let lower = text.to_lowercase();
    let weekday_range = [
        "monday through friday",
        "monday to friday",
        "monday-friday",
        "monday – friday",
        "monday - friday",
        "mon-fri",
        "mon – fri",
        "weekdays",
        "monday–friday",
    ]
    .iter()
    .any(|p| lower.contains(p));
    days.iter().all(|d| {
        let full = DAY_NAMES
            .iter()
            .find(|(k, _)| k == d)
            .map(|(_, n)| *n)
            .unwrap_or("");
        let weekday = !matches!(d.as_str(), "sat" | "sun");
        (!full.is_empty() && lower.contains(full)) || (weekday && weekday_range)
    })
}

/// One hours range written together in a single clause of `text`:
/// every day (by name or weekday range) and both times, in a clause
/// that does not say closed (review #22).
fn range_written(days: &[String], open: &str, close: &str, text: &str) -> bool {
    clauses(text).iter().any(|c| {
        let negated = c.contains("closed") || c.contains("not open") || c.contains("no service");
        !negated && days_written(days, c) && time_written(open, c) && time_written(close, c)
    })
}

/// `PO Box N` written in `text`, normalized.
fn po_box_in(text: &str) -> Option<String> {
    // ASCII lowercase: the index slices `text` below.
    let lower = text.to_ascii_lowercase();
    let (i, marker) = lower
        .find("po box")
        .map(|i| (i, 6))
        .or_else(|| lower.find("p.o. box").map(|i| (i, 8)))?;
    // Digits must follow the marker directly (spaces, '#' or 'no.'
    // only): never the next number further on (review #18).
    let rest = text[i + marker..].trim_start();
    let rest = rest
        .strip_prefix('#')
        .or_else(|| rest.strip_prefix("No."))
        .or_else(|| rest.strip_prefix("no."))
        .unwrap_or(rest)
        .trim_start();
    let digits: String = rest
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '-')
        .collect();
    let digits = digits.trim_end_matches('-');
    (!digits.is_empty() && digits.starts_with(|c: char| c.is_ascii_digit()))
        .then(|| format!("PO Box {digits}"))
}

/// Country for an address: the model's value only when written in the
/// fact; `US` only for a two-letter state plus a US ZIP.
fn country_of(model: &str, region: &str, zip: &str, text: &str) -> String {
    if !model.is_empty() && written_in(model, text) {
        return model.to_string();
    }
    let us_zip = zip.len() >= 5 && zip.bytes().take(5).all(|b| b.is_ascii_digit());
    if region.len() == 2 && region.bytes().all(|b| b.is_ascii_uppercase()) && us_zip {
        "US".to_string()
    } else {
        String::new()
    }
}

fn directions(line1: &str, city: &str, region: &str) -> String {
    let q: String = format!("{line1} {city} {region}")
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_string()
            } else {
                "%20".to_string()
            }
        })
        .collect();
    format!("https://www.openstreetmap.org/search?query={q}")
}

/// Guard one proposed office against the accepted facts; returns the
/// cleaned office (contracts `plan.json` → `offices[]`, minus id/status)
/// and its guard lines, or `None` when nothing grounded survives.
pub fn guard_office(raw: &Value, facts: &[Grounding]) -> (Option<Value>, Vec<Check>) {
    let by_id: HashMap<i64, &Grounding> = facts.iter().map(|f| (f.id, f)).collect();
    let fits = |id: i64| {
        by_id
            .get(&id)
            .filter(|f| OFFICE_TOPICS.contains(&f.topic.as_str()))
            .copied()
    };
    let s = |v: &Value, k: &str| v[k].as_str().unwrap_or("").trim().to_string();
    let mut checks = Vec::new();
    let mut used: Vec<i64> = Vec::new();

    // Street address.
    let a = &raw["address"];
    let address = fits(a["fact_id"].as_i64().unwrap_or(0)).and_then(|f| {
        let (line1, city, zip) = (s(a, "line1"), s(a, "city"), s(a, "postal_code"));
        let region = s(a, "region");
        let ok = written_in(&line1, &f.text)
            && written_in(&city, &f.text)
            && (zip.is_empty() || written_in(&zip, &f.text))
            && (region.is_empty() || written_in(&region, &f.text));
        ok.then(|| {
            used.push(f.id);
            json!({"line1": line1, "line2": Value::Null, "city": city, "region": region,
                   "postal_code": zip, "country": country_of(&s(a, "country"), &region, &zip, &f.text),
                   "fact_id": f.id})
        })
    });
    checks.push(Check {
        id: "address_ok".into(),
        ok: address.is_some() || s(a, "line1").is_empty(),
        detail: match &address {
            Some(v) => format!("street, city and ZIP all in fact {}", v["fact_id"]),
            None if s(a, "line1").is_empty() => "no street address proposed".into(),
            None => "proposed address is not written in its fact: dropped".into(),
        },
    });

    // Mailing address; a PO box only from cited text.
    let m = &raw["mail"];
    let mail = fits(m["fact_id"].as_i64().unwrap_or(0)).and_then(|f| {
        let (line1, city, zip, region) = (
            s(m, "line1"),
            s(m, "city"),
            s(m, "postal_code"),
            s(m, "region"),
        );
        // The model's line must be written in the fact; it is a PO box
        // only when it names one whose digits the fact carries (#18).
        let fact_box = po_box_in(&f.text);
        let line_box = po_box_in(&line1).filter(|b| fact_box.as_deref() == Some(b.as_str()));
        let ok = written_in(&line1, &f.text)
            && written_in(&city, &f.text)
            && (zip.is_empty() || written_in(&zip, &f.text))
            && (region.is_empty() || written_in(&region, &f.text));
        ok.then(|| {
            used.push(f.id);
            json!({"po_box": line_box, "line1": line_box.clone().unwrap_or(line1), "city": city,
                   "region": region, "postal_code": zip,
                   "country": country_of(&s(m, "country"), &region, &zip, &f.text),
                   "usps_only": line_box.is_some(), "fact_id": f.id})
        })
    });
    checks.push(Check {
        id: "po_box_from_text".into(),
        ok: mail.is_some() || s(m, "line1").is_empty(),
        detail: match &mail {
            Some(v) if v["po_box"].is_string() => format!(
                "{} quoted in fact {}",
                v["po_box"].as_str().unwrap_or(""),
                v["fact_id"]
            ),
            Some(v) => format!("mailing address in fact {}", v["fact_id"]),
            None if s(m, "line1").is_empty() => "no mailing address proposed".into(),
            None => "proposed mailing address is not written in its fact: dropped".into(),
        },
    });

    // Hours.
    let mut hours = Vec::new();
    let mut bad_hours = 0;
    for h in raw["hours"].as_array().into_iter().flatten() {
        let days: Vec<String> = h["days"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|d| d.as_str().map(str::to_string))
            .collect();
        let (open, close) = (s(h, "open"), s(h, "close"));
        let fact = fits(h["fact_id"].as_i64().unwrap_or(0));
        let ok = fact.is_some_and(|f| {
            !days.is_empty()
                && days.iter().all(|d| DAYS.contains(&d.as_str()))
                && minutes(&open)
                    .zip(minutes(&close))
                    .is_some_and(|(o, c)| o < c)
                && range_written(&days, &open, &close, &f.text)
        });
        if let (true, Some(f)) = (ok, fact) {
            used.push(f.id);
            hours.push(json!({"days": days, "open": open, "close": close, "fact_id": f.id}));
        } else {
            bad_hours += 1;
        }
    }
    checks.push(Check {
        id: "hours_ok".into(),
        ok: bad_hours == 0,
        detail: if bad_hours == 0 {
            format!("{} range(s); days and times written in the cited facts", hours.len())
        } else {
            format!("{bad_hours} range(s) dropped: a day or time is not in the cited fact, or open is not before close")
        },
    });

    // Timezone: only when a cited fact names it.
    let tz = s(raw, "timezone");
    let tz_fact = fits(raw["timezone_fact_id"].as_i64().unwrap_or(0));
    let tz_ok = ZONES
        .iter()
        .find(|(zone, _)| *zone == tz)
        .zip(tz_fact)
        .is_some_and(|((_, words), f)| {
            // Whole-word cues only (review #52): never the "pt" in "except".
            let toks: Vec<String> = f
                .text
                .to_lowercase()
                .split(|c: char| !c.is_alphanumeric() && c != '+')
                .filter(|t| !t.is_empty())
                .map(str::to_string)
                .collect();
            let joined = format!(" {} ", toks.join(" "));
            words
                .iter()
                .any(|w| joined.contains(&format!(" {} ", w.trim())))
        });
    if tz_ok && let Some(f) = tz_fact {
        used.push(f.id);
    }
    checks.push(Check {
        id: "timezone".into(),
        ok: tz_ok,
        detail: if tz_ok {
            format!("{tz} quoted")
        } else {
            "not quoted: needs staff confirmation".into()
        },
    });

    // Visit kind (walk-in / appointment / mail only).
    let visit_facts: Vec<i64> = raw["visit"]["fact_ids"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_i64)
        .filter(|id| fits(*id).is_some())
        .collect();
    // A visit kind needs a cited fact of its own topic (review #23).
    let has_topic = |topic: &str| {
        visit_facts
            .iter()
            .filter_map(|id| by_id.get(id))
            .any(|f| f.topic == topic)
    };
    let kind = match s(&raw["visit"], "kind").as_str() {
        "walk_in" if has_topic("walkin") => "walk_in".to_string(),
        "appointment" if has_topic("appointment") => "appointment".to_string(),
        _ if address.is_none() => "mail_only".to_string(),
        _ => "unknown".to_string(),
    };
    let notes = s(&raw["visit"], "notes");
    let notes_ok = notes.is_empty()
        || !visit_facts.is_empty()
            && crate::translate::guard_ok(
                &notes,
                &visit_facts
                    .iter()
                    .filter_map(|id| by_id.get(id))
                    .map(|f| f.text.as_str())
                    .collect::<Vec<_>>()
                    .join("\n"),
            );
    used.extend(&visit_facts);
    if address.is_none() && mail.is_none() && hours.is_empty() {
        return (None, checks);
    }
    used.sort_unstable();
    used.dedup();
    let name = s(raw, "name");
    let directions_url = address.as_ref().map(|a| {
        directions(
            a["line1"].as_str().unwrap_or(""),
            a["city"].as_str().unwrap_or(""),
            a["region"].as_str().unwrap_or(""),
        )
    });
    (
        Some(json!({
            "name": if name.is_empty() || name.chars().count() > 80 { "Apostille office".to_string() } else { name },
            "visit": {"kind": kind, "appointment_required": kind == "appointment",
                      "notes": if notes_ok { notes } else { String::new() }, "fact_ids": visit_facts},
            "address": address, "mail": mail, "hours": hours,
            "timezone": if tz_ok { Value::String(tz) } else { Value::Null },
            "timezone_source": if tz_ok { json!("quoted") } else { Value::Null },
            "phone": Value::Null, "directions_url": directions_url, "fact_ids": used,
        })),
        checks,
    )
}

// ------------------------------------------------------------------ DB

/// Boot DDL on the metadata DB (`handbook.sqlite`): `CREATE TABLE IF
/// NOT EXISTS` only — no ALTER, DROP or RENAME (AGENTS.md §7).
pub fn migrate_tables(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS office_record (
            id INTEGER PRIMARY KEY,
            authority_slug TEXT NOT NULL,
            status TEXT NOT NULL,
            office_json TEXT NOT NULL,
            guards_json TEXT NOT NULL,
            run_id INTEGER,
            created_at INTEGER NOT NULL,
            decided_at INTEGER
        );
        CREATE INDEX IF NOT EXISTS office_record_slug ON office_record(authority_slug, status);",
    )
}

fn row_view(id: i64, slug: &str, status: &str, office: &str, decided: Option<i64>) -> Value {
    let mut v: Value = serde_json::from_str(office).unwrap_or(json!({}));
    v["id"] = json!(id);
    v["authority_slug"] = json!(slug);
    v["status"] = json!(status);
    v["accepted_at"] = json!(decided);
    v
}

/// Accepted offices for one authority, shaped as
/// `docs/lanes/contracts/plan.json` → `offices[]` (`plan.rs` reads this).
pub fn accepted_for_slug(conn: &Connection, slug: &str) -> rusqlite::Result<Vec<Value>> {
    let has: bool = conn
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type='table' AND name='office_record'",
            [],
            |_| Ok(true),
        )
        .optional()?
        .unwrap_or(false);
    if !has {
        return Ok(Vec::new());
    }
    let mut stmt = conn.prepare(
        "SELECT id, status, office_json, decided_at FROM office_record
         WHERE authority_slug = ?1 AND status = 'accepted' ORDER BY id",
    )?;
    stmt.query_map([slug], |r| {
        Ok(row_view(
            r.get(0)?,
            slug,
            &r.get::<_, String>(1)?,
            &r.get::<_, String>(2)?,
            r.get(3)?,
        ))
    })?
    .collect()
}

pub fn insert_proposed(
    conn: &Connection,
    slug: &str,
    office: &Value,
    checks: &[Check],
    run_id: Option<u64>,
    now: i64,
) -> rusqlite::Result<i64> {
    conn.execute(
        "INSERT INTO office_record(authority_slug, status, office_json, guards_json, run_id, created_at)
         VALUES (?1, 'proposed', ?2, ?3, ?4, ?5)",
        rusqlite::params![slug, office.to_string(), serde_json::to_string(checks).unwrap_or_default(), run_id.and_then(|i| i64::try_from(i).ok()), now],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Accept (optionally with a staff-confirmed timezone) or reject one
/// proposed office; accept supersedes an accepted office of the same
/// name. Returns `(slug, status)` or `None` when not proposed.
pub fn decide(
    conn: &Connection,
    id: i64,
    accept: bool,
    timezone: Option<&str>,
    actor: &str,
    now: i64,
) -> rusqlite::Result<Option<(String, String)>> {
    let tx = conn.unchecked_transaction()?;
    let row: Option<(String, String, String)> = tx
        .query_row(
            "SELECT authority_slug, status, office_json FROM office_record WHERE id = ?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    let Some((slug, _, office)) = row.filter(|r| r.1 == "proposed") else {
        return Ok(None);
    };
    let mut office: Value = serde_json::from_str(&office).unwrap_or(json!({}));
    let to = if accept { "accepted" } else { "rejected" };
    if accept {
        if let Some(tz) = timezone.filter(|tz| ZONES.iter().any(|(z, _)| z == tz)) {
            office["timezone"] = json!(tz);
            office["timezone_source"] = json!("staff_confirmed");
        }
        let name = office["name"].as_str().unwrap_or("").to_string();
        let olds: Vec<(i64, String)> = tx
            .prepare("SELECT id, office_json FROM office_record WHERE authority_slug = ?1 AND status = 'accepted'")?
            .query_map([&slug], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        for (old_id, old) in olds {
            let old: Value = serde_json::from_str(&old).unwrap_or(json!({}));
            if old["name"].as_str() == Some(name.as_str()) {
                tx.execute(
                    "UPDATE office_record SET status = 'superseded' WHERE id = ?1",
                    [old_id],
                )?;
            }
        }
    }
    tx.execute(
        "UPDATE office_record SET status = ?1, decided_at = ?2, office_json = ?3 WHERE id = ?4",
        rusqlite::params![to, now, office.to_string(), id],
    )?;
    tx.execute(
        "INSERT INTO audit(at, actor, action, entity, entity_id, before_ref, after_ref, reason)
         VALUES (?1, ?2, ?3, 'office_record', ?4, 'proposed', ?5, ?6)",
        rusqlite::params![
            now,
            actor,
            if accept {
                "office_accept"
            } else {
                "office_reject"
            },
            id,
            to,
            slug
        ],
    )?;
    tx.commit()?;
    Ok(Some((slug, to.to_string())))
}

// -------------------------------------------------------------- compose

fn schema(fact_ids: &[i64]) -> Value {
    let ids: Vec<i64> = std::iter::once(0).chain(fact_ids.iter().copied()).collect();
    let id = json!({"type": "integer", "enum": ids});
    let addr = json!({"type": "object", "additionalProperties": false,
        "required": ["line1", "city", "region", "postal_code", "country", "fact_id"],
        "properties": {"line1": {"type": "string"}, "city": {"type": "string"}, "region": {"type": "string"},
                       "postal_code": {"type": "string"}, "country": {"type": "string"}, "fact_id": id}});
    json!({"type": "object", "additionalProperties": false, "required": ["offices"],
    "properties": {"offices": {"type": "array", "items": {"type": "object", "additionalProperties": false,
        "required": ["name", "visit", "address", "mail", "hours", "timezone", "timezone_fact_id"],
        "properties": {
            "name": {"type": "string"},
            "visit": {"type": "object", "additionalProperties": false, "required": ["kind", "notes", "fact_ids"],
                "properties": {"kind": {"type": "string", "enum": ["walk_in", "appointment", "mail_only"]},
                               "notes": {"type": "string"}, "fact_ids": {"type": "array", "items": id}}},
            "address": addr, "mail": addr,
            "hours": {"type": "array", "items": {"type": "object", "additionalProperties": false,
                "required": ["days", "open", "close", "fact_id"],
                "properties": {"days": {"type": "array", "items": {"type": "string", "enum": DAYS}},
                               "open": {"type": "string"}, "close": {"type": "string"}, "fact_id": id}}},
            "timezone": {"type": "string", "enum": ["", "America/Los_Angeles", "America/Denver", "America/Chicago", "America/New_York", "Asia/Ho_Chi_Minh", "Europe/Amsterdam"]},
            "timezone_fact_id": id,
        }}}}})
}

/// Compose, guard and store proposed offices; returns a summary.
pub fn compose_blocking(
    store_root: &std::path::Path,
    kek: Option<&config::Kek>,
    slug: &str,
    run_id: u64,
    on_step: &dyn Fn(&str),
) -> Result<Value, String> {
    let store = handbook_store::store::Store::new(store_root);
    let conn = db::open(&store).map_err(|e| format!("store unavailable: {e}"))?;
    migrate_tables(&conn).map_err(|e| format!("store: {e}"))?;
    let name: String = conn
        .query_row("SELECT name FROM authority WHERE slug = ?1", [slug], |r| {
            r.get(0)
        })
        .map_err(|_| "unknown authority".to_string())?;
    let facts: Vec<_> = crate::approval::accepted_fact_views(&conn, store_root, slug)
        .unwrap_or_default()
        .into_iter()
        .filter(|f| OFFICE_TOPICS.contains(&f.topic.as_str()))
        .collect();
    if facts.is_empty() {
        return Err("needs accepted address, hours, walk-in or mailing facts".to_string());
    }
    let grounding: Vec<Grounding> = facts.iter().map(Grounding::from_view).collect();
    let ids: Vec<i64> = facts.iter().map(|f| f.id).collect();
    let (developer, per_run) = PROMPT
        .split_once(PER_RUN_MARKER)
        .expect("offices prompt carries a PER-RUN part");
    let lines: Vec<String> = facts
        .iter()
        .map(|f| {
            format!(
                "- {} · {} · {} · \"{}\"",
                f.id,
                f.topic,
                f.statement,
                f.citations.first().map(|c| c.quote.as_str()).unwrap_or("")
            )
        })
        .collect();
    let user = per_run
        .replace("{{name}}", &name)
        .replace("{{slug}}", slug)
        .replace("{{facts}}", &lines.join("\n"))
        .trim()
        .to_string();
    let client = crate::draft::live_client(&conn, kek)?.with_observer(
        crate::telemetry::DbObserver::new(store_root.to_path_buf()),
        "offices",
        Some(run_id),
    );
    let resp = client
        .chat_json(&ChatJsonRequest {
            model: OFFICES_MODEL,
            developer,
            user,
            schema_name: "OfficeSetV1",
            schema: schema(&ids),
            reasoning_effort: "medium",
            prompt_cache_key: "authority-offices-v1".to_string(),
            max_completion_tokens: 8_192,
            pipeline: "offices",
            run_id: Some(run_id),
        })
        .map_err(|e| format!("offices failed: {e}"))?;
    on_step("guards");
    let now = unix_now();
    let mut proposed = Vec::new();
    let mut dropped = 0;
    for raw in resp.value["offices"]
        .as_array()
        .into_iter()
        .flatten()
        .take(6)
    {
        match guard_office(raw, &grounding) {
            (Some(office), checks) => {
                let id = insert_proposed(&conn, slug, &office, &checks, Some(run_id), now)
                    .map_err(|e| format!("store: {e}"))?;
                proposed.push(id);
            }
            (None, _) => dropped += 1,
        }
    }
    Ok(
        json!({"proposed": proposed, "dropped": dropped, "model": OFFICES_MODEL,
              "latency_ms": resp.latency_ms, "input_tokens": resp.usage.input_tokens,
              "output_tokens": resp.usage.output_tokens, "cached_tokens": resp.usage.cached_tokens,
              "reasoning_tokens": resp.usage.reasoning_tokens}),
    )
}

// ------------------------------------------------------------- handlers

#[derive(Debug, Deserialize)]
pub struct ComposeBody {
    pub slug: String,
}

async fn compose(
    _auth: AdminAuth,
    State(state): State<AppState>,
    Json(body): Json<ComposeBody>,
) -> Response {
    let store = handbook_store::store::Store::new(&state.store_root);
    let Ok(conn) = db::open(&store) else {
        return store_unavailable();
    };
    if crate::telemetry::over_daily_cap(&conn, &state, unix_now()) {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            Json(json!({"error": "daily model-call cap reached (meta.daily_call_cap)"})),
        )
            .into_response();
    }
    // Synchronous preconditions (a doomed run never answers 202).
    let known: bool = conn
        .query_row(
            "SELECT 1 FROM authority WHERE slug = ?1",
            [&body.slug],
            |_| Ok(true),
        )
        .optional()
        .unwrap_or(None)
        .unwrap_or(false);
    if !known {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "unknown authority"})),
        )
            .into_response();
    }
    let placeholders = OFFICE_TOPICS
        .iter()
        .map(|t| format!("'{t}'"))
        .collect::<Vec<_>>()
        .join(",");
    let office_facts: i64 = conn
        .query_row(
            &format!(
                "SELECT COUNT(*) FROM fact f JOIN authority a ON a.id = f.authority_id
                 WHERE a.slug = ?1 AND f.status = 'accepted' AND f.topic IN ({placeholders})"
            ),
            [&body.slug],
            |r| r.get(0),
        )
        .unwrap_or(0);
    if office_facts == 0 {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"error": "needs accepted address, hours, walk-in or mailing facts", "accepted": 0})),
        )
            .into_response();
    }
    drop(conn);
    let run_id = state.draft_runs.reserve_id();
    let (bus, root, kek): (_, _, Option<Arc<config::Kek>>) = (
        state.bus.clone(),
        state.store_root.clone(),
        state.kek.clone(),
    );
    let slug = body.slug.clone();
    tokio::spawn(async move {
        let topic = format!("run:{run_id}");
        bus.publish(
            &topic,
            json!({"type": "offices.started", "topic": topic, "run_id": run_id, "slug": slug})
                .to_string(),
        );
        let job = slug.clone();
        let step_bus = bus.clone();
        let step_topic = topic.clone();
        let out = tokio::task::spawn_blocking(move || {
            let on_step = |step: &str| {
                step_bus.publish(
                    &step_topic,
                    json!({"type": "offices.step", "topic": step_topic, "run_id": run_id, "step": step}).to_string(),
                );
            };
            compose_blocking(&root, kek.as_deref(), &job, run_id, &on_step)
        })
        .await;
        let (kind, body) = match out {
            Ok(Ok(summary)) => ("offices.done", summary),
            Ok(Err(e)) => ("offices.failed", json!({"error": e})),
            Err(e) => (
                "offices.failed",
                json!({"error": format!("offices task failed: {e}")}),
            ),
        };
        let mut event = json!({"type": kind, "topic": topic, "run_id": run_id, "slug": slug});
        if let (Some(m), Some(extra)) = (event.as_object_mut(), body.as_object()) {
            m.extend(extra.clone());
        }
        bus.publish(&topic, event.to_string());
    });
    (
        StatusCode::ACCEPTED,
        Json(json!({"run_id": run_id, "topic": format!("run:{run_id}")})),
    )
        .into_response()
}

#[derive(Debug, Deserialize)]
pub struct ListQuery {
    pub status: Option<String>,
    pub slug: Option<String>,
}

async fn list(
    _auth: AdminAuth,
    State(state): State<AppState>,
    Query(q): Query<ListQuery>,
) -> Response {
    let store = handbook_store::store::Store::new(&state.store_root);
    let Ok(conn) = db::open(&store) else {
        return store_unavailable();
    };
    let status = q.status.unwrap_or_else(|| "proposed".to_string());
    let slug = q.slug.unwrap_or_default();
    let rows: Vec<Value> = conn
        .prepare(
            "SELECT id, authority_slug, status, office_json, guards_json, decided_at, run_id FROM office_record
             WHERE (?1 = 'all' OR status = ?1) AND (?2 = '' OR authority_slug = ?2) ORDER BY id DESC LIMIT 50",
        )
        .and_then(|mut s| {
            s.query_map(rusqlite::params![status, slug], |r| {
                let mut v = row_view(r.get(0)?, &r.get::<_, String>(1)?, &r.get::<_, String>(2)?, &r.get::<_, String>(3)?, r.get(5)?);
                v["guards"] = serde_json::from_str(&r.get::<_, String>(4)?).unwrap_or(json!([]));
                v["run_id"] = json!(r.get::<_, Option<i64>>(6)?);
                Ok(v)
            })?
            .collect()
        })
        .unwrap_or_default();
    Json(json!({"offices": rows})).into_response()
}

#[derive(Debug, Default, Deserialize)]
pub struct DecideBody {
    #[serde(default)]
    pub timezone: Option<String>,
}

async fn decide_route(
    auth: AdminAuth,
    state: AppState,
    id: i64,
    accept: bool,
    body: DecideBody,
) -> Response {
    let store = handbook_store::store::Store::new(&state.store_root);
    let Ok(conn) = db::open(&store) else {
        return store_unavailable();
    };
    let now = unix_now();
    match decide(
        &conn,
        id,
        accept,
        body.timezone.as_deref(),
        &auth.user.user_id,
        now,
    ) {
        Ok(Some((slug, status))) => {
            if accept {
                crate::ws_bus::publish_authority_event(
                    &state.bus,
                    &slug,
                    "offices_accepted",
                    json!({"office_ids": [id]}),
                    now,
                );
                crate::ws_bus::publish_ops(
                    &state.bus,
                    "offices_accepted",
                    json!({"slug": slug, "office_id": id}),
                    now,
                );
            }
            Json(json!({"id": id, "authority_slug": slug, "status": status})).into_response()
        }
        Ok(None) => (
            StatusCode::CONFLICT,
            Json(json!({"error": "office is not proposed"})),
        )
            .into_response(),
        Err(_) => store_unavailable(),
    }
}

async fn accept(
    auth: AdminAuth,
    State(state): State<AppState>,
    Path(id): Path<i64>,
    body: Option<Json<DecideBody>>,
) -> Response {
    decide_route(auth, state, id, true, body.map(|b| b.0).unwrap_or_default()).await
}

async fn reject(auth: AdminAuth, State(state): State<AppState>, Path(id): Path<i64>) -> Response {
    decide_route(auth, state, id, false, DecideBody::default()).await
}

/// Staff routes merged onto the ADMIN listener only (the builder wraps
/// them in the admin guard stack; handlers take `AdminAuth`).
pub fn admin_router(state: AppState) -> Router {
    Router::<AppState>::new()
        .route("/apostille/api/v1/admin/offices", get(list))
        .route("/apostille/api/v1/admin/offices/compose", post(compose))
        .route("/apostille/api/v1/admin/offices/{id}/accept", post(accept))
        .route("/apostille/api/v1/admin/offices/{id}/reject", post(reject))
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn g(id: i64, topic: &str, text: &str) -> Grounding {
        Grounding {
            id,
            topic: topic.into(),
            text: text.into(),
        }
    }

    fn facts() -> Vec<Grounding> {
        vec![
            g(
                202,
                "hours",
                "The counter is open Monday through Friday, 8:00 a.m. to 4:30 p.m. Pacific time.",
            ),
            g(
                203,
                "po_box",
                "Mailed requests go to PO Box 4000, Olympia, WA 98504.",
            ),
            g(
                204,
                "address",
                "Walk-in service is at 100 Fixture Way, Olympia, WA 98501.",
            ),
            g(
                205,
                "walkin",
                "Walk-in requests are processed the same day.",
            ),
        ]
    }

    fn proposed() -> Value {
        json!({
            "name": "Apostille counter",
            "visit": {"kind": "walk_in", "notes": "Walk-in requests are processed the same day.", "fact_ids": [205]},
            "address": {"line1": "100 Fixture Way", "city": "Olympia", "region": "WA", "postal_code": "98501", "country": "US", "fact_id": 204},
            "mail": {"line1": "PO Box 4000", "city": "Olympia", "region": "WA", "postal_code": "98504", "country": "US", "fact_id": 203},
            "hours": [{"days": ["mon", "tue", "wed", "thu", "fri"], "open": "08:00", "close": "16:30", "fact_id": 202}],
            "timezone": "America/Los_Angeles", "timezone_fact_id": 202,
        })
    }

    #[test]
    fn grounded_office_passes_every_guard() {
        let (office, checks) = guard_office(&proposed(), &facts());
        let office = office.unwrap();
        assert!(checks.iter().all(|c| c.ok), "{checks:?}");
        assert_eq!(office["mail"]["po_box"], "PO Box 4000");
        assert_eq!(office["mail"]["usps_only"], true);
        assert_eq!(office["hours"][0]["close"], "16:30");
        assert_eq!(office["timezone"], "America/Los_Angeles");
        assert_eq!(office["timezone_source"], "quoted");
        assert!(
            office["directions_url"]
                .as_str()
                .unwrap()
                .contains("100%20Fixture%20Way")
        );
        assert_eq!(office["fact_ids"], json!([202, 203, 204, 205]));
    }

    #[test]
    fn invented_pieces_are_dropped() {
        let mut p = proposed();
        p["address"]["line1"] = json!("200 Invented Ave");
        p["mail"]["line1"] = json!("PO Box 5000");
        p["hours"] = json!([
            {"days": ["sat"], "open": "08:00", "close": "16:30", "fact_id": 202},
            {"days": ["mon"], "open": "08:00", "close": "18:00", "fact_id": 202},
            {"days": ["mon"], "open": "16:30", "close": "08:00", "fact_id": 202},
            {"days": ["mon"], "open": "08:00", "close": "16:30", "fact_id": 999}
        ]);
        p["timezone"] = json!("America/New_York");
        let (office, checks) = guard_office(&p, &facts());
        assert!(office.is_none(), "nothing grounded survives");
        let failed: Vec<&str> = checks
            .iter()
            .filter(|c| !c.ok)
            .map(|c| c.id.as_str())
            .collect();
        assert_eq!(
            failed,
            ["address_ok", "po_box_from_text", "hours_ok", "timezone"]
        );
    }

    #[test]
    fn absurd_model_values_never_panic() {
        assert_eq!(minutes("4294967295:00"), None);
        assert_eq!(minutes("99:99"), None);
        assert_eq!(
            po_box_in("İİ Mail to PO Box 40228"),
            Some("PO Box 40228".into())
        );
    }

    #[test]
    fn guards_hold_word_and_clause_boundaries() {
        assert!(
            !time_written("08:00", "open until 18:00"),
            "8:00 inside 18:00"
        );
        assert!(
            !time_written("12:00", "open in the afternoon"),
            "noon inside afternoon"
        );
        assert!(time_written("12:00", "closes at noon."));
        let days = vec!["sat".to_string(), "sun".to_string()];
        assert!(!range_written(
            &days,
            "08:00",
            "16:30",
            "Open Monday to Friday 8:00 a.m. to 4:30 p.m. Closed Saturday and Sunday."
        ));
        assert!(range_written(
            &["mon".into()],
            "08:00",
            "16:30",
            "Open Monday to Friday 8:00 a.m. to 4:30 p.m. Closed Saturday and Sunday."
        ));
        assert_eq!(
            po_box_in("PO Box drop-off at 801 Capitol Way"),
            None,
            "no box number, no box"
        );
        assert_eq!(
            po_box_in("PO Box #40228, Olympia"),
            Some("PO Box 40228".into())
        );
        assert!(!written_in("100", "Suite 1000, Olympia"));
        assert_eq!(country_of("", "WA", "98501", "x"), "US");
        assert_eq!(country_of("", "", "100000", "Hà Nội"), "");
    }

    #[test]
    fn am_pm_must_agree_with_the_text() {
        assert!(time_written("16:30", "until 4:30 p.m."));
        assert!(
            !time_written("04:30", "until 4:30 p.m."),
            "4:30 am is not 4:30 pm"
        );
        assert!(time_written("08:00", "from 8 a.m."));
        assert!(!time_written("20:00", "from 8 a.m."));
        assert!(days_written(
            &["mon".into(), "fri".into()],
            "Monday to Friday"
        ));
        assert!(!days_written(&["sat".into()], "Monday to Friday"));
    }

    #[test]
    fn accept_confirms_a_zone_and_the_read_seam_serves_it() {
        let conn = Connection::open_in_memory().unwrap();
        migrate_tables(&conn).unwrap();
        conn.execute_batch("CREATE TABLE audit(id INTEGER PRIMARY KEY, at INTEGER, actor TEXT, action TEXT, entity TEXT, entity_id INTEGER, before_ref TEXT, after_ref TEXT, reason TEXT);").unwrap();
        let mut p = proposed();
        p["timezone"] = json!("");
        let (office, checks) = guard_office(&p, &facts());
        let id =
            insert_proposed(&conn, "usa-wa-sos", &office.unwrap(), &checks, Some(3), 10).unwrap();
        assert!(accepted_for_slug(&conn, "usa-wa-sos").unwrap().is_empty());
        decide(&conn, id, true, Some("America/Los_Angeles"), "staff-1", 20)
            .unwrap()
            .unwrap();
        let served = accepted_for_slug(&conn, "usa-wa-sos").unwrap();
        assert_eq!(served.len(), 1);
        assert_eq!(served[0]["timezone_source"], "staff_confirmed");
        assert_eq!(served[0]["status"], "accepted");
        let seed: Value =
            serde_json::from_str(include_str!("../../../docs/lanes/contracts/plan.json")).unwrap();
        for key in seed["offices"][0].as_object().unwrap().keys() {
            assert!(served[0].get(key).is_some(), "office key {key}");
        }
        assert!(
            decide(&conn, id, false, None, "staff-1", 30)
                .unwrap()
                .is_none()
        );
    }
}
