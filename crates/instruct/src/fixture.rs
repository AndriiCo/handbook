//! Washington-state demo fixture — SYNTHETIC, never production.
//!
//! Seeds a temp store with one state authority, three accepted
//! facts, and three triggers (email fee change, comment hours
//! note, transcript Q&A). Shared by the crate's fixture test and
//! `xtask instruct-demo` so the drill humans see is the drill
//! tests assert. Every string below is invented for the demo.

use rusqlite::Connection;

use crate::error::Result;

/// Demo authority slug.
pub const WA_SLUG: &str = "usa-washington-sos";

/// Seeded demo rows.
#[derive(Debug)]
pub struct WaDemo {
    pub authority_id: i64,
    pub capture_ids: Vec<i64>,
}

/// Fixture evidence texts (shared by the seeder and the span test).
pub const WA_EMAIL: &str = "Subject: Apostille fee update\n\nEffective March 1, 2026, the \
     apostille filing fee is $30 per document. All other fees unchanged.";
pub const WA_COMMENT: &str = "Visited Friday: the counter closes at 4pm on Fridays now, not 5pm. \
     Plan accordingly.";
pub const WA_QA: &str = "Q: Do I need an appointment for in-person apostille?\nA: Yes — as of \
     this month, in-person visits require an appointment booked online.";

/// Seed the WA demo into an open migrated store.
pub fn seed_wa_demo(conn: &Connection, store_root: &std::path::Path, now: i64) -> Result<WaDemo> {
    conn.execute(
        "INSERT INTO authority(kind, country, region, name, slug)
         VALUES ('state', 'United States', 'Washington', 'Washington Secretary of State', ?1)",
        [WA_SLUG],
    )?;
    let authority_id = conn.last_insert_rowid();
    // One seed capture owns the accepted facts (kind page).
    conn.execute(
        "INSERT INTO capture(authority_id, source_id, captured_at, kind, path, bytes_raw,
                              bytes_stored, sha256, trust)
         VALUES (?1, NULL, ?2, 'file', 'seed', 10, 10, 'seed', 'official')",
        rusqlite::params![authority_id, now],
    )?;
    let seed_capture = conn.last_insert_rowid();
    for (topic, statement) in [
        ("fees", "filing: $20 per apostille"),
        ("walkin", "hours: Mon-Fri 8:00am-5:00pm"),
        ("mailing", "address: PO Box 40220, Olympia WA 98504"),
    ] {
        conn.execute(
            "INSERT INTO fact(authority_id, capture_id, topic, statement, status,
                              citation_start, citation_end, created_by, created_at)
             VALUES (?1, ?2, ?3, ?4, 'accepted', -1, -1, 'fixture', ?5)",
            rusqlite::params![authority_id, seed_capture, topic, statement, now],
        )?;
    }
    let triggers: [(&str, &str, &str); 3] = [
        ("email", "official", WA_EMAIL),
        ("comment", "community", WA_COMMENT),
        ("transcript", "staff_verified", WA_QA),
    ];
    std::fs::create_dir_all(store_root.join("text")).map_err(crate::error::Error::Io)?;
    let mut capture_ids = Vec::new();
    for (kind, trust, text) in triggers {
        conn.execute(
            "INSERT INTO capture(authority_id, source_id, captured_at, kind, path, bytes_raw,
                                  bytes_stored, sha256, trust)
             VALUES (?1, NULL, ?2, ?3, 'demo', ?4, ?4, 'demo', ?5)",
            rusqlite::params![authority_id, now, kind, text.len() as i64, trust],
        )?;
        let id = conn.last_insert_rowid();
        std::fs::write(store_root.join("text").join(format!("{id}.txt")), text)
            .map_err(crate::error::Error::Io)?;
        capture_ids.push(id);
    }
    Ok(WaDemo {
        authority_id,
        capture_ids,
    })
}

/// Scripted model replies for the demo: draft sections, then one
/// Friday-hours override. Citations index the fixture texts above
/// (spans verified by the fixture test, not trusted blind).
pub fn wa_script(capture_ids: &[i64]) -> Vec<String> {
    let email = capture_ids[0];
    let comment = capture_ids[1];
    let qa = capture_ids[2];
    vec![
        serde_json::json!({
            "sections": [
                {"id": "fees", "topic": "fees", "heading": "Fees",
                 "body": "The apostille filing fee is $30 per document effective March 1, 2026.",
                 "statements": [{"field": "filing", "value": "$30 per document"}],
                 "citations": [{"capture": email, "start": 0, "end": 40}]},
                {"id": "walkin", "topic": "walkin", "heading": "Walk-in hours",
                 "body": "Walk-in counter hours are Mon-Fri 8:00am-5:00pm.",
                 "statements": [{"field": "hours", "value": "Mon-Fri 8:00am-5:00pm"}],
                 "citations": [{"capture": comment, "start": 0, "end": 20}]},
                {"id": "appointment", "topic": "appointment", "heading": "Appointments",
                 "body": "In-person visits require an appointment booked online.",
                 "statements": [{"field": "in_person", "value": "appointment required"}],
                 "citations": [{"capture": qa, "start": 0, "end": 30}]},
            ],
        })
        .to_string(),
        serde_json::json!({
            "overrides": [
                {"section": "walkin",
                 "refined_body": "Walk-in counter hours are Mon-Thu 8:00am-5:00pm and Fri 8:00am-4:00pm.",
                 "refined_statements": [
                     {"field": "hours", "value": "Mon-Thu 8:00am-5:00pm, Fri 8:00am-4:00pm"},
                 ],
                 "reason": "comment evidence: Friday counter closes at 4pm"},
            ],
        })
        .to_string(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn script_spans_fit_fixture_texts() {
        // Spans must be in-bounds; the worker validates this live,
        // but the fixture pins it too so drift fails fast.
        for (text, end) in [(WA_EMAIL, 40), (WA_COMMENT, 20), (WA_QA, 30)] {
            assert!(text.len() >= end);
            assert!(text.get(0..end).is_some());
        }
    }
}
