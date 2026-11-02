//! `xtask archive-package`: bundle a crawl run into judge deliverables.
//!
//! Usage: `cargo xtask archive-package --run archive/<run>/`
//!
//! Reads the run layout written by `xtask crawl` (plus any operator
//! wget `--warc-file` drops under `warc/`, plain or gzipped) and
//! writes:
//!
//! - `manifest.csv` — every deliverable file with sha256 + byte +
//!   record counts. Columns: `file,sha256,bytes,records,kind` where
//!   `records` means WARC records (warc/wacz), data rows (csv),
//!   lines (cdxj/txt), array rows (`seeds.json`), `1` (html).
//! - `authorities.csv` — one row per crawled URL. Columns:
//!   `pack,slug,authority_name,seed_url,depth,robots_verdict,status,
//!   http_status,bytes,warc_file,record_id,fetched_at,note`.
//! - `coverage.csv` — one row per pack. Columns:
//!   `pack,authorities,seed_urls,ok,failed,disallowed,missing,bytes,
//!   terms_note,robots_note`. `terms_note` defaults to
//!   unknown/restrictive (per-party terms review is out of scope for
//!   the packager; see `docs/ops/world-archive-rights.md`).
//! - `cdx/index.cdxj` — one line per response record:
//!   `<url> <timestamp> <json>` with `{"record_id","warc_file",
//!   "digest","mime","status"}`; timestamp is the WARC-Date
//!   compacted to `YYYYMMDDHHMMSS`.
//! - `world.wacz` + `wacz/<ISO2>.wacz` — WACZ 1.1 (stored, no
//!   compression) holding `archive/data.warc.gz` (concatenated run
//!   WARCs), `indexes/index.cdxj`, `pages/pages.jsonl`, and
//!   `datapackage.json`. Written with a minimal built-in ZIP writer
//!   (no new dependencies); unzip-readable, `wacz validate`
//!   compatibility is best-effort (the tool is unavailable here).
//! - `text/<sha256>.txt` — tag-stripped text of textual responses,
//!   named by the sha256 of the extracted text.
//! - `SHA256SUMS` — GNU-style checksums of every run file except
//!   itself.
//! - `index.html` — human overview with the mandatory HCCH source
//!   credit and the open-vs-token split.
//!
//! SHA-256, CRC-32, and ZIP here are small self-contained
//! implementations (known-vector tested) so the packager adds no new
//! manifest dependencies. No network, no live crawling in tests:
//! fixtures are built with `ingest::warc` in a temp dir.

use std::collections::{HashMap, HashSet};

use crate::required;

/// Defensive mirror of the crawl `seeds.json` row (missing fields
/// default; the packager never crashes on a thinner row).
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
        match ingest::warc::read_records(file) {
            Err(e) => eprintln!("archive-package: WARN unreadable {}: {e}", file.display()),
            Ok(records) => {
                for record in &records {
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
                        body: http_body(block).to_vec(),
                    });
                }
            }
        }
    }

    write_authorities_csv(&run.join("authorities.csv"), &seeds);
    write_coverage_csv(&run.join("coverage.csv"), &seeds);
    let cdx_lines = write_cdx(&run.join("cdx/index.cdxj"), &captures);
    write_texts(&run.join("text"), &captures);
    write_wacz(&run, "world.wacz", &captures, &warc_files, &cdx_lines, None);
    write_per_country_wacz(&run, &captures, &warc_files, &cdx_lines);
    write_index_html(&run, &seeds, &captures);
    write_manifest(&run, &seeds, &captures);
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

fn write_authorities_csv(path: &std::path::Path, seeds: &[SeedRow]) {
    let mut out = String::from(
        "pack,slug,authority_name,seed_url,depth,robots_verdict,status,\
         http_status,bytes,warc_file,record_id,fetched_at,note\n",
    );
    for seed in seeds {
        out.push_str(&format!(
            "{},{},{},{},{},{},{},{},{},{},{},{},{}\n",
            csv_escape(&seed.pack),
            csv_escape(&seed.slug),
            csv_escape(&seed.authority),
            csv_escape(&seed.uri),
            seed.depth,
            csv_escape(&seed.robots_verdict),
            csv_escape(&seed.status),
            seed.http_status,
            seed.bytes,
            csv_escape(&seed.warc_file),
            csv_escape(&seed.record_id),
            csv_escape(&seed.fetched_at),
            csv_escape(&seed.note),
        ));
    }
    write_file(path, out.as_bytes());
}

