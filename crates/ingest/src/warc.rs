//! WARC/1.1 archive writer plus the polite-fetch core for the world crawl.
//!
//! The world crawler (`xtask crawl`, SA-8 slice of the operator-ordered
//! ADDENDUM in `docs/ops/push-plan.md`) stores every capture as WARC/1.1
//! records: one `warcinfo` per file, one `request` + `response` record per
//! redirect hop, and one `metadata` record per capture carrying the pack
//! slug, the robots verdict, and the TLS peer chain. Each record is
//! individually gzipped (one gzip member per record, concatenated), so a
//! `.warc.gz` file stays readable even if a later member is truncated.
//!
//! Record envelope (conventional header order, order is not significant):
//!
//! - `WARC-Type`, `WARC-Date` (UTC `YYYY-MM-DDTHH:MM:SSZ`), `WARC-Record-ID`
//!   (`<urn:uuid:...>`), `WARC-Target-URI` (all but `warcinfo`),
//!   `Content-Type` (`application/http; msgtype=request|response` or
//!   `application/warc-fields`), `Content-Length`, `WARC-Block-Digest`
//!   (`sha1:<BASE32>` over the record block), `WARC-Payload-Digest`
//!   (`sha1:<BASE32>` over the response body; on `request` records only
//!   when the request carries a body, which a GET never does), and
//!   `WARC-Concurrent-To` linking request <-> response <-> metadata.
//!
//! Fetch policy implemented here (crawl policy such as depth and host
//! parallelism lives in `xtask/src/crawl.rs`):
//!
//! - redirects are followed manually (up to [`MAX_REDIRECT_HOPS`]) so every
//!   hop gets its own request/response record pair;
//! - HTTP 429 honours `Retry-After` (seconds form, capped) with one retry;
//! - response bodies are capped at [`MAX_BODY_BYTES`] (25 MiB);
//! - robots checks FAIL CLOSED: an unreachable or unparseable robots.txt
//!   (or URL) disallows the fetch. This is stricter than the thin watch
//!   (`fetch::check_robots`, which fails open) and is required by the
//!   world-archive rights review (`docs/ops/world-archive-rights.md`).

use std::io::{self, BufWriter, Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// WARC format version written by [`WarcWriter`].
pub const WARC_VERSION: &str = "WARC/1.1";

/// Crawler user agent (exact string from the operator spec).
pub const CRAWL_USER_AGENT: &str = "handbook-crawl/0.1 (+https://andrii.ai/apostille)";

/// Largest response body kept (25 MiB PDF cap from the spec).
pub const MAX_BODY_BYTES: u64 = 25 * 1024 * 1024;

/// Maximum redirect hops followed per seed (each hop is recorded).
pub const MAX_REDIRECT_HOPS: usize = 8;

/// Upper bound honoured for a `Retry-After` delay (seconds).
pub const MAX_RETRY_AFTER_SECS: u64 = 300;

/// `sha1:<BASE32>` digest of `data` (RFC 4648 base32, no padding), the
/// digest format used for `WARC-Block-Digest` and `WARC-Payload-Digest`.
pub fn sha1_base32(data: &[u8]) -> String {
    use sha1::Digest as _;
    let digest = sha1::Sha1::digest(data);
    format!("sha1:{}", data_encoding::BASE32_NOPAD.encode(&digest))
}

/// New random `WARC-Record-ID` value (`<urn:uuid:...>`).
///
/// The 128 bits come from SHA-1 over wall-clock nanos, the process id,
/// and an atomic counter — no extra RNG dependency, unique within and
/// across runs for any realistic crawl.
pub fn new_record_id() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    use sha1::Digest as _;
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let salt = COUNTER.fetch_add(1, Ordering::Relaxed);
    let mut hasher = sha1::Sha1::new();
    hasher.update(format!("{}-{}-{salt}", nanos, std::process::id()));
    let digest = hasher.finalize();
    let h = |i: usize| format!("{:02x}", digest[i]);
    let mut hex = String::with_capacity(32);
    for i in 0..16 {
        hex.push_str(&h(i));
    }
    format!(
        "<urn:uuid:{}-{}-{}-{}-{}>",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

/// Current UTC time as a `WARC-Date` value (`YYYY-MM-DDTHH:MM:SSZ`).
pub fn now_warc_date() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    format_warc_date(secs)
}

/// Format unix seconds as `YYYY-MM-DDTHH:MM:SSZ` (proleptic Gregorian).
fn format_warc_date(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (hour, minute, second) = (
        (rem / 3600) as u32,
        ((rem % 3600) / 60) as u32,
        (rem % 60) as u32,
    );
    // days_from_civil inverse (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = if month <= 2 { year + 1 } else { year };
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

/// Split `http(s)://host[:port][/path?query]` URL into parts.
///
/// Returns `None` for non-HTTP(S) URLs, missing hosts, or embedded
/// whitespace. The path always starts with `/` (defaults to `/`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UrlParts {
    /// `http` or `https`.
    pub scheme: String,
    /// Lower-cased host without port.
    pub host: String,
    /// Explicit or default (80/443) port.
    pub port: u16,
    /// Path plus optional query, always starting with `/`.
    pub path: String,
}

impl UrlParts {
    /// `scheme://host[:port]` origin prefix (for robots.txt URLs).
    pub fn origin(&self) -> String {
        if (self.scheme == "https" && self.port == 443)
            || (self.scheme == "http" && self.port == 80)
        {
            format!("{}://{}", self.scheme, self.host)
        } else {
            format!("{}://{}:{}", self.scheme, self.host, self.port)
        }
    }
}

/// Parse an absolute HTTP(S) URL. See [`UrlParts`] for the contract.
pub fn split_url(url: &str) -> Option<UrlParts> {
    if url.chars().any(char::is_whitespace) {
        return None;
    }
    let (scheme, rest) = url
        .strip_prefix("https://")
        .map(|rest| ("https", rest))
        .or_else(|| url.strip_prefix("http://").map(|rest| ("http", rest)))?;
    let (authority, path) = match rest.find('/') {
        Some(i) => (&rest[..i], rest[i..].to_string()),
        None => (rest, "/".to_string()),
    };
    if authority.is_empty() || path.contains(['\\', ' ', '\t']) {
        return None;
    }
    let (host, port) = match authority.rsplit_once(':') {
        Some((h, p)) if !h.is_empty() => match p.parse::<u16>() {
            Ok(port) => (h.to_ascii_lowercase(), port),
            Err(_) => return None,
        },
        _ => (
            authority.to_ascii_lowercase(),
            if scheme == "https" { 443 } else { 80 },
        ),
    };
    if host.is_empty() {
        return None;
    }
    Some(UrlParts {
        scheme: scheme.to_string(),
        host,
        port,
        path,
    })
}

/// Resolve a `Location` header value against the request URL.
///
/// Handles absolute, scheme-relative (`//host/path`), root-relative
/// (`/path`), and bare-relative (`sibling`) targets. Returns `None`
/// when the target is not a usable HTTP(S) URL.
pub fn resolve_redirect(base_url: &str, location: &str) -> Option<String> {
    let location = location.trim();
    if location.is_empty() {
        return None;
    }
    if location.starts_with("http://") || location.starts_with("https://") {
        return split_url(location).map(|_| location.to_string());
    }
    let base = split_url(base_url)?;
    if let Some(rest) = location.strip_prefix("//") {
        let candidate = format!("{}://{rest}", base.scheme);
        return split_url(&candidate).map(|_| candidate);
    }
    if location.starts_with('/') {
        let path = location.split(['#']).next().unwrap_or(location);
        return Some(format!("{}{path}", base.origin()));
    }
    // Any other scheme (ftp:, mailto:, ...) is not a crawl target.
    if has_scheme(location) {
        return None;
    }
    // Bare-relative: merge with the base directory.
    let dir = match base.path.rfind('/') {
        Some(i) => &base.path[..=i],
        None => "/",
    };
    let path = format!("{dir}{location}");
    Some(format!("{}{}", base.origin(), normalize_dots(&path)))
}

/// True for `scheme:`-prefixed targets (`http:`, `ftp:`, `mailto:`, ...).
fn has_scheme(location: &str) -> bool {
    let Some(colon) = location.find(':') else {
        return false;
    };
    let scheme = &location[..colon];
    let mut chars = scheme.chars();
    match chars.next() {
        Some(first) if first.is_ascii_alphabetic() => {}
        _ => return false,
    }
    scheme.len() <= 16 && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
}

/// Collapse `.` and `..` segments of an absolute path (no decoding).
fn normalize_dots(path: &str) -> String {
    let mut out: Vec<&str> = Vec::new();
    for seg in path.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                out.pop();
            }
            _ => out.push(seg),
        }
    }
    let collapsed = out.join("/");
    format!("/{collapsed}")
}

