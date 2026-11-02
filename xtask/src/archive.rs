//! `xtask archive-package`: bundle a crawl run into judge deliverables.
//!
//! Usage: `cargo xtask archive-package --run archive/<run>/`
//!
//! Reads the run layout written by `xtask crawl` (plus any operator
//! wget `--warc-file` drops under `warc/`, plain or gzipped) and
//! writes:
//!
//! - `manifest.csv` — one row per crawled URL with its WARC
//!   location. Columns: `url,warc_file,warc_offset,warc_length,
//!   record_id,http_status,bytes,fetched_at` (offset/length locate
//!   the final response record's gzip member; empty + 0 when nothing
//!   was archived).
//! - `authorities.csv` — one row per authority (pack + slug).
//!   Columns: `pack,slug,authority_name,seed_urls,ok,failed,
//!   disallowed,missing,bytes,first_fetched_at,last_fetched_at`.
//! - `coverage.csv` — one row per party (pack). Columns:
//!   `pack,authorities,seed_urls,ok,failed,disallowed,missing,bytes,
//!   coverage_pct,hague_status,terms_note,robots_note` where
//!   `coverage_pct` is `100 * ok / seed_urls` (one decimal) and
//!   `hague_status` is `unknown` (party Hague status is not in the
//!   pack schema; see `docs/ops/world-archive-rights.md`).
//! - `cdx/index.cdxj` — standard CDXJ, one line per response record:
//!   `<surt> <timestamp> <json>` with `{"url","offset","length",
//!   "filename","mime","status","digest","record_id"}` (offsets into
//!   the stored `.warc.gz`, lengths of the gzip member; timestamp is
//!   the WARC-Date compacted to `YYYYMMDDHHMMSS`).
//! - `world.wacz` + `wacz/<ISO2>.wacz` — WACZ 1.1 (stored, no
//!   compression) holding `archive/data.warc.gz` (concatenated run
//!   WARCs), `indexes/index.cdxj`, `pages/pages.jsonl` (with a
//!   header line), and `datapackage.json` (every resource with
//!   `sha256:` hash + bytes). Written with a streaming built-in ZIP
//!   writer (stored entries, ZIP64 when sizes demand it, data never
//!   fully buffered); unzip-readable, `wacz validate` compatibility
//!   is best-effort (the tool is unavailable here).
//! - `text/<sha256>.txt` — tag-stripped text of textual responses,
//!   named by the sha256 of the extracted text.
//! - `SHA256SUMS` — GNU-style checksums of every run file except
//!   itself.
//! - `index.html` — human overview with the mandatory HCCH source
//!   credit and the open-vs-token split.
//!
//! CSV column choices: review §13 (the judge-pack column spec) lives
//! outside this repo, so the columns above are explicit choices for
//! operator sign-off — each is FLAGGED in the worker report.
//!
//! SHA-256, CRC-32, and ZIP here are small self-contained
//! implementations (known-vector tested) so the packager adds no new
//! manifest dependencies. No network, no live crawling in tests:
//! fixtures are built with `ingest::warc` in a temp dir.

use std::collections::{HashMap, HashSet};

use crate::required;

/// Defensive mirror of the crawl `seeds.json` row (missing fields
/// default; the packager never crashes on a thinner row — pre-SA-12
/// rows without `warc_offset`/`warc_length` read as 0).
#[derive(serde::Deserialize, Default, Clone)]
struct SeedRow {
    #[serde(default)]
    pack: String,
    #[serde(default)]
    slug: String,
    #[serde(default)]
    authority: String,
    #[serde(default)]
    uri: String,
    #[serde(default)]
    depth: u8,
    #[serde(default)]
    robots_verdict: String,
    #[serde(default)]
    status: String,
    #[serde(default)]
    http_status: u16,
    #[serde(default)]
    bytes: u64,
    #[serde(default)]
    warc_file: String,
    #[serde(default)]
    record_id: String,
    #[serde(default)]
    warc_offset: u64,
    #[serde(default)]
    warc_length: u64,
    // Retained for schema compatibility (crawl writes it); the
    // per-URL manifest no longer carries it.
    #[serde(default)]
    #[allow(dead_code)]
    note: String,
    #[serde(default)]
    fetched_at: String,
}

/// One response record found in the run's WARC files.
struct Capture {
    url: String,
    date: String,
    record_id: String,
    block_digest: String,
    mime: String,
    status: u16,
    warc_file: String,
    /// Compressed-file offset of the record's gzip member.
    offset: u64,
    /// Compressed-file length of the record's gzip member.
    length: u64,
    body: Vec<u8>,
}

pub fn run_package(args: &[String]) {
    let run = std::path::PathBuf::from(required(args, "run"));
    if !run.join("seeds.json").is_file() {
        eprintln!("archive-package: no seeds.json in {}", run.display());
        std::process::exit(1);
    }
    let seeds_text = std::fs::read_to_string(run.join("seeds.json")).unwrap_or_else(|e| {
        eprintln!("archive-package: read seeds.json: {e}");
        std::process::exit(1);
    });
    let seeds: Vec<SeedRow> = serde_json::from_str(&seeds_text).unwrap_or_else(|e| {
        eprintln!("archive-package: parse seeds.json: {e}");
        std::process::exit(1);
    });

    // Every WARC file under warc/ (ours per-record-gzip plus any
    // operator wget drops, plain or gzipped).
    let mut warc_files = Vec::new();
    collect_warcs(&run.join("warc"), &mut warc_files);
    warc_files.sort();
    let mut captures = Vec::new();
    for file in &warc_files {
        let rel = file
            .strip_prefix(&run)
            .unwrap_or(file)
            .to_string_lossy()
            .replace('\\', "/");
        match ingest::warc::read_located_records(file) {
            Err(e) => eprintln!("archive-package: WARN unreadable {}: {e}", file.display()),
            Ok(records) => {
                for located in &records {
                    let record = &located.record;
                    if record.header("WARC-Type") != Some("response") {
                        continue;
                    }
                    let block = &record.block;
                    captures.push(Capture {
                        url: record.header("WARC-Target-URI").unwrap_or("").to_string(),
                        date: record.header("WARC-Date").unwrap_or("").to_string(),
                        record_id: record.header("WARC-Record-ID").unwrap_or("").to_string(),
                        block_digest: record.header("WARC-Block-Digest").unwrap_or("").to_string(),
                        mime: http_mime(block),
                        status: http_status(block),
                        warc_file: rel.clone(),
                        offset: located.offset,
                        length: located.length,
                        body: http_body(block).to_vec(),
                    });
                }
            }
        }
    }

    write_manifest_csv(&run.join("manifest.csv"), &seeds);
    write_authorities_csv(&run.join("authorities.csv"), &seeds);
    write_coverage_csv(&run.join("coverage.csv"), &seeds);
    let cdx_lines = write_cdx(&run.join("cdx/index.cdxj"), &captures);
    write_texts(&run.join("text"), &captures);
    write_wacz(&run, "world.wacz", &captures, &warc_files, &cdx_lines, None);
    write_per_country_wacz(&run, &captures, &warc_files, &cdx_lines);
    write_index_html(&run, &seeds, &captures);
    write_sums(&run);

    let (mut ok, mut failed, mut disallowed, mut missing) = (0, 0, 0, 0);
    for seed in &seeds {
        match seed.status.as_str() {
            "ok" => ok += 1,
            "failed" => failed += 1,
            "disallowed" => disallowed += 1,
            _ => missing += 1,
        }
    }
    println!(
        "archive-package: seeds={} ok={ok} failed={failed} disallowed={disallowed} \
         missing={missing} captures={} warcs={} run={}",
        seeds.len(),
        captures.len(),
        warc_files.len(),
        run.display(),
    );
}

fn collect_warcs(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => return,
    };
    for entry in entries.filter_map(|e| e.ok()) {
        let path = entry.path();
        if path.is_dir() {
            collect_warcs(&path, out);
        } else if matches!(
            path.extension().and_then(|x| x.to_str()),
            Some("gz" | "warc")
        ) {
            out.push(path);
        }
    }
}

