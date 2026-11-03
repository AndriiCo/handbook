//! WARC/1.1 archive writer plus the polite-fetch core for the world crawl.
//!
//! The world crawler (`xtask crawl`, SA-8 slice of the operator-ordered
//! archive addendum, `docs/ops/world-archive-rights.md`) stores every capture as WARC/1.1
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
//! - SSRF guard on every hop: the host is resolved and every returned
//!   address is rejected when it is loopback, RFC 1918, link-local,
//!   CGNAT, ULA, or otherwise special (see [`ip_blocked`]); the fetch
//!   connects to a vetted address directly, so the resolved IP is pinned
//!   for the connection (no TOCTOU between check and connect);
//! - HTTP/1.1 is spoken over a manually-managed socket (plain or rustls)
//!   so the archived bytes are wire-faithful: the `request` record holds
//!   the exact bytes written, `Accept-Encoding: identity` is sent and no
//!   transparent decoding ever applies, and `Transfer-Encoding` is
//!   dropped from a recorded head whose body was de-chunked;
//! - robots.txt is judged per RFC 9309 (most-specific group, `*` / `$`
//!   path wildcards) and checks FAIL CLOSED: an unreachable or
//!   unparseable robots.txt (or URL) disallows the fetch. This is
//!   stricter than the thin watch (`fetch::check_robots`, which fails
//!   open) and is required by the world-archive rights review
//!   (`docs/ops/world-archive-rights.md`).
//!
//! Loopback fixtures: production crawling always denies private
//! addresses, but the unit tests and the `xtask crawl --allow-loopback`
//! fixture path run against `127.0.0.1` servers. The `allow_loopback`
//! switch exists for exactly that case and is never set by production
//! flags.

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

/// Largest HTTP response head kept (status line + headers).
pub const MAX_HEAD_BYTES: u64 = 64 * 1024;

/// Redirect hops followed while fetching `/robots.txt` itself.
pub const MAX_ROBOTS_HOPS: usize = 3;

/// Product token matched against robots.txt `User-agent` lines
/// (case-insensitive; the line may carry a version suffix).
pub const CRAWLER_PRODUCT_TOKEN: &str = "handbook-crawl";

/// `sha1:<BASE32>` digest of `data` (RFC 4648 base32, no padding), the
/// digest format used for `WARC-Block-Digest` and `WARC-Payload-Digest`.
pub fn sha1_base32(data: &[u8]) -> String {
    use sha1::Digest as _;
    let digest = sha1::Sha1::digest(data);
    format!("sha1:{}", data_encoding::BASE32_NOPAD.encode(&digest))
}

/// New random `WARC-Record-ID` value (`<urn:uuid:...>` with UUID v4 bits).
///
/// The 128 bits come from SHA-1 over wall-clock nanos, the process id,
/// and an atomic counter — no extra RNG dependency, unique within and
/// across runs for any realistic crawl. The version nibble is forced to
/// `4` and the variant bits to `10xx` (RFC 4122 §4.1), so validators
/// reading the UUID version/variant see v4 even though the entropy is
/// hash-derived rather than from an RNG.
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
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let mut hex = String::with_capacity(32);
    for byte in bytes {
        hex.push_str(&format!("{byte:02x}"));
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
    /// Host for display/dial contexts: bracketed when an IPv6 literal.
    pub fn display_host(&self) -> String {
        if self.host.contains(':') {
            format!("[{}]", self.host)
        } else {
            self.host.clone()
        }
    }

    /// `scheme://host[:port]` origin prefix (for robots.txt URLs and
    /// cross-origin redirect decisions; IPv6 literals stay bracketed).
    pub fn origin(&self) -> String {
        if (self.scheme == "https" && self.port == 443)
            || (self.scheme == "http" && self.port == 80)
        {
            format!("{}://{}", self.scheme, self.display_host())
        } else {
            format!("{}://{}:{}", self.scheme, self.display_host(), self.port)
        }
    }

    /// True for the default port of the scheme (the `Host` header then
    /// carries the bare host; otherwise `host:port`).
    pub fn default_port(&self) -> bool {
        (self.scheme == "https" && self.port == 443) || (self.scheme == "http" && self.port == 80)
    }
}

/// Parse an absolute HTTP(S) URL. See [`UrlParts`] for the contract.
///
/// IPv6 literals must be bracketed (`http://[::1]:8080/path`); the
/// brackets are stripped from [`UrlParts::host`].
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
    let default_port = if scheme == "https" { 443 } else { 80 };
    let (host, port) = if let Some(bracketed) = authority.strip_prefix('[') {
        let end = bracketed.find(']')?;
        let host = &bracketed[..end];
        if host.is_empty() {
            return None;
        }
        let port = match bracketed[end + 1..].strip_prefix(':') {
            Some(p) if !p.is_empty() => p.parse::<u16>().ok()?,
            Some(_) => return None,
            None if bracketed.len() == end + 1 => default_port,
            None => return None,
        };
        (host.to_ascii_lowercase(), port)
    } else {
        match authority.rsplit_once(':') {
            Some((h, p)) if !h.is_empty() => match p.parse::<u16>() {
                Ok(port) => (h.to_ascii_lowercase(), port),
                Err(_) => return None,
            },
            _ => (authority.to_ascii_lowercase(), default_port),
        }
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

/// Pointer to a record just written (for `Concurrent-To` linking and
/// for the per-URL manifest / CDXJ offset + length columns).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordRef {
    /// The `WARC-Record-ID` value, including angle brackets.
    pub id: String,
    /// The `WARC-Date` value.
    pub date: String,
    /// Compressed-file offset of the record's gzip member.
    pub offset: u64,
    /// Compressed-file length of the record's gzip member.
    pub length: u64,
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
        let offset = self.bytes;
        self.out.write_all(&member)?;
        self.records += 1;
        self.bytes += member.len() as u64;
        Ok(RecordRef {
            id,
            date,
            offset,
            length: member.len() as u64,
        })
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

/// A WARC record with its position in the stored file, for the
/// per-URL manifest (`warc_offset`/`warc_length`) and the CDXJ
/// `offset`/`length` fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocatedRecord {
    /// The parsed record.
    pub record: WarcRecord,
    /// Compressed-file offset of the record's gzip member (or the
    /// record itself in a plain `.warc` file).
    pub offset: u64,
    /// Compressed-file length of the record's gzip member (or the
    /// record itself in a plain `.warc` file).
    pub length: u64,
}

/// Read back every record with its stored-file position.
///
/// Gzip members are walked one by one (tracking compressed offsets),
/// so each record of our per-record-gzip files points at its own
/// member. A member holding several records (some operator wget drops
/// do this) attributes the whole member span to each of its records.
/// Plain (uncompressed) `.warc` files report raw byte positions.
pub fn read_located_records(path: &Path) -> io::Result<Vec<LocatedRecord>> {
    let raw = std::fs::read(path)?;
    if raw.len() >= 2 && raw[0] == 0x1f && raw[1] == 0x8b {
        let mut out = Vec::new();
        let mut pos = 0u64;
        for (member, decompressed) in split_gzip_members(&raw)? {
            let records = parse_records(&decompressed)?;
            for record in records {
                out.push(LocatedRecord {
                    record,
                    offset: pos,
                    length: member as u64,
                });
            }
            pos += member as u64;
        }
        Ok(out)
    } else {
        // Plain WARC (or empty): parse with raw offsets.
        let mut out = Vec::new();
        for (record, start, end) in parse_records_spans(&raw)? {
            out.push(LocatedRecord {
                record,
                offset: start as u64,
                length: (end - start) as u64,
            });
        }
        Ok(out)
    }
}

