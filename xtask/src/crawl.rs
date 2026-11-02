//! `xtask crawl`: polite world-pack crawl into WARC/1.1 + run CSVs.
//!
//! Usage: `cargo xtask crawl --packs packs/world --out archive/<run>/`
//!
//! Seeds come from `packs/world/<ISO2>.json` `watch[].uri` entries
//! (SA-7 world index; parsed defensively — every field is optional
//! with a default because SA-7 is still iterating). Each seed is
//! fetched at depth 0; same-host links whose URL matches
//! `apostille|legaliz|authentic|fee|form` are fetched at depth 1.
//!
//! Policy (operator spec + `docs/ops/world-archive-rights.md`):
//!
//! - UA `handbook-crawl/0.1 (+https://andrii.ai/apostille)`;
//! - robots FAILS CLOSED per URL (unreachable robots.txt skips the
//!   fetch), logged to `robots.csv`;
//! - `businessexpress.ny.gov` and `egypt-nyc.com` (and subdomains) are
//!   always skipped (`Disallow: /` per the rights review), logged as
//!   `disallowed-static`;
//! - 1 request / 2 s per host; `hcch.net` floors at 10 s; a robots
//!   `Crawl-delay` raises the floor;
//! - HTTP 429 honours `Retry-After` (once per URL, in the fetch core);
//! - up to 32 host worker threads; 25 MiB body cap; at most 50
//!   depth-1 URLs per page; redirect chains recorded hop by hop
//!   (see `ingest::warc`);
//! - WARC files rotate at 1 GB to `warc/<ISO2>/<slug>-NNNNN.warc.gz`.
//!
//! Run layout (`--out archive/<run>/`):
//!
//! - `warc/<ISO2>/<slug>-NNNNN.warc.gz` — the captures;
//! - `robots.csv` — every crawl-time robots verdict;
//! - `seeds.json` — one object per seed/depth-1 URL (status, bytes,
//!   record pointers; the input to `archive-package`);
//! - `t1-urls.txt` — deduped depth-0 seeds + discovered depth-1 URLs
//!   for the operator's wget T0–T1 pass (wget re-enforces robots=on).
//!
//! `robots.csv` columns: `url,host,verdict,reason,crawl_delay_s,
//! checked_at,source` where `verdict` is `allowed|disallowed`,
//! `reason` is the `ingest::warc` verdict text, and `source` is
//! `robots-txt|static-denylist`.
//!
//! No live crawling in tests: fixtures are loopback servers only.

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::{flag, required, unix_now};

/// Seconds between same-host requests (spec default).
const POLITENESS_SECS: u64 = 2;
/// Stricter floor for hcch.net (rights review: Crawl-delay 10).
const HCCH_MIN_DELAY_SECS: u64 = 10;
/// Hosts from the rights review with `Disallow: /` (suffix match).
const STATIC_DENY_SUFFIXES: &[&str] = &["businessexpress.ny.gov", "egypt-nyc.com"];
/// Host worker threads.
const MAX_HOSTS_PARALLEL: usize = 32;
/// Depth-1 URL cap per seed page.
const MAX_DEPTH1_PER_PAGE: usize = 50;
/// WARC file rotation size (1 GB).
const MAX_WARC_BYTES: u64 = 1_000_000_000;
/// Depth-1 link keywords (spec: apostille|legaliz|authentic|fee|form).
const DEPTH1_KEYWORDS: &[&str] = &["apostille", "legaliz", "authentic", "fee", "form"];
/// Whole-call timeout per fetch / TLS probe.
const FETCH_TIMEOUT: Duration = Duration::from_secs(30);
/// Timeout for robots.txt fetches.
const ROBOTS_TIMEOUT: Duration = Duration::from_secs(15);

/// Defensive SA-7 pack shape: every field optional (SA-7 iterates).
#[derive(serde::Deserialize, Default)]
struct WorldPack {
    #[serde(default)]
    pack: Option<String>,
    #[serde(default)]
    authorities: Vec<PackAuthority>,
    #[serde(default)]
    watch: Vec<PackWatch>,
}

/// Defensive authority entry (only `slug` + `name` are consumed).
#[derive(serde::Deserialize, Default)]
struct PackAuthority {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    slug: Option<String>,
}

/// Defensive watch entry (only `authority` + `uri` are consumed).
#[derive(serde::Deserialize, Default)]
struct PackWatch {
    #[serde(default)]
    authority: Option<String>,
    #[serde(default)]
    uri: Option<String>,
}