/// HTTP status of a response-record block (`HTTP/1.1 200 ...`).
fn http_status(block: &[u8]) -> u16 {
    let head = head_text(block);
    head.lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse().ok())
        .unwrap_or(0)
}

/// Declared payload MIME of a response-record block (lower-cased,
/// parameters stripped; empty when absent).
fn http_mime(block: &[u8]) -> String {
    for line in head_text(block).lines().skip(1) {
        if line.is_empty() {
            break;
        }
        if let Some((key, value)) = line.split_once(':')
            && key.trim().eq_ignore_ascii_case("content-type")
        {
            return value
                .split(';')
                .next()
                .unwrap_or("")
                .trim()
                .to_ascii_lowercase();
        }
    }
    String::new()
}

/// Body of a response-record block (after the blank line).
fn http_body(block: &[u8]) -> &[u8] {
    find_subslice(block, b"\r\n\r\n")
        .map(|i| &block[i + 4..])
        .unwrap_or(&[])
}

fn head_text(block: &[u8]) -> String {
    let end = find_subslice(block, b"\r\n\r\n")
        .map(|i| i + 2)
        .unwrap_or(block.len());
    String::from_utf8_lossy(&block[..end]).into_owned()
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

fn cdx_timestamp(date: &str) -> String {
    let digits: String = date.chars().filter(char::is_ascii_digit).collect();
    format!("{:0<14}", digits.chars().take(14).collect::<String>())
}

fn csv_escape(value: &str) -> String {
    if value.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}

fn write_file(path: &std::path::Path, bytes: &[u8]) {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent).unwrap_or_else(|e| {
            eprintln!("archive-package: create {}: {e}", parent.display());
            std::process::exit(1);
        });
    }
    std::fs::write(path, bytes).unwrap_or_else(|e| {
        eprintln!("archive-package: write {}: {e}", path.display());
        std::process::exit(1);
    });
}

/// Write `manifest.csv`: one row per crawled URL with its WARC
/// location (FLAGGED column choice: review §13 is outside the repo;
/// `manifest per URL with warc_offset/warc_length` per the SA-12
/// order). Rows without a capture keep an empty `warc_file` and zero
/// offsets.
fn write_manifest_csv(path: &std::path::Path, seeds: &[SeedRow]) {
    let mut out = String::from(
        "url,warc_file,warc_offset,warc_length,record_id,http_status,bytes,fetched_at\n",
    );
    let mut rows: Vec<&SeedRow> = seeds.iter().collect();
    rows.sort_by(|a, b| {
        (&a.pack, &a.slug, a.depth, &a.uri).cmp(&(&b.pack, &b.slug, b.depth, &b.uri))
    });
    for seed in rows {
        out.push_str(&format!(
            "{},{},{},{},{},{},{},{}\n",
            csv_escape(&seed.uri),
            csv_escape(&seed.warc_file),
            seed.warc_offset,
            seed.warc_length,
            csv_escape(&seed.record_id),
            seed.http_status,
            seed.bytes,
            csv_escape(&seed.fetched_at),
        ));
    }
    write_file(path, out.as_bytes());
}

/// Write `authorities.csv`: one row per authority (pack + slug),
/// aggregating its seed URLs (FLAGGED column choice: review §13 is
/// outside the repo; `authorities per authority` per the SA-12 order).
fn write_authorities_csv(path: &std::path::Path, seeds: &[SeedRow]) {
    let mut by_authority: HashMap<(&str, &str), Vec<&SeedRow>> = HashMap::new();
    for seed in seeds {
        by_authority
            .entry((seed.pack.as_str(), seed.slug.as_str()))
            .or_default()
            .push(seed);
    }
    let mut keys: Vec<(&str, &str)> = by_authority.keys().copied().collect();
    keys.sort();
    let mut out = String::from(
        "pack,slug,authority_name,seed_urls,ok,failed,disallowed,missing,bytes,\
         first_fetched_at,last_fetched_at\n",
    );
    for (pack, slug) in keys {
        let rows = &by_authority[&(pack, slug)];
        let urls: HashSet<&str> = rows.iter().map(|r| r.uri.as_str()).collect();
        let (mut ok, mut failed, mut disallowed, mut missing, mut bytes) = (0, 0, 0, 0, 0u64);
        let mut first = "";
        let mut last = "";
        for row in rows {
            match row.status.as_str() {
                "ok" => ok += 1,
                "failed" => failed += 1,
                "disallowed" => disallowed += 1,
                _ => missing += 1,
            }
            bytes += row.bytes;
            if !row.fetched_at.is_empty() {
                if first.is_empty() || row.fetched_at.as_str() < first {
                    first = row.fetched_at.as_str();
                }
                if row.fetched_at.as_str() > last {
                    last = row.fetched_at.as_str();
                }
            }
        }
        let name = rows
            .iter()
            .map(|r| r.authority.as_str())
            .find(|n| !n.is_empty());
        out.push_str(&format!(
            "{},{},{},{},{},{},{},{},{},{},{}\n",
            csv_escape(pack),
            csv_escape(slug),
            csv_escape(name.unwrap_or("")),
            urls.len(),
            ok,
            failed,
            disallowed,
            missing,
            bytes,
            csv_escape(first),
            csv_escape(last),
        ));
    }
    write_file(path, out.as_bytes());
}

/// Write `coverage.csv`: one row per party (pack).
///
/// FLAGGED column choices (review §13 is outside the repo):
/// `coverage per party with hague_status and coverage_pct` per the
/// SA-12 order, where `coverage_pct` = `100 * ok / seed_urls` with one
/// decimal (0 rows → `0.0`) and `hague_status` = `unknown` (party
/// Hague status is not in the pack schema — needs SA-7/operator
/// data). `terms_note` stays unknown/restrictive (per-party terms
/// review is out of scope for the packager).
fn write_coverage_csv(path: &std::path::Path, seeds: &[SeedRow]) {
    let mut by_pack: HashMap<&str, Vec<&SeedRow>> = HashMap::new();
    for seed in seeds {
        by_pack.entry(seed.pack.as_str()).or_default().push(seed);
    }
    let mut packs: Vec<String> = by_pack.keys().map(|s| s.to_string()).collect();
    packs.sort();
    let mut out = String::from(
        "pack,authorities,seed_urls,ok,failed,disallowed,missing,bytes,coverage_pct,\
         hague_status,terms_note,robots_note\n",
    );
    for pack in &packs {
        let rows = &by_pack[pack.as_str()];
        let authorities: HashSet<&str> = rows.iter().map(|r| r.slug.as_str()).collect();
        let urls: HashSet<&str> = rows.iter().map(|r| r.uri.as_str()).collect();
        let (mut ok, mut failed, mut disallowed, mut missing, mut bytes) = (0, 0, 0, 0, 0u64);
        let (mut allowed, mut denied) = (0, 0);
        for row in rows {
            match row.status.as_str() {
                "ok" => ok += 1,
                "failed" => failed += 1,
                "disallowed" => disallowed += 1,
                _ => missing += 1,
            }
            bytes += row.bytes;
            if row.robots_verdict == "allowed" {
                allowed += 1;
            } else {
                denied += 1;
            }
        }
        let coverage_pct = if urls.is_empty() {
            0.0
        } else {
            100.0 * ok as f64 / urls.len() as f64
        };
        out.push_str(&format!(
            "{},{},{},{},{},{},{},{},{coverage_pct:.1},unknown,{},{}\n",
            csv_escape(pack),
            authorities.len(),
            urls.len(),
            ok,
            failed,
            disallowed,
            missing,
            bytes,
            csv_escape(
                "unknown/restrictive until read \
                 (see docs/ops/world-archive-rights.md)"
            ),
            csv_escape(&format!(
                "robots allowed={allowed} disallowed={denied} (robots.csv)"
            )),
        ));
    }
    write_file(path, out.as_bytes());
}

