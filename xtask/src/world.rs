//! `xtask world-index`: HCCH 1961 world index (SA-7).
//!
//! Builds one jurisdiction pack per Contracting Party from the public
//! HCCH status table (`cid=41`) plus every party's competent-authority
//! page (EN). The table and the index links parse deterministically;
//! the per-party `details3` pages parse with fixed structural rules:
//! `<em>` designations, `<br>`/`<p>` item segments split further on
//! `;` outside parentheses, and `General website` / `Useful Links`
//! URL rows. Segments the page itself disclaims (`...not competent
//! to issue apostilles`) and editor notes (`(courtesy translation)`)
//! never become names. Pages where those rules yield nothing are
//! *irregular*: with `--live-model` (and `META_API_KEY` in the
//! environment, never in the repo) a Meta Spark call extracts them,
//! otherwise they stay empty.
//!
//! Quote rule (the `analyze.rs` / `runner.rs` pattern, adapted to raw
//! HTML): every emitted name and URL must be contained in the cited
//! page after entity-unescaping, lowercasing and whitespace collapse.
//! Values that fail the check are dropped, never emitted.
//!
//! Output (all `needs_review`, never asserted facts):
//! `packs/world/<ISO2>.json` per party plus
//! `packs/world/archive/<run>/authorities.csv`.
//! Offline default reads the recorded fixtures in `evals/world`
//! (only parties with a recorded `authority-sid*.html` are processed);
//! `--live` fetches every party from `www.hcch.net` instead.
//!
//! Every live page fetch lands in the fetch cache and every Spark call
//! in the Meta call cache (`docs/ops/call-cache.md`): a rerun replays
//! both, and `--live --cache-mode replay-only` reruns fully offline.
//!
//! Wiring (builder): `mod world;` plus
//! `Some("world-index") => world::run(&args[1..]),`
//! in `xtask/src/main.rs`.

use std::path::Path;
use std::process::Command;
use std::time::Duration;

use callcache::{CacheMode, FetchCache, MetaCache};

use crate::{flag, unix_now};

/// The ONE status-table URI (the F2 feed).
pub(crate) const STATUS_TABLE_URL: &str =
    "https://www.hcch.net/en/instruments/conventions/status-table/?cid=41";
/// HCCH site root: authority/details hrefs are root-relative.
const HCCH_ORIGIN: &str = "https://www.hcch.net";
/// Identified crawl agent (review §13 politeness).
const CRAWL_UA: &str = "handbook-world-index/0.1 (+https://andrii.ai/apostille)";
/// Live fetch budget per request.
const FETCH_SECS: &str = "30";
/// One request per two seconds per host (review §13).
const POLITENESS: Duration = Duration::from_secs(2);
/// Default Model API root (same default as the analysis agent).
const META_BASE_URL: &str = "https://api.meta.ai/v1";
/// The only model this task may call (organizer rule).
const DRAFT_MODEL: &str = "muse-spark-1.3";
/// Live-call retry budget (error-handling docs: 3-5 with jitter).
const MAX_ATTEMPTS: u32 = 4;
/// Validation backlog ref for world-index observations/steps.
const WORLD_REF: &str = "W-7";

/// One status-table row: canonical party name, authority-page sid and
/// entry into force.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct PartyRow {
    pub(crate) name: String,
    pub(crate) sid: String,
    /// Entry into force (`YYYY-MM-DD`) off the EIF column (5th cell).
    /// A date in the future marks a state that has joined but cannot
    /// issue or receive apostilles yet. `None` when the cell is empty or
    /// unreadable.
    pub(crate) eif: Option<String>,
}

/// One `details3` link off an authority index page.
#[derive(Debug, Clone, PartialEq, Eq)]
struct DetailsLink {
    aid: String,
    title: String,
    url: String,
}

/// One extracted URL plus where it was listed: the `details3` page
/// (`page`) and, when the source names it (Spark output pairs URLs with
/// a candidate), the authority it belongs to. Never its position in a
/// list: positional pairing gave "Oberlandesgericht" the patent
/// office's site (P2-1).
#[derive(Debug, Clone, PartialEq, Eq)]
struct UrlHit {
    url: String,
    useful_link: bool,
    quote_ok: bool,
    page: String,
    name: Option<String>,
}

/// One extracted authority name plus its citation page.
#[derive(Debug, Clone, PartialEq, Eq)]
struct NameHit {
    name: String,
    source_url: String,
    quote_ok: bool,
}

/// Per-party roll-up for packs, CSV and the summary line.
#[derive(Debug, Clone)]
struct PartyIndex {
    party: String,
    iso2: String,
    /// Entry into force off the status table (`YYYY-MM-DD`).
    eif: Option<String>,
    authority_url: String,
    names: Vec<NameHit>,
    urls: Vec<UrlHit>,
    origin: &'static str,
}

pub fn run(args: &[String]) {
    let fixtures = flag(args, "fixtures").unwrap_or_else(|| "evals/world".to_string());
    let out = flag(args, "out").unwrap_or_else(|| "packs/world".to_string());
    let live = args.iter().any(|a| a == "--live");
    let live_model = args.iter().any(|a| a == "--live-model");
    let base_url = flag(args, "base-url").unwrap_or_else(|| META_BASE_URL.to_string());
    let run_id = flag(args, "run").unwrap_or_else(|| format!("run-{}", unix_now()));
    let party_only = flag(args, "party");
    let limit: usize = match flag(args, "limit") {
        None => usize::MAX,
        Some(s) => s.parse().unwrap_or_else(|_| {
            eprintln!("world-index: bad --limit (positive integer)");
            std::process::exit(2);
        }),
    };

    let fetch_cache = crate::call_cache::fetch_cache(args);
    let status_html = if live {
        fetch_page(&fetch_cache, STATUS_TABLE_URL).unwrap_or_else(|reason| {
            eprintln!("world-index: fetch {STATUS_TABLE_URL}: {reason}");
            std::process::exit(1);
        })
    } else {
        let path = Path::new(&fixtures).join("status-table-cid41.html");
        std::fs::read_to_string(&path).unwrap_or_else(|e| {
            eprintln!(
                "world-index: read {}: {e} (record fixtures first)",
                path.display()
            );
            std::process::exit(1);
        })
    };
    let mut parties = parse_status_table(&status_html);
    if let Some(want) = party_only.as_deref() {
        let needle = want.to_lowercase();
        parties.retain(|p| p.name.to_lowercase().contains(&needle));
    }
    parties.truncate(limit);
    if parties.is_empty() {
        eprintln!("world-index: no parties parsed (bad table input?)");
        std::process::exit(1);
    }

    let meta_cache = live_model.then(|| crate::call_cache::meta_cache(args));
    let offline = meta_cache
        .as_ref()
        .is_some_and(|c| c.mode() == CacheMode::ReplayOnly);
    let model_key = if live_model {
        match std::env::var("META_API_KEY") {
            Ok(key) if !key.trim().is_empty() => Some(key),
            // Replay-only never sends the key.
            _ if offline => Some(String::new()),
            _ => {
                eprintln!("world-index: --live-model needs META_API_KEY in the environment");
                std::process::exit(2);
            }
        }
    } else {
        None
    };

    let out_dir = Path::new(&out);
    let archive_dir = out_dir.join("archive").join(&run_id);
    std::fs::create_dir_all(&archive_dir).unwrap_or_else(|e| {
        eprintln!("world-index: create {}: {e}", archive_dir.display());
        std::process::exit(1);
    });

    let mut indexed = Vec::new();
    let mut skipped_no_fixture = 0usize;
    let mut kept_hand_built = 0usize;
    let mut status_refreshed = 0usize;
    for party in &parties {
        let authority_url = format!(
            "{HCCH_ORIGIN}/en/instruments/conventions/status-table/authority/?sid={}&cid=41",
            party.sid
        );
        // No authority page this run: the status-table line of the
        // committed pack still follows the (whole) table.
        let mut refresh = |index: &PartyIndex| {
            if refresh_status(&out_dir.join(format!("{}.json", index.iso2)), index) {
                status_refreshed += 1;
            }
        };
        let authority_html = if live {
            match fetch_page(&fetch_cache, &authority_url) {
                Ok(html) => Some(html),
                Err(reason) => {
                    eprintln!("world-index: {}: authority page: {reason}", party.name);
                    None
                }
            }
        } else {
            let path = Path::new(&fixtures).join(format!("authority-sid{}.html", party.sid));
            match std::fs::read_to_string(&path) {
                Ok(html) => Some(html),
                Err(_) => {
                    skipped_no_fixture += 1;
                    refresh(&PartyIndex::empty(party, &authority_url));
                    continue;
                }
            }
        };
        let Some(authority_html) = authority_html else {
            let index = PartyIndex::empty(party, &authority_url);
            refresh(&index);
            indexed.push(index);
            continue;
        };
        let links = parse_authority_links(&authority_html);
        let mut index = PartyIndex::empty(party, &authority_url);
        let mut pages: Vec<(String, String)> = Vec::new();
        for link in &links {
            let details_html = if live {
                match fetch_page(&fetch_cache, &link.url) {
                    Ok(html) => html,
                    Err(reason) => {
                        eprintln!("world-index: {}: {}: {reason}", party.name, link.url);
                        continue;
                    }
                }
            } else {
                let path = Path::new(&fixtures).join(format!("details-aid{}.html", link.aid));
                match std::fs::read_to_string(&path) {
                    Ok(html) => html,
                    Err(_) => continue,
                }
            };
            index.absorb_details(&link.url, &details_html);
            pages.push((link.url.clone(), details_html));
        }
        if index.names.is_empty() && index.urls.is_empty() {
            index.origin = "none";
            if let (Some(key), Some(cache)) = (model_key.as_deref(), meta_cache.as_deref()) {
                let meta = SparkCall {
                    base_url: base_url.as_str(),
                    key,
                    cache,
                };
                match spark_fill(party, &authority_url, &pages, &meta) {
                    Ok(filled) => index = filled,
                    Err(reason) => eprintln!("world-index: {}: spark: {reason}", party.name),
                }
            }
        }
        let pack_path = out_dir.join(format!("{}.json", index.iso2));
        if !write_pack(&pack_path, &index) {
            kept_hand_built += 1;
        }
        indexed.push(index);
    }

    let parsed_total = status_count(&status_html);
    write_csv(&archive_dir.join("authorities.csv"), &indexed);

    let authorities: usize = indexed.iter().map(|i| i.authority_rows()).sum();
    let urls: usize = indexed.iter().map(|i| i.urls.len()).sum();
    let zero_url = indexed.iter().filter(|i| i.urls.is_empty()).count();
    let spark_n = indexed.iter().filter(|i| i.origin == "spark").count();
    println!(
        "world-index: parties={parsed_total} processed={} skipped_no_fixture={skipped_no_fixture} \
         authorities={authorities} urls={urls} zero_url_parties={zero_url} spark_filled={spark_n} \
         kept_hand_built={kept_hand_built} status_refreshed={status_refreshed} out={} archive={}",
        indexed.len(),
        out_dir.display(),
        archive_dir.display()
    );
    if let Some(cache) = &meta_cache {
        println!("world-index: {}", cache.summary());
    }
}

impl PartyIndex {
    fn empty(party: &PartyRow, authority_url: &str) -> Self {
        Self {
            party: party.name.clone(),
            iso2: party_iso2(&party.name).unwrap_or("ZZ").to_string(),
            eif: party.eif.clone(),
            authority_url: authority_url.to_string(),
            names: Vec::new(),
            urls: Vec::new(),
            origin: "deterministic",
        }
    }

    /// Merge one `details3` page: structural names + URLs, each with
    /// its quote verdict against that page.
    fn absorb_details(&mut self, page_url: &str, html: &str) {
        let probe = html_unescape(html);
        for name in designation_names(html) {
            if self.names.iter().any(|hit| hit.name == name) {
                continue;
            }
            let quote_ok = page_contains(&probe, &name);
            self.names.push(NameHit {
                name,
                source_url: page_url.to_string(),
                quote_ok,
            });
        }
        for hit in page_urls(html) {
            if self.urls.iter().any(|seen| seen.url == hit.url) {
                continue;
            }
            let quote_ok = page_contains(&probe, &hit.url);
            self.urls.push(UrlHit {
                url: hit.url,
                useful_link: hit.useful_link,
                quote_ok,
                page: page_url.to_string(),
                name: None,
            });
        }
    }