fn write_coverage_csv(path: &std::path::Path, seeds: &[SeedRow]) {
    let mut packs: Vec<String> = Vec::new();
    let mut by_pack: HashMap<&str, Vec<&SeedRow>> = HashMap::new();
    for seed in seeds {
        by_pack.entry(seed.pack.as_str()).or_default().push(seed);
    }
    packs.extend(by_pack.keys().map(|s| s.to_string()));
    packs.sort();
    let mut out = String::from(
        "pack,authorities,seed_urls,ok,failed,disallowed,missing,bytes,terms_note,robots_note\n",
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
        out.push_str(&format!(
            "{},{},{},{},{},{},{},{},{},{}\n",
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

/// Write `cdx/index.cdxj`; returns the lines (reused for WACZ indexes).
fn write_cdx(path: &std::path::Path, captures: &[Capture]) -> Vec<String> {
    let mut lines = Vec::new();
    for capture in captures {
        let status = capture.status.to_string();
        let mut sorted = [
            ("digest", capture.block_digest.as_str()),
            ("mime", capture.mime.as_str()),
            ("record_id", capture.record_id.as_str()),
            ("status", status.as_str()),
            ("warc_file", capture.warc_file.as_str()),
        ];
        sorted.sort_by(|a, b| a.0.cmp(b.0));
        let json = sorted
            .iter()
            .map(|(k, v)| format!("\"{k}\":\"{}\"", json_escape(v)))
            .collect::<Vec<_>>()
            .join(",");
        lines.push(format!(
            "{} {} {{{json}}}",
            capture.url,
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
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    let mut padded = data.to_vec();
    let bit_len = (data.len() as u64).wrapping_mul(8);
    padded.push(0x80);
    while padded.len() % 64 != 56 {
        padded.push(0);
    }
    padded.extend_from_slice(&bit_len.to_be_bytes());
    for block in padded.as_chunks::<64>().0 {
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
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let t1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
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
    h.iter().map(|w| format!("{w:08x}")).collect()
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

/// CRC-32/ISO-HDLC (known-vector tested below).
fn crc32(data: &[u8]) -> u32 {
    const TABLE: [u32; 256] = crc32_table();
    let mut crc = 0xffff_ffffu32;
    for byte in data {
        crc = TABLE[((crc ^ u32::from(*byte)) & 0xff) as usize] ^ (crc >> 8);
    }
    !crc
}

/// Minimal stored (method 0) ZIP writer: local headers + central
/// directory, fixed DOS timestamp (deterministic builds), UTF-8
/// flag. Readable by any unzip; the WACZ files use only `stored`
/// entries, which the WACZ 1.1 profile permits.
struct ZipStored {
    files: Vec<ZipEntry>,
}

struct ZipEntry {
    name: String,
    data: Vec<u8>,
}

impl ZipStored {
    fn new() -> Self {
        Self { files: Vec::new() }
    }

    fn add(&mut self, name: &str, data: Vec<u8>) {
        if !self.files.iter().any(|f| f.name == name) {
            self.files.push(ZipEntry {
                name: name.to_string(),
                data,
            });
        }
    }

    fn finish(&self) -> Vec<u8> {
        // Fixed DOS timestamp: 2026-11-02 00:00:00 UTC.
        const DOS_TIME: u16 = 0x0000;
        const DOS_DATE: u16 = ((2026 - 1980) << 9) | (11 << 5) | 2;
        let mut out = Vec::new();
        let mut central = Vec::new();
        for file in &self.files {
            let name = file.name.as_bytes();
            let crc = crc32(&file.data);
            let offset = out.len() as u32;
            push_u32(&mut out, 0x0403_4b50);
            out.extend_from_slice(&[20, 0, 0x00, 0x08, 0, 0]);
            push_u16(&mut out, DOS_TIME);
            push_u16(&mut out, DOS_DATE);
            push_u32(&mut out, crc);
            push_u32(&mut out, file.data.len() as u32);
            push_u32(&mut out, file.data.len() as u32);
            push_u16(&mut out, name.len() as u16);
            push_u16(&mut out, 0);
            out.extend_from_slice(name);
            out.extend_from_slice(&file.data);
            push_u32(&mut central, 0x0201_4b50);
            central.extend_from_slice(&[63, 0, 20, 0, 0x00, 0x08, 0, 0]);
            push_u16(&mut central, DOS_TIME);
            push_u16(&mut central, DOS_DATE);
            push_u32(&mut central, crc);
            push_u32(&mut central, file.data.len() as u32);
            push_u32(&mut central, file.data.len() as u32);
            push_u16(&mut central, name.len() as u16);
            // extra-len + comment-len + disk-start + internal-attr (2
            // bytes each) + external-attr (4 bytes), all zero.
            central.extend_from_slice(&[0; 12]);
            push_u32(&mut central, offset);
            central.extend_from_slice(name);
        }
        let cd_offset = out.len() as u32;
        let cd_size = central.len() as u32;
        out.extend_from_slice(&central);
        push_u32(&mut out, 0x0605_4b50);
        out.extend_from_slice(&[0, 0, 0, 0]);
        push_u16(&mut out, self.files.len() as u16);
        push_u16(&mut out, self.files.len() as u16);
        push_u32(&mut out, cd_size);
        push_u32(&mut out, cd_offset);
        push_u16(&mut out, 0);
        out
    }
}

fn push_u16(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn push_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
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

/// One WACZ file: world (`only_pack` unset) or per-country.
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
    let mut warc_bytes = Vec::new();
    for file in warc_files {
        let rel = file
            .strip_prefix(run)
            .unwrap_or(file)
            .to_string_lossy()
            .replace('\\', "/");
        let include = only_pack.is_none_or(|p| warc_pack(&rel) == p);
        if include && let Ok(bytes) = std::fs::read(file) {
            warc_bytes.extend_from_slice(&bytes);
        }
    }
    // CDXJ lines of the selected captures (lines start with the URL).
    let mut index = cdx_lines
        .iter()
        .filter(|line| selected.iter().any(|c| line.starts_with(c.url.as_str())))
        .cloned()
        .collect::<Vec<_>>();
    index.sort();
    let title = match only_pack {
        Some(pack) => format!("Apostille world archive, country {pack}"),
        None => "Apostille world archive (all countries)".to_string(),
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
    let datapackage = format!(
        "{{\n  \"wacz_version\": \"1.1\",\n  \"title\": \"{}\",\n  \
         \"created\": \"{}\",\n  \
         \"description\": \"{}\",\n  \
         \"resources\": [\n    {{\"name\": \"archive/data.warc.gz\", \"path\": \"archive/data.warc.gz\"}},\n    \
         {{\"name\": \"indexes/index.cdxj\", \"path\": \"indexes/index.cdxj\"}},\n    \
         {{\"name\": \"pages/pages.jsonl\", \"path\": \"pages/pages.jsonl\"}}\n  ]\n}}\n",
        json_escape(&title),
        ingest::warc::now_warc_date(),
        json_escape(
            "Polite crawl of apostille competent-authority pages. \
             HCCH source acknowledged (non-commercial research reuse). \
             Verbatim payloads are judges-token only; see index.html."
        ),
    );
    let mut zip = ZipStored::new();
    zip.add("datapackage.json", datapackage.into_bytes());
    zip.add("archive/data.warc.gz", warc_bytes);
    zip.add("indexes/index.cdxj", index.join("\n").into_bytes());
    let mut pages_text = pages.join("\n");
    if !pages.is_empty() {
        pages_text.push('\n');
    }
    zip.add("pages/pages.jsonl", pages_text.into_bytes());
    write_file(&run.join(name), &zip.finish());
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

/// `manifest.csv` over every deliverable (not itself / not SHA256SUMS).
fn write_manifest(run: &std::path::Path, seeds: &[SeedRow], captures: &[Capture]) {
    let mut files = list_files(run);
    files.retain(|rel| rel != "manifest.csv" && rel != "SHA256SUMS");
    let mut out = String::from("file,sha256,bytes,records,kind\n");
    for rel in &files {
        let bytes = std::fs::read(run.join(rel)).unwrap_or_default();
        let kind = classify(rel);
        let records = count_records(run, rel, &kind, seeds, captures);
        out.push_str(&format!(
            "{},{},{},{},{}\n",
            csv_escape(rel),
            sha256_hex(&bytes),
            bytes.len(),
            records,
            kind,
        ));
    }
    write_file(&run.join("manifest.csv"), out.as_bytes());
}

fn classify(rel: &str) -> String {
    if rel.starts_with("warc/") {
        "warc".to_string()
    } else if rel.ends_with(".wacz") {
        "wacz".to_string()
    } else if rel.ends_with(".csv") {
        "csv".to_string()
    } else if rel.ends_with(".cdxj") {
        "cdxj".to_string()
    } else if rel.starts_with("text/") {
        "txt".to_string()
    } else if rel.ends_with(".html") {
        "html".to_string()
    } else if rel.ends_with(".json") {
        "json".to_string()
    } else {
        "txt".to_string()
    }
}

fn count_records(
    run: &std::path::Path,
    rel: &str,
    kind: &str,
    seeds: &[SeedRow],
    captures: &[Capture],
) -> usize {
    match kind {
        "warc" => ingest::warc::read_records(&run.join(rel))
            .map(|records| records.len())
            .unwrap_or(0),
        "wacz" if rel == "world.wacz" => captures.len(),
        "wacz" => {
            let pack = rel.trim_start_matches("wacz/").trim_end_matches(".wacz");
            captures
                .iter()
                .filter(|c| warc_pack(&c.warc_file) == pack)
                .count()
        }
        "csv" => std::fs::read_to_string(run.join(rel))
            .map(|text| text.lines().count().saturating_sub(1))
            .unwrap_or(0),
        "cdxj" | "txt" => std::fs::read_to_string(run.join(rel))
            .map(|text| text.lines().count())
            .unwrap_or(0),
        "json" => seeds.len(),
        _ => 1,
    }
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
    fn zip_stored_roundtrip() {
        let mut zip = ZipStored::new();
        zip.add("a.txt", b"hello".to_vec());
        zip.add("dir/b.bin", vec![0u8, 1, 2, 3]);
        zip.add("empty", Vec::new());
        let bytes = zip.finish();
        let mut files = parse_zip(&bytes);
        files.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(files.len(), 3);
        assert_eq!(files[0], ("a.txt".to_string(), b"hello".to_vec()));
        assert_eq!(files[1].0, "dir/b.bin");
        assert_eq!(files[2], ("empty".to_string(), Vec::new()));
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
        }
        writer.finish().expect("finish");
        let seeds = format!(
            r#"[{{"pack": "XX", "slug": "xx-ca-1", "authority": "Fixture",
                 "uri": "http://127.0.0.1:9/a", "depth": 0, "robots_verdict": "allowed",
                 "status": "ok", "http_status": 200, "bytes": 10,
                 "warc_file": "warc/XX/xx-ca-1-00001.warc.gz", "record_id": "{}",
                 "note": "", "fetched_at": "2026-11-02T00:00:00Z"}},
                {{"pack": "XX", "slug": "xx-ca-1", "authority": "Fixture",
                 "uri": "http://127.0.0.1:9/b", "depth": 1, "robots_verdict": "allowed",
                 "status": "ok", "http_status": 200, "bytes": 10,
                 "warc_file": "warc/XX/xx-ca-1-00001.warc.gz", "record_id": "{}",
                 "note": "", "fetched_at": "2026-11-02T00:00:00Z"}},
                {{"pack": "XX", "slug": "xx-ca-1", "authority": "Fixture",
                 "uri": "http://127.0.0.1:9/c", "depth": 0, "robots_verdict": "disallowed",
                 "status": "disallowed", "http_status": 0,
                 "bytes": 0, "warc_file": "", "record_id": "", "note": "",
                 "fetched_at": "2026-11-02T00:00:00Z"}}]"#,
            record_ids[0], record_ids[1],
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
        // Authorities + coverage shapes.
        let authorities = std::fs::read_to_string(dir.join("authorities.csv")).expect("auth csv");
        assert!(authorities.starts_with("pack,slug,authority_name,seed_url,depth,"));
        assert_eq!(authorities.lines().count(), 4, "header + 3 seeds");
        let coverage = std::fs::read_to_string(dir.join("coverage.csv")).expect("cov csv");
        assert!(coverage.starts_with("pack,authorities,seed_urls,ok,"));
        assert!(coverage.contains("unknown/restrictive until read"));
        // CDXJ: two response records.
        let cdx = std::fs::read_to_string(dir.join("cdx/index.cdxj")).expect("cdx");
        assert_eq!(cdx.lines().count(), 2);
        assert!(cdx.contains("\"status\":\"200\""));
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
        // Manifest rows point at real files with matching hashes.
        let manifest = std::fs::read_to_string(dir.join("manifest.csv")).expect("manifest");
        assert!(manifest.starts_with("file,sha256,bytes,records,kind\n"));
        for line in manifest.lines().skip(1) {
            let rel = line.split(',').next().expect("manifest shape");
            let bytes = std::fs::read(dir.join(rel)).expect("manifest file");
            assert!(line.contains(&sha256_hex(&bytes)), "manifest: {rel}");
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
        assert!(String::from_utf8_lossy(&datapackage.1).contains("\"wacz_version\": \"1.1\""));
        // index.html carries the HCCH credit.
        let index = std::fs::read_to_string(dir.join("index.html")).expect("index");
        assert!(index.contains("HCCH"));
        std::fs::remove_dir_all(&dir).ok();
    }
}