/// Pointer to a record just written (for `Concurrent-To` linking).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordRef {
    /// The `WARC-Record-ID` value, including angle brackets.
    pub id: String,
    /// The `WARC-Date` value.
    pub date: String,
}

/// Totals reported by [`WarcWriter::finish`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileStats {
    /// Records written (each one gzip member).
    pub records: u64,
    /// Compressed bytes on disk.
    pub bytes: u64,
}

/// WARC/1.1 writer: one gzip member per record, appended to `path`.
///
/// Create with [`WarcWriter::create`], write a `warcinfo` first, then
/// request/response/metadata triples. Call [`WarcWriter::finish`] (or
/// drop the writer) to flush. Parent directories are created.
pub struct WarcWriter {
    out: BufWriter<std::fs::File>,
    records: u64,
    bytes: u64,
}

impl WarcWriter {
    /// Create (or truncate) `path` for writing, creating parents.
    ///
    /// # Errors
    ///
    /// When the file or a parent directory cannot be created.
    pub fn create(path: &Path) -> io::Result<Self> {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)?;
        }
        Ok(Self {
            out: BufWriter::new(std::fs::File::create(path)?),
            records: 0,
            bytes: 0,
        })
    }

    /// Records written so far (for the 1 GiB file-rotation rule).
    pub fn records_written(&self) -> u64 {
        self.records
    }

    /// Compressed bytes written so far (for rotation).
    pub fn bytes_written(&self) -> u64 {
        self.bytes
    }

    /// Write the per-file `warcinfo` record (crawler identity + caller
    /// `extra` fields such as filename, operator, pack).
    ///
    /// # Errors
    ///
    /// On underlying I/O failure.
    pub fn write_warcinfo(&mut self, extra: &[(&str, &str)]) -> io::Result<RecordRef> {
        let mut block = String::from("software: handbook-crawl/0.1\r\n");
        block.push_str("format: WARC File Format 1.1\r\n");
        for (name, value) in extra {
            block.push_str(&sanitize_field(name));
            block.push_str(": ");
            block.push_str(&sanitize_field(value));
            block.push_str("\r\n");
        }
        self.append_record(
            "warcinfo",
            "application/warc-fields",
            None,
            block.as_bytes(),
            None,
            None,
        )
    }

    /// Write a `request` record holding the raw HTTP request bytes.
    ///
    /// # Errors
    ///
    /// On underlying I/O failure.
    pub fn write_request(
        &mut self,
        target_uri: &str,
        request_bytes: &[u8],
        concurrent_to: Option<&str>,
    ) -> io::Result<RecordRef> {
        self.append_record(
            "request",
            "application/http; msgtype=request",
            Some(target_uri),
            request_bytes,
            None,
            concurrent_to,
        )
    }

    /// Write a `response` record: `response_bytes` is the full HTTP
    /// response message (status line + headers + body) and `payload` is
    /// the body alone (for `WARC-Payload-Digest`).
    ///
    /// # Errors
    ///
    /// On underlying I/O failure.
    pub fn write_response(
        &mut self,
        target_uri: &str,
        response_bytes: &[u8],
        payload: &[u8],
        concurrent_to: Option<&str>,
    ) -> io::Result<RecordRef> {
        self.append_record(
            "response",
            "application/http; msgtype=response",
            Some(target_uri),
            response_bytes,
            Some(&sha1_base32(payload)),
            concurrent_to,
        )
    }

    /// Write a `metadata` record (`application/warc-fields` block with
    /// one `name: value` line per field: slug, pack, robots verdict,
    /// TLS chain, depth, hop index, ...).
    ///
    /// # Errors
    ///
    /// On underlying I/O failure.
    pub fn write_metadata(
        &mut self,
        target_uri: &str,
        fields: &[(&str, &str)],
        concurrent_to: Option<&str>,
    ) -> io::Result<RecordRef> {
        let mut block = String::new();
        for (name, value) in fields {
            block.push_str(&sanitize_field(name));
            block.push_str(": ");
            block.push_str(&sanitize_field(value));
            block.push_str("\r\n");
        }
        self.append_record(
            "metadata",
            "application/warc-fields",
            Some(target_uri),
            block.as_bytes(),
            None,
            concurrent_to,
        )
    }

    /// Flush and report totals.
    ///
    /// # Errors
    ///
    /// On underlying I/O failure.
    pub fn finish(mut self) -> io::Result<FileStats> {
        self.out.flush()?;
        Ok(FileStats {
            records: self.records,
            bytes: self.bytes,
        })
    }

    fn append_record(
        &mut self,
        warc_type: &str,
        content_type: &str,
        target_uri: Option<&str>,
        block: &[u8],
        payload_digest: Option<&str>,
        concurrent_to: Option<&str>,
    ) -> io::Result<RecordRef> {
        let id = new_record_id();
        let date = now_warc_date();
        let mut raw = Vec::with_capacity(block.len() + 512);
        raw.extend_from_slice(WARC_VERSION.as_bytes());
        raw.extend_from_slice(b"\r\n");
        push_header(&mut raw, "WARC-Type", warc_type);
        push_header(&mut raw, "WARC-Date", &date);
        push_header(&mut raw, "WARC-Record-ID", &id);
        if let Some(uri) = target_uri {
            push_header(&mut raw, "WARC-Target-URI", &sanitize_field(uri));
        }
        push_header(&mut raw, "Content-Type", content_type);
        if let Some(concurrent) = concurrent_to {
            push_header(&mut raw, "WARC-Concurrent-To", concurrent);
        }
        if let Some(digest) = payload_digest {
            push_header(&mut raw, "WARC-Payload-Digest", digest);
        }
        push_header(&mut raw, "WARC-Block-Digest", &sha1_base32(block));
        push_header(&mut raw, "Content-Length", &block.len().to_string());
        raw.extend_from_slice(b"\r\n");
        raw.extend_from_slice(block);
        raw.extend_from_slice(b"\r\n\r\n");
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(&raw)?;
        let member = encoder.finish()?;
        self.out.write_all(&member)?;
        self.records += 1;
        self.bytes += member.len() as u64;
        Ok(RecordRef { id, date })
    }
}