    /// Authority rows this party emits (at least one: the fallback).
    fn authority_rows(&self) -> usize {
        self.names.len().max(1)
    }
}

/// Sleep between live requests (review §13 politeness). Called on
/// live paths only; offline mode and tests never sleep.
fn politeness_pause() {
    std::thread::sleep(POLITENESS);
}

/// One page through the fetch cache: a recorded answer is served from
/// disk (no politeness pause, no network); a miss waits the politeness
/// budget, fetches with `curl` and records the answer whatever its
/// status. Non-2xx answers are errors here, as `curl -f` made them.
fn fetch_page(cache: &FetchCache, url: &str) -> Result<String, String> {
    let record = cache
        .fetch(url, || {
            politeness_pause();
            curl_fetch(url)
        })
        .map_err(|e| match e {
            callcache::CallError::Live(reason) => reason,
            callcache::CallError::Cache(e) => e.to_string(),
        })?;
    if !(200..300).contains(&record.status) {
        return Err(format!("http {}", record.status));
    }
    Ok(String::from_utf8_lossy(&record.body).into_owned())
}

/// Fetch one URL with `curl`: status, headers and body (temp files are
/// removed before returning).
fn curl_fetch(url: &str) -> Result<callcache::Fetched, String> {
    let dir = std::env::temp_dir().join("handbook-world-index");
    std::fs::create_dir_all(&dir).map_err(|e| format!("temp dir: {e}"))?;
    let stamp = unix_now_nanos();
    let out = dir.join(format!("fetch-{stamp}.html"));
    let hdr = dir.join(format!("fetch-{stamp}.hdr"));
    let run = Command::new("curl")
        .args([
            "-sS",
            "--compressed",
            "-m",
            FETCH_SECS,
            "-A",
            CRAWL_UA,
            "-D",
        ])
        .arg(&hdr)
        .arg("-o")
        .arg(&out)
        .args(["-w", "%{http_code}"])
        .arg(url)
        .output()
        .map_err(|e| format!("run curl: {e} (is it installed?)"))?;
    let body = std::fs::read(&out).unwrap_or_default();
    let head = std::fs::read_to_string(&hdr).unwrap_or_default();
    let _ = std::fs::remove_file(&out);
    let _ = std::fs::remove_file(&hdr);
    if !run.status.success() {
        return Err(format!("curl exit {}", run.status.code().unwrap_or(-1)));
    }
    let status = String::from_utf8_lossy(&run.stdout)
        .trim()
        .parse()
        .unwrap_or(0);
    Ok(callcache::Fetched {
        status,
        headers: header_pairs(&head),
        body,
        head: None,
        request: None,
    })
}

/// `name: value` pairs of a `curl -D` dump (lowercase names).
fn header_pairs(head: &str) -> Vec<(String, String)> {
    head.lines()
        .skip(1)
        .filter_map(|line| line.split_once(':'))
        .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string()))
        .collect()
}

fn unix_now_nanos() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
}

/// Deterministic status-table parse: every `<tr>` with a first `<td>`
/// is a party row (the header uses `<th>`); the authority sid comes
/// from the row's `authority/?sid=` link and the entry into force from
/// the EIF column. Rows without a sid keep an empty sid and are
/// reported as zero-URL parties downstream. The `discovery.bootstrap`
/// worker builds its country list from these rows.
pub(crate) fn parse_status_table(html: &str) -> Vec<PartyRow> {
    let mut parties = Vec::new();
    for row in split_rows(html) {
        if row.contains("<th") {
            continue;
        }
        let Some(first) = first_cell(row) else {
            continue;
        };
        let name = collapse_ws(&html_unescape(&strip_tags(&first)));
        if name.is_empty() {
            continue;
        }
        let sid = sid_in_row(row).unwrap_or_default();
        let eif = cell_texts(row).get(4).and_then(|c| hcch_date(c));
        parties.push(PartyRow { name, sid, eif });
    }
    parties
}

/// Canonical party names off the status table (same row rule); the
/// `research.nonparty` worker subtracts them from the UN member list.
pub(crate) fn party_names(html: &str) -> Vec<String> {
    parse_status_table(html)
        .into_iter()
        .map(|party| party.name)
        .collect()
}

/// Plain text of every `<td…>…</td>` in a row, in order.
fn cell_texts(row: &str) -> Vec<String> {
    let mut cells = Vec::new();
    let mut rest = row;
    while let Some(td) = rest.find("<td") {
        let Some(open_end) = rest[td..].find('>') else {
            break;
        };
        let body = &rest[td + open_end + 1..];
        let Some(close) = body.find("</td>") else {
            break;
        };
        cells.push(collapse_ws(&html_unescape(&strip_tags(&body[..close]))));
        rest = &body[close..];
    }
    cells
}

/// HCCH table date (`28-II-2027`: day, Roman month, year) as
/// `2027-02-28`.
fn hcch_date(text: &str) -> Option<String> {
    let mut parts = text.trim().split('-');
    let day: u32 = parts.next()?.parse().ok()?;
    let month = match parts.next()? {
        "I" => 1,
        "II" => 2,
        "III" => 3,
        "IV" => 4,
        "V" => 5,
        "VI" => 6,
        "VII" => 7,
        "VIII" => 8,
        "IX" => 9,
        "X" => 10,
        "XI" => 11,
        "XII" => 12,
        _ => return None,
    };
    let year: u32 = parts.next()?.parse().ok()?;
    (parts.next().is_none() && (1..=31).contains(&day) && year >= 1961)
        .then(|| format!("{year:04}-{month:02}-{day:02}"))
}

/// Best-effort party count for the summary line (same row rule).
fn status_count(html: &str) -> usize {
    parse_status_table(html).len()
}

/// Split `<tr…>…</tr>` bodies (case-sensitive: the table is fixed).
fn split_rows(html: &str) -> Vec<&str> {
    let mut rows = Vec::new();
    let mut rest = html;
    while let Some(start) = rest.find("<tr") {
        let body = &rest[start..];
        let Some(end) = body.find("</tr>") else {
            break;
        };
        rows.push(&body[..end]);
        rest = &body[end + "</tr>".len()..];
    }
    rows
}

/// Text of the row's first `<td…>…</td>`, tags included.
fn first_cell(row: &str) -> Option<String> {
    let td = row.find("<td")?;
    let open_end = row[td..].find('>')?;
    let body = &row[td + open_end + 1..];
    let close = body.find("</td>")?;
    Some(body[..close].to_string())
}

fn sid_in_row(row: &str) -> Option<String> {
    let marker = row.find("authority/?sid=")?;
    let digits: String = row[marker + "authority/?sid=".len()..]
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    if digits.is_empty() {
        None
    } else {
        Some(digits)
    }
}

/// Authority index links: `details3/?aid=` hrefs with link text.
fn parse_authority_links(html: &str) -> Vec<DetailsLink> {
    let mut links = Vec::new();
    let mut rest = html;
    while let Some(a) = rest.find("<a ") {
        let tag = &rest[a..];
        let Some(end) = tag.find('>') else {
            break;
        };
        let open = &tag[..end];
        let after = &tag[end + 1..];
        let Some(close) = after.find("</a>") else {
            rest = after;
            continue;
        };
        if let Some(href) = href_in(open)
            && href.contains("states/authorities/details3/")
        {
            let aid = aid_in_href(&href).unwrap_or_default();
            let title = collapse_ws(&html_unescape(&strip_tags(&after[..close])));
            let url = abs_url(&href);
            if !aid.is_empty() && !links.iter().any(|l: &DetailsLink| l.aid == aid) {
                links.push(DetailsLink { aid, title, url });
            }
        }
        rest = &after[close + "</a>".len()..];
    }
    links
}

fn href_in(open_tag: &str) -> Option<String> {
    let needle = "href=\"";
    let start = open_tag.find(needle)? + needle.len();
    let end = open_tag[start..].find('"')?;
    Some(html_unescape(&open_tag[start..start + end]))
}

fn aid_in_href(href: &str) -> Option<String> {
    let marker = href.find("aid=")? + "aid=".len();
    let digits: String = href[marker..]
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    if digits.is_empty() {
        None
    } else {
        Some(digits)
    }
}

fn abs_url(href: &str) -> String {
    if href.starts_with("http://") || href.starts_with("https://") {
        href.to_string()
    } else if let Some(path) = href.strip_prefix('/') {
        format!("{HCCH_ORIGIN}/{path}")
    } else {
        format!("{HCCH_ORIGIN}/{href}")
    }
}

/// Designated-authority names on a `details3` page. `<em>` segments
/// are names (deduped, numbering stripped). The block's remaining
/// item segments — `<br>`/`<p>`-separated text carrying no `<em>` —
/// join when they add something new, so a lone parenthetical `<em>`
/// never hides the printed name, while scope notes glued to an
/// `<em>` stay out. Prose designations (whole paragraphs) are kept
/// verbatim: they are the page's designation statement.
fn designation_names(html: &str) -> Vec<String> {
    let Some(block) = designation_block(html) else {
        return Vec::new();
    };
    let mut ems = Vec::new();
    for raw in em_in(&block) {
        // An `<em>` may itself hold `<br>`-separated items (Algeria):
        // split it the same way, then `;`-split each head.
        for chunk in item_segments(&raw) {
            let head = collapse_ws(&html_unescape(&strip_tags(chunk)));
            for piece in split_items(&head) {
                if piece.len() < 3 || !starts_namey(&piece) {
                    continue;
                }
                let name = strip_enumerator(&piece);
                if !ems.contains(&name) {
                    ems.push(name);
                }
            }
        }
    }
    let mut names = ems.clone();
    for segment in item_segments(&block) {
        // Any segment touching `<em>` markup defers to em extraction:
        // its names already came from there, and the leftovers are
        // scope notes or sentence fragments. Table cells are contact
        // info (addresses, phones), never designations.
        if segment.contains("<em")
            || segment.contains("</em")
            || segment.contains("<td")
            || segment.contains("<th")
        {
            continue;
        }
        let line = collapse_ws(&html_unescape(&strip_tags(segment)));
        for piece in split_items(&line) {
            if piece.len() < 8
                || piece.starts_with("Designated Competent Authorit")
                || piece == "Contact details:"
                || piece.starts_with('(')
                || is_disclaimed_contact(&piece)
            {
                continue;
            }
            if ems
                .iter()
                .any(|em| piece.contains(em) || em.contains(&piece))
            {
                continue;
            }
            let name = strip_enumerator(&piece);
            if !names.contains(&name) {
                names.push(name);
            }
        }
    }
    names.retain(|name| !is_meta_note(name) && !is_meta_sentence(name));
    names
}

/// Sentences the page uses to talk *about* the designation list
/// (counts, cross-references) are never designations themselves.
/// (SA-17: an earlier Mexico world pack shipped "There are 32
/// competent authorities…" and "for more information see below…"
/// as authority names.)
fn is_meta_sentence(name: &str) -> bool {
    let text = name.to_lowercase();
    text.starts_with("for more information")
        || text.starts_with("see below")
        || text.starts_with("see above")
        || text.starts_with("please see")
        || (text.starts_with("there are ") && text.contains("competent authorit"))
        || (text.starts_with("there is ") && text.contains("competent authorit"))
}

/// Split an item segment on `;` separators outside parentheses, so
/// `Ministry A;Ministry B` yields two names. Parenthesized asides
/// stay glued to their item; empty pieces drop out.
fn split_items(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut cur = String::new();
    for c in line.chars() {
        match c {
            '(' => {
                depth += 1;
                cur.push(c);
            }
            ')' => {
                depth = depth.saturating_sub(1);
                cur.push(c);
            }
            ';' if depth == 0 => {
                out.push(std::mem::take(&mut cur));
            }
            _ => cur.push(c),
        }
    }
    out.push(cur);
    out.into_iter()
        .map(|piece| {
            piece
                .trim()
                .trim_start_matches([':', '-', '–', '—'])
                .trim()
                .to_string()
        })
        .filter(|piece| !piece.is_empty())
        .collect()
}

