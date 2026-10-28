//! `xtask fetch-slug`: source-driven sweep for one authority (Slice B).
//!
//! Staff-triggered, manual for v1: no timer/scheduler. Reads the
//! packs-loaded website `source` rows for `--slug` and fetches each
//! through the same pipeline as `fetch-url` (robots gate, fetch,
//! extract, ingest, failure tasks). One bad URL never aborts the sweep.

use crate::{authority_id, flag, open_db, open_store, record_fetch_failure, required, unix_now};

const DEFAULT_LIMIT: usize = 25;
const FETCH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

pub fn run(args: &[String]) {
    let store = open_store(args);
    let conn = open_db(&store);
    if let Err(e) = handbook_store::db::migrate(&conn) {
        eprintln!("migrate: {e}");
        std::process::exit(1);
    }
    let slug = required(args, "slug");
    let actor = flag(args, "actor").unwrap_or_else(|| "xtask".to_string());
    let trust = flag(args, "trust").unwrap_or_else(|| "official".to_string());
    if !matches!(trust.as_str(), "official" | "staff_verified" | "community") {
        eprintln!("fetch-slug: bad --trust (official|staff_verified|community)");
        std::process::exit(2);
    }
    let limit: usize = match flag(args, "limit") {
        None => DEFAULT_LIMIT,
        Some(s) => s.parse().unwrap_or_else(|_| {
            eprintln!("fetch-slug: bad --limit (positive integer)");
            std::process::exit(2);
        }),
    };
    let authority = authority_id(&conn, &slug);
    let now = unix_now();
    let rows: Vec<(i64, String, String)> = conn
        .prepare(
            "SELECT id, kind, uri FROM source WHERE authority_id = ?1
             AND kind IN ('website_official', 'website_thirdparty')
             ORDER BY id LIMIT ?2",
        )
        .unwrap_or_else(|e| {
            eprintln!("fetch-slug: database error: {e}");
            std::process::exit(1);
        })
        .query_map(rusqlite::params![authority, limit as i64], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })
        .unwrap_or_else(|e| {
            eprintln!("fetch-slug: database error: {e}");
            std::process::exit(1);
        })
        .collect::<rusqlite::Result<_>>()
        .unwrap_or_else(|e| {
            eprintln!("fetch-slug: database error: {e}");
            std::process::exit(1);
        });
    let (mut ok, mut dup, mut failed, mut skipped) = (0, 0, 0, 0);
    for (source_id, kind, uri) in &rows {
        if !(uri.starts_with("http://") || uri.starts_with("https://")) {
            println!("skipped (not http): {uri}");
            skipped += 1;
            continue;
        }
        if kind == "website_thirdparty" {
            let verdict = ingest::fetch::check_robots(uri, FETCH_TIMEOUT);
            conn.execute(
                "UPDATE source SET robots = ?1 WHERE id = ?2",
                rusqlite::params![format!("{} at {now}", verdict.text), source_id],
            )
            .unwrap_or_else(|e| {
                eprintln!("fetch-slug: record robots verdict: {e}");
                std::process::exit(1);
            });
            if !verdict.allowed {
                record_fetch_failure(&conn, authority, &slug, uri, "robots disallow", &actor, now);
                println!("skipped (robots disallow): {uri}");
                skipped += 1;
                continue;
            }
        }
        let fetched = match ingest::fetch::fetch_url(uri, FETCH_TIMEOUT) {
            Ok(f) => f,
            Err(e) => {
                record_fetch_failure(&conn, authority, &slug, uri, &e.to_string(), &actor, now);
                println!("failed: {uri} ({e})");
                failed += 1;
                continue;
            }
        };
        let text = ingest::capture::extract_text(&fetched.bytes);
        let outcome = ingest::capture::ingest_bytes(
            &conn,
            &store,
            &ingest::capture::IngestInput {
                authority_id: authority,
                source_id: Some(*source_id),
                kind: "warc",
                trust: &trust,
                bytes: &fetched.bytes,
                text: &text,
                captured_at: now,
                actor: &actor,
                origin: uri,
            },
        )
        .unwrap_or_else(|e| {
            eprintln!("fetch-slug: ingest: {e}");
            std::process::exit(1);
        });
        match outcome {
            ingest::capture::IngestOutcome::Created { capture_id } => {
                println!("fetched: {uri} capture={capture_id}");
                ok += 1;
            }
            ingest::capture::IngestOutcome::Duplicate { capture_id } => {
                println!("dedup: {uri} capture={capture_id}");
                dup += 1;
            }
        }
    }
    println!(
        "fetch-slug: {slug} urls={} fetched={ok} dedup={dup} failed={failed} skipped={skipped}",
        rows.len()
    );
}