/// Split concatenated gzip members: `(compressed_len, decompressed)`.
///
/// Member boundaries are walked exactly: the gzip header is parsed by
/// hand (a buffered `GzDecoder` over-reads past the member end, which
/// would lose the offsets), the raw deflate stream is decoded with
/// `DeflateDecoder` (whose `total_in` is exact), and the trailer's
/// CRC-32 + ISIZE are verified against the output. Any mismatch is an
/// error — a boundary attributed to the wrong bytes would corrupt
/// every later CDXJ offset.
fn split_gzip_members(raw: &[u8]) -> io::Result<Vec<(usize, Vec<u8>)>> {
    let mut members = Vec::new();
    let mut pos = 0usize;
    while pos < raw.len() {
        let header_len = gzip_header_len(&raw[pos..])?;
        let stream = &raw[pos + header_len..];
        let mut decoder = flate2::read::DeflateDecoder::new(stream);
        let mut decompressed = Vec::new();
        decoder
            .read_to_end(&mut decompressed)
            .map_err(|e| invalid(format!("deflate error at offset {pos}: {e}")))?;
        let trailer_at = pos + header_len + decoder.total_in() as usize;
        if raw.len() < trailer_at + 8 {
            return Err(invalid(format!("short gzip trailer at offset {pos}")));
        }
        let crc_expect =
            u32::from_le_bytes(raw[trailer_at..trailer_at + 4].try_into().expect("trailer"));
        let isize_expect = u32::from_le_bytes(
            raw[trailer_at + 4..trailer_at + 8]
                .try_into()
                .expect("trailer"),
        );
        if isize_expect != decompressed.len() as u32 {
            return Err(invalid(format!("gzip length mismatch at offset {pos}")));
        }
        let mut crc = flate2::Crc::new();
        crc.update(&decompressed);
        if crc.sum() != crc_expect {
            return Err(invalid(format!("gzip crc mismatch at offset {pos}")));
        }
        let member_len = header_len + decoder.total_in() as usize + 8;
        members.push((member_len, decompressed));
        pos += member_len;
    }
    Ok(members)
}

/// Length of the gzip member header starting at `data` (RFC 1952:
/// magic + method + flags + mtime/xfl/os + optional extra, name,
/// comment, header-CRC).
fn gzip_header_len(data: &[u8]) -> io::Result<usize> {
    if data.len() < 10 || data[0] != 0x1f || data[1] != 0x8b || data[2] != 8 {
        return Err(invalid("bad gzip header"));
    }
    let flags = data[3];
    if flags & 0xe0 != 0 {
        return Err(invalid("reserved gzip flags set"));
    }
    let mut pos = 10usize;
    if flags & 0x04 != 0 {
        if data.len() < pos + 2 {
            return Err(invalid("short gzip extra length"));
        }
        let extra = u16::from_le_bytes([data[pos], data[pos + 1]]) as usize;
        pos += 2 + extra;
    }
    for bit in [0x08, 0x10] {
        if flags & bit != 0 {
            match data
                .get(pos..)
                .and_then(|tail| tail.iter().position(|&b| b == 0))
            {
                Some(end) => pos += end + 1,
                None => return Err(invalid("unterminated gzip string")),
            }
        }
    }
    if flags & 0x02 != 0 {
        pos += 2;
    }
    if data.len() < pos {
        return Err(invalid("short gzip header"));
    }
    Ok(pos)
}

/// SURT (Sort-friendly URI Reordering Transform) key for a URL, the
/// canonical CDXJ key: reversed comma-joined host labels, non-default
/// port attached, then `)/path?query`.
///
/// Examples: `https://example.gov/a?b=c` → `gov,example)/a?b=c`;
/// `http://example.gov:8080/x` → `gov,example:8080)/x`. IPv6 literals
/// keep their whole host (`::1)/x`); unparseable URLs yield `None`.
pub fn surt_key(url: &str) -> Option<String> {
    let parts = split_url(url)?;
    let host_part = if parts.host.contains(':') {
        // IPv6 literal: not dot-reversible; keep it whole.
        parts.host.clone()
    } else {
        let mut labels: Vec<&str> = parts.host.split('.').collect();
        labels.reverse();
        labels.join(",")
    };
    let host_part = if parts.default_port() {
        host_part
    } else {
        format!("{host_part}:{}", parts.port)
    };
    let path = parts.path.trim_start_matches('/');
    Some(format!("{host_part})/{path}"))
}

/// Parse concatenated WARC records from decompressed bytes.
fn parse_records(raw: &[u8]) -> io::Result<Vec<WarcRecord>> {
    parse_records_spans(raw).map(|spans| spans.into_iter().map(|(record, _, _)| record).collect())
}

