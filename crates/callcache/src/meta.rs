//! Content-addressed record/replay cache for Meta Model API calls.
//!
//! Key: hex sha256 over `meta-cache/v1\n<METHOD> <url path>\n<body>`,
//! where `<body>` is the canonical JSON request ([`canonical_json`]:
//! sorted keys, no whitespace; it carries the model id) or, for
//! multipart, the canonical JSON of the text fields plus the file
//! part's field name, content type and sha256 ([`multipart_key`]; the
//! filename is left out so temp names still hit). The host is left
//! out too: a staging stub and production share recordings.
//!
//! Layout under the cache root (`meta.cache_dir`):
//!
//! - `v1/<k[0..2]>/<key>.body`: the raw response bytes (SSE or JSON),
//!   exactly as read off the wire;
//! - `v1/<k[0..2]>/<key>.json`: `{v, key, method, path, model, status,
//!   stored_at, sha256, len}`; written last, so it marks a whole entry;
//! - `ledger/<YYYY-MM-DD>.jsonl`: one line per live call ([`Budget`]).
//!
//! Only 2xx answers are stored: a 429/5xx is transient and a 4xx is
//! worth re-asking once the request or the key is fixed.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::budget::Budget;
use crate::{CacheError, CacheMode, canonical_json, sha256_hex, shard, unix_now, write_atomic};

/// Key-format tag; bump to orphan every entry at once.
const KEY_TAG: &str = "meta-cache/v1";

/// Path (and query) of `url`: everything from the first `/` after
/// `scheme://host`. A bare path is returned as-is.
pub fn url_path(url: &str) -> &str {
    match url.find("://") {
        Some(i) => {
            let rest = &url[i + 3..];
            rest.find('/').map_or("/", |j| &rest[j..])
        }
        None => url,
    }
}

/// Key over raw request bytes.
pub fn bytes_key(method: &str, url: &str, body: &[u8]) -> String {
    let mut material = format!("{KEY_TAG}\n{method} {}\n", url_path(url)).into_bytes();
    material.extend_from_slice(body);
    sha256_hex(&material)
}

/// Key for a JSON POST (the body includes the model id).
pub fn json_key(url: &str, body: &Value) -> String {
    bytes_key("POST", url, canonical_json(body).as_bytes())
}

/// One text part of a multipart request, for [`multipart_key`].
#[derive(Debug, Clone, Copy)]
pub struct TextPart<'a> {
    pub name: &'a str,
    pub value: &'a str,
    pub content_type: Option<&'a str>,
}

/// Key for a multipart POST: the text fields in wire order plus the
/// file part's field, content type and sha256.
pub fn multipart_key(
    url: &str,
    file_field: &str,
    file_content_type: &str,
    file_bytes: &[u8],
    fields: &[TextPart<'_>],
) -> String {
    let fields: Vec<Value> = fields
        .iter()
        .map(|f| json!({"name": f.name, "value": f.value, "content_type": f.content_type}))
        .collect();
    let shape = json!({
        "fields": fields,
        "file": {
            "field": file_field,
            "content_type": file_content_type,
            "sha256": sha256_hex(file_bytes),
        },
    });
    bytes_key("POST", url, canonical_json(&shape).as_bytes())
}

/// What a call answered, from disk (`hit`) or the network.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Answer {
    pub status: u16,
    pub body: Vec<u8>,
    pub hit: bool,
}

/// A cached call failed before (`Cache`) or during (`Live`) the call.
#[derive(Debug)]
pub enum CallError<E> {
    Cache(CacheError),
    Live(E),
}

/// Who a call is, for entry metadata and the ledger.
#[derive(Debug, Clone, Copy)]
pub struct Request<'a> {
    pub method: &'a str,
    pub url: &'a str,
    /// Model id from the body (`""` for `/files`).
    pub model: &'a str,
}

#[derive(Debug, Serialize, Deserialize)]
struct EntryMeta {
    v: u32,
    key: String,
    method: String,
    path: String,
    model: String,
    status: u16,
    stored_at: i64,
    sha256: String,
    len: u64,
}

/// The Meta call cache (cheap to share behind an `Arc`).
#[derive(Debug)]
pub struct MetaCache {
    root: PathBuf,
    mode: CacheMode,
    budget: Option<Budget>,
    hits: AtomicU64,
    live: AtomicU64,
}