/// One crawl outcome (a `seeds.json` row; also feeds `authorities.csv`).
#[derive(serde::Serialize, serde::Deserialize, Clone)]
struct SeedOutcome {
    pack: String,
    slug: String,
    authority: String,
    uri: String,
    depth: u8,
    robots_verdict: String,
    robots_reason: String,
    status: String,
    http_status: u16,
    bytes: u64,
    warc_file: String,
    record_id: String,
    note: String,
    fetched_at: String,
}

/// One `robots.csv` row.
#[derive(Clone)]
struct RobotsRow {
    url: String,
    host: String,
    verdict: String,
    reason: String,
    crawl_delay_s: u64,
    checked_at: i64,
    source: String,
}

struct Seed {
    pack: String,
    slug: String,
    authority: String,
    uri: String,
}

pub fn run(args: &[String]) {
    let packs_dir = flag(args, "packs").unwrap_or_else(|| "packs/world".to_string());
    let out_dir = required(args, "out");
    let started = Instant::now();
    let out = std::path::PathBuf::from(&out_dir);

    let mut pack_files = read_pack_dir(&packs_dir);
    pack_files.sort();
    if pack_files.is_empty() {
        eprintln!("crawl: no pack files in {packs_dir}");
        std::process::exit(1);
    }

    // Depth-0 seeds from every pack's watch list.
    let mut seeds: Vec<Seed> = Vec::new();
    let mut names: HashMap<String, String> = HashMap::new();
    let mut parties = 0;
    for file in &pack_files {
        let stem = std::path::Path::new(file)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("xx");
        let iso2 = stem.to_ascii_uppercase();
        let text = std::fs::read_to_string(file).unwrap_or_else(|e| {
            eprintln!("crawl: read {}: {e}", file.display());
            std::process::exit(1);
        });
        let pack: WorldPack = serde_json::from_str(&text).unwrap_or_else(|e| {
            eprintln!("crawl: parse {}: {e}", file.display());
            std::process::exit(1);
        });
        parties += 1;
        for authority in &pack.authorities {
            if let Some(slug) = authority.slug.clone().filter(|s| !s.trim().is_empty()) {
                names.insert(slug, authority.name.clone().unwrap_or_default());
            }
        }
        for (index, watch) in pack.watch.iter().enumerate() {
            let slug = watch
                .authority
                .clone()
                .filter(|s| !s.trim().is_empty())
                .unwrap_or_else(|| format!("{}-w{}", stem.to_ascii_lowercase(), index + 1));
            let authority = names.get(&slug).cloned().unwrap_or_default();
            seeds.push(Seed {
                pack: pack.pack.clone().unwrap_or_else(|| iso2.clone()),
                slug,
                authority,
                uri: watch.uri.clone().unwrap_or_default().trim().to_string(),
            });
        }
    }

    let shared = Shared::new(&out);
    // Depth-0 outcomes (missing/denylisted decided inline; the rest
    // are grouped by host for the parallel crawl).
    let mut by_host: HashMap<String, Vec<Seed>> = HashMap::new();
    let mut outcomes: Vec<SeedOutcome> = Vec::new();
    for seed in seeds {
        let uri = seed.uri.clone();
        if uri.is_empty() || ingest::warc::split_url(&uri).is_none() {
            outcomes.push(missing_outcome(&seed, 0, "empty-or-unparseable-uri"));
            continue;
        }
        let host = ingest::warc::split_url(&uri).expect("checked").host;
        if static_denied(&host) {
            let row = RobotsRow {
                url: uri.clone(),
                host,
                verdict: "disallowed".to_string(),
                reason: "disallowed-static".to_string(),
                crawl_delay_s: 0,
                checked_at: unix_now(),
                source: "static-denylist".to_string(),
            };
            shared.push_robots(row.clone());
            outcomes.push(disallowed_outcome(
                &seed,
                0,
                &row,
                "static-denylist: Disallow: /",
            ));
            continue;
        }
        by_host.entry(host).or_default().push(seed);
    }

    // Crawl host groups, 32 hosts at a time.
    let mut hosts: Vec<(String, Vec<Seed>)> = by_host.into_iter().collect();
    hosts.sort_by(|a, b| a.0.cmp(&b.0));
    for chunk in hosts.chunks(MAX_HOSTS_PARALLEL) {
        std::thread::scope(|scope| {
            for (_, group) in chunk {
                scope.spawn(|| crawl_host(group, &shared));
            }
        });
    }

    outcomes.extend(shared.take_outcomes());
    outcomes.sort_by(|a, b| {
        (&a.pack, &a.slug, a.depth, &a.uri).cmp(&(&b.pack, &b.slug, b.depth, &b.uri))
    });

    // t1-urls.txt: deduped depth-0 seeds + discovered depth-1 URLs.
    let mut t1: Vec<String> = Vec::new();
    let mut seen_urls: HashSet<&str> = HashSet::new();
    for outcome in &outcomes {
        if outcome.uri.is_empty() {
            continue;
        }
        if seen_urls.insert(outcome.uri.as_str()) {
            t1.push(outcome.uri.clone());
        }
    }

    std::fs::create_dir_all(&out).unwrap_or_else(|e| {
        eprintln!("crawl: create {out_dir}: {e}");
        std::process::exit(1);
    });
    write_robots_csv(&out.join("robots.csv"), &shared.take_robots());
    write_json(&out.join("seeds.json"), &outcomes);
    std::fs::write(out.join("t1-urls.txt"), t1.join("\n") + "\n").unwrap_or_else(|e| {
        eprintln!("crawl: write t1-urls.txt: {e}");
        std::process::exit(1);
    });
    shared.finish_writers();

    let (mut ok, mut failed, mut disallowed, mut missing) = (0, 0, 0, 0);
    let mut bytes = 0u64;
    for outcome in &outcomes {
        match outcome.status.as_str() {
            "ok" => {
                ok += 1;
                bytes += outcome.bytes;
            }
            "failed" => failed += 1,
            "disallowed" => disallowed += 1,
            _ => missing += 1,
        }
    }
    println!(
        "crawl: parties={parties} authorities={} urls={} ok={ok} failed={failed} \
         disallowed={disallowed} missing={missing} bytes={bytes} urls_txt={} elapsed_s={} out={out_dir}",
        names.len(),
        outcomes.len(),
        t1.len(),
        started.elapsed().as_secs(),
    );
}