fn push_header(raw: &mut Vec<u8>, name: &str, value: &str) {
    raw.extend_from_slice(name.as_bytes());
    raw.extend_from_slice(b": ");
    raw.extend_from_slice(sanitize_field(value).as_bytes());
    raw.extend_from_slice(b"\r\n");
}

/// Strip CR/LF from a header or field value (single-line guarantee).
fn sanitize_field(value: &str) -> String {
    value.replace(['\r', '\n'], " ")
}

/// One decoded WARC record (see [`read_records`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WarcRecord {
    /// Format version line, normally `WARC/1.1`.
    pub version: String,
    /// Envelope headers in file order.
    pub headers: Vec<(String, String)>,
    /// Raw record block (`Content-Length` bytes).
    pub block: Vec<u8>,
}

impl WarcRecord {
    /// First envelope header value for `name` (case-insensitive).
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    /// Recompute the block digest and compare with `WARC-Block-Digest`.
    pub fn block_digest_ok(&self) -> bool {
        self.header("WARC-Block-Digest")
            .is_some_and(|d| d == sha1_base32(&self.block))
    }
}

/// Read back every record of a multi-member `.warc.gz` file.
///
/// This is the independent validation path: it gunzips with
/// `MultiGzDecoder` and re-parses the envelope from scratch, so a
/// writer/reader round trip proves the bytes on disk are well-formed
/// WARC/1.1, not just that the writer ran.
///
/// # Errors
///
/// On I/O failure or malformed envelopes (bad version line, missing
/// `Content-Length`, short blocks, missing record terminator).
pub fn read_records(path: &Path) -> io::Result<Vec<WarcRecord>> {
    let mut file = std::fs::File::open(path)?;
    let mut head = [0u8; 2];
    let sniffed = file.read_exact(&mut head).is_ok();
    let mut raw = Vec::new();
    if sniffed && head == [0x1f, 0x8b] {
        // Multi-member gzip: our per-record members and wget's output.
        use std::io::Seek as _;
        file.rewind()?;
        let mut decoder = flate2::read::MultiGzDecoder::new(file);
        decoder.read_to_end(&mut raw)?;
    } else if sniffed {
        // Plain (uncompressed) WARC, e.g. a wget `--warc-file` drop.
        use std::io::Seek as _;
        file.rewind()?;
        file.read_to_end(&mut raw)?;
    }
    // Empty files yield zero records (a present-but-empty drop is not
    // an error; the packager reports its record count as 0).
    parse_records(&raw)
}

/// Parse concatenated WARC records from decompressed bytes.
fn parse_records(raw: &[u8]) -> io::Result<Vec<WarcRecord>> {
    let mut records = Vec::new();
    let mut pos = 0;
    while pos < raw.len() {
        let line_end = find_crlf(raw, pos).ok_or_else(|| invalid("missing version line"))?;
        let version = std::str::from_utf8(&raw[pos..line_end])
            .map_err(|_| invalid("non-UTF8 version line"))?;
        if version != WARC_VERSION {
            return Err(invalid(format!("bad version line: {version}")));
        }
        let head_start = line_end + 2;
        let head_end =
            find_header_end(raw, head_start).ok_or_else(|| invalid("unterminated headers"))?;
        let mut headers = Vec::new();
        for line in raw[head_start..head_end].split(|b| *b == b'\n') {
            let line = strip_cr(line);
            if line.is_empty() {
                continue;
            }
            let colon = line
                .iter()
                .position(|b| *b == b':')
                .ok_or_else(|| invalid("header without colon"))?;
            let name = String::from_utf8_lossy(&line[..colon]).trim().to_string();
            let value = String::from_utf8_lossy(&line[colon + 1..])
                .trim()
                .to_string();
            headers.push((name, value));
        }
        let length: usize = headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("Content-Length"))
            .and_then(|(_, v)| v.parse().ok())
            .ok_or_else(|| invalid("missing Content-Length"))?;
        let block_start = head_end + 4;
        let block_end = block_start
            .checked_add(length)
            .ok_or_else(|| invalid("length overflow"))?;
        if block_end + 4 > raw.len() {
            return Err(invalid("short block"));
        }
        if &raw[block_end..block_end + 4] != b"\r\n\r\n" {
            return Err(invalid("missing record terminator"));
        }
        records.push(WarcRecord {
            version: version.to_string(),
            headers,
            block: raw[block_start..block_end].to_vec(),
        });
        pos = block_end + 4;
    }
    Ok(records)
}

