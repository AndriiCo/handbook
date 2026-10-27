//! PII sweep: `xtask pii-sweep [--store <path>]`.
//!
//! Scans publishable surfaces — `fact.statement`, `capture.notes`,
//! and `text/*.txt` sidecars — for email addresses, phone-like digit
//! runs, and SSN-shaped numbers. Prints every hit as
//! `location: pattern` and exits 1 on any hit (fail closed: nothing
//! reaches publish with detectable PII). Real customer data must
//! never live in these tables; this is the tripwire.

use handbook_store::store::Store;
use regex::Regex;

/// One sweep hit: where, and which pattern matched (never the value).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    pub location: String,
    pub pattern: &'static str,
}

/// Scan the store. Returns all hits (empty = clean).
pub fn sweep(store: &Store) -> Result<Vec<Hit>, String> {
    let email = Regex::new(r"[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}").unwrap();
    let phone = Regex::new(r"\+?1?[-.\s()]*\d{3}[-.\s()]*\d{3}[-.\s()]*\d{4}").unwrap();
    let ssn = Regex::new(r"\b\d{3}-\d{2}-\d{4}\b").unwrap();
    let patterns = [(&email, "email"), (&phone, "phone"), (&ssn, "ssn")];

    let mut hits = Vec::new();
    let mut check = |location: String, text: &str| {
        for (re, name) in &patterns {
            if re.is_match(text) {
                hits.push(Hit {
                    location: location.clone(),
                    pattern: name,
                });
            }
        }
    };

    let conn = handbook_store::db::open(store).map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare("SELECT id, statement FROM fact")
        .map_err(|e| e.to_string())?;
    let facts: Vec<(i64, String)> = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<_>>()
        .map_err(|e| e.to_string())?;
    for (id, statement) in &facts {
        check(format!("fact#{id}"), statement);
    }
    let mut stmt = conn
        .prepare("SELECT id, notes FROM capture WHERE notes != ''")
        .map_err(|e| e.to_string())?;
    let notes: Vec<(i64, String)> = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<_>>()
        .map_err(|e| e.to_string())?;
    for (id, text) in &notes {
        check(format!("capture#{id}:notes"), text);
    }

    let text_dir = store.root().join("text");
    if text_dir.is_dir() {
        let mut files: Vec<_> = std::fs::read_dir(&text_dir)
            .map_err(|e| e.to_string())?
            .collect::<Result<_, _>>()
            .map_err(|e| e.to_string())?;
        files.sort_by_key(|e| e.file_name());
        for entry in files {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("txt") {
                continue;
            }
            let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
            check(
                format!("text/{}", path.file_name().unwrap().to_string_lossy()),
                &String::from_utf8_lossy(&bytes),
            );
        }
    }
    Ok(hits)
}

/// CLI entry: print hits, exit 1 on any hit.
pub fn run(args: &[String]) {
    let store = crate::open_store(args);
    match sweep(&store) {
        Ok(hits) if hits.is_empty() => println!("pii-sweep: clean"),
        Ok(hits) => {
            for hit in &hits {
                println!("{}: {}", hit.location, hit.pattern);
            }
            eprintln!("pii-sweep: {} hit(s) — refusing", hits.len());
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("pii-sweep: {e}");
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seeded(pii: bool) -> (tempfile::TempDir, Store) {
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
        // Fictional-only PII: example.invalid, 555 numbers, 000 SSN area.
        let statement = if pii {
            "call +1-555-010-0199 or clerk@example.invalid"
        } else {
            "PO Box 40220"
        };
        conn.execute(
            "INSERT INTO capture(authority_id, captured_at, kind, path,
                                 bytes_raw, bytes_stored, sha256, trust)
             VALUES (?1, 1, 'file', 'blob/0', 1, 1, ?2, 'staff_verified')",
            rusqlite::params![wa, "a".repeat(64)],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO fact(authority_id, capture_id, topic, statement,
                              status, created_by, created_at)
             VALUES (?1, 1, 'contacts', ?2, 'accepted', 'test', 1)",
            rusqlite::params![wa, statement],
        )
        .unwrap();
        (dir, store)
    }

    #[test]
    fn flags_email_and_phone() {
        let (_dir, store) = seeded(true);
        let hits = sweep(&store).unwrap();
        assert_eq!(hits.len(), 2);
        assert!(
            hits.iter()
                .any(|h| h.pattern == "email" && h.location == "fact#1")
        );
        assert!(
            hits.iter()
                .any(|h| h.pattern == "phone" && h.location == "fact#1")
        );
    }

    #[test]
    fn clean_store_has_no_hits() {
        let (_dir, store) = seeded(false);
        assert!(sweep(&store).unwrap().is_empty());
    }
}
