//! URL-keyed fetch cache: content-addressed bodies plus a manifest.
//!
//! Layout under the cache root (`paths.fetch_cache_dir`):
//!
//! - `blobs/<sha[0..2]>/<sha256>`: body bytes, stored once per digest;
//! - `manifest.jsonl`: one line per fetch, newest last:
//!   `{url, fetched_at, status, headers: [[name, value]…], sha256,
//!   len}` plus optional `head_sha256` / `request_sha256` blobs (the
//!   raw response head and request bytes a WARC writer needs to replay
//!   a capture byte for byte). Header names are lowercase.
//!
//! Modes ([`CacheMode`]): `record` serves the newest stored answer for
//! a URL unless it was transient (429/5xx) or a bodiless 304, and
//! fetches + appends on a miss; `replay-only` serves the newest answer whatever its status and
//! never touches the network (the rerun-from-cache mode); `refresh`
//! always fetches and appends (watch sweeps: every fetch is archived).

use std::collections::HashMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::meta::CallError;
use crate::{CacheError, CacheMode, ensure_dir, sha256_hex, shard, unix_now, write_atomic};

/// One manifest line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestLine {
    pub url: String,
    pub fetched_at: i64,
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub sha256: String,
    pub len: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_sha256: Option<String>,
}

/// One fetch, bytes in hand.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchRecord {
    pub url: String,
    pub fetched_at: i64,
    pub status: u16,
    /// Lowercase names, wire order.
    pub headers: Vec<(String, String)>,
    pub sha256: String,
    pub body: Vec<u8>,
    /// Raw response head (status line + headers), when recorded.
    pub head: Option<Vec<u8>>,
    /// Raw request bytes, when recorded.
    pub request: Option<Vec<u8>>,
    /// Served from disk, not the network.
    pub from_cache: bool,
}