fn find_crlf(raw: &[u8], from: usize) -> Option<usize> {
    raw[from..]
        .windows(2)
        .position(|w| w == b"\r\n")
        .map(|i| from + i)
}

fn find_header_end(raw: &[u8], from: usize) -> Option<usize> {
    raw[from..]
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map(|i| from + i)
}

fn strip_cr(line: &[u8]) -> &[u8] {
    line.strip_suffix(b"\r".as_slice()).unwrap_or(line)
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

/// Verdict of a fail-closed robots check (see [`check_robots_closed`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RobotsVerdict {
    /// Fetch may proceed only when true.
    pub allowed: bool,
    /// Machine-readable reason (`allowed`, `no-robots`, `disallowed`,
    /// `robots-unreachable-closed`, `robots-status-closed:<code>`,
    /// `unparseable-uri-closed`).
    pub text: String,
    /// Honoured `Crawl-delay` for our agent group (seconds, 0 = none).
    /// Callers sleep at least this long between same-host requests.
    pub crawl_delay_secs: u64,
}

/// Fetch `<origin>/robots.txt` for `url` and judge `url` FAIL CLOSED.
///
/// Differences from the thin watch gate (`fetch::check_robots`, which
/// fails open): any fetch error, non-2xx/non-404 status, or
/// unparseable URL disallows the fetch, and path-prefix rules are
/// honoured per agent group (`*` or `handbook*`), longest match wins
/// (ties go to `Allow`). A 404 or empty body means "no robots" and
/// allows the fetch.
pub fn check_robots_closed(url: &str, timeout: Duration) -> RobotsVerdict {
    install_crypto_provider();
    let Some(parts) = split_url(url) else {
        return RobotsVerdict {
            allowed: false,
            text: "unparseable-uri-closed".to_string(),
            crawl_delay_secs: 0,
        };
    };
    // Status-as-success: a 404 robots.txt is data ("no robots"), not
    // a transport failure. Anything else non-2xx still fails closed.
    let config = ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        .http_status_as_error(false)
        .build();
    let agent = ureq::Agent::new_with_config(config);
    let robots_url = format!("{}/robots.txt", parts.origin());
    let response = agent
        .get(&robots_url)
        .header("User-Agent", CRAWL_USER_AGENT)
        .call();
    match response {
        Err(_) => RobotsVerdict {
            allowed: false,
            text: "robots-unreachable-closed".to_string(),
            crawl_delay_secs: 0,
        },
        Ok(mut response) => {
            let status = response.status().as_u16();
            if status == 404 {
                return RobotsVerdict {
                    allowed: true,
                    text: "no-robots".to_string(),
                    crawl_delay_secs: 0,
                };
            }
            if !(200..300).contains(&status) {
                return RobotsVerdict {
                    allowed: false,
                    text: format!("robots-status-closed:{status}"),
                    crawl_delay_secs: 0,
                };
            }
            let body = response
                .body_mut()
                .read_to_string()
                .unwrap_or_else(|_| String::new());
            parse_robots(&body, &parts.path)
        }
    }
}

/// Judge `path` against a robots.txt `body` (pure, no I/O).
///
/// Groups apply when a `User-agent` token is `*` or mentions
/// `handbook` (case-insensitive); the longest matching `Allow` /
/// `Disallow` rule wins, ties allow. `Crawl-delay` of the first
/// applicable group is reported (fractional values round up).
/// Empty bodies allow everything.
pub fn parse_robots(body: &str, path: &str) -> RobotsVerdict {
    if body.trim().is_empty() {
        return RobotsVerdict {
            allowed: true,
            text: "no-robots".to_string(),
            crawl_delay_secs: 0,
        };
    }
    #[derive(Default)]
    struct Group {
        applies: bool,
        has_rules: bool,
        best_allow: usize,
        best_disallow: usize,
        delay: Option<u64>,
    }
    let mut groups: Vec<Group> = vec![Group::default()];
    // A rule line belongs to the current group until a new User-agent
    // line starts a group (standard "first group with our agent" walk:
    // keep it simple — accumulate every applicable group, judge with
    // the union, which is the conservative reading).
    for raw_line in body.lines() {
        let line = raw_line.split('#').next().unwrap_or("").trim();
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let (key, value) = (key.trim().to_ascii_lowercase(), value.trim());
        match key.as_str() {
            "user-agent" => {
                let token = value.to_ascii_lowercase();
                let applies = token == "*" || token.contains("handbook");
                if groups.last().is_some_and(|g| !g.has_rules && !g.applies) {
                    groups.last_mut().expect("last group").applies |= applies;
                } else {
                    groups.push(Group {
                        applies,
                        ..Group::default()
                    });
                }
            }
            "allow" | "disallow" => {
                let group = groups.last_mut().expect("at least one group");
                group.has_rules = true;
                if !group.applies || value.is_empty() {
                    continue;
                }
                if rule_matches(value, path) {
                    if key == "allow" {
                        group.best_allow = group.best_allow.max(value.len());
                    } else {
                        group.best_disallow = group.best_disallow.max(value.len());
                    }
                }
            }
            "crawl-delay" => {
                let group = groups.last_mut().expect("at least one group");
                group.has_rules = true;
                if group.applies && group.delay.is_none() {
                    group.delay = value
                        .parse::<f64>()
                        .ok()
                        .map(|d| d.ceil().clamp(0.0, 3_600.0) as u64);
                }
            }
            _ => {}
        }
    }
    let delay = groups
        .iter()
        .filter(|g| g.applies)
        .find_map(|g| g.delay)
        .unwrap_or(0);
    let allow = groups
        .iter()
        .filter(|g| g.applies)
        .map(|g| g.best_allow)
        .max()
        .unwrap_or(0);
    let disallow = groups
        .iter()
        .filter(|g| g.applies)
        .map(|g| g.best_disallow)
        .max()
        .unwrap_or(0);
    if disallow > allow {
        RobotsVerdict {
            allowed: false,
            text: "disallowed".to_string(),
            crawl_delay_secs: delay,
        }
    } else {
        RobotsVerdict {
            allowed: true,
            text: "allowed".to_string(),
            crawl_delay_secs: delay,
        }
    }
}

