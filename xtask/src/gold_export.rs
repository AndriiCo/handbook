//! `cargo xtask gold-export --store <path> [--out evals/authorities-real]
//! [--per-authority 10]`: real gold cases from accepted facts.
//!
//! The 30 cases in `evals/authorities/` are synthetic passages. This
//! turns a store's accepted facts into cases on real official pages, in
//! the same shape: every accepted fact whose citation slices an
//! `official` or `staff_verified` web or file capture
//! (`<store>/text/<capture>.txt`, byte offsets) becomes one case.
//! `evidence` is the cited passage widened to whole sentences (at most
//! [`MAX_EVIDENCE`] bytes), `value` the part of the statement after its
//! `field: ` key, kept only when it appears inside the evidence (the
//! grader's value-in-quote check needs that), and `topic` the fact's
//! topic. Email, call, photo and community captures are skipped: they
//! can carry personal data. One file per authority; the store is only
//! read and no model is called.
//!
//! Run it on a copy of a production backup, then grade the cases on
//! plant with an `eval.gold` job whose payload names the directory
//! (`{"live": true, "gold": "authorities-real"}`). The default eval and
//! the offline replay corpus keep using `evals/authorities/`.

use std::collections::BTreeMap;
use std::path::Path;

use handbook_agent::analyze::normalize;
use handbook_store::store::Store;
use serde_json::{Value, json};

use crate::{flag, open_db, open_store};

/// Longest evidence passage kept, in bytes.
const MAX_EVIDENCE: usize = 600;
/// Shortest evidence passage worth grading, in bytes.
const MIN_EVIDENCE: usize = 40;
/// Cases per authority unless `--per-authority` says otherwise.
const DEFAULT_PER_AUTHORITY: usize = 10;

/// One exported case (`evals/authorities/*.json` shape).
#[derive(Debug, Clone, PartialEq, Eq)]
struct Case {
    authority: String,
    id: String,
    field: String,
    topic: String,
    value: String,
    evidence: String,
}

pub fn run(args: &[String]) {
    let store = open_store(args);
    let out = flag(args, "out").unwrap_or_else(|| "evals/authorities-real".to_string());
    let per = flag(args, "per-authority")
        .and_then(|n| n.parse().ok())
        .unwrap_or(DEFAULT_PER_AUTHORITY)
        .max(1);
    let cases = match cases(&store, per) {
        Ok(cases) => cases,
        Err(e) => {
            eprintln!("gold-export: {e}");
            std::process::exit(1);
        }
    };
    match write(Path::new(&out), &cases) {
        Ok(files) => println!(
            "gold-export: {} case(s) from {files} authorit{} into {out}",
            cases.len(),
            if files == 1 { "y" } else { "ies" }
        ),
        Err(e) => {
            eprintln!("gold-export: {e}");
            std::process::exit(1);
        }
    }
}

/// Every exportable accepted fact, at most `per` per authority, in fact
/// id order.
fn cases(store: &Store, per: usize) -> Result<Vec<Case>, String> {
    let conn = open_db(store);
    let mut stmt = conn
        .prepare(
            "SELECT f.id, a.slug, f.topic, f.statement, f.capture_id,
                    f.citation_start, f.citation_end
             FROM fact f
             JOIN authority a ON a.id = f.authority_id
             JOIN capture c ON c.id = f.capture_id
             WHERE f.status = 'accepted' AND f.citation_start >= 0
               AND f.citation_end > f.citation_start
               AND c.kind IN ('warc', 'file')
               AND c.trust IN ('official', 'staff_verified')
               AND a.slug != 'mail-inbox'
             ORDER BY f.id",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, i64>(5)?,
                row.get::<_, i64>(6)?,
            ))
        })
        .map_err(|e| e.to_string())?;
    let mut per_authority: BTreeMap<String, usize> = BTreeMap::new();
    let mut out = Vec::new();
    for row in rows {
        let (id, slug, topic, statement, capture, start, end) = row.map_err(|e| e.to_string())?;
        let taken = per_authority.entry(slug.clone()).or_default();
        if *taken >= per {
            continue;
        }
        let Ok(text) = std::fs::read(store.root().join("text").join(format!("{capture}.txt")))
        else {
            continue;
        };
        let Some(evidence) = widen(&text, start as usize, end as usize) else {
            continue;
        };
        let Some((field, value)) = split_statement(&statement, &topic) else {
            continue;
        };
        if !normalize(&evidence).contains(&normalize(&value)) {
            continue;
        }
        *taken += 1;
        out.push(Case {
            id: format!("{slug}-r{id}"),
            authority: slug,
            field,
            topic,
            value,
            evidence,
        });
    }
    Ok(out)
}