/// Parse concatenated WARC records with `(record, block_start_offset,
/// record_end_offset)` spans relative to the input start.
fn parse_records_spans(raw: &[u8]) -> io::Result<Vec<(WarcRecord, usize, usize)>> {
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
        let record_start = pos;
        records.push((
            WarcRecord {
                version: version.to_string(),
                headers,
                block: raw[block_start..block_end].to_vec(),
            },
            record_start,
            block_end + 4,
        ));
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
/// Production-safe default: private/loopback addresses are rejected.
/// Fixture and unit-test callers that crawl loopback servers use
/// [`check_robots_closed_loopback`] instead.
///
/// Differences from the thin watch gate (`fetch::check_robots`, which
/// fails open): any fetch error, non-2xx/non-404 status, or
/// unparseable URL disallows the fetch, and RFC 9309 rules of the
/// most-specific matching group apply (longest match wins, ties go to
/// `Allow`). A 404 or empty body means "no robots" and allows the
/// fetch.
pub fn check_robots_closed(url: &str, timeout: Duration) -> RobotsVerdict {
    check_robots_closed_opt(url, timeout, false)
}

/// [`check_robots_closed`] with loopback allowed (fixture/test crawls
/// against `127.0.0.1` servers only; never used by production flags).
pub fn check_robots_closed_loopback(url: &str, timeout: Duration) -> RobotsVerdict {
    check_robots_closed_opt(url, timeout, true)
}

fn check_robots_closed_opt(url: &str, timeout: Duration, allow_loopback: bool) -> RobotsVerdict {
    let Some(parts) = split_url(url) else {
        return RobotsVerdict {
            allowed: false,
            text: "unparseable-uri-closed".to_string(),
            crawl_delay_secs: 0,
        };
    };
    match fetch_robots_body(&parts.origin(), timeout, allow_loopback) {
        Err(text) => RobotsVerdict {
            allowed: false,
            text,
            crawl_delay_secs: 0,
        },
        Ok(None) => RobotsVerdict {
            allowed: true,
            text: "no-robots".to_string(),
            crawl_delay_secs: 0,
        },
        Ok(Some(body)) => parse_robots(&body, &parts.path),
    }
}

/// Raw `robots.txt` body for `origin` (`scheme://host[:port]`).
///
/// Follows up to [`MAX_ROBOTS_HOPS`] redirects with the SSRF guard on
/// every hop (a robots URL on a blocked address fails closed).
/// Returns `Ok(None)` for 404/410/empty ("no robots"); any transport
/// failure, SSRF refusal, or other non-2xx status is `Err` with a
/// `robots-*-closed` reason.
pub fn fetch_robots_body(
    origin: &str,
    timeout: Duration,
    allow_loopback: bool,
) -> Result<Option<String>, String> {
    let mut url = format!("{origin}/robots.txt");
    for _ in 0..=MAX_ROBOTS_HOPS {
        let response = match http_get_single(&url, timeout, allow_loopback, false) {
            Err(FetchError::SsrfBlocked(note)) => {
                return Err(format!("robots-unreachable-closed:{note}"));
            }
            Err(_) => return Err("robots-unreachable-closed".to_string()),
            Ok(response) => response,
        };
        if matches!(response.status, 301 | 302 | 303 | 307 | 308)
            && let Some(location) = response.header("location")
            && let Some(next) = resolve_redirect(&url, location)
        {
            url = next;
            continue;
        }
        // Status-as-success: 404/410 is data ("no robots"), not a
        // transport failure. Anything else non-2xx still fails closed.
        if response.status == 404 || response.status == 410 {
            return Ok(None);
        }
        if !(200..300).contains(&response.status) {
            return Err(format!("robots-status-closed:{}", response.status));
        }
        let body = String::from_utf8_lossy(&response.body).into_owned();
        if body.trim().is_empty() {
            return Ok(None);
        }
        return Ok(Some(body));
    }
    Err("robots-unreachable-closed:too-many-redirects".to_string())
}

/// True when `ip` must never be dialled (with `allow_loopback` open
/// only for `127.0.0.0/8` + `::1` fixture servers).
///
/// Rejected classes: loopback, RFC 1918, link-local (covers the
/// `169.254.169.254` cloud-metadata address), CGNAT `100.64.0.0/10`
/// (covers `100.100.100.200`), ULA, plus — defence in depth —
/// unspecified, broadcast, multicast, reserved, documentation,
/// benchmarking, and protocol-assignment ranges. IPv4-mapped IPv6
/// addresses are judged as their embedded IPv4 address; other
/// IPv4-embedded forms (6to4, Teredo) are rejected outright.
pub fn ip_blocked(ip: &std::net::IpAddr, allow_loopback: bool) -> bool {
    use std::net::IpAddr;
    match *ip {
        IpAddr::V4(v4) => ipv4_blocked(v4, allow_loopback),
        IpAddr::V6(v6) => {
            if let Some(mapped) = v6.to_ipv4_mapped() {
                return ipv4_blocked(mapped, allow_loopback);
            }
            ipv6_blocked(v6, allow_loopback)
        }
    }
}

fn ipv4_blocked(ip: std::net::Ipv4Addr, allow_loopback: bool) -> bool {
    let o = ip.octets();
    // Loopback 127.0.0.0/8 (the fixture switch re-opens it).
    if o[0] == 127 {
        return !allow_loopback;
    }
    // RFC 1918 private: 10/8, 172.16/12, 192.168/16.
    if o[0] == 10 || (o[0] == 172 && o[1] & 0xf0 == 16) || (o[0] == 192 && o[1] == 168) {
        return true;
    }
    // Link-local 169.254.0.0/16 (incl. the cloud metadata services at
    // 169.254.169.254).
    if o[0] == 169 && o[1] == 254 {
        return true;
    }
    // Shared-address space (CGNAT) 100.64.0.0/10 (incl. 100.100.100.200).
    if o[0] == 100 && o[1] & 0xc0 == 64 {
        return true;
    }
    // This-network 0/8 and broadcast 255.255.255.255.
    if o[0] == 0 || ip == std::net::Ipv4Addr::BROADCAST {
        return true;
    }
    // IETF protocol assignments 192.0.0.0/24, documentation
    // 192.0.2.0/24 + 198.51.100.0/24 + 203.0.113.0/24, 6to4-relay
    // 192.88.99.0/24, benchmarking 192.18.0.0/15 + 198.18.0.0/15.
    if (o[0] == 192 && o[1] == 0 && (o[2] == 0 || o[2] == 2))
        || (o[0] == 192 && o[1] == 88 && o[2] == 99)
        || (o[0] == 192 && (o[1] == 18 || o[1] == 19))
        || (o[0] == 198 && (o[1] == 18 || o[1] == 19))
        || (o[0] == 198 && o[1] == 51 && o[2] == 100)
        || (o[0] == 203 && o[1] == 0 && o[2] == 113)
    {
        return true;
    }
    // Multicast 224.0.0.0/4 and reserved 240.0.0.0/4.
    if o[0] >= 224 {
        return true;
    }
    false
}

fn ipv6_blocked(ip: std::net::Ipv6Addr, allow_loopback: bool) -> bool {
    if ip.is_loopback() {
        return !allow_loopback;
    }
    if ip.is_unspecified() {
        return true;
    }
    let seg = ip.segments();
    // ULA fc00::/7.
    if seg[0] & 0xfe00 == 0xfc00 {
        return true;
    }
    // Link-local fe80::/10.
    if seg[0] & 0xffc0 == 0xfe80 {
        return true;
    }
    // Multicast ff00::/8.
    if seg[0] & 0xff00 == 0xff00 {
        return true;
    }
    // Documentation 2001:db8::/32, Teredo 2001::/32, 6to4 2002::/16,
    // NAT64 64:ff9b::/96, discard 100::/64.
    if (seg[0] == 0x2001 && (seg[1] == 0 || seg[1] == 0xdb8))
        || seg[0] == 0x2002
        || (seg[0] == 0x64 && seg[1] == 0xff9b)
        || seg[0] == 0x100
    {
        return true;
    }
    false
}

/// Resolve `host:port` and keep only diallable addresses.
///
/// Every returned address passed [`ip_blocked`]; an empty result (DNS
/// failure, no addresses, or all blocked) is `Err`. Callers connect
/// to one of the returned addresses directly, so the vetted IP is
/// pinned for the connection.
pub fn resolve_guarded(
    host: &str,
    port: u16,
    allow_loopback: bool,
) -> Result<Vec<std::net::SocketAddr>, String> {
    use std::net::ToSocketAddrs as _;
    let dial = if host.contains(':') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    };
    let addrs: Vec<std::net::SocketAddr> = dial
        .to_socket_addrs()
        .map_err(|e| format!("dns: {e}"))?
        .collect();
    if addrs.is_empty() {
        return Err("dns: no address".to_string());
    }
    let mut allowed = Vec::new();
    let mut blocked = 0u32;
    for addr in addrs {
        if ip_blocked(&addr.ip(), allow_loopback) {
            blocked += 1;
        } else {
            allowed.push(addr);
        }
    }
    if allowed.is_empty() {
        return Err(format!(
            "ssrf-blocked: {blocked} address(es) rejected for {host}"
        ));
    }
    Ok(allowed)
}

/// Judge `path` against a robots.txt `body` (pure, no I/O), per RFC 9309.
///
/// Group selection (§2.2.2): consecutive `User-agent` lines form one
/// group; the crawler uses the single group with the most specific
/// matching line — our product token ([`CRAWLER_PRODUCT_TOKEN`],
/// case-insensitive, optional `/version` suffix) beats `*`, longer
/// matching lines beat shorter ones. A body with no matching group
/// allows everything. Rule lines before the first `User-agent` line
/// are ignored.
///
/// Path rules (§2.2.3–§2.2.4): `Allow` / `Disallow` values support `*`
/// (any run, including `/` and the empty run) and a trailing `$`
/// (end-of-path anchor); the longest matching rule wins, ties go to
/// `Allow`, and an empty `Disallow` value matches nothing. `Crawl-delay`
/// of the selected group is reported (fractional values round up,
/// capped at one hour). Empty bodies allow everything.
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
        /// Best user-agent specificity (0 = `*`; absent = no match).
        specificity: Option<usize>,
        /// `true` once a rule line closed the user-agent preamble, so a
        /// later `User-agent` line starts a new group.
        closed: bool,
        best_allow: usize,
        best_disallow: usize,
        delay: Option<u64>,
    }
    let mut groups: Vec<Group> = Vec::new();
    for raw_line in body.lines() {
        let line = raw_line.split('#').next().unwrap_or("").trim();
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let (key, value) = (key.trim().to_ascii_lowercase(), value.trim());
        match key.as_str() {
            "user-agent" => {
                let specificity = ua_specificity(value);
                let start_new = match groups.last() {
                    None => true,
                    Some(last) => last.closed,
                };
                if start_new {
                    groups.push(Group {
                        specificity,
                        ..Group::default()
                    });
                } else if let Some(best) = specificity {
                    let last = groups.last_mut().expect("pushed group");
                    last.specificity = Some(last.specificity.unwrap_or(0).max(best));
                }
            }
            "allow" | "disallow" => {
                let Some(group) = groups.last_mut() else {
                    continue;
                };
                group.closed = true;
                let Some(_) = group.specificity else {
                    continue;
                };
                if value.is_empty() {
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
                let Some(group) = groups.last_mut() else {
                    continue;
                };
                group.closed = true;
                if group.specificity.is_none() || group.delay.is_some() {
                    continue;
                }
                group.delay = value
                    .parse::<f64>()
                    .ok()
                    .filter(|d| d.is_finite() && *d >= 0.0)
                    .map(|d| d.ceil().clamp(0.0, 3_600.0) as u64);
            }
            _ => {}
        }
    }
    // RFC 9309 §2.2.2: the most specific matching group alone applies.
    let selected = groups
        .iter()
        .filter(|g| g.specificity.is_some())
        .max_by_key(|g| g.specificity);
    let Some(group) = selected else {
        return RobotsVerdict {
            allowed: true,
            text: "allowed".to_string(),
            crawl_delay_secs: 0,
        };
    };
    if group.best_disallow > group.best_allow {
        RobotsVerdict {
            allowed: false,
            text: "disallowed".to_string(),
            crawl_delay_secs: group.delay.unwrap_or(0),
        }
    } else {
        RobotsVerdict {
            allowed: true,
            text: "allowed".to_string(),
            crawl_delay_secs: group.delay.unwrap_or(0),
        }
    }
}