/// True when a robots path rule matches `path` (prefix match; a rule
/// of `/` matches everything, query included).
fn rule_matches(rule: &str, path: &str) -> bool {
    path.starts_with(rule)
}

/// One redirect hop of a fetch (see [`fetch_with_hops`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HopCapture {
    /// URL requested for this hop.
    pub url: String,
    /// Raw HTTP request bytes (what a `request` record holds).
    pub request_bytes: Vec<u8>,
    /// HTTP status code (2xx–5xx; transport failures abort the fetch).
    pub status: u16,
    /// Raw HTTP response head: status line + headers + blank line.
    pub response_head: Vec<u8>,
    /// Response body (capped at [`MAX_BODY_BYTES`]).
    pub body: Vec<u8>,
}

impl HopCapture {
    /// Full HTTP response message (head + body) for a `response` record.
    pub fn response_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.response_head.len() + self.body.len());
        out.extend_from_slice(&self.response_head);
        out.extend_from_slice(&self.body);
        out
    }
}

/// Captured fetch: every redirect hop plus the TLS peer chain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchResult {
    /// One entry per hop; the last entry is the final response
    /// (which may itself be a 3xx that exceeded [`MAX_REDIRECT_HOPS`]
    /// or a 4xx/5xx — both are archived faithfully).
    pub hops: Vec<HopCapture>,
    /// `sha1:<BASE32>` fingerprints of the presented peer certificates
    /// (leaf first) for the final HTTPS host; empty for plain HTTP.
    pub tls_chain: Vec<String>,
    /// `tls-ok`, `tls-not-applicable-http`, or `tls-unavailable:<why>`.
    pub tls_note: String,
}

/// Fetch failure kinds (no records are written for these).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FetchError {
    /// DNS/TLS/connection/timeout/read failures.
    Transport(String),
    /// Body (or declared `Content-Length`) exceeded [`MAX_BODY_BYTES`].
    TooLarge(u64),
    /// Redirect chain exhausted without a final response.
    TooManyRedirects,
}

impl std::fmt::Display for FetchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FetchError::Transport(message) => write!(f, "fetch transport error: {message}"),
            FetchError::TooLarge(len) => write!(f, "fetch body exceeds cap ({len} bytes)"),
            FetchError::TooManyRedirects => {
                write!(f, "fetch exceeded {MAX_REDIRECT_HOPS} redirect hops")
            }
        }
    }
}

impl std::error::Error for FetchError {}

/// GET `start_url` with manual redirect following.
///
/// Returns one [`HopCapture`] per hop (request bytes reconstructed as
/// sent: `GET path HTTP/1.1` + `Host` + crawler `User-Agent`). On HTTP
/// 429 the `Retry-After` value (seconds form, capped at
/// [`MAX_RETRY_AFTER_SECS`], default 1 s) is honoured once per URL;
/// a second 429 is returned as the final hop. Bodies larger than
/// [`MAX_BODY_BYTES`] abort with [`FetchError::TooLarge`].
///
/// After the final hop, the TLS peer chain of the final host is
/// captured best-effort (HTTPS only); a TLS failure never fails the
/// fetch, it is reported in [`FetchResult::tls_note`].
///
/// # Errors
///
/// [`FetchError::Transport`] on connection/TLS/timeout failure,
/// [`FetchError::TooLarge`] over the body cap,
/// [`FetchError::TooManyRedirects`] when every hop redirects.
pub fn fetch_with_hops(start_url: &str, timeout: Duration) -> Result<FetchResult, FetchError> {
    install_crypto_provider();
    let config = ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        .http_status_as_error(false)
        .max_redirects(0)
        .build();
    let agent = ureq::Agent::new_with_config(config);
    let mut hops = Vec::new();
    let mut url = start_url.to_string();
    loop {
        let hop = fetch_single(&agent, &url, false)?;
        let redirect = matches!(hop.status, 301 | 302 | 303 | 307 | 308);
        hops.push(hop);
        if !redirect {
            break;
        }
        if hops.len() > MAX_REDIRECT_HOPS {
            return Err(FetchError::TooManyRedirects);
        }
        let location = last_hop_location(hops.last().expect("just pushed"));
        match location.and_then(|loc| resolve_redirect(&url, loc)) {
            Some(next) => url = next,
            None => break,
        }
    }
    let final_url = hops.last().expect("non-empty").url.clone();
    let (tls_chain, tls_note) = probe_tls(&final_url, timeout);
    Ok(FetchResult {
        hops,
        tls_chain,
        tls_note,
    })
}

fn last_hop_location(hop: &HopCapture) -> Option<&str> {
    response_header_value(&hop.response_head, "location")
}

/// Single GET with one 429/Retry-After retry when `retried` is false.
fn fetch_single(agent: &ureq::Agent, url: &str, retried: bool) -> Result<HopCapture, FetchError> {
    let parts =
        split_url(url).ok_or_else(|| FetchError::Transport(format!("unparseable URL: {url}")))?;
    let request_bytes = build_request_bytes(&parts);
    let mut response = agent
        .get(url)
        .header("User-Agent", CRAWL_USER_AGENT)
        .header("Accept", "*/*")
        .call()
        .map_err(|e| FetchError::Transport(format!("{url}: {e}")))?;
    let status = response.status().as_u16();
    if status == 429 && !retried {
        let wait = retry_after_secs(&mut response).min(MAX_RETRY_AFTER_SECS);
        std::thread::sleep(Duration::from_secs(wait.max(1)));
        return fetch_single(agent, url, true);
    }
    let head = build_response_head(&mut response);
    let declared = response_header_value(&head, "content-length")
        .and_then(|v| v.trim().parse::<u64>().ok())
        .unwrap_or(0);
    if declared > MAX_BODY_BYTES {
        return Err(FetchError::TooLarge(declared));
    }
    use std::io::Read as _;
    let mut body = Vec::new();
    response
        .body_mut()
        .as_reader()
        .take(MAX_BODY_BYTES + 1)
        .read_to_end(&mut body)
        .map_err(|e| FetchError::Transport(format!("{url}: read body: {e}")))?;
    if body.len() as u64 > MAX_BODY_BYTES {
        return Err(FetchError::TooLarge(body.len() as u64));
    }
    Ok(HopCapture {
        url: url.to_string(),
        request_bytes,
        status,
        response_head: head,
        body,
    })
}