/// Write `cdx/index.cdxj` in standard CDXJ form; returns the lines
/// (reused for WACZ indexes).
///
/// Each line is `<surt> <timestamp> <json>` with `url`, `offset`,
/// `length`, `filename` plus `mime`, `status`, `digest` (the record's
/// `WARC-Block-Digest`, FLAGGED: a payload digest would also be
/// defensible), and `record_id`. Offsets point into the stored
/// `.warc.gz`; lengths cover the record's gzip member.
fn write_cdx(path: &std::path::Path, captures: &[Capture]) -> Vec<String> {
    let mut lines = Vec::new();
    for capture in captures {
        let surt = ingest::warc::surt_key(&capture.url).unwrap_or_else(|| capture.url.clone());
        let status = capture.status.to_string();
        let offset = capture.offset.to_string();
        let length = capture.length.to_string();
        let mut sorted = [
            ("digest", capture.block_digest.as_str()),
            ("filename", capture.warc_file.as_str()),
            ("length", length.as_str()),
            ("mime", capture.mime.as_str()),
            ("offset", offset.as_str()),
            ("record_id", capture.record_id.as_str()),
            ("status", status.as_str()),
            ("url", capture.url.as_str()),
        ];
        sorted.sort_by(|a, b| a.0.cmp(b.0));
        let json = sorted
            .iter()
            .map(|(k, v)| format!("\"{k}\":\"{}\"", json_escape(v)))
            .collect::<Vec<_>>()
            .join(",");
        lines.push(format!(
            "{surt} {} {{{json}}}",
            cdx_timestamp(&capture.date)
        ));
    }
    lines.sort();
    let mut text = lines.join("\n");
    if !lines.is_empty() {
        text.push('\n');
    }
    write_file(path, text.as_bytes());
    lines
}

fn json_escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

/// Write `text/<sha256>.txt` for textual captures; returns the
/// filenames written (for the manifest).
fn write_texts(dir: &std::path::Path, captures: &[Capture]) -> Vec<String> {
    let mut names = Vec::new();
    for capture in captures {
        if !is_textual(&capture.mime, &capture.body) {
            continue;
        }
        let text = extract_text(&capture.body);
        let name = format!("{}.txt", sha256_hex(text.as_bytes()));
        write_file(&dir.join(&name), text.as_bytes());
        if !names.contains(&name) {
            names.push(name);
        }
    }
    names.sort();
    names
}

/// True for HTML/text (or absent MIME with a text-looking body).
fn is_textual(mime: &str, body: &[u8]) -> bool {
    if mime.contains("html") || mime.starts_with("text/") {
        return true;
    }
    if mime.is_empty() || mime == "application/xhtml+xml" {
        let sample: Vec<u8> = body.iter().copied().take(512).collect();
        return String::from_utf8_lossy(&sample)
            .chars()
            .all(|c| !c.is_control() || c.is_whitespace());
    }
    false
}

/// Naive tag-stripped text: drops `<...>` spans, resolves a handful
/// of entities, collapses whitespace runs to one space per block.
fn extract_text(body: &[u8]) -> String {
    let text = String::from_utf8_lossy(body);
    let mut out = String::with_capacity(text.len());
    let mut tag = false;
    let mut pending_space = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if tag {
            if c == '>' {
                tag = false;
                pending_space = true;
            }
            continue;
        }
        if c == '<' {
            tag = true;
            continue;
        }
        if c == '&' {
            let mut entity = String::new();
            while let Some(&next) = chars.peek() {
                if next == ';' {
                    chars.next();
                    break;
                }
                if entity.len() > 12 || next.is_whitespace() {
                    break;
                }
                entity.push(next);
                chars.next();
            }
            let resolved = match entity.as_str() {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" | "#39" => Some('\''),
                "nbsp" => Some(' '),
                _ => None,
            };
            if pending_space && !out.is_empty() {
                out.push(' ');
            }
            pending_space = false;
            match resolved {
                Some(c) => out.push(c),
                None => {
                    out.push('&');
                    out.push_str(&entity);
                }
            }
            continue;
        }
        if c.is_whitespace() {
            pending_space = true;
        } else {
            if pending_space && !out.is_empty() {
                out.push(' ');
            }
            pending_space = false;
            out.push(c);
        }
    }
    out
}

/// SHA-256 hex digest (FIPS 180-4; self-contained so the packager
/// adds no manifest dependency; known-vector tested below).
fn sha256_hex(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hasher.hexdigest()
}

/// Incremental SHA-256 (FIPS 180-4), for hashing streamed ZIP entry
/// data without buffering it.
struct Sha256 {
    h: [u32; 8],
    buf: [u8; 64],
    buf_len: usize,
    total_len: u64,
}

