//! `xtask fetch-slug`: source-driven sweep for one authority (Slice B).
//!
//! Staff-triggered, manual for v1: no timer/scheduler. Reads the
//! packs-loaded website `source` rows for `--slug` and polls each
//! through the shared [`ingest::sweep`] core (robots gate, conditional
//! fetch, change stack, failure tasks). One bad URL never aborts the
//! sweep. See `watch-sweep` for the cadence-driven all-authorities run.

use crate::{flag, open_db, open_store, required, unix_now};

const DEFAULT_LIMIT: i64 = 25;
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
    let limit: i64 = match flag(args, "limit") {
        None => DEFAULT_LIMIT,
        Some(s) => s.parse().unwrap_or_else(|_| {
            eprintln!("fetch-slug: bad --limit (positive integer)");
            std::process::exit(2);
        }),
    };
    let now = unix_now();
    let rows = ingest::sweep::authority_sources(&conn, &slug, limit).unwrap_or_else(|e| {
        eprintln!("fetch-slug: database error: {e}");
        std::process::exit(1);
    });
    let (mut ok, mut dup, mut fresh, mut neardup, mut failed, mut skipped) = (0, 0, 0, 0, 0, 0);
    for row in &rows {
        match ingest::sweep::poll_source(&conn, &store, row, &trust, &actor, now, FETCH_TIMEOUT) {
            ingest::sweep::SourceOutcome::Created { capture_id } => {
                println!("fetched: {} capture={capture_id}", row.uri);
                ok += 1;
            }
            ingest::sweep::SourceOutcome::Duplicate { capture_id } => {
                println!("dedup: {} capture={capture_id}", row.uri);
                dup += 1;
            }
            ingest::sweep::SourceOutcome::NotModified => {
                println!("unmodified: {}", row.uri);
                fresh += 1;
            }
            ingest::sweep::SourceOutcome::NearDuplicate => {
                println!("near-dup: {}", row.uri);
                neardup += 1;
            }
            ingest::sweep::SourceOutcome::Failed { reason } => {
                println!("failed: {} ({reason})", row.uri);
                failed += 1;
            }
            ingest::sweep::SourceOutcome::Skipped { reason } => {
                println!("skipped ({reason}): {}", row.uri);
                skipped += 1;
            }
        }
    }
    println!(
        "fetch-slug: {slug} urls={} fetched={ok} dedup={dup} unmodified={fresh} \
         near-dup={neardup} failed={failed} skipped={skipped}",
        rows.len()
    );
}