/// `Retry-After` in seconds form (delta-seconds); anything else (HTTP
/// dates, garbage, missing) falls back to 1 s — the date form names an
/// absolute time, not a wait, and re-resolving clocks is out of scope.
fn retry_after_secs(response: &mut ureq::http::Response<ureq::Body>) -> u64 {
    response
        .headers()
        .get("retry-after")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<u64>().ok())
        .unwrap_or(1)
}

/// Case-insensitive header lookup over raw head bytes.
fn response_header_value<'a>(head: &'a [u8], name: &str) -> Option<&'a str> {
    let text = std::str::from_utf8(head).ok()?;
    for line in text.split("\r\n").skip(1) {
        if line.is_empty() {
            break;
        }
        if let Some((key, value)) = line.split_once(':')
            && key.trim().eq_ignore_ascii_case(name)
        {
            return Some(value.trim());
        }
    }
    None
}

fn build_request_bytes(parts: &UrlParts) -> Vec<u8> {
    format!(
        "GET {} HTTP/1.1\r\nHost: {}\r\nUser-Agent: {CRAWL_USER_AGENT}\r\nAccept: */*\r\nConnection: close\r\n\r\n",
        parts.path, parts.host
    )
    .into_bytes()
}

fn build_response_head(response: &mut ureq::http::Response<ureq::Body>) -> Vec<u8> {
    let status = response.status();
    let reason = status.canonical_reason().unwrap_or("Unknown");
    let mut head = format!("HTTP/1.1 {} {reason}\r\n", status.as_u16()).into_bytes();
    for (name, value) in response.headers() {
        head.extend_from_slice(name.as_str().as_bytes());
        head.extend_from_slice(b": ");
        head.extend_from_slice(value.as_bytes());
        head.extend_from_slice(b"\r\n");
    }
    head.extend_from_slice(b"\r\n");
    head
}

/// Install the workspace's single rustls provider (aws-lc-rs).
/// First install wins; a prior install is fine, not an error.
fn install_crypto_provider() {
    let provider = rustls::crypto::aws_lc_rs::default_provider();
    let _ = rustls::crypto::CryptoProvider::install_default(provider);
}

/// `sha1:<BASE32>` fingerprint of one DER certificate (TLS chain entry).
pub fn tls_fingerprint(der: &[u8]) -> String {
    sha1_base32(der)
}

/// Capture the presented TLS peer chain for `final_url` (best effort).
///
/// Returns `(fingerprints, note)` where the note is `tls-ok`,
/// `tls-not-applicable-http`, or `tls-unavailable:<why>`. HTTPS only;
/// plain-HTTP URLs skip the handshake. A failure never fails the
/// fetch — the note records it in the `metadata` record instead.
fn probe_tls(final_url: &str, timeout: Duration) -> (Vec<String>, String) {
    let Some(parts) = split_url(final_url) else {
        return (Vec::new(), "tls-unavailable:unparseable-url".to_string());
    };
    if parts.scheme != "https" {
        return (Vec::new(), "tls-not-applicable-http".to_string());
    }
    match capture_tls_chain(&parts.host, parts.port, timeout) {
        Ok(chain) => (chain, "tls-ok".to_string()),
        Err(why) => (Vec::new(), format!("tls-unavailable:{why}")),
    }
}

/// TLS handshake against `host:port`, returning leaf-first peer
/// certificate fingerprints. Verifies against the platform web PKI
/// roots; any failure is an `Err` string for the metadata note.
///
/// # Errors
///
/// DNS, TCP, timeout, TLS handshake/verification failures.
pub fn capture_tls_chain(host: &str, port: u16, timeout: Duration) -> Result<Vec<String>, String> {
    use std::net::ToSocketAddrs as _;
    install_crypto_provider();
    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let config = rustls::ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    let server_name = rustls::pki_types::ServerName::try_from(host.to_string())
        .map_err(|e| format!("bad server name: {e}"))?;
    let mut conn = rustls::ClientConnection::new(std::sync::Arc::new(config), server_name)
        .map_err(|e| format!("tls setup: {e}"))?;
    let addr = format!("{host}:{port}")
        .to_socket_addrs()
        .map_err(|e| format!("dns: {e}"))?
        .next()
        .ok_or_else(|| "dns: no address".to_string())?;
    let mut sock =
        std::net::TcpStream::connect_timeout(&addr, timeout).map_err(|e| format!("tcp: {e}"))?;
    sock.set_read_timeout(Some(timeout))
        .map_err(|e| format!("tcp: {e}"))?;
    sock.set_write_timeout(Some(timeout))
        .map_err(|e| format!("tcp: {e}"))?;
    while conn.is_handshaking() {
        conn.complete_io(&mut sock)
            .map_err(|e| format!("handshake: {e}"))?;
    }
    let peer = conn
        .peer_certificates()
        .ok_or_else(|| "no peer certificates".to_string())?;
    Ok(peer
        .iter()
        .map(|cert| tls_fingerprint(cert.as_ref()))
        .collect())
}

