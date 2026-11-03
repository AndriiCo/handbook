//! Record/replay caches for paid and polite network calls.
//!
//! Operator decision 2026-11-03: every Meta Model API call and every
//! URL fetch is saved to disk and reused on reruns, so retries and
//! revisions never re-spend budget. Two caches share one mode switch:
//!
//! - [`meta::MetaCache`]: content-addressed (sha256 of the canonical
//!   request) raw response bytes + HTTP status, so every existing
//!   SSE/JSON parser runs unchanged on a hit. Live calls count toward
//!   the daily cap ([`budget::Budget`]); hits never do.
//! - [`fetch::FetchCache`]: URL-keyed, content-addressed bodies plus a
//!   `manifest.jsonl` (`url, fetched_at, status, headers, sha256`).
//!
//! No HTTP lives here: each seam keeps its transport and hands the
//! cache a closure for the live call. `xtask/ops/a2-record.py` writes
//! the same on-disk format (see `docs/ops/call-cache.md`).

pub mod budget;
pub mod fetch;
pub mod meta;
pub mod settings;

use std::fmt;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

pub use budget::Budget;
pub use fetch::{FetchCache, FetchRecord, Fetched};
pub use meta::{Answer, CallError, MetaCache};
pub use settings::{CacheSettings, Overrides};

/// How a cache treats a lookup.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CacheMode {
    /// Default: a hit returns the stored bytes; a miss calls live and
    /// stores the answer.
    #[default]
    Record,
    /// Offline: a hit returns the stored bytes; a miss is an error and
    /// nothing touches the network.
    ReplayOnly,
    /// Always call live and overwrite the stored answer.
    Refresh,
}

impl CacheMode {
    /// `record | replay-only | refresh` (empty reads as `record`).
    pub fn parse(text: &str) -> Option<Self> {
        match text.trim() {
            "" | "record" => Some(Self::Record),
            "replay-only" | "replay" | "offline" => Some(Self::ReplayOnly),
            "refresh" => Some(Self::Refresh),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Record => "record",
            Self::ReplayOnly => "replay-only",
            Self::Refresh => "refresh",
        }
    }
}

impl fmt::Display for CacheMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Cache failures. `Miss` and `OverBudget` are the two a caller must
/// surface as-is: they are why no live call happened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CacheError {
    /// `replay-only` and nothing stored under this key.
    Miss { key: String },
    /// Today's live calls reached the daily cap.
    OverBudget { cap: u64, used: u64 },
    /// Disk trouble (the message names the path, never request data).
    Io(String),
}

impl fmt::Display for CacheError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CacheError::Miss { key } => write!(
                f,
                "cache miss in replay-only mode (key {}…); record mode calls live",
                &key[..key.len().min(12)]
            ),
            CacheError::OverBudget { cap, used } => write!(
                f,
                "daily model-call cap reached (meta.daily_call_cap {cap}, {used} live calls today)"
            ),
            CacheError::Io(msg) => write!(f, "call cache: {msg}"),
        }
    }
}

impl std::error::Error for CacheError {}

/// Lowercase hex sha256 of `bytes`.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(64);
    for byte in digest {
        out.push(char::from(HEX[usize::from(byte >> 4)]));
        out.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    out
}

const HEX: &[u8; 16] = b"0123456789abcdef";

/// Canonical JSON: object keys sorted by code point, no whitespace,
/// strings escaped as serde_json does. Byte-identical to Python's
/// `json.dumps(v, sort_keys=True, separators=(",", ":"),
/// ensure_ascii=False)` for the shapes we send (strings, integers,
/// short decimals, bools, nulls), so both writers agree on keys.
pub fn canonical_json(value: &serde_json::Value) -> String {
    let mut out = String::new();
    write_canonical(value, &mut out);
    out
}

fn write_canonical(value: &serde_json::Value, out: &mut String) {
    use serde_json::Value;
    match value {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            out.push('{');
            for (i, key) in keys.into_iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(&Value::String(key.clone()).to_string());
                out.push(':');
                write_canonical(&map[key], out);
            }
            out.push('}');
        }
        Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_canonical(item, out);
            }
            out.push(']');
        }
        other => out.push_str(&other.to_string()),
    }
}

/// Seconds since the Unix epoch (0 if the clock is before it).
pub fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

/// `YYYY-MM-DD` (UTC) for a Unix timestamp (civil-from-days).
pub fn utc_date(unix: i64) -> String {
    let days = unix.div_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

/// Create `dir` (and parents) owner-only: cached answers can hold
/// customer-derived text (transcripts, drafts).
pub(crate) fn ensure_dir(dir: &Path) -> Result<(), CacheError> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt as _;
        builder.mode(0o700);
    }
    builder
        .create(dir)
        .map_err(|e| CacheError::Io(format!("create {}: {e}", dir.display())))
}

/// Write `bytes` to `path` atomically (temp file + rename), so a
/// reader never sees half an entry and concurrent writers of the same
/// key both leave a whole file.
pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), CacheError> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    ensure_dir(dir)?;
    let tmp = temp_name(path);
    let io = |e: std::io::Error| CacheError::Io(format!("write {}: {e}", path.display()));
    {
        let mut file = std::fs::File::create(&tmp).map_err(io)?;
        file.write_all(bytes).map_err(io)?;
        file.sync_all().map_err(io)?;
    }
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        io(e)
    })
}

fn temp_name(path: &Path) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    path.with_file_name(format!(".{name}.{}.{nanos}.tmp", std::process::id()))
}

/// `<k[0..2]>/<key>` shard path under `root`.
pub(crate) fn shard(root: &Path, key: &str, suffix: &str) -> PathBuf {
    root.join(&key[..2.min(key.len())])
        .join(format!("{key}{suffix}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn modes_parse_and_print() {
        for mode in [CacheMode::Record, CacheMode::ReplayOnly, CacheMode::Refresh] {
            assert_eq!(CacheMode::parse(mode.as_str()), Some(mode));
        }
        assert_eq!(CacheMode::parse(""), Some(CacheMode::Record));
        assert_eq!(CacheMode::parse("offline"), Some(CacheMode::ReplayOnly));
        assert_eq!(CacheMode::parse("sometimes"), None);
    }

    #[test]
    fn canonical_json_sorts_nested_keys_without_spaces() {
        let v = json!({"z": 1, "a": {"y": [true, null, "é\n"], "b": 0.2}, "m": "x"});
        assert_eq!(
            canonical_json(&v),
            r#"{"a":{"b":0.2,"y":[true,null,"é\n"]},"m":"x","z":1}"#
        );
    }

    #[test]
    fn utc_dates_cross_month_and_leap_boundaries() {
        assert_eq!(utc_date(0), "1970-01-01");
        assert_eq!(utc_date(951_782_400), "2000-02-29");
        assert_eq!(utc_date(1_793_664_000), "2026-11-03");
        assert_eq!(utc_date(1_793_663_999), "2026-11-02");
    }

    #[test]
    fn sha_is_lowercase_hex() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