/// A fully parenthesized editor/translator note is not a
/// designation (`(courtesy translation)` on the Italy page): drop
/// it. Genuine parenthesized names (the Albanian designation) carry
/// no such keyword and survive.
fn is_meta_note(name: &str) -> bool {
    let trimmed = name.trim();
    if !(trimmed.starts_with('(') && trimmed.ends_with(')')) {
        return false;
    }
    let inner = trimmed[1..trimmed.len() - 1].to_lowercase();
    ["courtesy", "translation", "click here"]
        .iter()
        .any(|keyword| inner.contains(keyword))
}

/// HCCH boilerplate for a contact that is explicitly *not* a
/// competent authority ("...not competent to issue apostilles").
/// Such segments must never become authority names.
fn is_disclaimed_contact(line: &str) -> bool {
    line.to_lowercase()
        .contains("not competent to issue apostilles")
}

/// An `<em>` fragment starts a name when it opens with an
/// uppercase letter, a digit, or a parenthesis: lowercase continuations
/// (`...empowered<br /> to issue...`) are sentence fragments, not names.
fn starts_namey(piece: &str) -> bool {
    match piece.chars().next() {
        Some('(') => true,
        Some(c) => c.is_uppercase() || c.is_numeric(),
        None => false,
    }
}

/// Split a block on item separators: line/paragraph breaks plus list
/// and division tags (`<li>`, `<div>`, ...), which some pages use for
/// the designation list instead of emphasis.
fn item_segments(block: &str) -> Vec<&str> {
    let mut segments = Vec::new();
    let mut rest = block;
    loop {
        // NOTE: `<td>`/`<th>` are deliberately NOT separators: table
        // cells stay in one segment so the `<td` skip rule below can
        // recognize (and drop) contact-table content as a whole.
        let next = [
            "<br", "<p>", "<p ", "</p>", "<li", "<li ", "</li>", "<div", "<div ", "</div>", "<ol",
            "<ul", "</ol>", "</ul>", "<h1", "<h2", "<h3", "<h4", "<h5", "<h6", "<table",
            "</table>", "<tr", "</tr>",
        ]
        .into_iter()
        .filter_map(|tag| rest.find(tag).map(|i| (i, tag)))
        .min_by_key(|(i, _)| *i);
        match next {
            Some((i, tag)) => {
                segments.push(&rest[..i]);
                let after = &rest[i + tag.len()..];
                rest = after.find('>').map(|end| &after[end + 1..]).unwrap_or("");
            }
            None => {
                segments.push(rest);
                break;
            }
        }
    }
    segments
}

/// HTML between the designation heading and the next section.
/// Most pages head contact tables with `Contact details:`; some go
/// straight to tables and then `Practical Information:`. Without a
/// closing marker the page shape is unknown (fail closed: irregular).
fn designation_block(html: &str) -> Option<String> {
    let start = html.find("Designated Competent Authorit")?;
    let tail = &html[start..];
    let end = tail
        .find("Contact details:")
        .or_else(|| tail.find("Practical Information:"));
    Some(tail[..end?].to_string())
}

fn em_in(block: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = block;
    while let Some(start) = rest.find("<em>") {
        let body = &rest[start + "<em>".len()..];
        let Some(end) = body.find("</em>") else {
            break;
        };
        out.push(body[..end].to_string());
        rest = &body[end + "</em>".len()..];
    }
    out
}

/// Strip a leading list enumerator (`1. `, `b) `, `I. `, `- `).
/// Roman-numeral guard: a bare `St.`-like word is not an enumerator.
fn strip_enumerator(line: &str) -> String {
    let bytes = line.as_bytes();
    let mut i = 0;
    if bytes.first() == Some(&b'-') && bytes.get(1) == Some(&b' ') {
        return line[2..].to_string();
    }
    if bytes.first() == Some(&b'(') {
        if let Some(close) = line.find(')') {
            let inner = &line[1..close];
            if !inner.is_empty() && inner.len() <= 4 && inner.chars().all(|c| c.is_alphanumeric()) {
                return line[close + 1..].trim_start().to_string();
            }
        }
        return line.to_string();
    }
    while i < bytes.len() && bytes[i].is_ascii_alphanumeric() {
        i += 1;
    }
    if i > 0
        && i <= 4
        && i < bytes.len()
        && (bytes[i] == b'.' || bytes[i] == b')')
        && bytes.get(i + 1) == Some(&b' ')
    {
        let head = &line[..i];
        let is_word = head.len() > 1 && head.chars().all(|c| c.is_alphabetic());
        if !is_word {
            return line[i + 2..].to_string();
        }
    }
    line.to_string()
}

/// Website URLs on a `details3` page: hrefs in table rows whose first
/// cell names `General website` (label variants included) or
/// `Useful Links`. Cloudflare-obfuscated mail links never qualify.
fn page_urls(html: &str) -> Vec<UrlHit> {
    let mut urls = Vec::new();
    for row in split_rows(html) {
        let cells = table_cells(row);
        if cells.len() < 2 {
            continue;
        }
        let first = collapse_ws(&html_unescape(&strip_tags(&cells[0]))).to_lowercase();
        let useful = first.contains("useful link");
        if !first.contains("general website") && !useful {
            continue;
        }
        for href in hrefs_in(&cells[1]) {
            if (href.starts_with("http://") || href.starts_with("https://"))
                && !href.contains("cdn-cgi")
                && !urls.iter().any(|hit: &UrlHit| hit.url == href)
            {
                urls.push(UrlHit {
                    url: href,
                    useful_link: useful,
                    quote_ok: false,
                    page: String::new(),
                    name: None,
                });
            }
        }
    }
    urls
}

fn table_cells(row: &str) -> Vec<String> {
    let mut cells = Vec::new();
    let mut rest = row;
    while let Some(td) = rest.find("<td") {
        let body = &rest[td..];
        let Some(open_end) = body.find('>') else {
            break;
        };
        let after = &body[open_end + 1..];
        let Some(close) = after.find("</td>") else {
            break;
        };
        cells.push(after[..close].to_string());
        rest = &after[close + "</td>".len()..];
    }
    cells
}

fn hrefs_in(cell: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = cell;
    while let Some(a) = rest.find("<a ") {
        let tag = &rest[a..];
        let Some(end) = tag.find('>') else {
            break;
        };
        if let Some(href) = href_in(&tag[..end]) {
            out.push(href);
        }
        rest = &tag[end + 1..];
    }
    out
}