impl MetaCache {
    /// Cache rooted at `root` (created on first store).
    pub fn new(root: impl Into<PathBuf>, mode: CacheMode) -> Self {
        Self {
            root: root.into(),
            mode,
            budget: None,
            hits: AtomicU64::new(0),
            live: AtomicU64::new(0),
        }
    }

    /// Enforce `cap` live calls per UTC day (ledger under `ledger/`).
    #[must_use]
    pub fn with_budget(mut self, cap: u64) -> Self {
        self.budget = Some(Budget::new(self.root.join("ledger"), cap));
        self
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn mode(&self) -> CacheMode {
        self.mode
    }

    pub fn budget(&self) -> Option<&Budget> {
        self.budget.as_ref()
    }

    /// `(hits, live calls)` served through this handle so far.
    pub fn counts(&self) -> (u64, u64) {
        (
            self.hits.load(Ordering::Relaxed),
            self.live.load(Ordering::Relaxed),
        )
    }

    /// One-line run summary for CLI output.
    pub fn summary(&self) -> String {
        let (hits, live) = self.counts();
        format!(
            "meta cache ({}, {}): {hits} hit(s), {live} live call(s)",
            self.mode,
            self.root.display()
        )
    }

    fn entries(&self) -> PathBuf {
        self.root.join("v1")
    }

    /// Stored answer for `key` regardless of mode (corrupt or partial
    /// entries read as absent).
    pub fn peek(&self, key: &str) -> Option<(Answer, i64)> {
        let meta_path = shard(&self.entries(), key, ".json");
        let meta: EntryMeta = serde_json::from_slice(&std::fs::read(meta_path).ok()?).ok()?;
        let body = std::fs::read(shard(&self.entries(), key, ".body")).ok()?;
        if meta.key != key || sha256_hex(&body) != meta.sha256 {
            return None;
        }
        let answer = Answer {
            status: meta.status,
            body,
            hit: true,
        };
        Some((answer, meta.stored_at))
    }

    /// Mode-aware lookup: `Some` on a hit; `None` when the caller must
    /// call live (record miss, or refresh); `Err(Miss)` in replay-only.
    pub fn lookup(&self, key: &str) -> Result<Option<Answer>, CacheError> {
        self.lookup_within(key, None)
    }

    /// [`Self::lookup`], but in record mode an entry older than
    /// `max_age_secs` is a miss (uploaded file ids expire server-side).
    /// Replay-only serves any age: nothing is re-sent offline anyway.
    pub fn lookup_within(
        &self,
        key: &str,
        max_age_secs: Option<i64>,
    ) -> Result<Option<Answer>, CacheError> {
        if self.mode == CacheMode::Refresh {
            return Ok(None);
        }
        match self.peek(key) {
            Some((answer, stored_at)) => {
                let expired = self.mode == CacheMode::Record
                    && max_age_secs.is_some_and(|age| unix_now().saturating_sub(stored_at) > age);
                if expired {
                    return Ok(None);
                }
                self.hits.fetch_add(1, Ordering::Relaxed);
                Ok(Some(answer))
            }
            None if self.mode == CacheMode::ReplayOnly => Err(CacheError::Miss {
                key: key.to_string(),
            }),
            None => Ok(None),
        }
    }

    /// Gate one live attempt: replay-only never calls out, and the
    /// daily cap holds. Call before every attempt (retries included).
    pub fn admit(&self, key: &str) -> Result<(), CacheError> {
        if self.mode == CacheMode::ReplayOnly {
            return Err(CacheError::Miss {
                key: key.to_string(),
            });
        }
        match &self.budget {
            Some(budget) => budget.check(unix_now()),
            None => Ok(()),
        }
    }

    /// Count one finished live attempt toward the daily cap.
    pub fn spent(&self, req: &Request<'_>, status: u16) {
        self.live.fetch_add(1, Ordering::Relaxed);
        if let Some(budget) = &self.budget {
            // The call is already paid for; a ledger write failure must
            // not discard its answer.
            let _ = budget.spend(unix_now(), url_path(req.url), req.model, status);
        }
    }

    /// Store a live answer (2xx only; anything else is ignored).
    pub fn store(
        &self,
        key: &str,
        req: &Request<'_>,
        status: u16,
        body: &[u8],
    ) -> Result<(), CacheError> {
        if !(200..300).contains(&status) {
            return Ok(());
        }
        let meta = EntryMeta {
            v: 1,
            key: key.to_string(),
            method: req.method.to_string(),
            path: url_path(req.url).to_string(),
            model: req.model.to_string(),
            status,
            stored_at: unix_now(),
            sha256: sha256_hex(body),
            len: body.len() as u64,
        };
        let meta = serde_json::to_vec(&meta).map_err(|e| CacheError::Io(e.to_string()))?;
        write_atomic(&shard(&self.entries(), key, ".body"), body)?;
        write_atomic(&shard(&self.entries(), key, ".json"), &meta)
    }

    /// Whole round trip for a seam with no retry loop of its own (or
    /// one inside `live`, counted once): lookup, admit, call, count,
    /// store. A hit never touches the network or the ledger.
    pub fn call<E>(
        &self,
        key: &str,
        req: &Request<'_>,
        live: impl FnOnce() -> Result<(u16, Vec<u8>), E>,
    ) -> Result<Answer, CallError<E>> {
        if let Some(hit) = self.lookup(key).map_err(CallError::Cache)? {
            return Ok(hit);
        }
        self.admit(key).map_err(CallError::Cache)?;
        let (status, body) = live().map_err(CallError::Live)?;
        self.spent(req, status);
        // A full disk must not discard an answer already paid for.
        let _ = self.store(key, req, status, &body);
        Ok(Answer {
            status,
            body,
            hit: false,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    const URL: &str = "https://api.meta.ai/v1/chat/completions";

    fn req() -> Request<'static> {
        Request {
            method: "POST",
            url: URL,
            model: "muse-spark-1.3",
        }
    }

    fn body(text: &str) -> Value {
        json!({"model": "muse-spark-1.3", "messages": [{"role": "user", "content": text}]})
    }

    #[test]
    fn second_identical_call_makes_zero_transport_calls() {
        let dir = tempfile::tempdir().unwrap();
        let cache = MetaCache::new(dir.path(), CacheMode::Record).with_budget(10);
        let sent = Cell::new(0);
        let live = || {
            sent.set(sent.get() + 1);
            Ok::<_, String>((200, b"data: {\"x\":1}\n\n".to_vec()))
        };
        let key = json_key(URL, &body("hi"));
        let first = cache.call(&key, &req(), live).unwrap();
        assert!(!first.hit);
        let second = cache.call(&key, &req(), live).unwrap();
        assert!(second.hit);
        assert_eq!(second.body, first.body);
        assert_eq!(sent.get(), 1);
        // The hit never counted toward the cap.
        assert_eq!(cache.budget().unwrap().used(unix_now()), 1);
        assert_eq!(cache.counts(), (1, 1));
    }

    #[test]
    fn replay_only_serves_hits_offline_and_refuses_misses() {
        let dir = tempfile::tempdir().unwrap();
        let key = json_key(URL, &body("hi"));
        MetaCache::new(dir.path(), CacheMode::Record)
            .call(&key, &req(), || Ok::<_, String>((200, b"{}".to_vec())))
            .unwrap();
        let offline = MetaCache::new(dir.path(), CacheMode::ReplayOnly);
        let never = || -> Result<(u16, Vec<u8>), String> { panic!("network in replay-only") };
        assert!(offline.call(&key, &req(), never).unwrap().hit);
        let other = json_key(URL, &body("other"));
        assert!(matches!(
            offline.call(&other, &req(), never),
            Err(CallError::Cache(CacheError::Miss { .. }))
        ));
        assert!(offline.admit(&other).is_err());
    }

    #[test]
    fn refresh_always_calls_and_overwrites() {
        let dir = tempfile::tempdir().unwrap();
        let key = json_key(URL, &body("hi"));
        let record = MetaCache::new(dir.path(), CacheMode::Record);
        record
            .call(&key, &req(), || Ok::<_, String>((200, b"old".to_vec())))
            .unwrap();
        let refresh = MetaCache::new(dir.path(), CacheMode::Refresh);
        let fresh = refresh
            .call(&key, &req(), || Ok::<_, String>((200, b"new".to_vec())))
            .unwrap();
        assert!(!fresh.hit);
        assert_eq!(record.lookup(&key).unwrap().unwrap().body, b"new");
    }

    #[test]
    fn errors_are_not_stored_and_cap_blocks_live_calls() {
        let dir = tempfile::tempdir().unwrap();
        let cache = MetaCache::new(dir.path(), CacheMode::Record).with_budget(1);
        let key = json_key(URL, &body("hi"));
        let answer = cache
            .call(&key, &req(), || Ok::<_, String>((503, b"busy".to_vec())))
            .unwrap();
        assert_eq!(answer.status, 503);
        assert_eq!(cache.lookup(&key).unwrap(), None);
        // The 503 attempt was a real call: the cap of 1 is spent.
        assert!(matches!(
            cache.call(&key, &req(), || Ok::<_, String>((200, b"{}".to_vec()))),
            Err(CallError::Cache(CacheError::OverBudget { cap: 1, used: 1 }))
        ));
    }

    #[test]
    fn keys_cover_model_path_and_file_bytes_not_host_or_filename() {
        let a = json_key(URL, &body("hi"));
        let mut other_model = body("hi");
        other_model["model"] = json!("muse-wonder-1.0");
        assert_ne!(a, json_key(URL, &other_model));
        assert_ne!(a, json_key("https://api.meta.ai/v1/responses", &body("hi")));
        assert_eq!(
            a,
            json_key("http://127.0.0.1:9/v1/chat/completions", &body("hi"))
        );
        let part = [TextPart {
            name: "purpose",
            value: "user_data",
            content_type: None,
        }];
        let files = "https://api.meta.ai/v1/files";
        let k1 = multipart_key(files, "file", "audio/wav", b"one", &part);
        assert_ne!(k1, multipart_key(files, "file", "audio/wav", b"two", &part));
        assert_eq!(url_path("https://h/v1/x?y=1"), "/v1/x?y=1");
        assert_eq!(url_path("https://h"), "/");
    }

    #[test]
    fn key_format_is_pinned() {
        // Golden computed by the retired Python A2 recorder (P391-P409):
        // a key change orphans every recorded answer, so it stays fixed.
        let body = json!({
            "model": "muse-spark-1.3",
            "messages": [{"role": "user", "content": "Frais: 15 € — «ok»\n\ttab"}],
            "temperature": 0.2, "stream": false, "max_completion_tokens": 4000,
        });
        assert_eq!(
            json_key(URL, &body),
            "c7820bd13d0c034dafd6f86c205e4ad2c755c5cbe375cd531c76e63f7ad26fd6"
        );
        // An entry the Python recorder wrote (its json.dumps spacing).
        let dir = tempfile::tempdir().unwrap();
        let key = json_key(URL, &body);
        let shard_dir = dir.path().join("v1").join(&key[..2]);
        std::fs::create_dir_all(&shard_dir).unwrap();
        std::fs::write(shard_dir.join(format!("{key}.body")), b"{\"ok\":1}").unwrap();
        let meta = format!(
            "{{\"v\": 1, \"key\": \"{key}\", \"method\": \"POST\", \
             \"path\": \"/v1/chat/completions\", \"model\": \"muse-spark-1.3\", \
             \"status\": 200, \"stored_at\": 1, \"sha256\": \"{}\", \"len\": 8}}",
            sha256_hex(b"{\"ok\":1}")
        );
        std::fs::write(shard_dir.join(format!("{key}.json")), meta).unwrap();
        let cache = MetaCache::new(dir.path(), CacheMode::ReplayOnly);
        assert_eq!(cache.lookup(&key).unwrap().unwrap().body, b"{\"ok\":1}");
    }

    #[test]
    fn corrupt_bodies_read_as_misses_and_old_entries_can_expire() {
        let dir = tempfile::tempdir().unwrap();
        let cache = MetaCache::new(dir.path(), CacheMode::Record);
        let key = json_key(URL, &body("hi"));
        cache.store(&key, &req(), 200, b"good").unwrap();
        assert!(cache.lookup_within(&key, Some(3600)).unwrap().is_some());
        assert!(cache.lookup_within(&key, Some(-1)).unwrap().is_none());
        std::fs::write(shard(&dir.path().join("v1"), &key, ".body"), b"evil").unwrap();
        assert_eq!(cache.lookup(&key).unwrap(), None);
    }
}