/// Specificity of a robots.txt `User-agent` line for our crawler
/// (`None` = does not match): `*` scores 0, our product token
/// (bare or with a `/version` suffix) scores by line length so the
/// most specific matching group wins per RFC 9309 §2.2.2.
fn ua_specificity(line: &str) -> Option<usize> {
    let token = line.trim().to_ascii_lowercase();
    if token == "*" {
        return Some(0);
    }
    // The line may carry a version (`handbook-crawl/0.1`) or a comment
    // tail; the product token is the first whitespace-separated word.
    let first = token.split_whitespace().next().unwrap_or("");
    if first == CRAWLER_PRODUCT_TOKEN || first.starts_with(&format!("{CRAWLER_PRODUCT_TOKEN}/")) {
        Some(1000 + line.trim().len())
    } else {
        None
    }
}

/// True when a robots path rule matches `path` (RFC 9309 §2.2.3):
/// `*` spans any run (including `/` and empty), a trailing `$`
/// anchors the match at end-of-path, and matching is case-sensitive.
/// Without `$` the rule need only match a path prefix.
fn rule_matches(rule: &str, path: &str) -> bool {
    let (pattern, anchored) = match rule.strip_suffix('$') {
        Some(prefix) => (prefix, true),
        None => (rule, false),
    };
    if anchored {
        glob_match_full(pattern.as_bytes(), path.as_bytes())
    } else {
        glob_match_prefix(pattern.as_bytes(), path.as_bytes())
    }
}

/// True when `pattern` (`*` = any run) matches a prefix of `text`.
fn glob_match_prefix(pattern: &[u8], text: &[u8]) -> bool {
    let (mut pi, mut ti) = (0usize, 0usize);
    let (mut star, mut mark) = (None, 0usize);
    while ti < text.len() {
        // A consumed pattern already matches the prefix ending here.
        if pi == pattern.len() {
            return true;
        }
        if pattern[pi] == b'*' {
            star = Some(pi);
            pi += 1;
            mark = ti;
        } else if pattern[pi] == text[ti] {
            pi += 1;
            ti += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            mark += 1;
            ti = mark;
        } else {
            return false;
        }
    }
    while pi < pattern.len() && pattern[pi] == b'*' {
        pi += 1;
    }
    pi == pattern.len()
}

/// True when `pattern` (`*` = any run) matches all of `text`.
fn glob_match_full(pattern: &[u8], text: &[u8]) -> bool {
    let (mut pi, mut ti) = (0usize, 0usize);
    let (mut star, mut mark) = (None, 0usize);
    while ti < text.len() {
        if pi < pattern.len() && pattern[pi] == b'*' {
            star = Some(pi);
            pi += 1;
            mark = ti;
        } else if pi < pattern.len() && pattern[pi] == text[ti] {
            pi += 1;
            ti += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            mark += 1;
            ti = mark;
        } else {
            return false;
        }
    }
    while pi < pattern.len() && pattern[pi] == b'*' {
        pi += 1;
    }
    pi == pattern.len() && ti == text.len()
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
    /// The resolved address was rejected by the SSRF guard.
    SsrfBlocked(String),
    /// Body (or declared `Content-Length`) exceeded [`MAX_BODY_BYTES`].
    TooLarge(u64),
    /// Redirect chain exhausted without a final response.
    TooManyRedirects,
    /// A cross-origin redirect failed its robots re-check.
    RedirectDisallowed(String),
}

impl std::fmt::Display for FetchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FetchError::Transport(message) => write!(f, "fetch transport error: {message}"),
            FetchError::SsrfBlocked(note) => write!(f, "fetch SSRF-blocked: {note}"),
            FetchError::TooLarge(len) => write!(f, "fetch body exceeds cap ({len} bytes)"),
            FetchError::TooManyRedirects => {
                write!(f, "fetch exceeded {MAX_REDIRECT_HOPS} redirect hops")
            }
            FetchError::RedirectDisallowed(note) => {
                write!(f, "fetch redirect disallowed: {note}")
            }
        }
    }
}

impl std::error::Error for FetchError {}

/// GET `start_url` with manual redirect following.
///
/// Production-safe default: private/loopback addresses are rejected.
/// Fixture and unit-test callers that crawl loopback servers use
/// [`fetch_with_hops_loopback`] instead.
///
/// Returns one [`HopCapture`] per hop. The `request` record holds the
/// exact bytes written to the socket (`GET path HTTP/1.1` + `Host`
/// with port when non-default + crawler `User-Agent` +
/// `Accept-Encoding: identity`, so no transparent decoding ever
/// applies). The recorded response head is the exact status line and
/// headers received, except that `Transfer-Encoding` is dropped when
/// the body was de-chunked. On HTTP 429 the `Retry-After` value
/// (seconds form, capped at [`MAX_RETRY_AFTER_SECS`], default 1 s) is
/// honoured once per URL; a second 429 is returned as the final hop.
/// Bodies larger than [`MAX_BODY_BYTES`] abort with
/// [`FetchError::TooLarge`].
///
/// The TLS peer chain is captured from the final hop's own handshake
/// (HTTPS only; plain HTTP reports `tls-not-applicable-http`).
///
/// # Errors
///
/// [`FetchError::Transport`] on connection/TLS/timeout failure,
/// [`FetchError::SsrfBlocked`] when every resolved address is denied,
/// [`FetchError::TooLarge`] over the body cap,
/// [`FetchError::TooManyRedirects`] when every hop redirects.
pub fn fetch_with_hops(start_url: &str, timeout: Duration) -> Result<FetchResult, FetchError> {
    fetch_with_hops_opt(start_url, timeout, false, None)
}

/// [`fetch_with_hops`] with loopback allowed (fixture/test crawls
/// against `127.0.0.1` servers only; never used by production flags).
pub fn fetch_with_hops_loopback(
    start_url: &str,
    timeout: Duration,
) -> Result<FetchResult, FetchError> {
    fetch_with_hops_opt(start_url, timeout, true, None)
}

/// Cross-origin redirect hook: runs before following a redirect to
/// a NEW ORIGIN (the caller's robots + politeness re-check lives
/// there); returning `Err` aborts the fetch with that error.
pub type RedirectHook<'a> = Option<&'a mut dyn FnMut(&str) -> Result<(), FetchError>>;