/// Strip every `<…>` tag (comments included), keeping the text.
fn strip_tags(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut chars = html.chars();
    while let Some(c) = chars.next() {
        if c == '<' {
            for c in chars.by_ref() {
                if c == '>' {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Entity decoding for the entities HCCH pages use, plus numeric
/// character references. Only well-formed candidates (`&name;`,
/// `&#123;`, `&#x1F;`) are consumed: a bare `&` (page scripts are
/// full of them) is emitted literally, so it can never swallow a
/// later `;` and eat the text between. Unknown well-formed entities
/// pass through untouched (both sides of the quote check share this
/// function, so they still compare equal).
pub(crate) fn html_unescape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        let tail = &rest[amp..];
        let Some(len) = entity_len(tail) else {
            out.push('&');
            rest = &tail[1..];
            continue;
        };
        let entity = &tail[..len];
        if let Some(decoded) = named_entity(entity) {
            out.push_str(decoded);
        } else if let Some(ch) = numeric_entity(entity) {
            out.push(ch);
        } else {
            out.push_str(entity);
        }
        rest = &tail[len..];
    }
    out.push_str(rest);
    out
}

/// Byte length of the well-formed entity at the start of `tail`
/// (leading `&` included), or `None` for a bare `&`.
fn entity_len(tail: &str) -> Option<usize> {
    let body = tail.strip_prefix('&')?;
    let end = body.find(';')?;
    if end == 0 || end > 10 {
        return None;
    }
    let inner = &body[..end];
    let well_formed = if let Some(num) = inner.strip_prefix('#') {
        if let Some(hex) = num.strip_prefix('x').or_else(|| num.strip_prefix('X')) {
            !hex.is_empty() && hex.chars().all(|c| c.is_ascii_hexdigit())
        } else {
            !num.is_empty() && num.chars().all(|c| c.is_ascii_digit())
        }
    } else {
        !inner.is_empty()
            && inner.chars().all(|c| c.is_ascii_alphanumeric())
            && inner.starts_with(|c: char| c.is_ascii_alphabetic())
    };
    if well_formed { Some(end + 2) } else { None }
}

fn named_entity(entity: &str) -> Option<&'static str> {
    match entity {
        "&amp;" => Some("&"),
        "&lt;" => Some("<"),
        "&gt;" => Some(">"),
        "&quot;" => Some("\""),
        "&apos;" | "&#39;" | "&#x27;" => Some("'"),
        "&nbsp;" | "&#160;" | "&#xa0;" => Some(" "),
        "&rsquo;" | "&#8217;" => Some("’"),
        "&lsquo;" | "&#8216;" => Some("‘"),
        "&rdquo;" | "&#8221;" => Some("”"),
        "&ldquo;" | "&#8220;" => Some("“"),
        "&ndash;" | "&#8211;" => Some("–"),
        "&mdash;" | "&#8212;" => Some("—"),
        "&hellip;" => Some("…"),
        "&shy;" | "&#173;" => Some(""),
        "&laquo;" => Some("«"),
        "&raquo;" => Some("»"),
        "&iexcl;" => Some("¡"),
        "&iquest;" => Some("¿"),
        "&Agrave;" => Some("À"),
        "&Aacute;" => Some("Á"),
        "&Acirc;" => Some("Â"),
        "&Atilde;" => Some("Ã"),
        "&Auml;" => Some("Ä"),
        "&Aring;" => Some("Å"),
        "&AElig;" => Some("Æ"),
        "&Ccedil;" => Some("Ç"),
        "&Egrave;" => Some("È"),
        "&Eacute;" => Some("É"),
        "&Ecirc;" => Some("Ê"),
        "&Euml;" => Some("Ë"),
        "&Igrave;" => Some("Ì"),
        "&Iacute;" => Some("Í"),
        "&Icirc;" => Some("Î"),
        "&Iuml;" => Some("Ï"),
        "&Ntilde;" => Some("Ñ"),
        "&Ograve;" => Some("Ò"),
        "&Oacute;" => Some("Ó"),
        "&Ocirc;" => Some("Ô"),
        "&Otilde;" => Some("Õ"),
        "&Ouml;" => Some("Ö"),
        "&Oslash;" => Some("Ø"),
        "&Ugrave;" => Some("Ù"),
        "&Uacute;" => Some("Ú"),
        "&Ucirc;" => Some("Û"),
        "&Uuml;" => Some("Ü"),
        "&Yacute;" => Some("Ý"),
        "&szlig;" => Some("ß"),
        "&agrave;" => Some("à"),
        "&aacute;" => Some("á"),
        "&acirc;" => Some("â"),
        "&atilde;" => Some("ã"),
        "&auml;" => Some("ä"),
        "&aring;" => Some("å"),
        "&aelig;" => Some("æ"),
        "&ccedil;" => Some("ç"),
        "&egrave;" => Some("è"),
        "&eacute;" => Some("é"),
        "&ecirc;" => Some("ê"),
        "&euml;" => Some("ë"),
        "&igrave;" => Some("ì"),
        "&iacute;" => Some("í"),
        "&icirc;" => Some("î"),
        "&iuml;" => Some("ï"),
        "&ntilde;" => Some("ñ"),
        "&ograve;" => Some("ò"),
        "&oacute;" => Some("ó"),
        "&ocirc;" => Some("ô"),
        "&otilde;" => Some("õ"),
        "&ouml;" => Some("ö"),
        "&oslash;" => Some("ø"),
        "&ugrave;" => Some("ù"),
        "&uacute;" => Some("ú"),
        "&ucirc;" => Some("û"),
        "&uuml;" => Some("ü"),
        "&yacute;" => Some("ý"),
        "&yuml;" => Some("ÿ"),
        "&OElig;" => Some("Œ"),
        "&oelig;" => Some("œ"),
        _ => None,
    }
}

fn numeric_entity(entity: &str) -> Option<char> {
    let body = entity.strip_prefix("&#")?.strip_suffix(';')?;
    let code = if let Some(hex) = body.strip_prefix('x').or_else(|| body.strip_prefix('X')) {
        u32::from_str_radix(hex, 16).ok()?
    } else {
        body.parse::<u32>().ok()?
    };
    char::from_u32(code)
}

/// Lowercase + whitespace-collapse (the `analyze.rs` normal form).
fn normalize(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

pub(crate) fn collapse_ws(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The quote check: `value` must be contained in the entity-decoded
/// page under the normal form, and carry at least 3 chars (shorter
/// slices prove nothing about provenance).
fn page_contains(decoded_page: &str, value: &str) -> bool {
    let want = normalize(value);
    if want.chars().count() < 3 {
        return false;
    }
    normalize(decoded_page).contains(&want)
}

/// Lowercased host of an absolute `http(s)` URL.
fn host_of(url: &str) -> Option<String> {
    let rest = url
        .strip_prefix("http://")
        .or_else(|| url.strip_prefix("https://"))?;
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    if host.is_empty() || !host.contains('.') {
        return None;
    }
    Some(host.to_lowercase())
}

/// HCCH canonical party name to ISO 3166-1 alpha-2. Covers every
/// party on the live cid=41 table (checked 2026-11-02); `None` means
/// "unknown to this build" and the pack falls back to the `ZZ` stem.
/// Kosovo has no assigned code, so the user-assigned `XK` applies.
pub(crate) fn party_iso2(party: &str) -> Option<&'static str> {
    Some(match party {
        "Albania" => "AL",
        "Algeria" => "DZ",
        "Andorra" => "AD",
        "Antigua and Barbuda" => "AG",
        "Argentina" => "AR",
        "Armenia" => "AM",
        "Australia" => "AU",
        "Austria" => "AT",
        "Azerbaijan" => "AZ",
        "Bahamas" => "BS",
        "Bahrain" => "BH",
        "Bangladesh" => "BD",
        "Barbados" => "BB",
        "Belarus" => "BY",
        "Belgium" => "BE",
        "Belize" => "BZ",
        "Bolivia (Plurinational State of)" => "BO",
        "Bosnia and Herzegovina" => "BA",
        "Botswana" => "BW",
        "Brazil" => "BR",
        "Brunei Darussalam" => "BN",
        "Bulgaria" => "BG",
        "Burundi" => "BI",
        "Cabo Verde" => "CV",
        "Canada" => "CA",
        "Chile" => "CL",
        "China (the People's Republic of)" => "CN",
        "Colombia" => "CO",
        "Cook Islands" => "CK",
        "Costa Rica" => "CR",
        "Croatia" => "HR",
        "Cyprus" => "CY",
        "Czechia" => "CZ",
        "Denmark" => "DK",
        "Dominica" => "DM",
        "Dominican Republic" => "DO",
        "Ecuador" => "EC",
        "El Salvador" => "SV",
        "Estonia" => "EE",
        "Eswatini" => "SZ",
        "Fiji" => "FJ",
        "Finland" => "FI",
        "France" => "FR",
        "Georgia" => "GE",
        "Germany" => "DE",
        "Greece" => "GR",
        "Grenada" => "GD",
        "Guatemala" => "GT",
        "Guyana" => "GY",
        "Honduras" => "HN",
        "Hungary" => "HU",
        "Iceland" => "IS",
        "India" => "IN",
        "Indonesia" => "ID",
        "Ireland" => "IE",
        "Israel" => "IL",
        "Italy" => "IT",
        "Jamaica" => "JM",
        "Japan" => "JP",
        "Kazakhstan" => "KZ",
        "Kosovo" => "XK",
        "Kyrgyzstan" => "KG",
        "Latvia" => "LV",
        "Lesotho" => "LS",
        "Liberia" => "LR",
        "Liechtenstein" => "LI",
        "Lithuania" => "LT",
        "Luxembourg" => "LU",
        "Malawi" => "MW",
        "Malta" => "MT",
        "Marshall Islands" => "MH",
        "Mauritius" => "MU",
        "Mexico" => "MX",
        "Monaco" => "MC",
        "Mongolia" => "MN",
        "Montenegro" => "ME",
        "Morocco" => "MA",
        "Namibia" => "NA",
        "Netherlands (Kingdom of the)" => "NL",
        "New Zealand" => "NZ",
        "Nicaragua" => "NI",
        "Niue" => "NU",
        "North Macedonia" => "MK",
        "Norway" => "NO",
        "Oman" => "OM",
        "Pakistan" => "PK",
        "Palau" => "PW",
        "Panama" => "PA",
        "Paraguay" => "PY",
        "Peru" => "PE",
        "Philippines" => "PH",
        "Poland" => "PL",
        "Portugal" => "PT",
        "Republic of Korea" => "KR",
        "Republic of Moldova" => "MD",
        "Romania" => "RO",
        "Russian Federation" => "RU",
        "Rwanda" => "RW",
        "Saint Kitts and Nevis" => "KN",
        "Saint Lucia" => "LC",
        "Saint Vincent and the Grenadines" => "VC",
        "Samoa" => "WS",
        "San Marino" => "SM",
        "Sao Tome and Principe" => "ST",
        "Saudi Arabia" => "SA",
        "Senegal" => "SN",
        "Serbia" => "RS",
        "Seychelles" => "SC",
        "Singapore" => "SG",
        "Slovakia" => "SK",
        "Slovenia" => "SI",
        "South Africa" => "ZA",
        "Spain" => "ES",
        "Suriname" => "SR",
        "Sweden" => "SE",
        "Switzerland" => "CH",
        "Tajikistan" => "TJ",
        "Thailand" => "TH",
        "Tonga" => "TO",
        "Trinidad and Tobago" => "TT",
        "Tunisia" => "TN",
        "Türkiye" => "TR",
        "Ukraine" => "UA",
        "United Kingdom of Great Britain and Northern Ireland" => "GB",
        "United States of America" => "US",
        "Uruguay" => "UY",
        "Uzbekistan" => "UZ",
        "Vanuatu" => "VU",
        "Venezuela (Bolivarian Republic of)" => "VE",
        "Viet Nam" => "VN",
        _ => return None,
    })
}

/// Slug stem for authority ids: lowercased ISO code.
fn slug_base(iso2: &str) -> String {
    iso2.to_lowercase()
}

/// Matching key for an extracted name against a committed row:
/// whitespace collapsed, lowercased, list punctuation trimmed (so the
/// curated "Hungarian Ministry …" still matches "… Law,").
fn name_key(name: &str) -> String {
    collapse_ws(name)
        .to_lowercase()
        .trim_end_matches([':', ',', ';', '.', ' '])
        .to_string()
}

/// True when `file` is this command's own output: its title carries
/// the world-index mark and every row is `kind = country`. Anything
/// else (the hand-built US, MX and CA packs with their states,
/// provinces and federal offices) is never overwritten (P3-9).
fn is_world_index_output(file: &crate::packs::PackFile) -> bool {
    file.pack.title.contains("(world index")
        && file.pack.authorities.iter().all(|a| a.kind == "country")
}

/// Build the pack for one party, reusing the pack schema so the
/// output stays loadable. Quote-failed values never reach here: only
/// `quote_ok` hits are passed in.
///
/// `existing` is the committed pack for this party, if any: its rows
/// keep their slugs, curated names, `hidden` flags and order (a slug
/// is a production row; renumbering would orphan it). An extracted
/// name matches a row by its name or its `source_name`; a new name
/// gets the next free `{iso}-ca-{n}` and is emitted hidden when it
/// fails the name lint, so a re-run never publishes a page fragment.
/// URLs belong to the authority their source names, else the first
/// visible authority named on the same page, else the party's lead.
fn build_pack(
    index: &PartyIndex,
    names: &[NameHit],
    urls: &[UrlHit],
    existing: Option<&crate::packs::PackFile>,
) -> crate::packs::PackFile {
    use crate::packs::{
        AuthorityExt, Pack, PackAuthority, PackDomain, PackExt, PackFile, PackObservation,
        PackStep, PackWatch, name_lint,
    };
    let base = slug_base(&index.iso2);
    let fallback_slug = format!("{base}-ca");
    let mut authorities: Vec<PackAuthority> = Vec::new();
    let mut exts: Vec<AuthorityExt> = Vec::new();
    if let Some(file) = existing {
        for (i, authority) in file.pack.authorities.iter().enumerate() {
            authorities.push(authority.clone());
            exts.push(AuthorityExt {
                slug: authority.slug.clone(),
                ..file.ext_of(i)
            });
        }
    }
    let mut next = authorities
        .iter()
        .filter_map(|a| {
            a.slug
                .strip_prefix(&format!("{base}-ca-"))?
                .parse::<usize>()
                .ok()
        })
        .max()
        .unwrap_or(0)
        + 1;
    // The slug each extracted name lands on, in `names` order.
    let mut name_slugs: Vec<String> = Vec::new();
    for hit in names {
        let key = name_key(&hit.name);
        let found = authorities.iter().zip(&exts).find(|(a, x)| {
            name_key(&a.name) == key
                || (!x.source_name.is_empty() && name_key(&x.source_name) == key)
        });
        let slug = match found {
            Some((a, _)) => a.slug.clone(),
            None => {
                let slug = format!("{base}-ca-{next}");
                next += 1;
                authorities.push(PackAuthority {
                    kind: "country".to_string(),
                    country: index.party.clone(),
                    region: String::new(),
                    name: hit.name.clone(),
                    slug: slug.clone(),
                });
                exts.push(AuthorityExt {
                    slug: slug.clone(),
                    hidden: name_lint(&hit.name).map(|_| true),
                    ..AuthorityExt::default()
                });
                slug
            }
        };
        name_slugs.push(slug);
    }
    let visible = |exts: &[AuthorityExt], slug: &str| {
        exts.iter()
            .find(|x| x.slug == slug)
            .is_some_and(|x| x.hidden != Some(true))
    };
    // A real name supersedes the "no names yet" placeholder row.
    if name_slugs
        .iter()
        .any(|s| *s != fallback_slug && visible(&exts, s))
        && let Some(x) = exts.iter_mut().find(|x| x.slug == fallback_slug)
        && x.hidden.is_none()
    {
        x.hidden = Some(true);
    }
    if authorities.is_empty() {
        authorities.push(PackAuthority {
            kind: "country".to_string(),
            country: index.party.clone(),
            region: String::new(),
            name: format!("{} — Competent Authority (Art. 6)", index.party),
            slug: fallback_slug.clone(),
        });
        exts.push(AuthorityExt {
            slug: fallback_slug,
            ..AuthorityExt::default()
        });
    }
    let lead = authorities
        .iter()
        .map(|a| a.slug.clone())
        .find(|slug| visible(&exts, slug))
        .unwrap_or_else(|| authorities[0].slug.clone());
    let owner_of = |hit: &UrlHit| -> String {
        let named = hit.name.as_deref().and_then(|want| {
            names
                .iter()
                .zip(&name_slugs)
                .find(|(n, s)| n.name == want && visible(&exts, s))
        });
        let on_page = || {
            names
                .iter()
                .zip(&name_slugs)
                .find(|(n, s)| n.source_url == hit.page && visible(&exts, s))
        };
        named
            .or_else(on_page)
            .map_or_else(|| lead.clone(), |(_, slug)| slug.clone())
    };
    // Watch seeds: the HCCH citation page plus every listed URL.
    // Every watch host gets a domain verdict row in this pack.
    let mut watch = vec![PackWatch {
        authority: lead.clone(),
        kind: "website_official".to_string(),
        uri: index.authority_url.clone(),
        cadence: "weekly".to_string(),
        notes: format!(
            "HCCH competent-authority index for {} (citation page; staff capture pending).",
            index.party
        ),
    }];
    let page_of = |hit: &UrlHit| {
        if hit.page.is_empty() {
            index.authority_url.clone()
        } else {
            hit.page.clone()
        }
    };
    for hit in urls {
        let via = if hit.useful_link {
            "Useful Links"
        } else {
            "General website"
        };
        watch.push(PackWatch {
            authority: owner_of(hit),
            kind: "website_official".to_string(),
            uri: hit.url.clone(),
            cadence: "weekly".to_string(),
            notes: format!(
                "Listed under {via} on {} (needs staff capture; {WORLD_REF}).",
                page_of(hit)
            ),
        });
    }
    let mut hosts: Vec<(String, String)> = Vec::new();
    for seed in &watch {
        if let Some(host) = host_of(&seed.uri)
            && !hosts.iter().any(|(h, _)| *h == host)
        {
            hosts.push((host, seed.authority.clone()));
        }
    }
    let domains = hosts
        .into_iter()
        .map(|(host, owner)| PackDomain {
            name: host,
            authority: owner,
            status: "unverified".to_string(),
            notes: format!(
                "Seen on HCCH competent-authority pages for {} (staff capture pending).",
                index.party
            ),
        })
        .collect();
    let mut observations = vec![PackObservation {
        authority: lead.clone(),
        topic: "jurisdiction".to_string(),
        statement: status_statement(index),
        source_uri: STATUS_TABLE_URL.to_string(),
        status: "needs_review".to_string(),
        validation: WORLD_REF.to_string(),
    }];
    for (hit, slug) in names.iter().zip(&name_slugs) {
        if !visible(&exts, slug) {
            continue;
        }
        observations.push(PackObservation {
            authority: slug.clone(),
            topic: "contacts".to_string(),
            statement: format!(
                "OBSERVED (HCCH competent-authority page; staff capture pending): designated authority: {}.",
                hit.name
            ),
            source_uri: hit.source_url.clone(),
            status: "needs_review".to_string(),
            validation: WORLD_REF.to_string(),
        });
    }
    for hit in urls {
        let via = if hit.useful_link {
            "Useful Links"
        } else {
            "General website"
        };
        observations.push(PackObservation {
            authority: owner_of(hit),
            topic: "website".to_string(),
            statement: format!(
                "OBSERVED (HCCH competent-authority page; staff capture pending): page lists {} as an official website link (under {via}).",
                hit.url
            ),
            source_uri: page_of(hit),
            status: "needs_review".to_string(),
            validation: WORLD_REF.to_string(),
        });
    }
    let chain_steps = vec![PackStep {
        id: format!("{base}-index"),
        authority: lead,
        order: 0,
        title: "Verify HCCH competent-authority index".to_string(),
        detail: format!(
            "Confirm the designated authorities and website URLs for {} against a staff capture of the cited HCCH pages ({}).",
            index.party, index.origin
        ),
        status: "needs_review".to_string(),
        validation: WORLD_REF.to_string(),
    }];
    PackFile {
        pack: Pack {
            pack: index.iso2.clone(),
            version: 1,
            title: format!(
                "{} — HCCH 1961 competent authorities (world index, needs review)",
                index.party
            ),
            authorities,
            domains,
            watch,
            chain_steps,
            observations,
        },
        ext: PackExt {
            iso2: if index.iso2 == "ZZ" {
                String::new()
            } else {
                index.iso2.clone()
            },
            authorities: exts,
        },
    }
}

/// The pack's status-table line. Tense-free on purpose: "entry into
/// force for X: <date>" reads true before and after the date, so a
/// future party (Thailand, 2027-02-28) shows when the convention starts
/// to apply without the pack going stale on that day.
fn status_statement(index: &PartyIndex) -> String {
    let mut statement = format!(
        "OBSERVED (HCCH 1961 status table cid=41; staff capture pending): {} is a listed Contracting Party",
        index.party
    );
    if let Some(eif) = &index.eif {
        statement.push_str(&format!("; entry into force for {}: {eif}", index.party));
    }
    statement.push('.');
    statement
}

/// Bring the status-table observation of the committed pack at `path`
/// in line with the table, when this run has no authority page for the
/// party (offline without a recorded fixture, or a failed live fetch).
/// The table is recorded whole, so its lines can always be refreshed;
/// everything else in the pack stays as committed, and hand-built packs
/// are never touched. Returns true when the file changed.
fn refresh_status(path: &Path, index: &PartyIndex) -> bool {
    let Some(mut file) = std::fs::read(path)
        .ok()
        .and_then(|bytes| crate::packs::PackFile::parse(&bytes).ok())
    else {
        return false;
    };
    if !is_world_index_output(&file) {
        return false;
    }
    let statement = status_statement(index);
    let mut changed = false;
    for observation in &mut file.pack.observations {
        if observation.source_uri == STATUS_TABLE_URL
            && observation.topic == "jurisdiction"
            && observation.statement != statement
        {
            observation.statement.clone_from(&statement);
            changed = true;
        }
    }
    if changed {
        std::fs::write(path, file.render()).unwrap_or_else(|e| {
            eprintln!("world-index: write {}: {e}", path.display());
            std::process::exit(1);
        });
    }
    changed
}

/// Write one party's pack, merged with the committed one at `path`.
/// Returns false (and writes nothing) for a hand-built pack.
fn write_pack(path: &Path, index: &PartyIndex) -> bool {
    let existing = std::fs::read(path)
        .ok()
        .and_then(|bytes| crate::packs::PackFile::parse(&bytes).ok());
    if let Some(file) = &existing
        && !is_world_index_output(file)
    {
        println!(
            "world-index: {}: kept as is (hand-built pack, not world-index output)",
            path.display()
        );
        return false;
    }
    let names: Vec<NameHit> = index.names.iter().filter(|h| h.quote_ok).cloned().collect();
    let urls: Vec<UrlHit> = index.urls.iter().filter(|h| h.quote_ok).cloned().collect();
    let file = build_pack(index, &names, &urls, existing.as_ref());
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap_or_else(|e| {
            eprintln!("world-index: create {}: {e}", parent.display());
            std::process::exit(1);
        });
    }
    std::fs::write(path, file.render()).unwrap_or_else(|e| {
        eprintln!("world-index: write {}: {e}", path.display());
        std::process::exit(1);
    });
    true
}

/// `authorities.csv`: one row per emitted authority name plus one row
/// per party with no names (empty authority cell, still cited).
fn write_csv(path: &Path, indexed: &[PartyIndex]) {
    let mut out =
        String::from("iso2,party,authority_name,authority_url,source_url,quote_ok,origin\n");
    for index in indexed {
        if index.names.is_empty() && index.urls.is_empty() {
            out.push_str(&csv_row(&[
                &index.iso2,
                &index.party,
                "",
                "",
                &index.authority_url,
                "false",
                index.origin,
            ]));
        }
        for name in &index.names {
            // The URL its source pairs with this name, else the first
            // one listed on the same page (never the same list index).
            let url = index
                .urls
                .iter()
                .find(|hit| hit.name.as_deref() == Some(name.name.as_str()))
                .or_else(|| index.urls.iter().find(|hit| hit.page == name.source_url))
                .map(|hit| hit.url.as_str())
                .unwrap_or("");
            out.push_str(&csv_row(&[
                &index.iso2,
                &index.party,
                &name.name,
                url,
                &name.source_url,
                if name.quote_ok { "true" } else { "false" },
                index.origin,
            ]));
        }
        for hit in index.urls.iter().filter(|_| index.names.is_empty()) {
            out.push_str(&csv_row(&[
                &index.iso2,
                &index.party,
                "",
                &hit.url,
                &index.authority_url,
                if hit.quote_ok { "true" } else { "false" },
                index.origin,
            ]));
        }
    }
    std::fs::write(path, out).unwrap_or_else(|e| {
        eprintln!("world-index: write {}: {e}", path.display());
        std::process::exit(1);
    });
}

fn csv_row(cells: &[&str]) -> String {
    let mut row = String::new();
    for (i, cell) in cells.iter().enumerate() {
        if i > 0 {
            row.push(',');
        }
        if cell.contains([',', '"', '\n']) {
            row.push('"');
            row.push_str(&cell.replace('"', "\"\""));
            row.push('"');
        } else {
            row.push_str(cell);
        }
    }
    row.push('\n');
    row
}

/// One Spark-proposed authority (the quote check still applies).
#[derive(Debug, Clone, PartialEq, Eq)]
struct SparkAuthority {
    name: String,
    urls: Vec<String>,
}

/// Fill an irregular party via Muse Spark. `pages` are the already
/// fetched details pages (url + html); the model sees their text,
/// and every returned name/URL must still pass the quote check
/// against the cited page — failures are dropped, never emitted.
/// Where a Spark call goes and how it is cached.
struct SparkCall<'a> {
    base_url: &'a str,
    key: &'a str,
    cache: &'a MetaCache,
}

fn spark_fill(
    party: &PartyRow,
    authority_url: &str,
    pages: &[(String, String)],
    meta: &SparkCall<'_>,
) -> Result<PartyIndex, String> {
    let mut combined = String::new();
    for (url, html) in pages {
        combined.push_str("\n\n## ");
        combined.push_str(url);
        combined.push('\n');
        combined.push_str(&page_text_for_model(html));
    }
    if combined.trim().is_empty() {
        return Err("no page text to send".to_string());
    }
    let started = std::time::Instant::now();
    let body = spark_body(&combined, authority_url);
    let (reply, prompt_tokens, completion_tokens) = spark_post(meta, &body)?;
    let authorities = parse_spark_output(&reply)?;
    let mut index = PartyIndex::empty(party, authority_url);
    index.origin = "spark";
    for candidate in &authorities {
        let cited = pages
            .iter()
            .find(|(_, html)| page_contains(&html_unescape(html), &candidate.name));
        let Some((source_url, source_html)) = cited else {
            continue;
        };
        let probe = html_unescape(source_html);
        index.names.push(NameHit {
            name: candidate.name.clone(),
            source_url: source_url.clone(),
            quote_ok: true,
        });
        for url in &candidate.urls {
            if index.urls.iter().any(|hit| hit.url == *url) {
                continue;
            }
            if page_contains(&probe, url) {
                index.urls.push(UrlHit {
                    url: url.clone(),
                    useful_link: false,
                    quote_ok: true,
                    page: source_url.clone(),
                    name: Some(candidate.name.clone()),
                });
            }
        }
    }
    eprintln!(
        "world-index: spark {}: {} candidate(s), kept {} name(s) {} url(s), \
         tokens {prompt_tokens}/{completion_tokens}, {:?}",
        party.name,
        authorities.len(),
        index.names.len(),
        index.urls.len(),
        started.elapsed()
    );
    Ok(index)
}

/// Page text for the model: scripts/styles dropped, tags folded to
/// line breaks, entities decoded, capped so the request stays small.
fn page_text_for_model(html: &str) -> String {
    let mut scrubbed = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(tag) = rest.find('<') {
        scrubbed.push_str(&rest[..tag]);
        let after = &rest[tag..];
        let end = after.find('>').map(|i| i + 1).unwrap_or(after.len());
        let tag_text = after[..end].to_lowercase();
        if tag_text.starts_with("<script") || tag_text.starts_with("<style") {
            let closer = if tag_text.starts_with("<script") {
                "</script>"
            } else {
                "</style>"
            };
            rest = after[end..]
                .split_once(closer)
                .map(|(_, tail)| tail)
                .unwrap_or("");
            continue;
        }
        scrubbed.push('\n');
        rest = &after[end..];
    }
    scrubbed.push_str(rest);
    let text = collapse_ws(&html_unescape(&scrubbed));
    const CAP: usize = 30_000;
    if text.len() > CAP {
        format!("{}…[truncated]", &text[..CAP])
    } else {
        text
    }
}

/// Chat request body for the irregular-page pass. Shape follows the
/// repo's proven client (`handbook-agent/src/model.rs`, findings
/// F1/F3/F5): `developer` standing rules + `user` per-run values, no
/// `temperature` (Spark is tuned at its defaults), explicit
/// `reasoning_effort`, strict `json_schema` response format.
fn spark_body(page_text: &str, source_url: &str) -> serde_json::Value {
    serde_json::json!({
        "model": DRAFT_MODEL,
        "messages": [
            {"role": "developer", "content": "You extract apostille competent-authority names and official website URLs from one HCCH competent-authority page. Copy each name and each URL exactly as it appears on the page; invent nothing, translate nothing. Reply with JSON only."},
            {"role": "user", "content": format!("Source page: {source_url}\n\nPage text:\n{page_text}")},
        ],
        "reasoning_effort": "medium",
        "max_completion_tokens": 8000,
        "response_format": {
            "type": "json_schema",
            "json_schema": {
                "name": "WorldIndexAuthorities",
                "schema": {
                    "type": "object",
                    "properties": {
                        "authorities": {
                            "type": "array",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "name": {"type": "string"},
                                    "urls": {"type": "array", "items": {"type": "string"}}
                                },
                                "required": ["name", "urls"],
                                "additionalProperties": false
                            }
                        }
                    },
                    "required": ["authorities"],
                    "additionalProperties": false
                }
            }
        },
        "stream": false,
    })
}

