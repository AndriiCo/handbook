//! `xtask watch-sweep` + `xtask hcch-poll`: scheduled watch (B5/B10).
//!
//! Thin CLI over [`ingest::sweep`]: enumerate due website sources by
//! cadence (or poll the HCCH status table), print one line per source
//! plus a summary. One bad URL never aborts the sweep; fetch failures
//! land as `validate_fact` tasks inside the core.

use crate::{flag, open_db, open_store, unix_now};

const DEFAULT_LIMIT: i64 = 25;
const FETCH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

fn trust_flag(args: &[String], cmd: &str) -> String {
    let trust = flag(args, "trust").unwrap_or_else(|| "official".to_string());
    if !matches!(trust.as_str(), "official" | "staff_verified" | "community") {
        eprintln!("{cmd}: bad --trust (official|staff_verified|community)");
        std::process::exit(2);
    }
    trust
}

fn limit_flag(args: &[String], cmd: &str) -> i64 {
    match flag(args, "limit") {
        None => DEFAULT_LIMIT,
        Some(s) => s.parse().unwrap_or_else(|_| {
            eprintln!("{cmd}: bad --limit (positive integer)");
            std::process::exit(2);
        }),
    }
}

pub fn run_sweep(args: &[String]) {
    let store = open_store(args);
    let conn = open_db(&store);
    if let Err(e) = handbook_store::db::migrate(&conn) {
        eprintln!("migrate: {e}");
        std::process::exit(1);
    }
    let actor = flag(args, "actor").unwrap_or_else(|| "xtask".to_string());
    let trust = trust_flag(args, "watch-sweep");
    let limit = limit_flag(args, "watch-sweep");
    let now = unix_now();
    let rows = ingest::sweep::due_sources(&conn, now, limit).unwrap_or_else(|e| {
        eprintln!("watch-sweep: database error: {e}");
        std::process::exit(1);
    });
    let (mut created, mut dup, mut fresh, mut neardup, mut failed, mut skipped) =
        (0, 0, 0, 0, 0, 0);
    for row in &rows {
        match ingest::sweep::poll_source(&conn, &store, row, &trust, &actor, now, FETCH_TIMEOUT) {
            ingest::sweep::SourceOutcome::Created { capture_id } => {
                println!("fetched: {} capture={capture_id}", row.uri);
                created += 1;
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
        "watch-sweep: urls={} fetched={created} dedup={dup} unmodified={fresh} \
         near-dup={neardup} failed={failed} skipped={skipped}",
        rows.len()
    );
}

pub fn run_hcch(args: &[String]) {
    let store = open_store(args);
    let conn = open_db(&store);
    if let Err(e) = handbook_store::db::migrate(&conn) {
        eprintln!("migrate: {e}");
        std::process::exit(1);
    }
    let actor = flag(args, "actor").unwrap_or_else(|| "xtask".to_string());
    let now = unix_now();
    match ingest::sweep::poll_hcch(&conn, &store, &actor, now, FETCH_TIMEOUT) {
        Ok(ingest::sweep::HcchOutcome::NoSource) => {
            eprintln!(
                "hcch-poll: no website source under the hcch authority — seed the ONE \
                 status-table URI first (packs-load, then fetch-url from prod IPs)"
            );
            std::process::exit(2);
        }
        Ok(ingest::sweep::HcchOutcome::NotModified) => {
            println!("hcch-poll: unmodified");
        }
        Ok(ingest::sweep::HcchOutcome::Changed {
            capture_ids,
            task_id,
        }) => {
            let ids = capture_ids
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", #");
            println!("hcch-poll: changed captures=#{ids} task={task_id}");
        }
        Ok(ingest::sweep::HcchOutcome::Failed { reason }) => {
            eprintln!("hcch-poll: failed: {reason}");
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("hcch-poll: database error: {e}");
            std::process::exit(1);
        }
    }
}