/// Full [`fetch_with_hops`] variant: `on_redirect` runs before
/// following a redirect to a NEW ORIGIN (the caller's robots +
/// politeness re-check lives there); returning `Err` aborts the fetch
/// with that error. Same-origin redirects never invoke the hook.
pub fn fetch_with_hops_opt(
    start_url: &str,
    timeout: Duration,
    allow_loopback: bool,
    on_redirect: RedirectHook<'_>,
) -> Result<FetchResult, FetchError> {
    let mut on_redirect = on_redirect;
    let mut hops = Vec::new();
    let mut url = start_url.to_string();
    let mut tls: Option<(Vec<String>, String)>;
    loop {
        let origin = split_url(&url).map(|parts| parts.origin());
        let single = http_get_single(&url, timeout, allow_loopback, false)?;
        let redirect = matches!(single.status, 301 | 302 | 303 | 307 | 308);
        let location = single.header("location").map(str::to_string);
        tls = Some((single.tls_chain, single.tls_note));
        hops.push(HopCapture {
            url: url.clone(),
            request_bytes: single.request_bytes,
            status: single.status,
            response_head: single.head,
            body: single.body,
        });
        if !redirect {
            break;
        }
        if hops.len() > MAX_REDIRECT_HOPS {
            return Err(FetchError::TooManyRedirects);
        }
        match location.and_then(|loc| resolve_redirect(&url, &loc)) {
            Some(next) => {
                let new_origin = split_url(&next).map(|parts| parts.origin());
                if new_origin != origin
                    && let Some(hook) = on_redirect.as_deref_mut()
                {
                    hook(&next)?;
                }
                url = next;
            }
            None => break,
        }
    }
    // The loop body always runs at least once (a `?` return is the
    // only early exit), so the final hop's TLS state is present.
    let (tls_chain, tls_note) = tls.expect("fetch loop records one hop");
    Ok(FetchResult {
        hops,
        tls_chain,
        tls_note,
    })
}

/// One completed HTTP exchange (before hop bookkeeping).
struct SingleFetch {
    request_bytes: Vec<u8>,
    status: u16,
    /// Recorded head: the received status line + headers, minus
    /// `Transfer-Encoding` when the body was de-chunked.
    head: Vec<u8>,
    /// Headers as received (for `Location` / `Retry-After` lookups).
    headers: Vec<(String, String)>,
    body: Vec<u8>,
    tls_chain: Vec<String>,
    tls_note: String,
}

impl SingleFetch {
    /// First header value for `name` (case-insensitive).
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
}

/// Single GET with one 429/Retry-After retry when `retried` is false.
///
/// Every connection goes through [`resolve_guarded`]: the host is
/// resolved, blocked addresses are dropped, and the socket connects
/// to a vetted address directly (IP pinning — no DNS TOCTOU).
fn http_get_single(
    url: &str,
    timeout: Duration,
    allow_loopback: bool,
    retried: bool,
) -> Result<SingleFetch, FetchError> {
    let parts =
        split_url(url).ok_or_else(|| FetchError::Transport(format!("unparseable URL: {url}")))?;
    let request_bytes = build_request_bytes(&parts);
    let (mut stream, tls_chain, tls_note) = connect_guarded(&parts, timeout, allow_loopback)?;
    use std::io::Write as _;
    stream
        .write_all(&request_bytes)
        .map_err(|e| FetchError::Transport(format!("{url}: write request: {e}")))?;
    let (status_line, headers, mut reader) = read_head(&mut stream, url)?;
    let status = parse_status(&status_line)?;
    if status == 429 && !retried {
        let wait = retry_after_secs(
            headers
                .iter()
                .find(|(key, _)| key.eq_ignore_ascii_case("retry-after"))
                .map(|(_, value)| value.as_str()),
        )
        .min(MAX_RETRY_AFTER_SECS);
        std::thread::sleep(Duration::from_secs(wait.max(1)));
        return http_get_single(url, timeout, allow_loopback, true);
    }
    let chunked = headers.iter().any(|(key, value)| {
        key.eq_ignore_ascii_case("transfer-encoding")
            && value
                .split(',')
                .any(|token| token.trim().eq_ignore_ascii_case("chunked"))
    });
    let (body, dechunked) = read_body(&mut reader, &headers, url, chunked)?;
    let head = build_recorded_head(&status_line, &headers, dechunked);
    Ok(SingleFetch {
        request_bytes,
        status,
        head,
        headers,
        body,
        tls_chain,
        tls_note,
    })
}

/// Exact bytes written for a GET: `Host` carries `:port` whenever the
/// port is not the scheme default, and `Accept-Encoding: identity`
/// opts out of transparent content codings.
fn build_request_bytes(parts: &UrlParts) -> Vec<u8> {
    let host = if parts.default_port() {
        parts.display_host()
    } else {
        format!("{}:{}", parts.display_host(), parts.port)
    };
    format!(
        "GET {} HTTP/1.1\r\nHost: {host}\r\nUser-Agent: {CRAWL_USER_AGENT}\r\nAccept: */*\r\nAccept-Encoding: identity\r\nConnection: close\r\n\r\n",
        parts.path
    )
    .into_bytes()
}

/// `Retry-After` in seconds form (delta-seconds); anything else (HTTP
/// dates, garbage, missing) falls back to 1 s — the date form names an
/// absolute time, not a wait, and re-resolving clocks is out of scope.
fn retry_after_secs(value: Option<&str>) -> u64 {
    value
        .and_then(|v| v.trim().parse::<u64>().ok())
        .unwrap_or(1)
}

/// Plain or TLS fetch stream (one `Read` + `Write` interface so the
/// HTTP framing code is transport-agnostic). The TLS variant is boxed
/// to keep the enum small.
enum FetchStream {
    Plain(std::net::TcpStream),
    Tls(Box<rustls::StreamOwned<rustls::ClientConnection, std::net::TcpStream>>),
}

impl std::io::Read for FetchStream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            FetchStream::Plain(stream) => stream.read(buf),
            FetchStream::Tls(stream) => stream.read(buf),
        }
    }
}

impl std::io::Write for FetchStream {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            FetchStream::Plain(stream) => stream.write(buf),
            FetchStream::Tls(stream) => stream.write(buf),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            FetchStream::Plain(stream) => stream.flush(),
            FetchStream::Tls(stream) => stream.flush(),
        }
    }
}

/// Connect to `parts` through the SSRF guard, wrapping in TLS for
/// `https`. Returns the stream plus the peer chain (`tls-ok`) or the
/// plain-HTTP note; any failure is [`FetchError`].
fn connect_guarded(
    parts: &UrlParts,
    timeout: Duration,
    allow_loopback: bool,
) -> Result<(FetchStream, Vec<String>, String), FetchError> {
    install_crypto_provider();
    let addrs = match resolve_guarded(&parts.host, parts.port, allow_loopback) {
        Err(note) if note.starts_with("ssrf-blocked") => {
            return Err(FetchError::SsrfBlocked(note));
        }
        Err(note) => {
            return Err(FetchError::Transport(note));
        }
        Ok(addrs) => addrs,
    };
    let mut last_err = "no address".to_string();
    let mut sock = None;
    for addr in addrs {
        match std::net::TcpStream::connect_timeout(&addr, timeout) {
            Ok(stream) => {
                sock = Some(stream);
                break;
            }
            Err(e) => last_err = format!("tcp {addr}: {e}"),
        }
    }
    let sock = sock.ok_or(FetchError::Transport(last_err))?;
    sock.set_read_timeout(Some(timeout))
        .map_err(|e| FetchError::Transport(format!("tcp: {e}")))?;
    sock.set_write_timeout(Some(timeout))
        .map_err(|e| FetchError::Transport(format!("tcp: {e}")))?;
    if parts.scheme != "https" {
        return Ok((
            FetchStream::Plain(sock),
            Vec::new(),
            "tls-not-applicable-http".to_string(),
        ));
    }
    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let config = rustls::ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    let server_name = rustls::pki_types::ServerName::try_from(parts.host.clone())
        .map_err(|e| FetchError::Transport(format!("bad server name: {e}")))?;
    let conn = rustls::ClientConnection::new(std::sync::Arc::new(config), server_name)
        .map_err(|e| FetchError::Transport(format!("tls setup: {e}")))?;
    let mut tls = rustls::StreamOwned::new(conn, sock);
    while tls.conn.is_handshaking() {
        tls.conn
            .complete_io(&mut tls.sock)
            .map_err(|e| FetchError::Transport(format!("handshake: {e}")))?;
    }
    let chain: Vec<String> = tls
        .conn
        .peer_certificates()
        .ok_or_else(|| FetchError::Transport("tls: no peer certificates".to_string()))?
        .iter()
        .map(|cert| tls_fingerprint(cert.as_ref()))
        .collect();
    Ok((FetchStream::Tls(Box::new(tls)), chain, "tls-ok".to_string()))
}