/// POST the body with `curl --config` (the key travels in a 0600
/// file, never on the command line). Retries transport failures and
/// 429/5xx with jitter, honouring `Retry-After`; other statuses are
/// terminal. Returns the raw reply body plus token usage.
///
/// A recorded answer (same canonical body) replays from the call cache
/// with no curl at all; every live attempt is admitted against and
/// counted toward the daily cap; a 2xx reply is stored.
fn spark_post(
    meta: &SparkCall<'_>,
    body: &serde_json::Value,
) -> Result<(Vec<u8>, u64, u64), String> {
    let (base_url, key, cache) = (meta.base_url, meta.key, meta.cache);
    let url = format!("{}/chat/completions", base_url.trim_end_matches('/'));
    let cache_key = callcache::meta::json_key(&url, body);
    if let Some(hit) = cache.lookup(&cache_key).map_err(|e| e.to_string())? {
        let (prompt_tokens, completion_tokens) = reply_usage(&hit.body);
        return Ok((hit.body, prompt_tokens, completion_tokens));
    }
    let request = callcache::meta::Request {
        method: "POST",
        url: &url,
        model: DRAFT_MODEL,
    };
    let dir = std::env::temp_dir().join("handbook-world-index");
    std::fs::create_dir_all(&dir).map_err(|e| format!("temp dir: {e}"))?;
    let stamp = unix_now_nanos();
    let body_path = dir.join(format!("spark-body-{stamp}.json"));
    let cfg_path = dir.join(format!("spark-cfg-{stamp}.txt"));
    let out_path = dir.join(format!("spark-out-{stamp}.json"));
    let hdr_path = dir.join(format!("spark-hdr-{stamp}.txt"));
    std::fs::write(
        &body_path,
        serde_json::to_string(body).map_err(|e| e.to_string())?,
    )
    .map_err(|e| format!("write body: {e}"))?;
    {
        use std::os::unix::fs::OpenOptionsExt;
        let mut cfg = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&cfg_path)
            .map_err(|e| format!("write config: {e}"))?;
        write_cfg(&mut cfg, key)?;
    }
    let mut attempt_no = 0u32;
    loop {
        if let Err(e) = cache.admit(&cache_key) {
            // Never leave the key-bearing config behind.
            let _ = std::fs::remove_file(&cfg_path);
            return Err(e.to_string());
        }
        let code = Command::new("curl")
            .args(["-sS", "-m", "300", "-X", "POST", "--config"])
            .arg(&cfg_path)
            .args(["-H", "Content-Type: application/json", "--data", "@"])
            .arg(&body_path)
            .args(["-D", "--"])
            .arg(&hdr_path)
            .args(["-o", "--"])
            .arg(&out_path)
            .args(["-w", "%{http_code}"])
            .arg(&url)
            .output()
            .map_err(|e| format!("run curl: {e} (is it installed?)"))?;
        // Best effort: drop the key-bearing config at once.
        let _ = std::fs::remove_file(&cfg_path);
        if !code.status.success() {
            cache.spent(&request, 0);
            return Err("curl transport failure".to_string());
        }
        let status: u16 = String::from_utf8_lossy(&code.stdout)
            .trim()
            .parse()
            .unwrap_or(0);
        cache.spent(&request, status);
        let reply = std::fs::read(&out_path).unwrap_or_default();
        if status == 429 || (500..=504).contains(&status) {
            if attempt_no + 1 >= MAX_ATTEMPTS {
                return Err(format!(
                    "meta api: http {status} after {MAX_ATTEMPTS} attempts"
                ));
            }
            attempt_no += 1;
            std::thread::sleep(retry_delay(attempt_no, retry_after_secs(&hdr_path)));
            rewrite_cfg(&cfg_path, key)?;
            continue;
        }
        if !(200..300).contains(&status) {
            return Err(format!("meta api: http {status}: {}", clipped(&reply)));
        }
        // A full disk must not discard an answer already paid for.
        let _ = cache.store(&cache_key, &request, status, &reply);
        let (prompt_tokens, completion_tokens) = reply_usage(&reply);
        return Ok((reply, prompt_tokens, completion_tokens));
    }
}