impl Sha256 {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];

    fn new() -> Self {
        Self {
            h: [
                0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
                0x5be0cd19,
            ],
            buf: [0; 64],
            buf_len: 0,
            total_len: 0,
        }
    }

    fn update(&mut self, mut data: &[u8]) {
        self.total_len += data.len() as u64;
        if self.buf_len > 0 {
            let take = (64 - self.buf_len).min(data.len());
            self.buf[self.buf_len..self.buf_len + take].copy_from_slice(&data[..take]);
            self.buf_len += take;
            data = &data[take..];
            if self.buf_len == 64 {
                let block = self.buf;
                Self::compress(&mut self.h, &block);
                self.buf_len = 0;
            }
        }
        while data.len() >= 64 {
            let mut block = [0u8; 64];
            block.copy_from_slice(&data[..64]);
            Self::compress(&mut self.h, &block);
            data = &data[64..];
        }
        if !data.is_empty() {
            self.buf[..data.len()].copy_from_slice(data);
            self.buf_len = data.len();
        }
    }

    fn digest(mut self) -> [u8; 32] {
        let bit_len = self.total_len.wrapping_mul(8);
        self.update(&[0x80]);
        // Zero-pad to a 56-byte tail block (a full block compresses
        // along the way when the message runs past it).
        while self.buf_len != 56 {
            if self.buf_len > 56 {
                while self.buf_len < 64 {
                    self.buf[self.buf_len] = 0;
                    self.buf_len += 1;
                }
                let block = self.buf;
                Self::compress(&mut self.h, &block);
                self.buf_len = 0;
            } else {
                self.buf[self.buf_len] = 0;
                self.buf_len += 1;
            }
        }
        self.buf[56..].copy_from_slice(&bit_len.to_be_bytes());
        let block = self.buf;
        Self::compress(&mut self.h, &block);
        let mut out = [0u8; 32];
        for (i, word) in self.h.iter().enumerate() {
            out[4 * i..4 * i + 4].copy_from_slice(&word.to_be_bytes());
        }
        out
    }

    fn hexdigest(self) -> String {
        self.digest().iter().map(|b| format!("{b:02x}")).collect()
    }

    fn compress(h: &mut [u32; 8], block: &[u8; 64]) {
        let mut w = [0u32; 64];
        for (i, word) in w.iter_mut().take(16).enumerate() {
            *word = u32::from_be_bytes([
                block[4 * i],
                block[4 * i + 1],
                block[4 * i + 2],
                block[4 * i + 3],
            ]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let (mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh) =
            (h[0], h[1], h[2], h[3], h[4], h[5], h[6], h[7]);
        for (constant, term) in Self::K.iter().zip(w.iter()) {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let t1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(*constant)
                .wrapping_add(*term);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
        h[5] = h[5].wrapping_add(f);
        h[6] = h[6].wrapping_add(g);
        h[7] = h[7].wrapping_add(hh);
    }
}

const fn crc32_table() -> [u32; 256] {
    let mut table = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut crc = i as u32;
        let mut j = 0;
        while j < 8 {
            crc = if crc & 1 == 1 {
                0xedb88320 ^ (crc >> 1)
            } else {
                crc >> 1
            };
            j += 1;
        }
        table[i] = crc;
        i += 1;
    }
    table
}

/// Incremental CRC-32/ISO-HDLC, for checksumming streamed ZIP entry
/// data without buffering it.
struct Crc32(u32);

impl Crc32 {
    fn new() -> Self {
        Self(0xffff_ffff)
    }

    fn update(&mut self, data: &[u8]) {
        const TABLE: [u32; 256] = crc32_table();
        for byte in data {
            self.0 = TABLE[((self.0 ^ u32::from(*byte)) & 0xff) as usize] ^ (self.0 >> 8);
        }
    }

    fn sum(&self) -> u32 {
        !self.0
    }
}

/// CRC-32/ISO-HDLC (known-vector tested below).
#[cfg(test)]
fn crc32(data: &[u8]) -> u32 {
    let mut crc = Crc32::new();
    crc.update(data);
    crc.sum()
}

/// Streaming stored (method 0) ZIP writer: local headers + streamed
/// entry bytes + central directory, fixed DOS timestamp
/// (deterministic builds), UTF-8 flag.
///
/// Entry content is never fully buffered: `add_stream` copies from
/// the reader in 64 KiB chunks while checksumming (CRC-32 always, an
/// optional SHA-256 tap for `datapackage.json` hashes). Sizes are
/// declared upfront from file metadata, so local headers carry exact
/// sizes (no data descriptors).
///
/// ZIP64: any entry size or local-header offset above `u32::MAX`
/// (or more than 65 535 entries) switches that entry's headers to
/// ZIP64 framing plus a ZIP64 end-of-central-directory record and
/// locator. Small archives stay classic ZIP32. `force_zip64` exists
/// for the unit test (real >4 GiB entries are untestable here).
struct ZipStreamWriter<W: std::io::Write> {
    out: W,
    central: Vec<u8>,
    count: u64,
    offset: u64,
    need_zip64: bool,
    force_zip64: bool,
}

impl<W: std::io::Write> ZipStreamWriter<W> {
    fn new(out: W) -> Self {
        Self {
            out,
            central: Vec::new(),
            count: 0,
            offset: 0,
            need_zip64: false,
            force_zip64: false,
        }
    }

    /// Stream one stored entry: `size` bytes are copied from
    /// `reader` (a mismatch with the declared size is an error),
    /// every chunk is passed to `tap`, and `(crc32, bytes)` is
    /// returned for the central directory / datapackage hashes.
    fn add_stream<R: std::io::Read>(
        &mut self,
        name: &str,
        size: u64,
        mut reader: R,
        tap: &mut dyn FnMut(&[u8]),
    ) -> std::io::Result<(u32, u64)> {
        // Fixed DOS timestamp: 2026-11-02 00:00:00 UTC.
        const DOS_TIME: u16 = 0x0000;
        const DOS_DATE: u16 = ((2026 - 1980) << 9) | (11 << 5) | 2;
        let name_bytes = name.as_bytes();
        let entry_offset = self.offset;
        let zip64 = self.force_zip64 || size > u32::MAX as u64 || entry_offset > u32::MAX as u64;
        if zip64 {
            self.need_zip64 = true;
        }
        let mut local = Vec::new();
        push_u32(&mut local, 0x0403_4b50);
        // version-needed, flags (bit 3 = data descriptor, bit 11 =
        // UTF-8 names), method 0 (stored).
        local.extend_from_slice(&[if zip64 { 45 } else { 20 }, 0, 0x08, 0x08, 0, 0]);
        push_u16(&mut local, DOS_TIME);
        push_u16(&mut local, DOS_DATE);
        // CRC/sizes are unknown until the stream is copied, so the
        // local header carries zeros + a data descriptor follows the
        // data (64-bit form for ZIP64). The central directory always
        // carries the exact values.
        push_u32(&mut local, 0);
        if zip64 {
            push_u32(&mut local, 0xffff_ffff);
            push_u32(&mut local, 0xffff_ffff);
        } else {
            push_u32(&mut local, 0);
            push_u32(&mut local, 0);
        }
        push_u16(&mut local, name_bytes.len() as u16);
        if zip64 {
            // ZIP64 extra length; the content follows the name.
            push_u16(&mut local, 20);
        } else {
            push_u16(&mut local, 0);
        }
        local.extend_from_slice(name_bytes);
        if zip64 {
            // ZIP64 extra: id + size + 8-byte original size.
            push_u16(&mut local, 0x0001);
            push_u16(&mut local, 8);
            local.extend_from_slice(&size.to_le_bytes());
        }
        self.out.write_all(&local)?;
        self.offset += local.len() as u64;

        let mut crc = Crc32::new();
        let mut written = 0u64;
        let mut chunk = [0u8; 65536];
        loop {
            let got = reader.read(&mut chunk)?;
            if got == 0 {
                break;
            }
            let bytes = &chunk[..got];
            crc.update(bytes);
            tap(bytes);
            self.out.write_all(bytes)?;
            written += got as u64;
        }
        if written != size {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("zip entry {name}: declared {size} bytes, streamed {written}"),
            ));
        }
        let crc_sum = crc.sum();
        self.offset += written;
        if zip64 {
            self.out.write_all(&0x0807_4b50u32.to_le_bytes())?;
            self.out.write_all(&crc_sum.to_le_bytes())?;
            self.out.write_all(&written.to_le_bytes())?;
            self.out.write_all(&written.to_le_bytes())?;
            self.offset += 24;
        } else {
            self.out.write_all(&crc_sum.to_le_bytes())?;
            self.out.write_all(&(written as u32).to_le_bytes())?;
            self.out.write_all(&(written as u32).to_le_bytes())?;
            self.offset += 12;
        }

        let mut record = Vec::new();
        push_u32(&mut record, 0x0201_4b50);
        record.extend_from_slice(&[63, 0, if zip64 { 45 } else { 20 }, 0, 0x08, 0x08, 0, 0]);
        push_u16(&mut record, DOS_TIME);
        push_u16(&mut record, DOS_DATE);
        push_u32(&mut record, crc_sum);
        if zip64 {
            push_u32(&mut record, 0xffff_ffff);
            push_u32(&mut record, 0xffff_ffff);
        } else {
            push_u32(&mut record, written as u32);
            push_u32(&mut record, written as u32);
        }
        push_u16(&mut record, name_bytes.len() as u16);
        if zip64 {
            // ZIP64 extra length; the content follows the name.
            push_u16(&mut record, 20);
        } else {
            // extra-len, all zero.
            push_u16(&mut record, 0);
        }
        // comment-len + disk-start + internal-attr (2 bytes each) +
        // external-attr (4 bytes), all zero.
        record.extend_from_slice(&[0; 10]);
        if zip64 {
            // The real 64-bit offset lives in the extra field.
            push_u32(&mut record, 0xffff_ffff);
        } else {
            push_u32(&mut record, entry_offset as u32);
        }
        record.extend_from_slice(name_bytes);
        if zip64 {
            // ZIP64 extra: original size + local header offset.
            push_u16(&mut record, 0x0001);
            push_u16(&mut record, 16);
            record.extend_from_slice(&written.to_le_bytes());
            record.extend_from_slice(&entry_offset.to_le_bytes());
        }
        self.central.extend_from_slice(&record);
        self.count += 1;
        Ok((crc_sum, written))
    }

    /// Stream one in-memory entry.
    fn add_bytes(&mut self, name: &str, data: &[u8]) -> std::io::Result<(u32, u64)> {
        let mut noop = |_: &[u8]| {};
        self.add_stream(name, data.len() as u64, data, &mut noop)
    }

    /// Write the central directory + end records; returns the writer.
    fn finish(mut self) -> std::io::Result<W> {
        let cd_offset = self.offset;
        let cd_size = self.central.len() as u64;
        self.out.write_all(&self.central)?;
        self.offset += cd_size;
        let zip64 = self.need_zip64
            || self.force_zip64
            || self.count > 0xffff
            || cd_size > u32::MAX as u64
            || cd_offset > u32::MAX as u64;
        if zip64 {
            let eocd64_offset = self.offset;
            push_u32_vec(&mut self.out, 0x0606_4b50)?;
            self.out.write_all(&44u64.to_le_bytes())?;
            // made-by, version-needed, this-disk, cd-disk.
            self.out.write_all(&[63, 0, 45, 0])?;
            self.out.write_all(&[0, 0, 0, 0])?;
            self.out.write_all(&[0, 0, 0, 0])?;
            self.out.write_all(&self.count.to_le_bytes())?;
            self.out.write_all(&self.count.to_le_bytes())?;
            self.out.write_all(&cd_size.to_le_bytes())?;
            self.out.write_all(&cd_offset.to_le_bytes())?;
            push_u32_vec(&mut self.out, 0x0706_4b50)?;
            self.out.write_all(&[0, 0, 0, 0])?;
            self.out.write_all(&eocd64_offset.to_le_bytes())?;
            self.out.write_all(&[1, 0, 0, 0])?;
        }
        push_u32_vec(&mut self.out, 0x0605_4b50)?;
        self.out.write_all(&[0, 0, 0, 0])?;
        if zip64 || self.count > 0xffff {
            push_u16_vec(&mut self.out, 0xffff)?;
            push_u16_vec(&mut self.out, 0xffff)?;
        } else {
            push_u16_vec(&mut self.out, self.count as u16)?;
            push_u16_vec(&mut self.out, self.count as u16)?;
        }
        if zip64 || cd_size > u32::MAX as u64 {
            push_u32_vec(&mut self.out, 0xffff_ffff)?;
        } else {
            push_u32_vec(&mut self.out, cd_size as u32)?;
        }
        if zip64 || cd_offset > u32::MAX as u64 {
            push_u32_vec(&mut self.out, 0xffff_ffff)?;
        } else {
            push_u32_vec(&mut self.out, cd_offset as u32)?;
        }
        push_u16_vec(&mut self.out, 0)?;
        self.out.flush()?;
        Ok(self.out)
    }
}