/// `field: value` → both parts; a statement without a key uses the
/// topic as its field. `None` for an empty value.
fn split_statement(statement: &str, topic: &str) -> Option<(String, String)> {
    let (field, value) = match statement.split_once(": ") {
        Some((key, value)) if !key.is_empty() && !key.contains(' ') => (key, value),
        _ => (topic, statement),
    };
    let value = value.trim();
    (!value.is_empty()).then(|| (field.to_string(), value.to_string()))
}

/// The cited bytes `[start, end)` widened to sentence boundaries (`. `,
/// `! `, `? ` or a line break) within [`MAX_EVIDENCE`] bytes, as one
/// whitespace-collapsed line. `None` when the span is out of range, not
/// UTF-8 at its edges, or too short to grade.
fn widen(text: &[u8], start: usize, end: usize) -> Option<String> {
    let text = std::str::from_utf8(text).ok()?;
    if start >= end
        || end > text.len()
        || !text.is_char_boundary(start)
        || !text.is_char_boundary(end)
    {
        return None;
    }
    let room = MAX_EVIDENCE.saturating_sub(end - start) / 2;
    let floor = floor_char(text, start.saturating_sub(room));
    let ceil = ceil_char(text, (end + room).min(text.len()));
    // The closest boundary on each side: a line break, or the end of a
    // sentence (its mark stays with the sentence).
    let before = &text[floor..start];
    let from = ["\n", ". ", "! ", "? "]
        .iter()
        .filter_map(|mark| before.rfind(mark).map(|i| floor + i + mark.len()))
        .max()
        .unwrap_or(if floor == 0 { 0 } else { start });
    let after = &text[end..ceil];
    let to = [("\n", 0), (". ", 1), ("! ", 1), ("? ", 1)]
        .iter()
        .filter_map(|(mark, keep)| after.find(mark).map(|i| end + i + keep))
        .min()
        .unwrap_or(if ceil == text.len() { ceil } else { end });
    let passage = text[from..to]
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    (passage.len() >= MIN_EVIDENCE).then_some(passage)
}

fn floor_char(text: &str, mut at: usize) -> usize {
    while !text.is_char_boundary(at) {
        at -= 1;
    }
    at
}

fn ceil_char(text: &str, mut at: usize) -> usize {
    while !text.is_char_boundary(at) {
        at += 1;
    }
    at
}