fn write_cfg(cfg: &mut std::fs::File, key: &str) -> Result<(), String> {
    use std::io::Write;
    writeln!(cfg, "header = \"Authorization: Bearer {key}\"")
        .map_err(|e| format!("write config: {e}"))
}

fn rewrite_cfg(path: &Path, key: &str) -> Result<(), String> {
    {
        use std::os::unix::fs::OpenOptionsExt;
        let mut cfg = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(path)
            .map_err(|e| format!("write config: {e}"))?;
        write_cfg(&mut cfg, key)?;
    }
    Ok(())
}

fn retry_after_secs(hdr_path: &Path) -> Option<u64> {
    let headers = std::fs::read_to_string(hdr_path).ok()?;
    for line in headers.lines() {
        if line.to_lowercase().starts_with("retry-after:") {
            return line.split(':').nth(1)?.trim().parse().ok();
        }
    }
    None
}

/// Backoff for retry `attempt_no` (1-based): 2s doubling plus up to
/// +50% nanos jitter, or the server's `Retry-After` when present.
fn retry_delay(attempt_no: u32, retry_after: Option<u64>) -> Duration {
    if let Some(after) = retry_after {
        return Duration::from_secs(after.min(60));
    }
    let base = 2u64.saturating_mul(1u64 << attempt_no.saturating_sub(1).min(5));
    let jitter = (unix_now_nanos() % (base as u128 * 500 + 1)) as u64;
    Duration::from_secs(base) + Duration::from_millis(jitter)
}

fn clipped(reply: &[u8]) -> String {
    let text = String::from_utf8_lossy(reply);
    text.chars().take(120).collect()
}

/// Token usage from a non-streaming chat completion reply.
fn reply_usage(reply: &[u8]) -> (u64, u64) {
    let value: serde_json::Value = serde_json::from_slice(reply).unwrap_or_default();
    (
        value["usage"]["prompt_tokens"].as_u64().unwrap_or(0),
        value["usage"]["completion_tokens"].as_u64().unwrap_or(0),
    )
}