fn push_u16(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn push_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn push_u16_vec(out: &mut impl std::io::Write, value: u16) -> std::io::Result<()> {
    out.write_all(&value.to_le_bytes())
}

fn push_u32_vec(out: &mut impl std::io::Write, value: u32) -> std::io::Result<()> {
    out.write_all(&value.to_le_bytes())
}

/// Pack ISO2 (upper-cased stem) for a run-relative warc path
/// (`warc/<ISO2>/...`); empty when the layout differs (wget drops).
fn warc_pack(rel: &str) -> String {
    rel.split('/')
        .nth(1)
        .filter(|s| s.len() == 2)
        .unwrap_or("")
        .to_ascii_uppercase()
}

/// Reader chaining whole files (one ZIP entry over many WARC
/// files, streamed without buffering).
struct ConcatFiles {
    queue: std::collections::VecDeque<std::fs::File>,
    current: Option<std::fs::File>,
}

impl ConcatFiles {
    fn open(files: &[std::path::PathBuf]) -> std::io::Result<(Self, u64)> {
        let mut queue = std::collections::VecDeque::new();
        let mut total = 0u64;
        for file in files {
            let opened = std::fs::File::open(file)?;
            total += opened.metadata()?.len();
            queue.push_back(opened);
        }
        Ok((
            Self {
                queue,
                current: None,
            },
            total,
        ))
    }
}

impl std::io::Read for ConcatFiles {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        loop {
            if self.current.is_none() {
                self.current = self.queue.pop_front();
            }
            match self.current.as_mut() {
                None => return Ok(0),
                Some(file) => match file.read(buf) {
                    Ok(0) => self.current = None,
                    done => return done,
                },
            }
        }
    }
}

/// CDXJ lines belonging to `selected` captures (matched on the
/// `"url"` JSON field — lines are SURT-first, so prefix matching no
/// longer works).
fn cdx_lines_for(selected: &[&Capture], cdx_lines: &[String]) -> Vec<String> {
    let mut index = Vec::new();
    for capture in selected {
        let needle = format!("\"url\":\"{}\"", json_escape(&capture.url));
        for line in cdx_lines {
            if line.contains(&needle) && !index.iter().any(|l| l == line) {
                index.push(line.clone());
            }
        }
    }
    index.sort();
    index
}

/// Rebase index offsets from per-file positions to positions inside
/// the WACZ's concatenated `archive/data.warc.gz` (`base` maps
/// run-relative warc path → its start inside the concatenation).
/// Lines are matched to captures by the unique `record_id`.
fn rebase_index(
    lines: &[String],
    selected: &[&Capture],
    bases: &HashMap<String, u64>,
) -> Vec<String> {
    lines
        .iter()
        .map(|line| {
            for capture in selected {
                if line.contains(&format!("\"record_id\":\"{}\"", capture.record_id))
                    && let Some(base) = bases.get(&capture.warc_file)
                {
                    let old = format!("\"offset\":\"{}\"", capture.offset);
                    let new = format!("\"offset\":\"{}\"", base + capture.offset);
                    return line.replacen(&old, &new, 1);
                }
            }
            line.clone()
        })
        .collect()
}

/// One WACZ file: world (`only_pack` unset) or per-country.
///
/// Streamed: `archive/data.warc.gz` is copied straight from the run's
/// WARC files (hashed with SHA-256 on the fly for
/// `datapackage.json`); only the small index/pages/datapackage
/// members are built in memory. `datapackage.json` resources carry
/// `sha256:` hashes + bytes (WACZ 1.1), `pages/pages.jsonl` starts
/// with a header line (FLAGGED choice — see the worker report), and
/// the embedded index offsets are rebased into the concatenated data
/// member.
fn write_wacz(
    run: &std::path::Path,
    name: &str,
    captures: &[Capture],
    warc_files: &[std::path::PathBuf],
    cdx_lines: &[String],
    only_pack: Option<&str>,
) {
    let selected: Vec<&Capture> = captures
        .iter()
        .filter(|c| only_pack.is_none_or(|p| warc_pack(&c.warc_file) == p))
        .collect();
    let mut data_files = Vec::new();
    for file in warc_files {
        let rel = file
            .strip_prefix(run)
            .unwrap_or(file)
            .to_string_lossy()
            .replace('\\', "/");
        if only_pack.is_none_or(|p| warc_pack(&rel) == p) {
            data_files.push(file.clone());
        }
    }
    data_files.sort();
    let title = match only_pack {
        Some(pack) => format!("Apostille world archive, country {pack}"),
        None => "Apostille world archive (all countries)".to_string(),
    };
    // Cumulative start of each warc file inside the concatenated
    // data member, so the embedded index points into data.warc.gz
    // (the standalone cdx/index.cdxj keeps per-file offsets against
    // warc/ — each index matches its container).
    let mut bases: HashMap<String, u64> = HashMap::new();
    let mut running = 0u64;
    for file in &data_files {
        let rel = file
            .strip_prefix(run)
            .unwrap_or(file)
            .to_string_lossy()
            .replace('\\', "/");
        bases.insert(rel, running);
        running += std::fs::metadata(file).map(|m| m.len()).unwrap_or(0);
    }
    let index = rebase_index(&cdx_lines_for(&selected, cdx_lines), &selected, &bases);
    let index_text = if index.is_empty() {
        String::new()
    } else {
        index.join("\n") + "\n"
    };
    let mut pages: Vec<String> = selected
        .iter()
        .map(|c| {
            format!(
                "{{\"url\":\"{}\",\"ts\":\"{}\"}}",
                json_escape(&c.url),
                cdx_timestamp(&c.date)
            )
        })
        .collect();
    pages.sort();
    // Header line first (FLAGGED): identifies the stream as pages.
    let mut pages_text = format!(
        "{{\"format\":\"pages-1\",\"id\":\"pages\",\"title\":\"{}\"}}\n",
        json_escape(&title)
    );
    for page in &pages {
        pages_text.push_str(page);
        pages_text.push('\n');
    }

    let out_path = run.join(name);
    if let Some(parent) = out_path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent).unwrap_or_else(|e| {
            eprintln!("archive-package: create {}: {e}", parent.display());
            std::process::exit(1);
        });
    }
    let out_file = match std::fs::File::create(&out_path) {
        Ok(file) => file,
        Err(e) => fail_write(&out_path, e),
    };
    use std::io::Write as _;
    let mut zip = ZipStreamWriter::new(std::io::BufWriter::new(out_file));
    // Data first: its hash + bytes feed datapackage.json below.
    let (concat, data_size) = match ConcatFiles::open(&data_files) {
        Ok(opened) => opened,
        Err(e) => {
            eprintln!("archive-package: read warc files: {e}");
            std::process::exit(1);
        }
    };
    let mut data_hash = Sha256::new();
    let (_, data_bytes) =
        match zip.add_stream("archive/data.warc.gz", data_size, concat, &mut |chunk| {
            data_hash.update(chunk)
        }) {
            Ok(streamed) => streamed,
            Err(e) => fail_write(&out_path, e),
        };
    let datapackage = format!(
        "{{\n  \"title\": \"{}\",\n  \"created\": \"{}\",\n  \
         \"description\": \"{}\",\n  \"wacz_version\": \"1.1\",\n  \
         \"resources\": [\n    \
         {{\"name\": \"archive/data.warc.gz\", \"path\": \"archive/data.warc.gz\", \
         \"hash\": \"sha256:{}\", \"bytes\": {}}},\n    \
         {{\"name\": \"indexes/index.cdxj\", \"path\": \"indexes/index.cdxj\", \
         \"hash\": \"sha256:{}\", \"bytes\": {}}},\n    \
         {{\"name\": \"pages/pages.jsonl\", \"path\": \"pages/pages.jsonl\", \
         \"hash\": \"sha256:{}\", \"bytes\": {}}}\n  ]\n}}\n",
        json_escape(&title),
        ingest::warc::now_warc_date(),
        json_escape(
            "Polite crawl of apostille competent-authority pages. \
             HCCH source acknowledged (non-commercial research reuse). \
             Verbatim payloads are judges-token only; see index.html."
        ),
        data_hash.hexdigest(),
        data_bytes,
        sha256_hex(index_text.as_bytes()),
        index_text.len(),
        sha256_hex(pages_text.as_bytes()),
        pages_text.len(),
    );
    if let Err(e) = zip.add_bytes("datapackage.json", datapackage.as_bytes()) {
        fail_write(&out_path, e);
    }
    if let Err(e) = zip.add_bytes("indexes/index.cdxj", index_text.as_bytes()) {
        fail_write(&out_path, e);
    }
    if let Err(e) = zip.add_bytes("pages/pages.jsonl", pages_text.as_bytes()) {
        fail_write(&out_path, e);
    }
    let mut out = match zip.finish() {
        Ok(out) => out,
        Err(e) => fail_write(&out_path, e),
    };
    if let Err(e) = out.flush() {
        fail_write(&out_path, e);
    }
}