fn read_pack_dir(dir: &str) -> Vec<std::path::PathBuf> {
    let entries = std::fs::read_dir(dir).unwrap_or_else(|e| {
        eprintln!("crawl: read packs dir {dir}: {e}");
        std::process::exit(1);
    });
    entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("json"))
        .collect()
}

fn missing_outcome(seed: &Seed, depth: u8, note: &str) -> SeedOutcome {
    SeedOutcome {
        pack: seed.pack.clone(),
        slug: seed.slug.clone(),
        authority: seed.authority.clone(),
        uri: seed.uri.clone(),
        depth,
        robots_verdict: "uncheckable".to_string(),
        robots_reason: note.to_string(),
        status: "missing".to_string(),
        http_status: 0,
        bytes: 0,
        warc_file: String::new(),
        record_id: String::new(),
        note: note.to_string(),
        fetched_at: warc_now(),
    }
}

fn disallowed_outcome(seed: &Seed, depth: u8, row: &RobotsRow, note: &str) -> SeedOutcome {
    SeedOutcome {
        pack: seed.pack.clone(),
        slug: seed.slug.clone(),
        authority: seed.authority.clone(),
        uri: seed.uri.clone(),
        depth,
        robots_verdict: "disallowed".to_string(),
        robots_reason: row.reason.clone(),
        status: "disallowed".to_string(),
        http_status: 0,
        bytes: 0,
        warc_file: String::new(),
        record_id: String::new(),
        note: note.to_string(),
        fetched_at: warc_now(),
    }
}

fn warc_now() -> String {
    ingest::warc::now_warc_date()
}

/// True for the rights-review `Disallow: /` hosts (exact or subdomain).
fn static_denied(host: &str) -> bool {
    STATIC_DENY_SUFFIXES
        .iter()
        .any(|suffix| host == *suffix || host.ends_with(&format!(".{suffix}")))
}

/// HCCH hosts floor the politeness delay at 10 s (rights review).
fn host_floor_secs(host: &str) -> u64 {
    if host == "hcch.net" || host.ends_with(".hcch.net") {
        HCCH_MIN_DELAY_SECS
    } else {
        POLITENESS_SECS
    }
}

/// Shared crawl state behind one mutex each (hosts only meet here).
struct Shared {
    out: std::path::PathBuf,
    robots: Mutex<Vec<RobotsRow>>,
    outcomes: Mutex<Vec<SeedOutcome>>,
    writers: Mutex<HashMap<(String, String), WriterSlot>>,
    fetched: Mutex<HashMap<String, SeedOutcome>>,
}

struct WriterSlot {
    rel: String,
    writer: ingest::warc::WarcWriter,
    counter: u32,
}

impl Shared {
    fn new(out: &std::path::Path) -> Self {
        Self {
            out: out.to_path_buf(),
            robots: Mutex::new(Vec::new()),
            outcomes: Mutex::new(Vec::new()),
            writers: Mutex::new(HashMap::new()),
            fetched: Mutex::new(HashMap::new()),
        }
    }