/// One `<slug>.json` per authority under `dir`; returns the file count.
fn write(dir: &Path, cases: &[Case]) -> Result<usize, String> {
    let mut by_authority: BTreeMap<&str, Vec<Value>> = BTreeMap::new();
    for case in cases {
        by_authority
            .entry(&case.authority)
            .or_default()
            .push(json!({
                "id": case.id, "field": case.field, "topic": case.topic,
                "value": case.value, "evidence": case.evidence,
            }));
    }
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    for (slug, cases) in &by_authority {
        let path = dir.join(format!("{slug}.json"));
        let body = serde_json::to_string_pretty(&json!({"authority": slug, "cases": cases}))
            .map_err(|e| e.to_string())?;
        std::fs::write(&path, body + "\n").map_err(|e| format!("{}: {e}", path.display()))?;
    }
    Ok(by_authority.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: &str = "Apostille Program\nThe office is closed on state holidays. \
        Mail requests to PO Box 40228, Olympia, WA 98504-0228. Include a prepaid \
        return envelope.\nWalk-in service runs 8 a.m. to 4 p.m.";

    fn seeded() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().join("store"));
        store.init().unwrap();
        let conn = handbook_store::db::open(&store).unwrap();
        handbook_store::db::migrate(&conn).unwrap();
        let wa = handbook_store::db::add_authority(
            &conn,
            &handbook_store::db::NewAuthority {
                kind: "state",
                country: "USA",
                region: "Washington",
                name: "WA",
                slug: "usa-wa-sos",
                actor: "test",
                at: 1,
            },
        )
        .unwrap();
        for (kind, trust) in [
            ("warc", "official"),
            ("email", "official"),
            ("warc", "community"),
        ] {
            conn.execute(
                "INSERT INTO capture(authority_id, captured_at, kind, path,
                                     bytes_raw, bytes_stored, sha256, trust)
                 VALUES (?1, 1, ?2, 'blob/0', 1, 1, ?3, ?4)",
                rusqlite::params![wa, kind, "a".repeat(64), trust],
            )
            .unwrap();
        }
        std::fs::create_dir_all(store.root().join("text")).unwrap();
        for capture in 1..=3 {
            std::fs::write(store.root().join(format!("text/{capture}.txt")), PAGE).unwrap();
        }
        let quote = "PO Box 40228, Olympia, WA 98504-0228";
        let start = PAGE.find(quote).unwrap() as i64;
        let end = start + quote.len() as i64;
        for (capture, statement, status) in [
            (
                1,
                "mailing_po_box: PO Box 40228, Olympia, WA 98504-0228",
                "accepted",
            ),
            (1, "mailing_po_box: PO Box 99999", "accepted"), // value not in the passage
            (
                1,
                "mailing_po_box: PO Box 40228, Olympia, WA 98504-0228",
                "proposed",
            ),
            (
                2,
                "mailing_po_box: PO Box 40228, Olympia, WA 98504-0228",
                "accepted",
            ), // email
            (
                3,
                "mailing_po_box: PO Box 40228, Olympia, WA 98504-0228",
                "accepted",
            ), // community
        ] {
            conn.execute(
                "INSERT INTO fact(authority_id, capture_id, topic, statement, status,
                                  created_by, created_at, citation_start, citation_end)
                 VALUES (?1, ?2, 'mailing', ?3, ?4, 'test', 1, ?5, ?6)",
                rusqlite::params![wa, capture, statement, status, start, end],
            )
            .unwrap();
        }
        (dir, store)
    }

    #[test]
    fn exports_only_accepted_official_facts_whose_value_is_in_the_passage() {
        let (dir, store) = seeded();
        let got = cases(&store, 10).unwrap();
        assert_eq!(got.len(), 1, "{got:?}");
        let case = &got[0];
        assert_eq!(
            (case.id.as_str(), case.field.as_str(), case.topic.as_str()),
            ("usa-wa-sos-r1", "mailing_po_box", "mailing")
        );
        assert_eq!(
            case.evidence,
            "Mail requests to PO Box 40228, Olympia, WA 98504-0228."
        );
        let out = dir.path().join("gold");
        assert_eq!(write(&out, &got).unwrap(), 1);
        let file: Value =
            serde_json::from_str(&std::fs::read_to_string(out.join("usa-wa-sos.json")).unwrap())
                .unwrap();
        assert_eq!(file["authority"], "usa-wa-sos");
        assert_eq!(
            file["cases"][0]["value"],
            "PO Box 40228, Olympia, WA 98504-0228"
        );
        assert!(cases(&store, 1).unwrap().len() <= 1);
    }

    #[test]
    fn statements_split_on_their_key() {
        assert_eq!(
            split_statement("fees_per_document: $15", "fees"),
            Some(("fees_per_document".into(), "$15".into()))
        );
        assert_eq!(
            split_statement("Closed on state holidays", "hours"),
            Some(("hours".into(), "Closed on state holidays".into()))
        );
        assert_eq!(split_statement("key:  ", "hours"), None);
    }

    #[test]
    fn widening_stops_at_sentences_and_char_boundaries() {
        let text =
            "Ünïcode first. The fee is $15 per document, by check or money order. Next.\nMore."
                .as_bytes();
        let s = std::str::from_utf8(text).unwrap();
        let start = s.find("$15").unwrap();
        assert_eq!(
            widen(text, start, start + 3).as_deref(),
            Some("The fee is $15 per document, by check or money order.")
        );
        assert_eq!(widen(text, 1, 3), None, "not a char boundary");
        assert_eq!(widen(text, 5, 4), None);
    }
}