/// Extract absolute HTTP(S) links from an HTML body.
///
/// Scans `href="..."` / `href='...'` case-insensitively, resolves
/// each against `base_url` (same forms as [`resolve_redirect`]),
/// keeps only HTTP(S) targets, dedupes preserving first-seen order.
/// Fragments are stripped. Non-HTML bodies yield garbage-in (callers
/// only feed it HTML content types).
pub fn extract_links(html: &[u8], base_url: &str) -> Vec<String> {
    let text = String::from_utf8_lossy(html);
    let lower = text.to_ascii_lowercase();
    let mut out = Vec::new();
    let mut pos = 0;
    while let Some(found) = lower[pos..].find("href") {
        let mut i = pos + found + 4;
        let bytes = lower.as_bytes();
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if bytes.get(i) != Some(&b'=') {
            pos = i;
            continue;
        }
        i += 1;
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        let quote = bytes.get(i).copied();
        let (start, end) = if quote == Some(b'"') || quote == Some(b'\'') {
            let q = quote.expect("quoted");
            let start = i + 1;
            let rest = &lower[start..];
            let len = rest.find(char::from(q)).unwrap_or(rest.len());
            (start, start + len)
        } else {
            let start = i;
            let rest = &lower[start..];
            let len = rest
                .find(|c: char| c.is_whitespace() || c == '>')
                .unwrap_or(rest.len());
            (start, start + len)
        };
        pos = end + 1;
        let raw = text.get(start..end).unwrap_or("").trim();
        if raw.is_empty() || raw.starts_with('#') {
            continue;
        }
        let target = raw.split('#').next().unwrap_or(raw);
        if target.eq_ignore_ascii_case("javascript:void(0)") || target.starts_with("mailto:") {
            continue;
        }
        if let Some(resolved) = resolve_redirect(base_url, target)
            && !out.contains(&resolved)
        {
            out.push(resolved);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    /// Loopback HTTP server: the i-th connection gets `replies[i]`
    /// (the last reply repeats), then the thread exits after `takes`
    /// connections. Returns the base URL (`http://127.0.0.1:port`).
    fn serve_loop(replies: Vec<Vec<u8>>, takes: usize) -> (String, std::thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        let base = format!(
            "http://127.0.0.1:{}/",
            listener.local_addr().unwrap().port()
        );
        let handle = std::thread::spawn(move || {
            for i in 0..takes {
                let (mut stream, _) = listener.accept().expect("accept");
                let mut buf = [0u8; 4096];
                let _ = stream.read(&mut buf);
                let reply = if i < replies.len() {
                    &replies[i]
                } else {
                    replies.last().expect("at least one reply")
                };
                stream.write_all(reply).expect("write reply");
            }
        });
        (base, handle)
    }

    fn reply(status: &str, headers: &[(&str, &str)], body: &[u8]) -> Vec<u8> {
        let mut out = format!("{status}\r\nContent-Length: {}\r\n", body.len()).into_bytes();
        for (name, value) in headers {
            out.extend_from_slice(format!("{name}: {value}\r\n").as_bytes());
        }
        out.extend_from_slice(b"Connection: close\r\n\r\n");
        out.extend_from_slice(body);
        out
    }

    #[test]
    fn warc_date_epoch_and_shape() {
        assert_eq!(format_warc_date(0), "1970-01-01T00:00:00Z");
        assert_eq!(format_warc_date(86_400 + 3_661), "1970-01-02T01:01:01Z");
        let now = now_warc_date();
        assert_eq!(now.len(), 20);
        assert!(now.ends_with('Z'));
        assert_eq!(now.chars().nth(10), Some('T'));
    }

    #[test]
    fn record_ids_unique_and_shaped() {
        let ids: Vec<String> = (0..50).map(|_| new_record_id()).collect();
        for id in &ids {
            assert!(id.starts_with("<urn:uuid:"), "{id}");
            assert!(id.ends_with('>'), "{id}");
            assert_eq!(id.len(), "<urn:uuid:>".len() + 36);
        }
        let mut sorted = ids.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), ids.len());
    }

    #[test]
    fn sha1_base32_shape_and_stability() {
        let digest = sha1_base32(b"abc");
        assert!(digest.starts_with("sha1:"), "{digest}");
        assert_eq!(digest.len(), 5 + 32);
        assert_eq!(digest, sha1_base32(b"abc"));
        assert_ne!(digest, sha1_base32(b"abd"));
    }

    #[test]
    fn split_url_shapes() {
        let parts = split_url("https://Example.GOV:8443/a/b?c=d").expect("parse");
        assert_eq!(parts.scheme, "https");
        assert_eq!(parts.host, "example.gov");
        assert_eq!(parts.port, 8443);
        assert_eq!(parts.path, "/a/b?c=d");
        let bare = split_url("http://example.gov").expect("parse");
        assert_eq!((bare.port, bare.path.as_str()), (80, "/"));
        assert_eq!(split_url("not a url"), None);
        assert_eq!(split_url("ftp://example.gov/"), None);
        assert_eq!(split_url("https:///path"), None);
    }

    #[test]
    fn resolve_redirect_forms() {
        let base = "https://example.gov/dir/page.html";
        assert_eq!(
            resolve_redirect(base, "https://other.gov/x"),
            Some("https://other.gov/x".to_string())
        );
        assert_eq!(
            resolve_redirect(base, "//other.gov/x"),
            Some("https://other.gov/x".to_string())
        );
        assert_eq!(
            resolve_redirect(base, "/rooted"),
            Some("https://example.gov/rooted".to_string())
        );
        assert_eq!(
            resolve_redirect(base, "sibling?q=1"),
            Some("https://example.gov/dir/sibling?q=1".to_string())
        );
        assert_eq!(resolve_redirect(base, ""), None);
        assert_eq!(resolve_redirect(base, "ftp://example.gov/"), None);
    }

    #[test]
    fn robots_parsing_rules() {
        assert!(parse_robots("", "/anything").allowed);
        assert!(!parse_robots("User-agent: *\nDisallow: /", "/x").allowed);
        let body = "User-agent: *\nDisallow: /tmp\nAllow: /tmp/open\n";
        assert!(!parse_robots(body, "/tmp/secret").allowed);
        assert!(parse_robots(body, "/tmp/open/file").allowed);
        // Tie goes to Allow.
        let tie = "User-agent: *\nDisallow: /a\nAllow: /a\n";
        assert!(parse_robots(tie, "/a").allowed);
        // Foreign agents do not apply.
        let foreign = "User-agent: otherbot\nDisallow: /\n";
        assert!(parse_robots(foreign, "/x").allowed);
        // Our named agent applies.
        let named = "User-agent: handbook-crawl/0.1\nDisallow: /inner\n";
        assert!(!parse_robots(named, "/inner/page").allowed);
        // Crawl-delay parsed and rounded up.
        let delay = "User-agent: *\nCrawl-delay: 2.5\nDisallow: /x\n";
        let verdict = parse_robots(delay, "/ok");
        assert!(verdict.allowed);
        assert_eq!(verdict.crawl_delay_secs, 3);
        // HCCH-style delay.
        let hcch = "User-agent: *\nDisallow: /admin/\nCrawl-delay: 10\n";
        assert_eq!(parse_robots(hcch, "/en/page").crawl_delay_secs, 10);
    }

    #[test]
    fn writer_reader_roundtrip() {
        let dir = std::env::temp_dir().join(format!("warc-test-{}", std::process::id()));
        let path = dir.join("roundtrip.warc.gz");
        let mut writer = WarcWriter::create(&path).expect("create");
        writer
            .write_warcinfo(&[("filename", "roundtrip.warc.gz")])
            .expect("warcinfo");
        let req = writer
            .write_request("http://127.0.0.1:9/", b"GET / HTTP/1.1\r\n\r\n", None)
            .expect("request");
        let body = b"hello archive";
        let mut response_bytes = b"HTTP/1.1 200 OK\r\nContent-Length: 13\r\n\r\n".to_vec();
        response_bytes.extend_from_slice(body);
        let res = writer
            .write_response("http://127.0.0.1:9/", &response_bytes, body, Some(&req.id))
            .expect("response");
        writer
            .write_metadata(
                "http://127.0.0.1:9/",
                &[("slug", "xx-ca-1"), ("pack", "XX"), ("robots", "allowed")],
                Some(&res.id),
            )
            .expect("metadata");
        let stats = writer.finish().expect("finish");
        assert_eq!(stats.records, 4);

        let records = read_records(&path).expect("read back");
        assert_eq!(records.len(), 4);
        let types: Vec<&str> = records
            .iter()
            .map(|r| r.header("WARC-Type").unwrap_or("?"))
            .collect();
        assert_eq!(types, vec!["warcinfo", "request", "response", "metadata"]);
        for record in &records {
            assert_eq!(record.version, "WARC/1.1");
            assert!(record.block_digest_ok(), "digest: {record:?}");
            assert!(record.header("WARC-Record-ID").is_some());
            assert!(record.header("WARC-Date").is_some());
        }
        assert_eq!(
            records[2].header("WARC-Payload-Digest"),
            Some(sha1_base32(body).as_str())
        );
        assert_eq!(
            records[2].header("WARC-Concurrent-To"),
            Some(req.id.as_str())
        );
        assert_eq!(
            records[3].header("WARC-Concurrent-To"),
            Some(res.id.as_str())
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn reader_rejects_garbage() {
        assert!(parse_records(b"not a warc file").is_err());
        assert!(parse_records(b"WARC/1.1\r\nNo-Colon\r\n\r\n").is_err());
    }

    #[test]
    fn fetch_follows_redirect_hops() {
        let final_body = b"<html>authority</html>";
        let replies = vec![
            reply("HTTP/1.0 302 Found", &[("Location", "/final")], b""),
            reply(
                "HTTP/1.0 200 OK",
                &[("Content-Type", "text/html")],
                final_body,
            ),
        ];
        let (base, server) = serve_loop(replies, 2);
        let result = fetch_with_hops(&base, Duration::from_secs(5)).expect("fetch");
        assert_eq!(result.hops.len(), 2);
        assert_eq!(result.hops[0].status, 302);
        assert_eq!(result.hops[1].status, 200);
        assert_eq!(result.hops[1].body, final_body);
        assert_eq!(result.hops[1].url, format!("{base}final"));
        let request = String::from_utf8_lossy(&result.hops[0].request_bytes);
        assert!(request.starts_with("GET / HTTP/1.1"), "{request}");
        assert!(request.contains(CRAWL_USER_AGENT), "{request}");
        assert_eq!(result.tls_note, "tls-not-applicable-http");
        server.join().unwrap();
    }

    #[test]
    fn fetch_honours_retry_after_once() {
        let replies = vec![
            reply("HTTP/1.0 429 Too Many", &[("Retry-After", "0")], b"slow"),
            reply("HTTP/1.0 200 OK", &[], b"recovered"),
        ];
        let (base, server) = serve_loop(replies, 2);
        let result = fetch_with_hops(&base, Duration::from_secs(5)).expect("fetch");
        assert_eq!(result.hops.len(), 1);
        assert_eq!(result.hops[0].status, 200);
        assert_eq!(result.hops[0].body, b"recovered");
        server.join().unwrap();
    }

    #[test]
    fn fetch_rejects_declared_oversize() {
        let mut head =
            b"HTTP/1.0 200 OK\r\nContent-Length: 999999999\r\nConnection: close\r\n\r\n".to_vec();
        head.extend_from_slice(b"tiny");
        let (base, server) = serve_loop(vec![head], 1);
        let err = fetch_with_hops(&base, Duration::from_secs(5)).expect_err("oversize");
        assert!(matches!(err, FetchError::TooLarge(_)), "got: {err}");
        server.join().unwrap();
    }

    #[test]
    fn robots_gate_over_loopback() {
        let (base, server) = serve_loop(
            vec![reply(
                "HTTP/1.0 200 OK",
                &[],
                b"User-agent: *\nDisallow: /\n",
            )],
            1,
        );
        let verdict = check_robots_closed(&base, Duration::from_secs(5));
        assert!(!verdict.allowed, "got: {verdict:?}");
        assert_eq!(verdict.text, "disallowed");
        server.join().unwrap();

        let (open_base, open_server) =
            serve_loop(vec![reply("HTTP/1.0 404 Missing", &[], b"nope")], 1);
        let verdict = check_robots_closed(&open_base, Duration::from_secs(5));
        assert!(verdict.allowed, "got: {verdict:?}");
        assert_eq!(verdict.text, "no-robots");
        open_server.join().unwrap();

        let probe = TcpListener::bind("127.0.0.1:0").expect("bind probe");
        let port = probe.local_addr().unwrap().port();
        drop(probe);
        let verdict =
            check_robots_closed(&format!("http://127.0.0.1:{port}/"), Duration::from_secs(2));
        assert!(!verdict.allowed, "got: {verdict:?}");
        assert_eq!(verdict.text, "robots-unreachable-closed");
    }

    #[test]
    fn tls_fingerprint_shape() {
        let fp = tls_fingerprint(b"fake-der-bytes");
        assert!(fp.starts_with("sha1:"), "{fp}");
        assert_eq!(fp.len(), 5 + 32);
    }

    #[test]
    fn link_extraction_forms() {
        let base = "https://example.gov/dir/page.html";
        let html = br##"<html><body>
            <a HREF="https://example.gov/apostille-fees">fees</a>
            <a href='/legalization'>leg</a>
            <a href="sibling-form">form</a>
            <a href="https://other.gov/x">offsite</a>
            <a href="#frag">frag</a>
            <a href="mailto:a@b.c">mail</a>
            <a href="https://example.gov/apostille-fees">dup</a>
        </body></html>"##;
        let links = extract_links(html, base);
        assert_eq!(
            links,
            vec![
                "https://example.gov/apostille-fees".to_string(),
                "https://example.gov/legalization".to_string(),
                "https://example.gov/dir/sibling-form".to_string(),
                "https://other.gov/x".to_string(),
            ]
        );
    }
}