    fn push_robots(&self, row: RobotsRow) {
        lock(&self.robots).push(row);
    }

    fn push_outcome(&self, outcome: SeedOutcome) {
        lock(&self.outcomes).push(outcome);
    }

    fn take_outcomes(&self) -> Vec<SeedOutcome> {
        std::mem::take(&mut *lock(&self.outcomes))
    }

    fn take_robots(&self) -> Vec<RobotsRow> {
        let mut rows = std::mem::take(&mut *lock(&self.robots));
        rows.sort_by(|a, b| (&a.url, a.checked_at).cmp(&(&b.url, b.checked_at)));
        rows
    }

    /// Record one fetch's request/response/metadata triple, rotating
    /// the per-(pack, slug) WARC file at 1 GB. Returns the relative
    /// warc path and the metadata record id.
    fn archive_fetch(
        &self,
        pack: &str,
        slug: &str,
        fetch: &ingest::warc::FetchResult,
        robots_text: &str,
        depth: u8,
    ) -> (String, String) {
        let mut writers = lock(&self.writers);
        let key = (pack.to_string(), slug.to_string());
        let slot = writers
            .entry(key)
            .or_insert_with(|| open_slot(&self.out, pack, slug, 1));
        if slot.writer.bytes_written() >= MAX_WARC_BYTES {
            let next = slot.counter + 1;
            let finished = std::mem::replace(slot, open_slot(&self.out, pack, slug, next));
            let _ = finished.writer.finish();
        }
        let mut meta_id = String::new();
        for (index, hop) in fetch.hops.iter().enumerate() {
            let request = slot
                .writer
                .write_request(&hop.url, &hop.request_bytes, None)
                .expect("write request");
            let response = slot
                .writer
                .write_response(
                    &hop.url,
                    &hop.response_bytes(),
                    &hop.body,
                    Some(&request.id),
                )
                .expect("write response");
            let tls_chain = fetch.tls_chain.join(",");
            let meta = slot
                .writer
                .write_metadata(
                    &hop.url,
                    &[
                        ("slug", slug),
                        ("pack", pack),
                        ("depth", &depth.to_string()),
                        ("hop", &format!("{index}/{}", fetch.hops.len() - 1)),
                        ("robots", robots_text),
                        ("tls", &fetch.tls_note),
                        ("tls-chain", &tls_chain),
                        ("http-status", &hop.status.to_string()),
                    ],
                    Some(&response.id),
                )
                .expect("write metadata");
            if index == fetch.hops.len() - 1 {
                meta_id = meta.id.clone();
            }
        }
        (slot.rel.clone(), meta_id)
    }