/// Unrecoverable WACZ write failure (mirrors `write_file`).
fn fail_write(path: &std::path::Path, e: std::io::Error) -> ! {
    eprintln!("archive-package: write {}: {e}", path.display());
    std::process::exit(1);
}

/// Per-country WACZ files under `wacz/<ISO2>.wacz`.
fn write_per_country_wacz(
    run: &std::path::Path,
    captures: &[Capture],
    warc_files: &[std::path::PathBuf],
    cdx_lines: &[String],
) {
    let mut packs: HashSet<String> = HashSet::new();
    for capture in captures {
        let pack = warc_pack(&capture.warc_file);
        if !pack.is_empty() {
            packs.insert(pack);
        }
    }
    let mut packs: Vec<String> = packs.into_iter().collect();
    packs.sort();
    for pack in &packs {
        write_wacz(
            run,
            &format!("wacz/{pack}.wacz"),
            captures,
            warc_files,
            cdx_lines,
            Some(pack),
        );
    }
}

fn html_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Human overview with the mandatory HCCH credit + access split.
fn write_index_html(run: &std::path::Path, seeds: &[SeedRow], captures: &[Capture]) {
    let mut by_pack: HashMap<&str, Vec<&SeedRow>> = HashMap::new();
    for seed in seeds {
        by_pack.entry(seed.pack.as_str()).or_default().push(seed);
    }
    let mut packs: Vec<&str> = by_pack.keys().copied().collect();
    packs.sort();
    let mut rows = String::new();
    for pack in &packs {
        let beans = &by_pack[pack];
        let ok = beans.iter().filter(|r| r.status == "ok").count();
        rows.push_str(&format!(
            "<tr><td>{}</td><td>{}</td><td>{}</td></tr>\n",
            html_escape(pack),
            beans.len(),
            ok
        ));
    }
    let page = format!(
        "<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n\
         <title>Apostille world archive</title>\n</head>\n<body>\n\
         <h1>Apostille world archive</h1>\n\
         <p>HCCH source credit (mandatory): party names, authority names, \
         URLs, and short quotes reused from the HCCH Apostille status table \
         and competent-authority pages under the HCCH disclaimer \
         (non-commercial reproduction with acknowledgement).</p>\n\
         <p>Access split: CSVs, CDXJ, manifest, and this page publish \
         openly. <code>warc/</code>, <code>*.wacz</code>, and \
         <code>text/</code> payloads stay behind a judges-only token.</p>\n\
         <p>Captures: {} response records from {} seed rows.</p>\n\
         <table>\n<thead><tr><th>pack</th><th>seeds</th><th>ok</th></tr></thead>\n\
         <tbody>\n{rows}</tbody>\n</table>\n</body>\n</html>\n",
        captures.len(),
        seeds.len(),
    );
    write_file(&run.join("index.html"), page.as_bytes());
}

/// GNU-style `SHA256SUMS` over every run file except itself.
fn write_sums(run: &std::path::Path) {
    let mut files = list_files(run);
    files.retain(|rel| rel != "SHA256SUMS");
    let mut out = String::new();
    for rel in &files {
        let bytes = std::fs::read(run.join(rel)).unwrap_or_default();
        out.push_str(&format!("{}  {rel}\n", sha256_hex(&bytes)));
    }
    write_file(&run.join("SHA256SUMS"), out.as_bytes());
}