/// Parse the model reply: `choices[0].message.content` is itself a
/// JSON document with the `authorities` array.
fn parse_spark_output(reply: &[u8]) -> Result<Vec<SparkAuthority>, String> {
    let value: serde_json::Value =
        serde_json::from_slice(reply).map_err(|e| format!("reply JSON: {e}"))?;
    let content = value["choices"][0]["message"]["content"]
        .as_str()
        .ok_or_else(|| "reply has no choices[0].message.content".to_string())?;
    let inner: serde_json::Value =
        serde_json::from_str(content).map_err(|e| format!("content JSON: {e}"))?;
    let authorities = inner["authorities"]
        .as_array()
        .ok_or_else(|| "content has no authorities[]".to_string())?;
    let mut out = Vec::new();
    for entry in authorities {
        let name = entry["name"].as_str().unwrap_or("").trim().to_string();
        if name.is_empty() {
            continue;
        }
        let urls = entry["urls"]
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item.as_str().map(str::trim))
                    .filter(|url| !url.is_empty())
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        out.push(SparkAuthority { name, urls });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Recorded fixture bytes (public HCCH text, no network).
    fn fixture(name: &str) -> String {
        let dir = env!("CARGO_MANIFEST_DIR");
        std::fs::read_to_string(format!("{dir}/../evals/world/{name}"))
            .unwrap_or_else(|_| panic!("missing fixture {name}"))
    }

    /// `&str` view of a `Vec<String>` for readable expectations.
    fn strs(v: &[String]) -> Vec<&str> {
        v.iter().map(String::as_str).collect()
    }

    /// Extracted URL strings for one fixture details page.
    fn urls_of(page: &str) -> Vec<String> {
        let html = fixture(page);
        page_urls(&html).into_iter().map(|hit| hit.url).collect()
    }

    /// EIF column: every recorded party has a readable date, and the
    /// newest accessions (Algeria, Viet Nam; Thailand still ahead on the
    /// 2026-11-02 capture) carry their own dates.
    #[test]
    fn status_table_entry_into_force_dates() {
        let rows = parse_status_table(&fixture("status-table-cid41.html"));
        assert_eq!(rows.len(), 130);
        assert!(rows.iter().all(|row| row.eif.is_some()), "{rows:?}");
        let eif = |want: &str| {
            rows.iter()
                .find(|row| row.name == want)
                .and_then(|row| row.eif.clone())
                .unwrap_or_else(|| panic!("missing party {want}"))
        };
        assert_eq!(eif("Thailand"), "2027-02-28");
        assert_eq!(eif("Viet Nam"), "2026-09-11");
        assert_eq!(eif("Algeria"), "2026-07-09");
        assert_eq!(eif("Kosovo"), "2016-07-14");
        assert_eq!(hcch_date(" 5-X-1961 "), Some("1961-10-05".to_string()));
        for bad in ["", "28-XIII-2027", "32-I-2020", "1-I-2020-1", "A**"] {
            assert_eq!(hcch_date(bad), None, "{bad}");
        }
    }

    #[test]
    fn status_table_parses_130_parties() {
        let parties = parse_status_table(&fixture("status-table-cid41.html"));
        assert_eq!(parties.len(), 130);
        let by_name = |want: &str| {
            parties
                .iter()
                .find(|p| p.name == want)
                .unwrap_or_else(|| panic!("missing party {want}"))
                .clone()
        };
        assert_eq!(by_name("Albania").sid, "18");
        assert_eq!(by_name("Belarus").sid, "10");
        assert_eq!(by_name("Cook Islands").sid, "211");
        assert_eq!(by_name("France").sid, "39");
        assert_eq!(by_name("Japan").sid, "47");
        assert_eq!(by_name("Türkiye").sid, "74");
        assert_eq!(by_name("United States of America").sid, "76");
        assert_eq!(by_name("Viet Nam").sid, "230");
    }

    #[test]
    fn authority_links_point_at_details() {
        let cases = [
            ("authority-sid18.html", "305"),
            ("authority-sid10.html", "309"),
            ("authority-sid211.html", "632"),
            ("authority-sid39.html", "321"),
            ("authority-sid47.html", "328"),
            ("authority-sid74.html", "350"),
            ("authority-sid76.html", "353"),
            ("authority-sid230.html", "1240"),
        ];
        for (file, aid) in cases {
            let links = parse_authority_links(&fixture(file));
            assert_eq!(links.len(), 1, "{file}: one details link");
            assert_eq!(links[0].aid, aid);
            assert!(
                links[0]
                    .url
                    .starts_with("https://www.hcch.net/en/states/authorities/details3/?aid=")
            );
            assert!(links[0].title.contains("Competent Authorit"));
        }
    }

    #[test]
    fn belarus_names_and_urls() {
        let html = fixture("details-aid309.html");
        let names = designation_names(&html);
        assert_eq!(
            strs(&names),
            vec![
                "Ministry of Justice of the Republic of Belarus",
                "Regional Justice Administrations and Minsk City Executive Committee Justice Administration",
                "Ministry of Education of the Republic of Belarus",
                "Department of Archives and Records Management of the Ministry of Justice",
                "Ministry of Foreign Affairs of the Republic of Belarus",
            ]
        );
        let hits = page_urls(&html);
        let urls: Vec<String> = hits.into_iter().map(|hit| hit.url).collect();
        assert_eq!(
            strs(&urls),
            vec![
                "http://www.minjust.by/",
                "http://www.minedu.unibel.by/",
                "http://archives.gov.by/eindex.htm",
                "http://www.mfa.gov.by/",
            ]
        );
    }

    #[test]
    fn albania_keeps_plain_name_beside_parenthetical_em() {
        let names = designation_names(&fixture("details-aid305.html"));
        assert!(
            names
                .iter()
                .any(|n| n.contains("Office of Authentications")),
            "plain English name kept: {names:?}"
        );
        let urls = urls_of("details-aid305.html");
        assert_eq!(strs(&urls), vec!["http://www.mfa.gov.al"]);
    }

    #[test]
    fn vietnam_numbered_designations() {
        let names = designation_names(&fixture("details-aid1240.html"));
        assert_eq!(names.len(), 3);
        assert!(
            names[0].starts_with(
                "The Ministry of Foreign Affairs of the Socialist Republic of Viet Nam"
            )
        );
        assert!(names[1].contains("Consular Department"));
        assert!(names[2].contains("Ho Chi Minh City"));
        let urls = urls_of("details-aid1240.html");
        assert_eq!(
            strs(&urls),
            vec![
                "https://dichvucong.mofa.gov.vn",
                "http://www.mofahcm.gov.vn"
            ]
        );
    }

    #[test]
    fn japan_single_name_two_urls() {
        let names = designation_names(&fixture("details-aid328.html"));
        assert_eq!(
            strs(&names),
            vec!["The Ministry of Foreign Affairs in Tokyo"]
        );
        let urls = urls_of("details-aid328.html");
        assert_eq!(
            strs(&urls),
            vec![
                "http://www.mofa.go.jp/",
                "https://www.mofa.go.jp/ca/cs/page22e_000416.html"
            ]
        );
    }

    #[test]
    fn turkiye_section_heads_become_names() {
        let names = designation_names(&fixture("details-aid350.html"));
        assert!(
            names.iter().any(|n| n.contains("Administrative documents")),
            "{names:?}"
        );
        assert!(
            names.iter().any(|n| n.contains("Judicial documents")),
            "{names:?}"
        );
        let urls = urls_of("details-aid350.html");
        assert_eq!(strs(&urls), vec!["http://www.adalet.gov.tr/"]);
    }

    #[test]
    fn france_six_designations_plus_useful_links() {
        let names = designation_names(&fixture("details-aid321.html"));
        assert_eq!(names.len(), 6);
        assert!(names[0].contains("France métropolitaine"));
        // The page's disclaimed "Central Authority" contact (stated
        // not competent to issue apostilles) is never a name.
        assert!(!names.iter().any(|n| n.contains("Central Authority")));
        let urls = urls_of("details-aid321.html");
        assert_eq!(
            strs(&urls),
            vec![
                "https://www.service-public.fr/particuliers/vosdroits/F1400",
                "https://apostille.notaires.fr/accueil",
                "http://www.service-public.fr/particuliers/vosdroits/R47867",
            ]
        );
    }

    #[test]
    fn usa_variant_website_label_and_useful_link() {
        // The USA page designates in prose paragraphs (kept verbatim)
        // plus one numbered `<em>` office.
        let names = designation_names(&fixture("details-aid353.html"));
        assert_eq!(names.len(), 5);
        assert!(
            names
                .iter()
                .any(|n| n.contains("Office of Authentications issues Apostilles"))
        );
        assert!(
            names
                .iter()
                .any(|n| n.contains("Bureau of Consular Affairs"))
        );
        assert!(names.iter().any(|n| n.contains("Clerks and Deputy Clerks")));
        assert!(names.iter().any(|n| n.contains("Secretary of State")));
        assert!(
            names
                .iter()
                .any(|n| n.contains("United States Department of State."))
        );
        let urls = urls_of("details-aid353.html");
        assert_eq!(urls.len(), 2);
        assert!(urls[0].starts_with("https://travel.state.gov/"));
        assert_eq!(urls[1], "http://www.nass.org");
    }

    #[test]
    fn cook_islands_single_designation_useful_link_url() {
        let names = designation_names(&fixture("details-aid632.html"));
        assert_eq!(
            strs(&names),
            vec!["The Ministry of Foreign Affairs and Immigration"]
        );
        let urls = urls_of("details-aid632.html");
        assert_eq!(strs(&urls), vec!["http://www.mfai.gov.ck/"]);
    }

    #[test]
    fn quote_check_passes_every_fixture_value() {
        let pages = [
            "details-aid305.html",
            "details-aid309.html",
            "details-aid321.html",
            "details-aid328.html",
            "details-aid350.html",
            "details-aid353.html",
            "details-aid632.html",
            "details-aid1240.html",
        ];
        for page in pages {
            let html = fixture(page);
            let probe = html_unescape(&html);
            for name in designation_names(&html) {
                assert!(
                    page_contains(&probe, &name),
                    "{page}: name not on page: {name}"
                );
            }
            for hit in page_urls(&html) {
                assert!(
                    page_contains(&probe, &hit.url),
                    "{page}: url not on page: {}",
                    hit.url
                );
            }
        }
    }

    #[test]
    fn quote_check_refuses_tampered_values() {
        let html = fixture("details-aid309.html");
        let probe = html_unescape(&html);
        assert!(page_contains(
            &probe,
            "Ministry of Justice of the Republic of Belarus"
        ));
        assert!(!page_contains(
            &probe,
            "Ministry of Justice of the Republic of Belarus!"
        ));
        assert!(page_contains(&probe, "http://www.minjust.by/"));
        assert!(!page_contains(&probe, "http://www.minjust.by.evil/"));
        assert!(!page_contains(&probe, "x"));
    }

    #[test]
    fn entities_decode_before_matching() {
        assert_eq!(html_unescape("d&rsquo;appel &amp; co"), "d’appel & co");
        assert_eq!(html_unescape("A&#768;"), "A\u{300}");
        assert_eq!(html_unescape("&bogus;"), "&bogus;");
        // A bare `&` (page scripts) never swallows a later entity.
        assert_eq!(html_unescape("a & b; c&nbsp;d"), "a & b; c d");
        assert_eq!(html_unescape("x &nbsp;y"), "x  y");
        // The France page stores apostrophes as &rsquo;: the decoded
        // name still matches the decoded page.
        let html = fixture("details-aid321.html");
        let probe = html_unescape(&html);
        let names = designation_names(&html);
        assert!(names.iter().any(|n| n.contains("d’appel")));
        for name in &names {
            assert!(page_contains(&probe, name));
        }
    }

    #[test]
    fn enumerator_stripping() {
        assert_eq!(
            strip_enumerator("1. The Consular Department"),
            "The Consular Department"
        );
        assert_eq!(strip_enumerator("b) in towns: Head"), "in towns: Head");
        assert_eq!(
            strip_enumerator("I. Authentication Officer"),
            "Authentication Officer"
        );
        assert_eq!(strip_enumerator("- Designated body"), "Designated body");
        assert_eq!(strip_enumerator("(a) Judicial docs"), "Judicial docs");
        // Real words are not enumerators.
        assert_eq!(strip_enumerator("Saint Lucia office"), "Saint Lucia office");
        assert_eq!(
            strip_enumerator("In provinces: Governor"),
            "In provinces: Governor"
        );
    }

    #[test]
    fn iso2_covers_fixture_parties() {
        let cases = [
            ("Albania", "AL"),
            ("Belarus", "BY"),
            ("Cook Islands", "CK"),
            ("France", "FR"),
            ("Japan", "JP"),
            ("Türkiye", "TR"),
            ("United States of America", "US"),
            ("Viet Nam", "VN"),
            ("Kosovo", "XK"),
            ("United Kingdom of Great Britain and Northern Ireland", "GB"),
            ("Netherlands (Kingdom of the)", "NL"),
            ("China (the People's Republic of)", "CN"),
        ];
        for (party, code) in cases {
            assert_eq!(party_iso2(party), Some(code), "{party}");
        }
        assert_eq!(party_iso2("Not a Party"), None);
    }

    #[test]
    fn iso2_covers_every_table_party() {
        let parties = parse_status_table(&fixture("status-table-cid41.html"));
        let mut missing = Vec::new();
        for party in &parties {
            if party_iso2(&party.name).is_none() {
                missing.push(party.name.clone());
            }
        }
        assert!(missing.is_empty(), "parties without ISO2: {missing:?}");
    }

    #[test]
    fn spark_body_shape_follows_findings() {
        let body = spark_body("page text", "https://example.invalid/p");
        assert_eq!(body["model"], DRAFT_MODEL);
        assert!(
            body.get("temperature").is_none(),
            "no temperature (finding F1)"
        );
        assert_eq!(body["reasoning_effort"], "medium");
        assert_eq!(body["messages"][0]["role"], "developer");
        assert_eq!(body["messages"][1]["role"], "user");
        assert_eq!(body["response_format"]["type"], "json_schema");
        assert_eq!(
            body["response_format"]["json_schema"]["schema"]["required"],
            serde_json::json!(["authorities"])
        );
    }

    #[test]
    fn spark_output_parses_and_usage_reads() {
        let inner = serde_json::json!({
            "authorities": [
                {"name": "Ministry of Example", "urls": ["https://example.invalid/"]},
                {"name": "", "urls": []},
            ]
        })
        .to_string();
        let reply = serde_json::json!({
            "choices": [{"message": {"content": inner}}],
            "usage": {"prompt_tokens": 100, "completion_tokens": 20},
        });
        let bytes = serde_json::to_vec(&reply).unwrap();
        let authorities = parse_spark_output(&bytes).unwrap();
        assert_eq!(
            authorities,
            vec![SparkAuthority {
                name: "Ministry of Example".to_string(),
                urls: vec!["https://example.invalid/".to_string()],
            }]
        );
        assert_eq!(reply_usage(&bytes), (100, 20));
        assert!(parse_spark_output(b"{}").is_err());
    }

    #[test]
    fn built_pack_validates_clean() {
        let html = fixture("details-aid309.html");
        let party = PartyRow {
            name: "Belarus".to_string(),
            sid: "10".to_string(),
            ..PartyRow::default()
        };
        let mut index = PartyIndex::empty(&party, "https://www.hcch.net/en/x");
        index.absorb_details(
            "https://www.hcch.net/en/states/authorities/details3/?aid=309",
            &html,
        );
        assert_eq!(index.names.len(), 5);
        assert!(index.names.iter().all(|h| h.quote_ok));
        let names: Vec<NameHit> = index.names.clone();
        let urls: Vec<UrlHit> = index.urls.clone();
        let file = build_pack(&index, &names, &urls, None);
        let pack = &file.pack;
        assert_eq!(pack.pack, "BY");
        assert_eq!(file.ext.iso2, "BY");
        assert!(pack.authorities.len() == 5);
        assert!(pack.watch.len() == 1 + 4);
        assert!(pack.observations.iter().all(|o| o.status == "needs_review"));
        // One details page: every listed URL belongs to its first
        // named authority, not to the name at the same list index.
        assert!(
            pack.watch.iter().all(|w| w.authority == "by-ca-1"),
            "{:?}",
            pack.watch
        );
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("BY.json");
        std::fs::write(&path, file.render()).unwrap();
        assert!(crate::packs::validate_dir(dir.path()).is_empty());
    }

    fn hit(name: &str, page: &str) -> NameHit {
        NameHit {
            name: name.to_string(),
            source_url: page.to_string(),
            quote_ok: true,
        }
    }

    fn url(url: &str, page: &str, name: Option<&str>) -> UrlHit {
        UrlHit {
            url: url.to_string(),
            useful_link: false,
            quote_ok: true,
            page: page.to_string(),
            name: name.map(str::to_string),
        }
    }

    /// P2-1: fragments are emitted hidden; URLs go to a visible
    /// authority on their own page (or the one their source names).
    #[test]
    fn fragments_hide_and_urls_follow_their_page() {
        let party = PartyRow {
            name: "Germany".to_string(),
            sid: "1".to_string(),
            ..PartyRow::default()
        };
        let index = PartyIndex::empty(&party, "https://www.hcch.net/en/x");
        let (a, b) = ("https://www.hcch.net/a", "https://www.hcch.net/b");
        let names = [
            hit("Land", a),
            hit("Oberlandesgericht", a),
            hit("Federal Office of Justice", a),
            hit("Federal Foreign Office", b),
        ];
        let urls = [
            url("https://bfaa.diplo.de/x", a, None),
            url("https://www.dpma.de/", a, None),
            url("https://www.auswaertiges-amt.de/", b, None),
            url(
                "https://www.bundesjustizamt.de/",
                b,
                Some("Federal Office of Justice"),
            ),
        ];
        let file = build_pack(&index, &names, &urls, None);
        let hidden: Vec<bool> = (0..4)
            .map(|i| file.ext_of(i).hidden == Some(true))
            .collect();
        // "Land" is 4 characters; "Oberlandesgericht" passes the lint
        // (curation hides it: a court type, not one office).
        assert_eq!(hidden, [true, false, false, false]);
        let owners: Vec<(&str, &str)> = file
            .pack
            .watch
            .iter()
            .map(|w| (w.uri.as_str(), w.authority.as_str()))
            .collect();
        assert_eq!(
            owners,
            [
                ("https://www.hcch.net/en/x", "de-ca-2"),
                ("https://bfaa.diplo.de/x", "de-ca-2"),
                ("https://www.dpma.de/", "de-ca-2"),
                ("https://www.auswaertiges-amt.de/", "de-ca-4"),
                ("https://www.bundesjustizamt.de/", "de-ca-3"),
            ]
        );
        // No "designated authority" task for a hidden fragment.
        assert!(
            !file
                .pack
                .observations
                .iter()
                .any(|o| o.statement.contains("designated authority: Land."))
        );
    }

    /// A re-run keeps committed slugs, curated names and hidden flags,
    /// and numbers a new name after the highest slug.
    #[test]
    fn rerun_keeps_slugs_curation_and_hidden_rows() {
        let party = PartyRow {
            name: "Marshall Islands".to_string(),
            sid: "2".to_string(),
            ..PartyRow::default()
        };
        let index = PartyIndex::empty(&party, "https://www.hcch.net/en/x");
        let page = "https://www.hcch.net/p";
        let first = build_pack(
            &index,
            &[
                hit("Minister of Foreign Affairs,", page),
                hit("Limmatquai 50", page),
            ],
            &[],
            None,
        );
        assert_eq!(
            (0..2).map(|i| first.ext_of(i).hidden).collect::<Vec<_>>(),
            [Some(true), Some(true)]
        );
        // Curate: fix the first name from the page, keep the second hidden.
        let mut curated = first.clone();
        curated.pack.authorities[0].name = "Minister of Foreign Affairs".to_string();
        curated.ext.authorities[0].hidden = None;
        curated.ext.authorities[0].source_name = "Minister of Foreign Affairs,".to_string();
        let curated = crate::packs::PackFile::parse(curated.render().as_bytes()).unwrap();
        let again = build_pack(
            &index,
            &[
                hit("Limmatquai 50", page),
                hit(
                    "IRI Corporate and Maritime Services (Switzerland) A.G.",
                    page,
                ),
                hit("Minister of Foreign Affairs,", page),
            ],
            &[url("https://www.register-iri.com/", page, None)],
            Some(&curated),
        );
        let rows: Vec<(&str, &str, Option<bool>)> = again
            .pack
            .authorities
            .iter()
            .enumerate()
            .map(|(i, a)| (a.slug.as_str(), a.name.as_str(), again.ext_of(i).hidden))
            .collect();
        assert_eq!(
            rows,
            [
                ("mh-ca-1", "Minister of Foreign Affairs", None),
                ("mh-ca-2", "Limmatquai 50", Some(true)),
                (
                    "mh-ca-3",
                    "IRI Corporate and Maritime Services (Switzerland) A.G.",
                    None
                ),
            ]
        );
        assert_eq!(
            again.pack.watch[1].authority, "mh-ca-3",
            "first visible on the page"
        );
    }

    /// P3-9: a live run never rewrites the hand-built US, MX or CA packs.
    #[test]
    fn hand_built_packs_are_never_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        for iso in ["US", "MX", "CA"] {
            let committed = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../packs/world")
                .join(format!("{iso}.json"));
            let bytes = std::fs::read(&committed).unwrap();
            let path = dir.path().join(format!("{iso}.json"));
            std::fs::write(&path, &bytes).unwrap();
            let party = PartyRow {
                name: "Somewhere".to_string(),
                sid: "3".to_string(),
                ..PartyRow::default()
            };
            let mut index = PartyIndex::empty(&party, "https://www.hcch.net/en/x");
            index.iso2 = iso.to_string();
            index
                .names
                .push(hit("Ministry of Foreign Affairs", "https://www.hcch.net/p"));
            assert!(!write_pack(&path, &index), "{iso}");
            assert_eq!(std::fs::read(&path).unwrap(), bytes, "{iso}");
        }
    }

    /// D-1: a party's entry into force rides on its status-table line,
    /// tense-free, so a future party's dossier shows the date.
    #[test]
    fn status_line_carries_entry_into_force() {
        let parties = parse_status_table(&fixture("status-table-cid41.html"));
        let thailand = parties.iter().find(|p| p.name == "Thailand").unwrap();
        let index = PartyIndex::empty(thailand, "https://www.hcch.net/en/x");
        let file = build_pack(&index, &[], &[], None);
        assert_eq!(
            file.pack.observations[0].statement,
            "OBSERVED (HCCH 1961 status table cid=41; staff capture pending): Thailand is a \
             listed Contracting Party; entry into force for Thailand: 2027-02-28."
        );
        assert_eq!(file.pack.observations[0].source_uri, STATUS_TABLE_URL);
        let undated = PartyRow {
            name: "Noland".to_string(),
            ..PartyRow::default()
        };
        assert!(
            status_statement(&PartyIndex::empty(&undated, "https://www.hcch.net/en/x"))
                .ends_with("Noland is a listed Contracting Party.")
        );
    }

    /// D-1: without an authority page (offline, no fixture) only the
    /// status-table line of the committed pack changes; curated rows,
    /// hidden flags and every other line stay byte for byte, and the
    /// hand-built packs are left alone.
    #[test]
    fn refresh_touches_only_the_status_line() {
        let dir = tempfile::tempdir().unwrap();
        let world = Path::new(env!("CARGO_MANIFEST_DIR")).join("../packs/world");
        let parties = parse_status_table(&fixture("status-table-cid41.html"));
        for (iso, name) in [("TH", "Thailand"), ("MH", "Marshall Islands")] {
            let committed = std::fs::read_to_string(world.join(format!("{iso}.json"))).unwrap();
            let path = dir.path().join(format!("{iso}.json"));
            std::fs::write(&path, &committed).unwrap();
            let party = parties.iter().find(|p| p.name == name).unwrap();
            let index = PartyIndex::empty(party, "https://www.hcch.net/en/x");
            let want = status_statement(&index);
            let changed = !committed.contains(&want);
            assert_eq!(refresh_status(&path, &index), changed, "{iso}");
            let after = std::fs::read_to_string(&path).unwrap();
            assert!(after.contains(&want), "{iso}");
            let differing: Vec<(&str, &str)> = committed
                .lines()
                .zip(after.lines())
                .filter(|(a, b)| a != b)
                .collect();
            assert_eq!(committed.lines().count(), after.lines().count(), "{iso}");
            assert!(differing.len() <= 1, "{iso}: {differing:?}");
            assert!(
                differing
                    .iter()
                    .all(|(_, b)| b.contains("entry into force")),
                "{iso}"
            );
            // A second run is a no-op.
            assert!(!refresh_status(&path, &index), "{iso}");
        }
        let committed = std::fs::read(world.join("US.json")).unwrap();
        let path = dir.path().join("US.json");
        std::fs::write(&path, &committed).unwrap();
        let usa = parties
            .iter()
            .find(|p| p.name == "United States of America")
            .unwrap();
        assert!(!refresh_status(
            &path,
            &PartyIndex::empty(usa, "https://www.hcch.net/en/x")
        ));
        assert_eq!(std::fs::read(&path).unwrap(), committed);
        assert!(!refresh_status(
            &dir.path().join("ZZ.json"),
            &PartyIndex::empty(usa, "https://www.hcch.net/en/x")
        ));
    }

    #[test]
    fn csv_quotes_commas() {
        assert_eq!(csv_row(&["BY", "a,b", "plain"]), "BY,\"a,b\",plain\n");
    }

    #[test]
    fn semicolon_gloms_split_paren_aware() {
        let split = split_items(
            "The Ministry of the Interior;The Ministry of Justice;The Ministry of Posts",
        );
        assert_eq!(split.len(), 3);
        assert_eq!(split[1], "The Ministry of Justice");
        let glued = split_items(
            "Australian Embassies (except Consulates headed by an Honorary Consul; see note); Second Body",
        );
        assert_eq!(glued.len(), 2);
        assert!(glued[0].contains("(except Consulates headed by an Honorary Consul; see note)"));
        assert_eq!(
            split_items("The Consular Department (Headquarters in Hanoi);"),
            vec!["The Consular Department (Headquarters in Hanoi)".to_string()]
        );
    }

    #[test]
    fn meta_notes_drop_but_albanian_name_survives() {
        assert!(is_meta_note("(courtesy translation)"));
        assert!(is_meta_note("(please click here for details)"));
        assert!(!is_meta_note(
            "(Drejtoria Konsullore, Zyra e Legalizimeve, Ministria e Puneve te Jashtme)"
        ));
        assert!(!is_meta_note(
            "Ministry of Justice of the Republic of Belarus"
        ));
        let names = designation_names(&fixture("details-aid305.html"));
        assert_eq!(names.len(), 2);
    }

    #[test]
    fn em_inner_breaks_split_and_fragments_drop() {
        let html = "<p>Designated Competent Authority(ies):</p>\
            <p><em>First Ministry;<br />Second Ministry;<br />Third Ministry</em></p>\
            <p><em>Head Office empowered<br /> to issue papers</em></p>\
            <p><strong>Contact details:</strong></p>";
        let names = designation_names(html);
        assert_eq!(
            names,
            vec![
                "First Ministry".to_string(),
                "Second Ministry".to_string(),
                "Third Ministry".to_string(),
                "Head Office empowered".to_string(),
            ]
        );
    }

    #[test]
    fn block_ends_at_practical_information() {
        let html = "<p><strong>Designated Competent Authority(ies):</strong></p>\
            <p><em>Headquarters Office</em></p>\
            <table><tr><td>Address:</td><td>1 Main St</td></tr></table>\
            <p><strong>Practical Information:</strong></p>\
            <table><tr><td>Price:</td><td>Free</td></tr></table>";
        let names = designation_names(html);
        assert_eq!(names, vec!["Headquarters Office".to_string()]);
    }

    #[test]
    fn block_without_closing_marker_is_irregular() {
        let html = "<p><strong>Designated Competent Authority(ies):</strong></p>\
            <p>Some body with no sections after it";
        assert!(designation_block(html).is_none());
        assert!(designation_names(html).is_empty());
    }

    #[test]
    fn meta_sentences_never_become_names() {
        // SA-17 regression: the Mexico page's count/cross-reference
        // sentences shipped as authority names in an earlier pack.
        let html = "<p>Designated Competent Authority(ies):</p>\
            <p><em>Apostille for Federal Documents</em><br />\
            There are 32 competent authorities designated to issue Apostilles for state documents<br />\
            for more information see below under Contact Details.</p>\
            <p><strong>Contact details:</strong></p>";
        let names = designation_names(html);
        assert_eq!(names, vec!["Apostille for Federal Documents".to_string()]);
        assert!(is_meta_sentence(
            "There are 32 competent authorities designated to issue Apostilles"
        ));
        assert!(is_meta_sentence(
            "for more information see below under Contact Details."
        ));
        assert!(!is_meta_sentence(
            "The U.S. Department of State's Office of Authentications issues Apostilles"
        ));
    }

    #[test]
    fn list_items_without_em_parse() {
        let html = "<p>Designated Competent Authority(ies):</p>\
            <ol><li><div>Alpha Ministry</div></li><li><div>Beta Ministry</div></li></ol>\
            <p><strong>Contact details:</strong></p>";
        assert_eq!(
            designation_names(html),
            vec!["Alpha Ministry".to_string(), "Beta Ministry".to_string()]
        );
    }

    #[test]
    fn csv_skips_placeholder_when_urls_exist() {
        let party = PartyRow {
            name: "Noland".to_string(),
            sid: "9".to_string(),
            ..PartyRow::default()
        };
        let mut index = PartyIndex::empty(&party, "https://www.hcch.net/en/x");
        index.urls.push(UrlHit {
            url: "https://example.invalid/".to_string(),
            useful_link: false,
            quote_ok: true,
            page: "https://www.hcch.net/en/x".to_string(),
            name: None,
        });
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("authorities.csv");
        write_csv(&path, &[index]);
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(text.lines().count(), 2, "{text}");
        assert!(
            !text.contains(",,https://www.hcch.net/en/x,false"),
            "{text}"
        );
    }
}