    fn finish_writers(&self) {
        let mut writers = lock(&self.writers);
        for (_, slot) in writers.drain() {
            let _ = slot.writer.finish();
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

fn open_slot(out: &std::path::Path, pack: &str, slug: &str, counter: u32) -> WriterSlot {
    let iso2 = pack.to_ascii_uppercase();
    let file = format!("{slug}-{counter:05}.warc.gz");
    let rel = format!("warc/{iso2}/{file}");
    let path = out.join(&rel);
    let mut writer = ingest::warc::WarcWriter::create(&path).unwrap_or_else(|e| {
        eprintln!("crawl: create {}: {e}", path.display());
        std::process::exit(1);
    });
    writer
        .write_warcinfo(&[
            ("filename", &file),
            ("operator", "handbook-crawl/0.1"),
            ("pack", pack),
        ])
        .unwrap_or_else(|e| {
            eprintln!("crawl: warcinfo {}: {e}", path.display());
            std::process::exit(1);
        });
    WriterSlot {
        rel,
        writer,
        counter,
    }
}

/// Crawl one host's seeds: robots check, politeness sleep, fetch,
/// archive, then depth-1 same-host keyword links.
fn crawl_host(seeds: &[Seed], shared: &Shared) {
    let host = ingest::warc::split_url(&seeds[0].uri)
        .expect("grouped by parseable host")
        .host;
    let mut interval = host_floor_secs(&host);
    let mut last: Option<Instant> = None;
    for seed in seeds {
        // One robots verdict per seed URL (fail closed, CSV-logged).
        let verdict = ingest::warc::check_robots_closed(&seed.uri, ROBOTS_TIMEOUT);
        interval = interval.max(verdict.crawl_delay_secs);
        let row = RobotsRow {
            url: seed.uri.clone(),
            host: host.clone(),
            verdict: if verdict.allowed {
                "allowed".to_string()
            } else {
                "disallowed".to_string()
            },
            reason: verdict.text.clone(),
            crawl_delay_s: verdict.crawl_delay_secs,
            checked_at: unix_now(),
            source: "robots-txt".to_string(),
        };
        shared.push_robots(row.clone());
        if !verdict.allowed {
            shared.push_outcome(disallowed_outcome(
                seed,
                0,
                &row,
                "robots disallow (fail closed)",
            ));
            continue;
        }
        polite_wait(&mut last, interval);
        let outcome = fetch_and_archive(seed, 0, &verdict.text, shared);
        last = Some(Instant::now());
        let links = match &outcome.links {
            Some(links) => links.clone(),
            None => {
                shared.push_outcome(outcome.outcome);
                continue;
            }
        };
        shared.push_outcome(outcome.outcome);
        // Depth 1: same-host keyword links only.
        for link in links.into_iter().take(MAX_DEPTH1_PER_PAGE) {
            crawl_depth1(seed, &link, shared, &mut last, &mut interval);
        }
    }
}

struct FetchOutcome {
    outcome: SeedOutcome,
    /// Depth-1 candidates (only for archived HTML pages at depth 0).
    links: Option<Vec<String>>,
}

/// Fetch one URL and archive its hops; never panics on fetch errors.
fn fetch_and_archive(seed: &Seed, depth: u8, robots_text: &str, shared: &Shared) -> FetchOutcome {
    // Exact-duplicate URLs reuse the earlier capture (politeness: no
    // refetch; the outcome still points at real records).
    if let Some(prior) = lock(&shared.fetched).get(&seed.uri).cloned() {
        let mut outcome = prior;
        outcome.slug = seed.slug.clone();
        outcome.pack = seed.pack.clone();
        outcome.authority = seed.authority.clone();
        outcome.depth = depth;
        outcome.note = format!("reused-capture {}", outcome.record_id);
        return FetchOutcome {
            outcome,
            links: None,
        };
    }
    let fetched = match ingest::warc::fetch_with_hops(&seed.uri, FETCH_TIMEOUT) {
        Err(e) => {
            return FetchOutcome {
                outcome: SeedOutcome {
                    pack: seed.pack.clone(),
                    slug: seed.slug.clone(),
                    authority: seed.authority.clone(),
                    uri: seed.uri.clone(),
                    depth,
                    robots_verdict: "allowed".to_string(),
                    robots_reason: robots_text.to_string(),
                    status: "failed".to_string(),
                    http_status: 0,
                    bytes: 0,
                    warc_file: String::new(),
                    record_id: String::new(),
                    note: format!("fetch: {e}"),
                    fetched_at: warc_now(),
                },
                links: None,
            };
        }
        Ok(fetched) => fetched,
    };
    let (warc_file, record_id) =
        shared.archive_fetch(&seed.pack, &seed.slug, &fetched, robots_text, depth);
    let last = fetched.hops.last().expect("fetch has hops");
    let bytes: u64 = fetched.hops.iter().map(|h| h.body.len() as u64).sum();
    let ok = (200..300).contains(&last.status);
    let links = if depth == 0 && ok && is_html(&last.response_head, &last.body) {
        Some(depth1_links(&seed.uri, &last.body))
    } else {
        None
    };
    let outcome = SeedOutcome {
        pack: seed.pack.clone(),
        slug: seed.slug.clone(),
        authority: seed.authority.clone(),
        uri: seed.uri.clone(),
        depth,
        robots_verdict: "allowed".to_string(),
        robots_reason: robots_text.to_string(),
        status: if ok {
            "ok".to_string()
        } else {
            "failed".to_string()
        },
        http_status: last.status,
        bytes,
        warc_file,
        record_id: record_id.clone(),
        note: if ok {
            String::new()
        } else {
            format!("http {}", last.status)
        },
        fetched_at: warc_now(),
    };
    lock(&shared.fetched).insert(seed.uri.clone(), outcome.clone());
    FetchOutcome { outcome, links }
}

fn crawl_depth1(
    seed: &Seed,
    link: &str,
    shared: &Shared,
    last: &mut Option<Instant>,
    interval: &mut u64,
) {
    let parts = match ingest::warc::split_url(link) {
        Some(parts) => parts,
        None => return,
    };
    let seed_host = ingest::warc::split_url(&seed.uri).expect("seed host").host;
    if parts.host != seed_host || static_denied(&parts.host) {
        return;
    }
    let verdict = ingest::warc::check_robots_closed(link, ROBOTS_TIMEOUT);
    *interval = (*interval).max(verdict.crawl_delay_secs);
    let row = RobotsRow {
        url: link.to_string(),
        host: parts.host.clone(),
        verdict: if verdict.allowed {
            "allowed".to_string()
        } else {
            "disallowed".to_string()
        },
        reason: verdict.text.clone(),
        crawl_delay_s: verdict.crawl_delay_secs,
        checked_at: unix_now(),
        source: "robots-txt".to_string(),
    };
    shared.push_robots(row.clone());
    if !verdict.allowed {
        shared.push_outcome(disallowed_outcome(
            &Seed {
                pack: seed.pack.clone(),
                slug: seed.slug.clone(),
                authority: seed.authority.clone(),
                uri: link.to_string(),
            },
            1,
            &row,
            "robots disallow (fail closed)",
        ));
        return;
    }
    polite_wait(last, *interval);
    let child = Seed {
        pack: seed.pack.clone(),
        slug: seed.slug.clone(),
        authority: seed.authority.clone(),
        uri: link.to_string(),
    };
    let fetched = fetch_and_archive(&child, 1, &verdict.text, shared);
    *last = Some(Instant::now());
    shared.push_outcome(fetched.outcome);
}

/// Sleep until `interval` seconds passed since `last` request.
fn polite_wait(last: &mut Option<Instant>, interval: u64) {
    if let Some(when) = *last {
        let wait = Duration::from_secs(interval).saturating_sub(when.elapsed());
        if !wait.is_zero() {
            std::thread::sleep(wait);
        }
    }
}

/// True for HTML payloads (declared `*html*` content type, or no
/// content type with an HTML-looking body).
fn is_html(head: &[u8], body: &[u8]) -> bool {
    let text = String::from_utf8_lossy(head).to_ascii_lowercase();
    if text.contains("content-type:") {
        return text.contains("html");
    }
    let sniff: Vec<u8> = body
        .iter()
        .filter(|b| !b.is_ascii_whitespace())
        .take(64)
        .copied()
        .collect();
    let sniff = String::from_utf8_lossy(&sniff).to_ascii_lowercase();
    sniff.starts_with("<!doctypehtml") || sniff.starts_with("<html") || sniff.contains("<a")
}

/// Same-host keyword links of a seed page (deduped, capped by caller).
/// "Same host" compares the host string only: other ports on the same
/// host share its politeness budget and robots verdicts are still
/// checked per URL.
fn depth1_links(seed_url: &str, body: &[u8]) -> Vec<String> {
    let seed_host = ingest::warc::split_url(seed_url).expect("seed host").host;
    let mut out = Vec::new();
    for link in ingest::warc::extract_links(body, seed_url) {
        let Some(parts) = ingest::warc::split_url(&link) else {
            continue;
        };
        if parts.host != seed_host {
            continue;
        }
        let lowered = link.to_ascii_lowercase();
        if !DEPTH1_KEYWORDS.iter().any(|k| lowered.contains(k)) {
            continue;
        }
        if !out.contains(&link) {
            out.push(link);
        }
    }
    out
}

fn csv_escape(value: &str) -> String {
    if value.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}

fn write_robots_csv(path: &std::path::Path, rows: &[RobotsRow]) {
    let mut out = String::from("url,host,verdict,reason,crawl_delay_s,checked_at,source\n");
    for row in rows {
        out.push_str(&format!(
            "{},{},{},{},{},{},{}\n",
            csv_escape(&row.url),
            csv_escape(&row.host),
            csv_escape(&row.verdict),
            csv_escape(&row.reason),
            row.crawl_delay_s,
            row.checked_at,
            csv_escape(&row.source),
        ));
    }
    std::fs::write(path, out).unwrap_or_else(|e| {
        eprintln!("crawl: write {}: {e}", path.display());
        std::process::exit(1);
    });
}

fn write_json<T: serde::Serialize>(path: &std::path::Path, value: &T) {
    let text = serde_json::to_string_pretty(value).unwrap_or_else(|e| {
        eprintln!("crawl: encode {}: {e}", path.display());
        std::process::exit(1);
    });
    std::fs::write(path, text + "\n").unwrap_or_else(|e| {
        eprintln!("crawl: write {}: {e}", path.display());
        std::process::exit(1);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read as _, Write as _};
    use std::net::TcpListener;

    fn reply(status: &str, headers: &[(&str, &str)], body: &[u8]) -> Vec<u8> {
        let mut out = format!("{status}\r\nContent-Length: {}\r\n", body.len()).into_bytes();
        for (name, value) in headers {
            out.extend_from_slice(format!("{name}: {value}\r\n").as_bytes());
        }
        out.extend_from_slice(b"Connection: close\r\n\r\n");
        out.extend_from_slice(body);
        out
    }

    /// Bind loopback port 0; the port is known before serving, so
    /// page bodies can link to themselves.
    fn bind() -> (TcpListener, u16) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        let port = listener.local_addr().unwrap().port();
        (listener, port)
    }

    /// Serve `takes` connections on `listener`, each answered by
    /// `handler(request target)`.
    fn serve_on(
        listener: TcpListener,
        takes: usize,
        handler: impl Fn(&str) -> Vec<u8> + Send + 'static,
    ) -> std::thread::JoinHandle<()> {
        std::thread::spawn(move || {
            for _ in 0..takes {
                let (mut stream, _) = listener.accept().expect("accept");
                let mut buf = [0u8; 8192];
                let n = stream.read(&mut buf).unwrap_or(0);
                let head = String::from_utf8_lossy(&buf[..n]);
                let target = head
                    .lines()
                    .next()
                    .and_then(|line| line.split_whitespace().nth(1))
                    .unwrap_or("/");
                let response = handler(target);
                stream.write_all(&response).expect("write reply");
            }
        })
    }

    fn temp_case(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("crawl-test-{}-{name}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(dir.join("packs")).expect("case packs dir");
        dir
    }

    fn run_crawl(dir: &std::path::Path) {
        let args = vec![
            "crawl".to_string(),
            "--packs".to_string(),
            dir.join("packs").to_string_lossy().into_owned(),
            "--out".to_string(),
            dir.join("run").to_string_lossy().into_owned(),
        ];
        run(&args);
    }

    #[test]
    fn static_deny_and_floor_rules() {
        assert!(static_denied("businessexpress.ny.gov"));
        assert!(static_denied("www.businessexpress.ny.gov"));
        assert!(static_denied("egypt-nyc.com"));
        assert!(!static_denied("dos.ny.gov"));
        assert!(!static_denied("notbusinessexpress.ny.gov"));
        assert_eq!(host_floor_secs("www.hcch.net"), 10);
        assert_eq!(host_floor_secs("hcch.net"), 10);
        assert_eq!(host_floor_secs("example.gov"), 2);
    }

    #[test]
    fn depth1_link_policy() {
        let seed = "http://127.0.0.1:9/dir/page";
        let html = r#"<a href="http://127.0.0.1:9/apostille-fees">a</a>
               <a href="http://127.0.0.1:9/about">b</a>
               <a href="http://127.0.0.2:9/apostille">c</a>"#
            .to_string();
        let links = depth1_links(seed, html.as_bytes());
        assert_eq!(links, vec!["http://127.0.0.1:9/apostille-fees".to_string()]);
        assert!(is_html(
            b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\n\r\n",
            b"x"
        ));
        assert!(!is_html(
            b"HTTP/1.1 200 OK\r\nContent-Type: application/pdf\r\n\r\n",
            b"%PDF"
        ));
    }

    #[test]
    fn crawl_fixture_end_to_end() {
        let robots_open = reply("HTTP/1.0 200 OK", &[], b"User-agent: *\nDisallow: /admin\n");
        let robots_closed = reply("HTTP/1.0 200 OK", &[], b"User-agent: *\nDisallow: /\n");
        let not_found = reply("HTTP/1.0 404 Missing", &[], b"nope");

        // Seed host: robots + seed page + robots + depth-1 page.
        let (listener_a, port_a) = bind();
        let not_found_a = not_found.clone();
        let server_a = serve_on(listener_a, 4, move |target| match target {
            "/robots.txt" => robots_open.clone(),
            "/a" => {
                let page = format!(
                    "<html><body><a href=\"http://127.0.0.1:{port_a}/a-apostille-fees\">fees</a> \
                     <a href=\"http://127.0.0.1:{port_a}/about\">about</a></body></html>"
                );
                reply(
                    "HTTP/1.0 200 OK",
                    &[("Content-Type", "text/html")],
                    page.as_bytes(),
                )
            }
            "/a-apostille-fees" => reply(
                "HTTP/1.0 200 OK",
                &[("Content-Type", "text/html")],
                b"<html>fee schedule</html>",
            ),
            _ => not_found_a.clone(),
        });
        let (listener_b, port_b) = bind();
        let server_b = serve_on(listener_b, 1, move |target| match target {
            "/robots.txt" => robots_closed.clone(),
            _ => not_found.clone(),
        });

        let dir = temp_case("e2e");
        let pack = format!(
            r#"{{"pack": "XX", "version": 1, "extra_field": true,
                "authorities": [{{"slug": "fixture-ca-1", "name": "Fixture Authority"}}],
                "watch": [
                  {{"authority": "fixture-ca-1", "uri": "http://127.0.0.1:{port_a}/a"}},
                  {{"authority": "fixture-ca-1", "uri": "http://127.0.0.1:{port_b}/b"}}
                ]}}"#
        );
        std::fs::write(dir.join("packs/XX.json"), pack).expect("pack");
        run_crawl(&dir);

        // WARC output: one file under warc/XX/, records read back.
        let warc_dir = dir.join("run/warc/XX");
        let files: Vec<_> = std::fs::read_dir(&warc_dir)
            .expect("warc dir")
            .filter_map(|e| e.ok().map(|e| e.path()))
            .collect();
        assert_eq!(files.len(), 1, "files: {files:?}");
        assert!(
            files[0]
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("fixture-ca-1-")
        );
        let records = ingest::warc::read_records(&files[0]).expect("read warc");
        assert!(!records.is_empty());
        for record in &records {
            assert!(record.block_digest_ok(), "digest: {record:?}");
        }
        let metadata: Vec<_> = records
            .iter()
            .filter(|r| r.header("WARC-Type") == Some("metadata"))
            .collect();
        assert_eq!(metadata.len(), 2, "seed + depth-1 captures");
        for meta in &metadata {
            let block = String::from_utf8_lossy(&meta.block);
            assert!(block.contains("slug: fixture-ca-1"), "{block}");
            assert!(block.contains("pack: XX"), "{block}");
            assert!(block.contains("robots: allowed"), "{block}");
        }

        // seeds.json statuses.
        let seeds: Vec<SeedOutcome> = serde_json::from_str(
            &std::fs::read_to_string(dir.join("run/seeds.json")).expect("seeds.json"),
        )
        .expect("parse seeds");
        let by_uri: HashMap<&str, &SeedOutcome> =
            seeds.iter().map(|s| (s.uri.as_str(), s)).collect();
        let seed_a = format!("http://127.0.0.1:{port_a}/a");
        let seed_b = format!("http://127.0.0.1:{port_b}/b");
        assert_eq!(by_uri[seed_a.as_str()].status, "ok");
        assert_eq!(by_uri[seed_a.as_str()].depth, 0);
        assert!(!by_uri[seed_a.as_str()].warc_file.is_empty());
        assert_eq!(by_uri[seed_b.as_str()].status, "disallowed");
        let depth1 = format!("http://127.0.0.1:{port_a}/a-apostille-fees");
        assert_eq!(by_uri[depth1.as_str()].status, "ok");
        assert_eq!(by_uri[depth1.as_str()].depth, 1);

        // robots.csv + t1-urls.txt.
        let robots = std::fs::read_to_string(dir.join("run/robots.csv")).expect("robots.csv");
        assert!(robots.starts_with("url,host,verdict,reason,crawl_delay_s,checked_at,source\n"));
        assert!(!robots.contains(",static-denylist"));
        let t1 = std::fs::read_to_string(dir.join("run/t1-urls.txt")).expect("t1");
        assert!(t1.contains(&seed_a));
        assert!(t1.contains(&seed_b));
        assert!(t1.contains(&depth1));

        server_a.join().unwrap();
        server_b.join().unwrap();
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn static_denylist_needs_no_network() {
        let dir = temp_case("deny");
        let pack = r#"{"pack": "XX", "authorities": [],
            "watch": [{"authority": "xx-ca-1", "uri": "https://www.businessexpress.ny.gov/app/x"}]}"#;
        std::fs::write(dir.join("packs/XX.json"), pack).expect("pack");
        run_crawl(&dir);
        let seeds: Vec<SeedOutcome> = serde_json::from_str(
            &std::fs::read_to_string(dir.join("run/seeds.json")).expect("seeds.json"),
        )
        .expect("parse seeds");
        assert_eq!(seeds.len(), 1);
        assert_eq!(seeds[0].status, "disallowed");
        assert_eq!(seeds[0].robots_reason, "disallowed-static");
        let robots = std::fs::read_to_string(dir.join("run/robots.csv")).expect("robots.csv");
        assert!(robots.contains(",static-denylist"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn defensive_pack_parsing() {
        let dir = temp_case("defensive");
        // Unknown fields, missing uri, null authority, non-string slug.
        let pack = r#"{"pack": "XX", "future": {"nested": [1, 2]},
            "authorities": [{"name": "No Slug"}],
            "watch": [{}, {"authority": null}, {"authority": "x", "uri": "  "}]}"#;
        std::fs::write(dir.join("packs/XX.json"), pack).expect("pack");
        run_crawl(&dir);
        let seeds: Vec<SeedOutcome> = serde_json::from_str(
            &std::fs::read_to_string(dir.join("run/seeds.json")).expect("seeds.json"),
        )
        .expect("parse seeds");
        assert_eq!(seeds.len(), 3);
        assert!(seeds.iter().all(|s| s.status == "missing"));
        std::fs::remove_dir_all(&dir).ok();
    }
}