impl FetchRecord {
    /// First header named `name` (case-insensitive).
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

/// What a live fetch hands the cache.
#[derive(Debug, Clone, Default)]
pub struct Fetched {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    pub head: Option<Vec<u8>>,
    pub request: Option<Vec<u8>>,
}

/// The fetch cache. The manifest index loads on first use and tracks
/// this process's own appends; other processes' appends show up on the
/// next [`FetchCache::new`].
#[derive(Debug)]
pub struct FetchCache {
    root: PathBuf,
    mode: CacheMode,
    index: Mutex<Option<HashMap<String, ManifestLine>>>,
}

/// Transient statuses: stored in the manifest, never served in record
/// mode (the next run asks again).
fn transient(status: u16) -> bool {
    status == 0 || status == 429 || status >= 500
}

impl FetchCache {
    pub fn new(root: impl Into<PathBuf>, mode: CacheMode) -> Self {
        Self {
            root: root.into(),
            mode,
            index: Mutex::new(None),
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn mode(&self) -> CacheMode {
        self.mode
    }

    pub fn manifest_path(&self) -> PathBuf {
        self.root.join("manifest.jsonl")
    }

    fn blobs(&self) -> PathBuf {
        self.root.join("blobs")
    }

    fn with_index<R>(&self, f: impl FnOnce(&mut HashMap<String, ManifestLine>) -> R) -> R {
        let mut guard = self.index.lock().unwrap_or_else(|p| p.into_inner());
        let index = guard.get_or_insert_with(|| self.load_index());
        f(index)
    }

    fn load_index(&self) -> HashMap<String, ManifestLine> {
        let mut index = HashMap::new();
        let Ok(text) = std::fs::read_to_string(self.manifest_path()) else {
            return index;
        };
        for line in text.lines() {
            // A torn last line (crash mid-append) is skipped, not fatal.
            if let Ok(entry) = serde_json::from_str::<ManifestLine>(line) {
                index.insert(entry.url.clone(), entry);
            }
        }
        index
    }

    fn blob(&self, sha: &str) -> Option<Vec<u8>> {
        let bytes = std::fs::read(shard(&self.blobs(), sha, "")).ok()?;
        (sha256_hex(&bytes) == sha).then_some(bytes)
    }

    fn put_blob(&self, bytes: &[u8]) -> Result<String, CacheError> {
        let sha = sha256_hex(bytes);
        let path = shard(&self.blobs(), &sha, "");
        if !path.is_file() {
            write_atomic(&path, bytes)?;
        }
        Ok(sha)
    }

    /// Newest stored fetch of `url`, whatever its status and the mode.
    pub fn latest(&self, url: &str) -> Option<FetchRecord> {
        let line = self.with_index(|index| index.get(url).cloned())?;
        let body = self.blob(&line.sha256)?;
        let head = match line.head_sha256.as_deref() {
            Some(sha) => Some(self.blob(sha)?),
            None => None,
        };
        let request = line
            .request_sha256
            .as_deref()
            .and_then(|sha| self.blob(sha));
        Some(FetchRecord {
            url: line.url,
            fetched_at: line.fetched_at,
            status: line.status,
            headers: line.headers,
            sha256: line.sha256,
            body,
            head,
            request,
            from_cache: true,
        })
    }

    /// Mode-aware lookup: `Some` to serve from disk, `None` to fetch,
    /// `Err(Miss)` in replay-only with nothing stored.
    pub fn lookup(&self, url: &str) -> Result<Option<FetchRecord>, CacheError> {
        match self.mode {
            CacheMode::Refresh => Ok(None),
            // A 304 carries no body to serve a plain GET: fetch again.
            CacheMode::Record => Ok(self
                .latest(url)
                .filter(|r| !transient(r.status) && r.status != 304)),
            CacheMode::ReplayOnly => self.latest(url).map(Some).ok_or_else(|| CacheError::Miss {
                key: url.to_string(),
            }),
        }
    }

    /// Validators from the newest stored 2xx (for a conditional GET).
    pub fn validators(&self, url: &str) -> (Option<String>, Option<String>) {
        match self.latest(url).filter(|r| (200..300).contains(&r.status)) {
            Some(r) => (
                r.header("etag").map(str::to_string),
                r.header("last-modified").map(str::to_string),
            ),
            None => (None, None),
        }
    }

    /// Append one live fetch (blob first, then the manifest line).
    pub fn record(&self, url: &str, fetched: Fetched) -> Result<FetchRecord, CacheError> {
        let headers: Vec<(String, String)> = fetched
            .headers
            .into_iter()
            .map(|(k, v)| (k.to_ascii_lowercase(), v))
            .collect();
        let sha256 = self.put_blob(&fetched.body)?;
        let head_sha256 = fetched
            .head
            .as_deref()
            .map(|h| self.put_blob(h))
            .transpose()?;
        let request_sha256 = fetched
            .request
            .as_deref()
            .map(|r| self.put_blob(r))
            .transpose()?;
        let line = ManifestLine {
            url: url.to_string(),
            fetched_at: unix_now(),
            status: fetched.status,
            headers: headers.clone(),
            sha256: sha256.clone(),
            len: fetched.body.len() as u64,
            head_sha256,
            request_sha256,
        };
        let text = serde_json::to_string(&line).map_err(|e| CacheError::Io(e.to_string()))?;
        self.with_index(|index| {
            ensure_dir(&self.root)?;
            let path = self.manifest_path();
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
                .and_then(|mut f| f.write_all(format!("{text}\n").as_bytes()))
                .map_err(|e| CacheError::Io(format!("manifest {}: {e}", path.display())))?;
            index.insert(url.to_string(), line.clone());
            Ok::<_, CacheError>(())
        })?;
        Ok(FetchRecord {
            url: url.to_string(),
            fetched_at: line.fetched_at,
            status: fetched.status,
            headers,
            sha256,
            body: fetched.body,
            head: fetched.head,
            request: fetched.request,
            from_cache: false,
        })
    }

    /// Lookup, else fetch live and record. `live` failing (transport)
    /// records nothing: there is no answer to replay.
    pub fn fetch<E>(
        &self,
        url: &str,
        live: impl FnOnce() -> Result<Fetched, E>,
    ) -> Result<FetchRecord, CallError<E>> {
        if let Some(hit) = self.lookup(url).map_err(CallError::Cache)? {
            return Ok(hit);
        }
        let fetched = live().map_err(CallError::Live)?;
        self.record(url, fetched).map_err(CallError::Cache)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    fn page(status: u16, body: &str) -> Fetched {
        Fetched {
            status,
            headers: vec![("ETag".to_string(), "\"v1\"".to_string())],
            body: body.as_bytes().to_vec(),
            head: Some(format!("HTTP/1.1 {status} OK\r\n\r\n").into_bytes()),
            request: None,
        }
    }

    #[test]
    fn record_mode_fetches_once_and_reloads_from_the_manifest() {
        let dir = tempfile::tempdir().unwrap();
        let cache = FetchCache::new(dir.path(), CacheMode::Record);
        let calls = Cell::new(0);
        let live = || {
            calls.set(calls.get() + 1);
            Ok::<_, String>(page(200, "<html>fees</html>"))
        };
        let first = cache.fetch("https://a.test/fees", live).unwrap();
        assert!(!first.from_cache);
        let second = cache.fetch("https://a.test/fees", live).unwrap();
        assert!(second.from_cache);
        assert_eq!(second.body, first.body);
        assert_eq!(calls.get(), 1);
        // A fresh handle (next run) reads the manifest back.
        let rerun = FetchCache::new(dir.path(), CacheMode::ReplayOnly);
        let replayed = rerun.lookup("https://a.test/fees").unwrap().unwrap();
        assert_eq!(replayed.header("etag"), Some("\"v1\""));
        assert_eq!(replayed.head.unwrap(), b"HTTP/1.1 200 OK\r\n\r\n");
        assert!(matches!(
            rerun.lookup("https://a.test/other"),
            Err(CacheError::Miss { .. })
        ));
        let manifest = std::fs::read_to_string(rerun.manifest_path()).unwrap();
        assert_eq!(manifest.lines().count(), 1);
        assert!(manifest.contains("\"sha256\""));
    }

    #[test]
    fn transient_answers_are_logged_but_refetched_in_record_mode() {
        let dir = tempfile::tempdir().unwrap();
        let cache = FetchCache::new(dir.path(), CacheMode::Record);
        cache
            .fetch("https://a.test/", || Ok::<_, String>(page(503, "busy")))
            .unwrap();
        let again = cache
            .fetch("https://a.test/", || Ok::<_, String>(page(200, "ok")))
            .unwrap();
        assert!(!again.from_cache);
        // A 404 is an answer: served from disk on the next record run.
        cache
            .fetch("https://a.test/gone", || Ok::<_, String>(page(404, "")))
            .unwrap();
        let gone = cache
            .fetch("https://a.test/gone", || -> Result<Fetched, String> {
                panic!("refetched a stored 404")
            })
            .unwrap();
        assert_eq!(gone.status, 404);
        assert_eq!(
            cache.validators("https://a.test/").0.as_deref(),
            Some("\"v1\"")
        );
    }

    #[test]
    fn refresh_appends_every_fetch_and_identical_bodies_share_a_blob() {
        let dir = tempfile::tempdir().unwrap();
        let cache = FetchCache::new(dir.path(), CacheMode::Refresh);
        for _ in 0..2 {
            cache
                .fetch("https://a.test/", || Ok::<_, String>(page(200, "same")))
                .unwrap();
        }
        let manifest = std::fs::read_to_string(cache.manifest_path()).unwrap();
        assert_eq!(manifest.lines().count(), 2);
        let blobs: usize = std::fs::read_dir(dir.path().join("blobs"))
            .unwrap()
            .map(|d| std::fs::read_dir(d.unwrap().path()).unwrap().count())
            .sum();
        assert_eq!(blobs, 2); // the body + the shared head
    }
}
