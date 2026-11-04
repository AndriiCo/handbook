//! `xtask watch-sweep` + `xtask hcch-poll`: scheduled watch (B5/B10).
//!
//! Thin CLI over [`ingest::sweep`]: enumerate due website sources by
//! cadence (or poll the HCCH status table), print one line per source
//! plus a summary. One bad URL never aborts the sweep; fetch failures
//! land as `validate_fact` tasks inside the core.

use crate::{flag, open_db, open_store, unix_now};

// World scale: ~1,000 sources on mostly weekly cadence is ~150 due a
// day; the daily timer passes no --limit.
const DEFAULT_LIMIT: i64 = 150;
const FETCH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);
/// Gap between two requests to one host inside a sweep (the crawl's
/// politeness: 2 s, 10 s for hcch.net — every world pack cites it).
const HOST_GAP: std::time::Duration = std::time::Duration::from_secs(2);
const HCCH_GAP: std::time::Duration = std::time::Duration::from_secs(10);

/// `host` of an http(s) URI (lowercase, no port), or "" for anything else.
fn host_of(uri: &str) -> String {
    let rest = uri
        .strip_prefix("https://")
        .or_else(|| uri.strip_prefix("http://"))
        .unwrap_or("");
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = authority.rsplit('@').next().unwrap_or("");
    host.split(':').next().unwrap_or("").to_ascii_lowercase()
}

/// Sleep until `host` may be fetched again, then mark it fetched now.
fn wait_for_host(last: &mut std::collections::HashMap<String, std::time::Instant>, host: &str) {
    if host.is_empty() {
        return;
    }
    let gap = if host == "hcch.net" || host.ends_with(".hcch.net") {
        HCCH_GAP
    } else {
        HOST_GAP
    };
    if let Some(at) = last.get(host) {
        let since = at.elapsed();
        if since < gap {
            std::thread::sleep(gap - since);
        }
    }
    last.insert(host.to_string(), std::time::Instant::now());
}

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
    let mut last_fetch = std::collections::HashMap::new();
    // The sweep is always live (`refresh`), but every answer is kept in
    // the fetch cache with its manifest line, so any rerun or audit can
    // replay exactly what was downloaded.
    let cache = callcache::FetchCache::new(
        crate::call_cache::settings(args).fetch_dir,
        callcache::CacheMode::Refresh,
    );
    for row in &rows {
        wait_for_host(&mut last_fetch, &host_of(&row.uri));
        match ingest::sweep::poll_source_with(
            &conn,
            &store,
            row,
            &trust,
            &actor,
            now,
            FETCH_TIMEOUT,
            Some(&cache),
        ) {
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

pub fn run_mail(args: &[String]) {
    let store = open_store(args);
    let conn = open_db(&store);
    if let Err(e) = handbook_store::db::migrate(&conn) {
        eprintln!("migrate: {e}");
        std::process::exit(1);
    }
    let actor = flag(args, "actor").unwrap_or_else(|| "xtask".to_string());
    let (kek, _source) = sealed_config::load_kek().unwrap_or_else(|e| {
        eprintln!("mail-poll: {e}");
        std::process::exit(1);
    });
    let (cfg, warnings) = sealed_config::RuntimeConfig::load(&conn, &kek).unwrap_or_else(|e| {
        eprintln!("mail-poll: config: {e}");
        std::process::exit(1);
    });
    for warning in &warnings {
        eprintln!("mail-poll: config warning: {warning}");
    }
    let (Some(client_id), Some(client_secret), Some(refresh_token)) = (
        cfg.mail_client_id,
        cfg.mail_client_secret,
        cfg.mail_refresh_token,
    ) else {
        eprintln!(
            "mail-poll: mail.client_id/client_secret/refresh_token not set (admin config UI)"
        );
        std::process::exit(2);
    };
    let mut client = mail::client::MailClient::new(
        &cfg.mail_base_url,
        &cfg.mail_token_url,
        &client_id,
        client_secret.as_str(),
        refresh_token.as_str(),
    );
    let opts = mail::poll::PollOpts {
        actor,
        now: unix_now(),
        backoff_secs: vec![5, 20, 60],
        max_fetches: 200,
    };
    match mail::poll::poll(&conn, &store, &mut client, &opts) {
        Ok(report) => println!(
            "mail-poll: listed={} created={} suppressed={} watermark={} reseeded={}",
            report.listed, report.created, report.suppressed, report.watermark, report.reseeded
        ),
        Err(e) => {
            eprintln!("mail-poll: {e}");
            std::process::exit(1);
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_of_strips_scheme_userinfo_port_and_path() {
        assert_eq!(host_of("https://www.HCCH.net/en/x?sid=1"), "www.hcch.net");
        assert_eq!(host_of("http://u:p@example.gov:8080/a"), "example.gov");
        assert_eq!(host_of("ftp://example.com/x"), "");
    }

    #[test]
    fn wait_for_host_spaces_repeat_hosts_only() {
        let mut last = std::collections::HashMap::new();
        let start = std::time::Instant::now();
        wait_for_host(&mut last, "a.example");
        wait_for_host(&mut last, "b.example");
        assert!(start.elapsed() < HOST_GAP, "distinct hosts never wait");
        wait_for_host(&mut last, "a.example");
        assert!(start.elapsed() >= HOST_GAP, "a repeat host waits the gap");
    }
}