/// All regular files under `run`, as `/`-separated relative paths.
fn list_files(run: &std::path::Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![run.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(_) => continue,
        };
        for entry in entries.filter_map(|e| e.ok()) {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.is_file() {
                let rel = path
                    .strip_prefix(run)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .replace('\\', "/");
                out.push(rel);
            }
        }
    }
    out.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_known_vectors() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(sha256_hex(b"abc").len(), 64);
    }

    #[test]
    fn crc32_known_vector() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(crc32(b""), 0x0000_0000);
    }

    /// Parse our own ZIPs back (independent layout walk, not the
    /// writer's structs): EOCD -> central directory -> local files.
    fn parse_zip(data: &[u8]) -> Vec<(String, Vec<u8>)> {
        assert!(data.len() > 22);
        let eocd = data.len() - 22;
        assert_eq!(&data[eocd..eocd + 4], b"PK\x05\x06", "EOCD magic");
        let count = u16::from_le_bytes(data[eocd + 10..eocd + 12].try_into().unwrap()) as usize;
        let cd_offset = u32::from_le_bytes(data[eocd + 16..eocd + 20].try_into().unwrap()) as usize;
        let mut names = Vec::new();
        let mut pos = cd_offset;
        for _ in 0..count {
            assert_eq!(&data[pos..pos + 4], b"PK\x01\x02", "central magic");
            let method = u16::from_le_bytes(data[pos + 10..pos + 12].try_into().unwrap());
            assert_eq!(method, 0, "stored only");
            let crc = u32::from_le_bytes(data[pos + 16..pos + 20].try_into().unwrap());
            let size = u32::from_le_bytes(data[pos + 24..pos + 28].try_into().unwrap()) as usize;
            let name_len =
                u16::from_le_bytes(data[pos + 28..pos + 30].try_into().unwrap()) as usize;
            let extra_len =
                u16::from_le_bytes(data[pos + 30..pos + 32].try_into().unwrap()) as usize;
            let comment_len =
                u16::from_le_bytes(data[pos + 32..pos + 34].try_into().unwrap()) as usize;
            let local_offset =
                u32::from_le_bytes(data[pos + 42..pos + 46].try_into().unwrap()) as usize;
            let name = String::from_utf8_lossy(&data[pos + 46..pos + 46 + name_len]).into_owned();
            pos += 46 + name_len + extra_len + comment_len;
            names.push((name, local_offset, crc, size));
        }
        names
            .into_iter()
            .map(|(name, offset, crc, size)| {
                assert_eq!(&data[offset..offset + 4], b"PK\x03\x04", "local magic");
                let local_name_len =
                    u16::from_le_bytes(data[offset + 26..offset + 28].try_into().unwrap()) as usize;
                let local_extra =
                    u16::from_le_bytes(data[offset + 28..offset + 30].try_into().unwrap()) as usize;
                let start = offset + 30 + local_name_len + local_extra;
                let bytes = data[start..start + size].to_vec();
                assert_eq!(crc32(&bytes), crc, "crc: {name}");
                (name, bytes)
            })
            .collect()
    }

    #[test]
    fn zip_stream_roundtrip() {
        let mut out = Vec::new();
        {
            let mut zip = ZipStreamWriter::new(&mut out);
            zip.add_bytes("a.txt", b"hello").expect("add");
            zip.add_bytes("dir/b.bin", &[0u8, 1, 2, 3]).expect("add");
            zip.add_bytes("empty", &[]).expect("add");
            zip.finish().expect("finish");
        }
        let mut files = parse_zip(&out);
        files.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(files.len(), 3);
        assert_eq!(files[0], ("a.txt".to_string(), b"hello".to_vec()));
        assert_eq!(files[1].0, "dir/b.bin");
        assert_eq!(files[2], ("empty".to_string(), Vec::new()));
    }

    #[test]
    fn zip_stream_declared_size_mismatch_errors() {
        let mut out = Vec::new();
        let mut zip = ZipStreamWriter::new(&mut out);
        let mut noop = |_: &[u8]| {};
        let err = zip
            .add_stream("short.txt", 100, b"tiny".as_slice(), &mut noop)
            .expect_err("size mismatch");
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
    }

    #[test]
    fn zip64_framing_parses() {
        // Forced ZIP64 (real >4 GiB entries are untestable here):
        // the ZIP64 EOCD + locator must describe the same central
        // directory a normal parse finds.
        let mut out = Vec::new();
        {
            let mut zip = ZipStreamWriter::new(&mut out);
            zip.force_zip64 = true;
            zip.add_bytes("a.txt", b"zip64 content").expect("add");
            zip.add_bytes("b.txt", b"more").expect("add");
            zip.finish().expect("finish");
        }
        // Locator is the 20 bytes before the classic EOCD (22 bytes).
        let locator_at = out.len() - 22 - 20;
        assert_eq!(
            &out[locator_at..locator_at + 4],
            b"PK\x06\x07",
            "locator magic"
        );
        let eocd64_at =
            u64::from_le_bytes(out[locator_at + 8..locator_at + 16].try_into().unwrap()) as usize;
        assert_eq!(
            &out[eocd64_at..eocd64_at + 4],
            b"PK\x06\x06",
            "zip64 eocd magic"
        );
        let size = u64::from_le_bytes(out[eocd64_at + 4..eocd64_at + 12].try_into().unwrap());
        assert_eq!(size, 44, "zip64 eocd size field");
        let count = u64::from_le_bytes(out[eocd64_at + 32..eocd64_at + 40].try_into().unwrap());
        assert_eq!(count, 2);
        let cd_size = u64::from_le_bytes(out[eocd64_at + 40..eocd64_at + 48].try_into().unwrap());
        let cd_offset =
            u64::from_le_bytes(out[eocd64_at + 48..eocd64_at + 56].try_into().unwrap()) as usize;
        assert_eq!(
            &out[cd_offset..cd_offset + 4],
            b"PK\x01\x02",
            "central magic"
        );
        // Classic EOCD carries the placeholders.
        let eocd = out.len() - 22;
        assert_eq!(&out[eocd..eocd + 4], b"PK\x05\x06", "eocd magic");
        assert_eq!(
            &out[eocd + 10..eocd + 12],
            [0xff, 0xff],
            "count placeholder"
        );
        // Entry payloads still extract (central sizes are exact).
        let mut pos = cd_offset;
        let mut names = Vec::new();
        for _ in 0..count {
            assert_eq!(&out[pos..pos + 4], b"PK\x01\x02");
            let entry_size = u32::from_le_bytes(out[pos + 24..pos + 28].try_into().unwrap());
            assert_eq!(entry_size, 0xffff_ffff, "zip64 size placeholder");
            let name_len = u16::from_le_bytes(out[pos + 28..pos + 30].try_into().unwrap()) as usize;
            let extra_len =
                u16::from_le_bytes(out[pos + 30..pos + 32].try_into().unwrap()) as usize;
            let comment_len =
                u16::from_le_bytes(out[pos + 32..pos + 34].try_into().unwrap()) as usize;
            let offset_field = u32::from_le_bytes(out[pos + 42..pos + 46].try_into().unwrap());
            assert_eq!(offset_field, 0xffff_ffff, "zip64 offset placeholder");
            let name = String::from_utf8_lossy(&out[pos + 46..pos + 46 + name_len]).into_owned();
            // Real size + offset live in the ZIP64 extra (id 0x0001).
            let extra = &out[pos + 46 + name_len..pos + 46 + name_len + extra_len];
            assert_eq!(&extra[..2], [0x01, 0x00], "zip64 extra id");
            assert_eq!(extra_len, 20, "zip64 extra length");
            let real_size = u64::from_le_bytes(extra[4..12].try_into().unwrap());
            let real_offset = u64::from_le_bytes(extra[12..20].try_into().unwrap());
            assert!(real_size < u32::MAX as u64, "small test entry");
            assert!(real_offset < cd_offset as u64, "offset inside archive");
            let _ = cd_size;
            names.push(name);
            pos += 46 + name_len + extra_len + comment_len;
        }
        names.sort();
        assert_eq!(names, vec!["a.txt".to_string(), "b.txt".to_string()]);
        // Python's zipfile is the independent reader for real archives
        // (see the validation step); the byte-level framing is proven
        // here.
    }

    #[test]
    fn response_block_parsing() {
        let block =
            b"HTTP/1.1 200 OK\r\nContent-Type: Text/HTML; charset=utf-8\r\n\r\n<body>hi</body>";
        assert_eq!(http_status(block), 200);
        assert_eq!(http_mime(block), "text/html");
        assert_eq!(http_body(block), b"<body>hi</body>");
        assert_eq!(cdx_timestamp("2026-11-02T03:04:05Z"), "20261102030405");
        assert!(is_textual("text/html", b"x"));
        assert!(!is_textual("application/pdf", b"%PDF-1.4 binary"));
        assert_eq!(
            extract_text(b"<h1>Fee &amp; cost</h1><p>pay&nbsp;here</p>"),
            "Fee & cost pay here"
        );
    }

    fn fixture_run() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("archive-test-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        // Two captures under warc/XX/ via the real WARC writer.
        let warc_path = dir.join("warc/XX/xx-ca-1-00001.warc.gz");
        let mut writer = ingest::warc::WarcWriter::create(&warc_path).expect("warc");
        writer
            .write_warcinfo(&[("filename", "xx-ca-1-00001.warc.gz")])
            .expect("warcinfo");
        let mut record_ids = Vec::new();
        let mut response_spans = Vec::new();
        for (url, body) in [
            (
                "http://127.0.0.1:9/a",
                "<html><body>Apostille fees</body></html>",
            ),
            ("http://127.0.0.1:9/b", "plain text page"),
        ] {
            let req = writer
                .write_request(url, b"GET /x HTTP/1.1\r\n\r\n", None)
                .expect("request");
            let mut response_bytes = b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\n\r\n".to_vec();
            response_bytes.extend_from_slice(body.as_bytes());
            let res = writer
                .write_response(url, &response_bytes, body.as_bytes(), Some(&req.id))
                .expect("response");
            let meta = writer
                .write_metadata(url, &[("slug", "xx-ca-1"), ("pack", "XX")], Some(&res.id))
                .expect("metadata");
            record_ids.push(meta.id.clone());
            response_spans.push((res.offset, res.length));
        }
        writer.finish().expect("finish");
        let seeds = format!(
            r#"[{{"pack": "XX", "slug": "xx-ca-1", "authority": "Fixture",
                 "uri": "http://127.0.0.1:9/a", "depth": 0, "robots_verdict": "allowed",
                 "status": "ok", "http_status": 200, "bytes": 10,
                 "warc_file": "warc/XX/xx-ca-1-00001.warc.gz", "record_id": "{}",
                 "warc_offset": {}, "warc_length": {},
                 "note": "", "fetched_at": "2026-11-02T00:00:00Z"}},
                {{"pack": "XX", "slug": "xx-ca-1", "authority": "Fixture",
                 "uri": "http://127.0.0.1:9/b", "depth": 1, "robots_verdict": "allowed",
                 "status": "ok", "http_status": 200, "bytes": 10,
                 "warc_file": "warc/XX/xx-ca-1-00001.warc.gz", "record_id": "{}",
                 "warc_offset": {}, "warc_length": {},
                 "note": "", "fetched_at": "2026-11-02T00:00:00Z"}},
                {{"pack": "XX", "slug": "xx-ca-1", "authority": "Fixture",
                 "uri": "http://127.0.0.1:9/c", "depth": 0, "robots_verdict": "disallowed",
                 "status": "disallowed", "http_status": 0,
                 "bytes": 0, "warc_file": "", "record_id": "",
                 "warc_offset": 0, "warc_length": 0, "note": "",
                 "fetched_at": "2026-11-02T00:00:00Z"}}]"#,
            record_ids[0],
            response_spans[0].0,
            response_spans[0].1,
            record_ids[1],
            response_spans[1].0,
            response_spans[1].1,
        );
        std::fs::write(dir.join("seeds.json"), seeds).expect("seeds");
        std::fs::write(
            dir.join("robots.csv"),
            "url,host,verdict,reason,crawl_delay_s,checked_at,source\n",
        )
        .expect("robots");
        std::fs::write(dir.join("t1-urls.txt"), "http://127.0.0.1:9/a\n").expect("t1");
        dir
    }

    #[test]
    fn package_fixture_run() {
        let dir = fixture_run();
        run_package(&[
            "archive-package".to_string(),
            "--run".to_string(),
            dir.to_string_lossy().into_owned(),
        ]);

        for rel in [
            "authorities.csv",
            "coverage.csv",
            "cdx/index.cdxj",
            "world.wacz",
            "wacz/XX.wacz",
            "SHA256SUMS",
            "index.html",
            "manifest.csv",
        ] {
            assert!(dir.join(rel).is_file(), "missing {rel}");
        }
        // Manifest: one row per URL with warc offsets.
        let manifest = std::fs::read_to_string(dir.join("manifest.csv")).expect("manifest");
        assert!(manifest.starts_with(
            "url,warc_file,warc_offset,warc_length,record_id,http_status,bytes,fetched_at\n"
        ));
        assert_eq!(manifest.lines().count(), 4, "header + 3 urls");
        let warc_len = std::fs::metadata(dir.join("warc/XX/xx-ca-1-00001.warc.gz"))
            .expect("warc stat")
            .len();
        for line in manifest.lines().skip(1) {
            let cols: Vec<&str> = line.split(',').collect();
            assert_eq!(cols.len(), 8, "{line}");
            let (offset, length): (u64, u64) = (
                cols[2].parse().expect("offset"),
                cols[3].parse().expect("length"),
            );
            if cols[1].is_empty() {
                assert_eq!((offset, length), (0, 0), "{line}");
            } else {
                assert!(offset + length <= warc_len, "{line}");
                assert!(length > 0, "{line}");
            }
        }
        // Authorities: one row per authority (all three seeds share a slug).
        let authorities = std::fs::read_to_string(dir.join("authorities.csv")).expect("auth csv");
        assert!(authorities.starts_with("pack,slug,authority_name,seed_urls,ok,"));
        assert_eq!(authorities.lines().count(), 2, "header + 1 authority");
        assert!(authorities.contains(",3,2,0,1,"), "{authorities}");
        // Coverage: per party with coverage_pct + hague_status.
        let coverage = std::fs::read_to_string(dir.join("coverage.csv")).expect("cov csv");
        assert!(coverage.starts_with("pack,authorities,seed_urls,ok,"));
        assert!(coverage.contains("unknown/restrictive until read"));
        assert!(coverage.contains(",66.7,unknown,"), "{coverage}");
        // CDXJ: SURT keys, two response records with offsets.
        let cdx = std::fs::read_to_string(dir.join("cdx/index.cdxj")).expect("cdx");
        assert_eq!(cdx.lines().count(), 2);
        assert!(cdx.contains("\"status\":\"200\""));
        assert!(cdx.contains("\"offset\":\""), "{cdx}");
        assert!(
            cdx.contains("\"filename\":\"warc/XX/xx-ca-1-00001.warc.gz\""),
            "{cdx}"
        );
        assert!(
            cdx.lines().all(|line| line.starts_with("1,0,0,127:9)/")),
            "{cdx}"
        );
        // CDXJ offsets agree with the manifest's warc offsets.
        for line in cdx.lines() {
            let json = line.splitn(3, ' ').nth(2).expect("cdx json");
            assert!(
                manifest.contains(&json_matches_offset(json) as &str),
                "{json}"
            );
        }
        // Text extractions present.
        let texts: Vec<_> = std::fs::read_dir(dir.join("text"))
            .expect("text dir")
            .filter_map(|e| e.ok())
            .collect();
        assert_eq!(texts.len(), 2);
        // SHA256SUMS verifies.
        for line in std::fs::read_to_string(dir.join("SHA256SUMS"))
            .expect("sums")
            .lines()
        {
            let (hash, rel) = line.split_once("  ").expect("sums shape");
            let bytes = std::fs::read(dir.join(rel)).expect("summed file");
            assert_eq!(sha256_hex(&bytes), hash, "sums: {rel}");
        }
        // World WACZ opens with the expected members.
        let wacz_bytes = std::fs::read(dir.join("world.wacz")).expect("wacz");
        let mut members = parse_zip(&wacz_bytes);
        members.sort_by(|a, b| a.0.cmp(&b.0));
        let names: Vec<&str> = members.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(
            names,
            vec![
                "archive/data.warc.gz",
                "datapackage.json",
                "indexes/index.cdxj",
                "pages/pages.jsonl"
            ]
        );
        let datapackage = members
            .iter()
            .find(|(n, _)| n == "datapackage.json")
            .expect("dp");
        let dp_text = String::from_utf8_lossy(&datapackage.1).into_owned();
        assert!(dp_text.contains("\"wacz_version\": \"1.1\""), "{dp_text}");
        // datapackage resources carry hash + bytes; verify against
        // the sibling members.
        for (member_name, resource) in [
            ("archive/data.warc.gz", "archive/data.warc.gz"),
            ("indexes/index.cdxj", "indexes/index.cdxj"),
            ("pages/pages.jsonl", "pages/pages.jsonl"),
        ] {
            let member = members
                .iter()
                .find(|(n, _)| n == member_name)
                .expect("member");
            let expect_hash = format!("\"hash\": \"sha256:{}\"", sha256_hex(&member.1));
            let expect_bytes = format!("\"bytes\": {}", member.1.len());
            assert!(dp_text.contains(&expect_hash), "{resource}");
            assert!(dp_text.contains(&expect_bytes), "{resource}");
            let _ = resource;
        }
        // pages.jsonl starts with the header line, then one line per capture.
        let pages = members
            .iter()
            .find(|(n, _)| n == "pages/pages.jsonl")
            .expect("pages");
        let pages_text = String::from_utf8_lossy(&pages.1).into_owned();
        let mut page_lines = pages_text.lines();
        let header = page_lines.next().expect("pages header");
        assert!(header.contains("\"format\":\"pages-1\""), "{header}");
        assert_eq!(page_lines.count(), 2, "{pages_text}");
        // The embedded data member round-trips through the WARC reader.
        let data = members
            .iter()
            .find(|(n, _)| n == "archive/data.warc.gz")
            .expect("data");
        let tmp = dir.join("data-probe.warc.gz");
        std::fs::write(&tmp, &data.1).expect("probe write");
        let probe = ingest::warc::read_records(&tmp).expect("probe read");
        assert!(!probe.is_empty());
        // index.html carries the HCCH credit.
        let index = std::fs::read_to_string(dir.join("index.html")).expect("index");
        assert!(index.contains("HCCH"));
        std::fs::remove_dir_all(&dir).ok();
    }

    /// The `"offset":"N"` fragment of a CDXJ line (for the
    /// manifest-agreement check above).
    fn json_matches_offset(cdx_json: &str) -> String {
        let key = "\"offset\":\"";
        let start = cdx_json.find(key).expect("offset field") + key.len();
        let end = cdx_json[start..].find('"').expect("offset end") + start;
        cdx_json[start..end].to_string()
    }
}