/// Buffered remainder reader: head bytes already read stay available
/// to the body stage without a second syscall path.
struct BodyReader<'a> {
    stream: &'a mut FetchStream,
    buffered: Vec<u8>,
}

impl<'a> BodyReader<'a> {
    /// Read exactly `n` bytes (or fail on early EOF).
    fn read_exact(&mut self, n: usize, url: &str) -> Result<Vec<u8>, FetchError> {
        use std::io::Read as _;
        let mut out = Vec::with_capacity(n.min(1 << 20));
        let mut remaining = n;
        while remaining > 0 {
            if !self.buffered.is_empty() {
                let take = remaining.min(self.buffered.len());
                out.extend_from_slice(&self.buffered[..take]);
                self.buffered.drain(..take);
                remaining -= take;
                continue;
            }
            let mut chunk = [0u8; 8192];
            match self.stream.read(&mut chunk) {
                Ok(0) => {
                    return Err(FetchError::Transport(format!(
                        "{url}: connection closed ({} of {n} body bytes)",
                        out.len()
                    )));
                }
                Ok(got) => {
                    let take = remaining.min(got);
                    out.extend_from_slice(&chunk[..take]);
                    self.buffered.extend_from_slice(&chunk[take..got]);
                    remaining -= take;
                }
                Err(e) => {
                    return Err(FetchError::Transport(format!("{url}: read body: {e}")));
                }
            }
        }
        Ok(out)
    }

    /// Read one CRLF-terminated line without the terminator.
    fn read_line(&mut self, url: &str) -> Result<Vec<u8>, FetchError> {
        use std::io::Read as _;
        loop {
            if let Some(i) = find_subslice(&self.buffered, b"\r\n") {
                let line: Vec<u8> = self.buffered.drain(..i).collect();
                self.buffered.drain(..2);
                return Ok(line);
            }
            if self.buffered.len() > MAX_HEAD_BYTES as usize {
                return Err(FetchError::Transport(format!(
                    "{url}: line exceeds head cap"
                )));
            }
            let mut chunk = [0u8; 4096];
            match self.stream.read(&mut chunk) {
                Ok(0) => {
                    return Err(FetchError::Transport(format!(
                        "{url}: connection closed mid-line"
                    )));
                }
                Ok(got) => self.buffered.extend_from_slice(&chunk[..got]),
                Err(e) => {
                    return Err(FetchError::Transport(format!("{url}: read line: {e}")));
                }
            }
        }
    }

    /// Read until the server closes (no `Content-Length` framing).
    fn read_to_end(&mut self, url: &str) -> Result<Vec<u8>, FetchError> {
        use std::io::Read as _;
        let mut out = std::mem::take(&mut self.buffered);
        let mut chunk = [0u8; 16384];
        loop {
            if out.len() as u64 > MAX_BODY_BYTES {
                return Err(FetchError::TooLarge(out.len() as u64));
            }
            match self.stream.read(&mut chunk) {
                Ok(0) => return Ok(out),
                Ok(got) => out.extend_from_slice(&chunk[..got]),
                Err(e) => {
                    return Err(FetchError::Transport(format!("{url}: read body: {e}")));
                }
            }
        }
    }
}

/// Status line + headers + a reader positioned at the body start.
type HeadParts<'a> = (String, Vec<(String, String)>, BodyReader<'a>);

/// Read the status line + headers (capped at [`MAX_HEAD_BYTES`]),
/// returning them plus a reader positioned at the body start.
fn read_head<'a>(stream: &'a mut FetchStream, url: &str) -> Result<HeadParts<'a>, FetchError> {
    use std::io::Read as _;
    let mut buffered = Vec::new();
    let mut chunk = [0u8; 4096];
    let end = loop {
        if let Some(i) = find_subslice(&buffered, b"\r\n\r\n") {
            break i;
        }
        if buffered.len() > MAX_HEAD_BYTES as usize {
            return Err(FetchError::Transport(format!("{url}: head exceeds cap")));
        }
        match stream.read(&mut chunk) {
            Ok(0) => {
                return Err(FetchError::Transport(format!(
                    "{url}: connection closed before head"
                )));
            }
            Ok(got) => buffered.extend_from_slice(&chunk[..got]),
            Err(e) => {
                return Err(FetchError::Transport(format!("{url}: read head: {e}")));
            }
        }
    };
    let head_bytes = buffered.drain(..end + 4).collect::<Vec<u8>>();
    let text = String::from_utf8_lossy(&head_bytes).into_owned();
    let mut lines = text.split("\r\n");
    let status_line = lines.next().unwrap_or("").to_string();
    if status_line.is_empty() {
        return Err(FetchError::Transport(format!("{url}: empty status line")));
    }
    let mut headers = Vec::new();
    for line in lines {
        if line.is_empty() {
            continue;
        }
        let Some((key, value)) = line.split_once(':') else {
            return Err(FetchError::Transport(format!("{url}: bad header line")));
        };
        headers.push((key.trim().to_string(), value.trim().to_string()));
    }
    Ok((status_line, headers, BodyReader { stream, buffered }))
}

/// Parse `HTTP/1.x <code> ...` into the status code.
fn parse_status(status_line: &str) -> Result<u16, FetchError> {
    let mut words = status_line.split_whitespace();
    match (words.next(), words.next()) {
        (Some(version), Some(code))
            if version.starts_with("HTTP/")
                && code.len() == 3
                && code.bytes().all(|b| b.is_ascii_digit()) =>
        {
            code.parse::<u16>()
                .map_err(|_| FetchError::Transport(format!("bad status: {status_line}")))
        }
        _ => Err(FetchError::Transport(format!(
            "bad status line: {status_line}"
        ))),
    }
}

/// Read the body per the framing headers; returns `(body, dechunked)`.
///
/// Chunked bodies are de-chunked (callers drop `Transfer-Encoding`
/// from the recorded head); `Content-Length` bodies are read exactly
/// (a declared length over the cap aborts first); anything else reads
/// until close. A `Content-Encoding` is never decoded — the bytes are
/// archived exactly as received.
fn read_body(
    reader: &mut BodyReader<'_>,
    headers: &[(String, String)],
    url: &str,
    chunked: bool,
) -> Result<(Vec<u8>, bool), FetchError> {
    if chunked {
        return dechunk(reader, url).map(|body| (body, true));
    }
    if let Some(declared) = headers
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, value)| value.split(',').next())
        .and_then(|first| first.trim().parse::<u64>().ok())
    {
        if declared > MAX_BODY_BYTES {
            return Err(FetchError::TooLarge(declared));
        }
        let body = reader.read_exact(declared as usize, url)?;
        return Ok((body, false));
    }
    let body = reader.read_to_end(url)?;
    if body.len() as u64 > MAX_BODY_BYTES {
        return Err(FetchError::TooLarge(body.len() as u64));
    }
    Ok((body, false))
}

/// De-chunk a `Transfer-Encoding: chunked` body (chunk extensions and
/// trailers accepted and discarded).
fn dechunk(reader: &mut BodyReader<'_>, url: &str) -> Result<Vec<u8>, FetchError> {
    let mut body = Vec::new();
    loop {
        let line = reader.read_line(url)?;
        let line = String::from_utf8_lossy(&line);
        let size_text = line.split(';').next().unwrap_or("").trim();
        let size = usize::from_str_radix(size_text, 16)
            .map_err(|_| FetchError::Transport(format!("{url}: bad chunk size: {size_text}")))?;
        if body.len() as u64 + size as u64 > MAX_BODY_BYTES {
            return Err(FetchError::TooLarge(body.len() as u64 + size as u64));
        }
        if size == 0 {
            // Trailers to the blank line.
            loop {
                let trailer = reader.read_line(url)?;
                if trailer.is_empty() {
                    break;
                }
            }
            return Ok(body);
        }
        body.extend_from_slice(&reader.read_exact(size, url)?);
        let crlf = reader.read_exact(2, url)?;
        if crlf != b"\r\n" {
            return Err(FetchError::Transport(format!("{url}: bad chunk end")));
        }
    }
}

/// Rebuild the recorded head: the received status line + headers,
/// dropping `Transfer-Encoding` when the body was de-chunked (the
/// archived body is then identity-framed, so keeping the header
/// would misdescribe the record).
fn build_recorded_head(
    status_line: &str,
    headers: &[(String, String)],
    dechunked: bool,
) -> Vec<u8> {
    let mut head = format!("{status_line}\r\n").into_bytes();
    for (name, value) in headers {
        if dechunked && name.eq_ignore_ascii_case("transfer-encoding") {
            continue;
        }
        head.extend_from_slice(name.as_bytes());
        head.extend_from_slice(b": ");
        head.extend_from_slice(value.as_bytes());
        head.extend_from_slice(b"\r\n");
    }
    head.extend_from_slice(b"\r\n");
    head
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
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

/// TLS handshake against `host:port`, returning leaf-first peer
/// certificate fingerprints. Verifies against the platform web PKI
/// roots; any failure is an `Err` string for the metadata note.
///
/// The SSRF guard applies here too: the host is resolved through
/// [`resolve_guarded`] (private addresses rejected unless
/// `allow_loopback`) and the handshake runs against a vetted address.
///
/// # Errors
///
/// DNS, TCP, timeout, TLS handshake/verification failures.
pub fn capture_tls_chain(
    host: &str,
    port: u16,
    timeout: Duration,
    allow_loopback: bool,
) -> Result<Vec<String>, String> {
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
    let addrs = resolve_guarded(host, port, allow_loopback)?;
    let mut sock = None;
    let mut last_err = "no address".to_string();
    for addr in addrs {
        match std::net::TcpStream::connect_timeout(&addr, timeout) {
            Ok(stream) => {
                sock = Some(stream);
                break;
            }
            Err(e) => last_err = format!("tcp {addr}: {e}"),
        }
    }
    let mut sock = sock.ok_or(last_err)?;
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
        let result = fetch_with_hops_loopback(&base, Duration::from_secs(5)).expect("fetch");
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
        let result = fetch_with_hops_loopback(&base, Duration::from_secs(5)).expect("fetch");
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
        let err = fetch_with_hops_loopback(&base, Duration::from_secs(5)).expect_err("oversize");
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
        let verdict = check_robots_closed_loopback(&base, Duration::from_secs(5));
        assert!(!verdict.allowed, "got: {verdict:?}");
        assert_eq!(verdict.text, "disallowed");
        server.join().unwrap();

        let (open_base, open_server) =
            serve_loop(vec![reply("HTTP/1.0 404 Missing", &[], b"nope")], 1);
        let verdict = check_robots_closed_loopback(&open_base, Duration::from_secs(5));
        assert!(verdict.allowed, "got: {verdict:?}");
        assert_eq!(verdict.text, "no-robots");
        open_server.join().unwrap();

        let probe = TcpListener::bind("127.0.0.1:0").expect("bind probe");
        let port = probe.local_addr().unwrap().port();
        drop(probe);
        let verdict = check_robots_closed_loopback(
            &format!("http://127.0.0.1:{port}/"),
            Duration::from_secs(2),
        );
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

    #[test]
    fn record_id_has_uuid_v4_bits() {
        for _ in 0..20 {
            let id = new_record_id();
            let hex: String = id
                .trim_start_matches("<urn:uuid:")
                .trim_end_matches('>')
                .chars()
                .filter(|c| *c != '-')
                .collect();
            assert_eq!(hex.len(), 32);
            // Version nibble is `4`.
            assert_eq!(hex.chars().nth(12), Some('4'), "{id}");
            // Variant bits are `10xx`.
            let variant = u8::from_str_radix(&hex[16..17], 16).expect("hex");
            assert!((8..12).contains(&variant), "{id}");
        }
    }

    #[test]
    fn split_url_ipv6_literals() {
        let parts = split_url("http://[::1]:8080/a").expect("parse");
        assert_eq!(parts.host, "::1");
        assert_eq!(parts.port, 8080);
        assert_eq!(parts.origin(), "http://[::1]:8080");
        let loopback = split_url("http://[::1]/").expect("parse");
        assert_eq!(loopback.port, 80);
        assert_eq!(loopback.origin(), "http://[::1]");
        assert_eq!(split_url("http://[]/"), None);
        assert_eq!(split_url("http://[::1/x"), None);
    }

    #[test]
    fn ssrf_guard_rejects_special_ranges() {
        use std::net::IpAddr;
        // Blocked with and without the fixture switch.
        let blocked = [
            "10.0.0.1",
            "172.16.0.1",
            "172.31.255.255",
            "192.168.1.1",
            "169.254.0.1",
            "169.254.169.254",
            "100.64.0.1",
            "100.100.100.200",
            "100.127.255.255",
            "192.0.2.1",
            "198.51.100.7",
            "203.0.113.9",
            "224.0.0.1",
            "255.255.255.255",
            "0.0.0.0",
            "::",
            "fc00::1",
            "fd12:3456::1",
            "fe80::1",
            "ff02::1",
            "2001:db8::1",
            "::ffff:10.0.0.1",
            "::ffff:169.254.169.254",
        ];
        for addr in blocked {
            let ip: IpAddr = addr.parse().expect("fixture addr");
            assert!(ip_blocked(&ip, false), "should block {addr}");
            assert!(ip_blocked(&ip, true), "should block {addr} even open");
        }
        // Loopback opens only under the fixture switch.
        for addr in ["127.0.0.1", "127.200.1.9", "::1"] {
            let ip: IpAddr = addr.parse().expect("fixture addr");
            assert!(ip_blocked(&ip, false), "should block {addr}");
            assert!(!ip_blocked(&ip, true), "should allow {addr} open");
        }
        let loopback: IpAddr = "127.0.0.1".parse().unwrap();
        assert!(!ip_blocked(&loopback, true));
        let loopback6: IpAddr = "::1".parse().unwrap();
        assert!(!ip_blocked(&loopback6, true));
        // Ordinary public addresses pass either way.
        for addr in [
            "8.8.8.8",
            "1.1.1.1",
            "93.184.216.34",
            "2606:4700:4700::1111",
        ] {
            let ip: IpAddr = addr.parse().expect("fixture addr");
            assert!(!ip_blocked(&ip, false), "should allow {addr}");
            assert!(!ip_blocked(&ip, true), "should allow {addr}");
        }
        // 172.15/172.32 borders are public; 100.63/100.128 borders too.
        for addr in [
            "172.15.255.255",
            "172.32.0.1",
            "100.63.255.255",
            "100.128.0.1",
        ] {
            let ip: IpAddr = addr.parse().expect("fixture addr");
            assert!(!ip_blocked(&ip, false), "should allow {addr}");
        }
    }

    #[test]
    fn ssrf_guard_blocks_loopback_by_default() {
        // `localhost` here resolves to loopback on any sane machine; the
        // production default must refuse it before connecting.
        let err = fetch_with_hops("http://localhost:9/", Duration::from_secs(5))
            .expect_err("loopback denied");
        assert!(matches!(err, FetchError::SsrfBlocked(_)), "got: {err}");
        let err = fetch_with_hops("http://127.0.0.1:9/", Duration::from_secs(5))
            .expect_err("loopback denied");
        assert!(matches!(err, FetchError::SsrfBlocked(_)), "got: {err}");
    }

    #[test]
    fn robots_rfc9309_groups_and_wildcards() {
        // Most-specific group wins: our token over `*`, even when `*`
        // disallows everything.
        let body = "User-agent: *\nDisallow: /\n\nUser-agent: handbook-crawl\nDisallow: /private\n";
        assert!(parse_robots(body, "/public/page").allowed);
        assert!(!parse_robots(body, "/private/page").allowed);
        // Reversed order: same result (specificity, not position).
        let body = "User-agent: handbook-crawl\nDisallow: /private\n\nUser-agent: *\nDisallow: /\n";
        assert!(parse_robots(body, "/public/page").allowed);
        assert!(!parse_robots(body, "/private/page").allowed);
        // No matching group: allowed.
        let foreign = "User-agent: otherbot\nDisallow: /\n";
        assert!(parse_robots(foreign, "/anything").allowed);
        // `*` wildcard spans `/` and empty runs.
        let wild = "User-agent: *\nDisallow: /*private*\n";
        assert!(!parse_robots(wild, "/a/private/b").allowed);
        assert!(!parse_robots(wild, "/private").allowed);
        assert!(parse_robots(wild, "/public").allowed);
        // `$` anchors at end of path.
        let anchor = "User-agent: *\nDisallow: /*.pdf$\n";
        assert!(!parse_robots(anchor, "/docs/fee.pdf").allowed);
        assert!(parse_robots(anchor, "/docs/fee.pdf?page=2").allowed);
        assert!(parse_robots(anchor, "/docs/pdf-guide").allowed);
        // Longest match wins; ties go to Allow.
        let tie = "User-agent: *\nDisallow: /a\nAllow: /a\n";
        assert!(parse_robots(tie, "/a").allowed);
        let longer = "User-agent: *\nDisallow: /tmp\nAllow: /tmp/open\n";
        assert!(!parse_robots(longer, "/tmp/secret").allowed);
        assert!(parse_robots(longer, "/tmp/open/file").allowed);
        // Empty Disallow matches nothing.
        let empty = "User-agent: *\nDisallow:\n";
        assert!(parse_robots(empty, "/anything").allowed);
        // Crawl-delay comes from the selected group only.
        let delay = "User-agent: *\nCrawl-delay: 7\nDisallow: /x\n\n\
             User-agent: handbook-crawl\nDisallow: /y\n";
        let verdict = parse_robots(delay, "/ok");
        assert!(verdict.allowed);
        assert_eq!(verdict.crawl_delay_secs, 0);
        let delay2 = "User-agent: *\nDisallow: /x\n\n\
             User-agent: handbook-crawl\nCrawl-delay: 3\nDisallow: /y\n";
        assert_eq!(parse_robots(delay2, "/ok").crawl_delay_secs, 3);
    }

    #[test]
    fn surt_key_shapes() {
        assert_eq!(
            surt_key("https://example.gov/a?b=c"),
            Some("gov,example)/a?b=c".to_string())
        );
        assert_eq!(
            surt_key("http://www.example.gov:8080/x"),
            Some("gov,example,www:8080)/x".to_string())
        );
        assert_eq!(
            surt_key("http://127.0.0.1:9/a"),
            Some("1,0,0,127:9)/a".to_string())
        );
        assert_eq!(
            surt_key("http://[::1]:8080/a"),
            Some("::1:8080)/a".to_string())
        );
        assert_eq!(surt_key("not a url"), None);
    }

    #[test]
    fn request_host_carries_nondefault_port() {
        // Serve one connection while capturing the raw request head, so
        // the test observes the real bytes on the wire.
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        let port = listener.local_addr().unwrap().port();
        let seen_head = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
        let seen_head2 = seen_head.clone();
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            let mut buf = [0u8; 4096];
            let n = stream.read(&mut buf).unwrap_or(0);
            *seen_head2.lock().unwrap() = String::from_utf8_lossy(&buf[..n]).into_owned();
            stream
                .write_all(&reply("HTTP/1.0 200 OK", &[], b"hi"))
                .expect("reply");
        });
        let url = format!("http://127.0.0.1:{port}/p");
        let result = fetch_with_hops_loopback(&url, Duration::from_secs(5)).expect("fetch");
        handle.join().unwrap();
        let head = seen_head.lock().unwrap().clone();
        assert!(head.contains(&format!("Host: 127.0.0.1:{port}")), "{head}");
        assert!(!head.contains("gzip"), "{head}");
        let recorded = String::from_utf8_lossy(&result.hops[0].request_bytes).into_owned();
        assert!(
            recorded.contains(&format!("Host: 127.0.0.1:{port}")),
            "{recorded}"
        );
    }

    #[test]
    fn chunked_body_drops_transfer_encoding() {
        let chunked = [
            b"7\r\n".to_vec(),
            b"Mozilla\r\n".to_vec(),
            b"9\r\n".to_vec(),
            b"Developer\r\n".to_vec(),
            b"7\r\n".to_vec(),
            b"Network\r\n".to_vec(),
            b"0\r\n".to_vec(),
            b"\r\n".to_vec(),
        ]
        .concat();
        let mut raw =
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nContent-Type: text/plain\r\n\r\n"
                .to_vec();
        raw.extend_from_slice(&chunked);
        let (base, server) = serve_loop(vec![raw], 1);
        let result = fetch_with_hops_loopback(&base, Duration::from_secs(5)).expect("fetch");
        assert_eq!(result.hops[0].body, b"MozillaDeveloperNetwork");
        let head = String::from_utf8_lossy(&result.hops[0].response_head).into_owned();
        assert!(
            !head.to_ascii_lowercase().contains("transfer-encoding"),
            "{head}"
        );
        assert!(head.contains("Content-Type: text/plain"), "{head}");
        server.join().unwrap();
    }

    #[test]
    fn gzip_body_passes_through_undecoded() {
        use std::io::Write as _;
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(b"gzip stays gzipped").unwrap();
        let gzipped = encoder.finish().unwrap();
        let mut raw =
            b"HTTP/1.1 200 OK\r\nContent-Encoding: gzip\r\nContent-Type: text/html\r\n\r\n"
                .to_vec();
        raw.extend_from_slice(&gzipped);
        let (base, server) = serve_loop(vec![raw], 1);
        let result = fetch_with_hops_loopback(&base, Duration::from_secs(5)).expect("fetch");
        assert_eq!(result.hops[0].body, gzipped);
        let head = String::from_utf8_lossy(&result.hops[0].response_head).into_owned();
        assert!(head.contains("Content-Encoding: gzip"), "{head}");
        server.join().unwrap();
    }

    #[test]
    fn located_records_track_member_offsets() {
        let dir = std::env::temp_dir().join(format!("warc-offsets-{}", std::process::id()));
        let path = dir.join("offsets.warc.gz");
        let mut writer = WarcWriter::create(&path).expect("create");
        writer.write_warcinfo(&[]).expect("warcinfo");
        let req = writer
            .write_request("http://127.0.0.1:9/", b"GET / HTTP/1.1\r\n\r\n", None)
            .expect("request");
        let body = b"payload";
        let mut response_bytes = b"HTTP/1.1 200 OK\r\n\r\n".to_vec();
        response_bytes.extend_from_slice(body);
        let res = writer
            .write_response("http://127.0.0.1:9/", &response_bytes, body, Some(&req.id))
            .expect("response");
        writer.finish().expect("finish");

        let located = read_located_records(&path).expect("located");
        assert_eq!(located.len(), 3);
        // Members are contiguous from zero.
        assert_eq!(located[0].offset, 0);
        for pair in located.windows(2) {
            assert_eq!(pair[1].offset, pair[0].offset + pair[0].length);
        }
        let file_len = std::fs::metadata(&path).expect("stat").len();
        let last = located.last().expect("last");
        assert_eq!(last.offset + last.length, file_len);
        assert_eq!(
            located[2].record.header("WARC-Record-ID"),
            Some(res.id.as_str())
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn resolve_guarded_rejects_loopback_by_default() {
        let err = resolve_guarded("localhost", 80, false).expect_err("blocked");
        assert!(err.starts_with("ssrf-blocked"), "{err}");
        let ok = resolve_guarded("localhost", 80, true).expect("loopback fixture");
        assert!(!ok.is_empty());
    }
}
