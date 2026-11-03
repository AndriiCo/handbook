//! Source fetch with an on-disk cache (reruns skip the network).
//!
//! URL sources live under `<out>/cache/<name>/` (`bytes` +
//! `meta.json` with validators + sha). Pack-relative file paths are
//! read directly. Re-running with a fresh cache prints what it
//! skipped; `--refresh` forces a re-fetch.
//!
//! With a shared [`FetchCache`] (`paths.fetch_cache_dir`), every live
//! fetch is also recorded there (blob + manifest line), a fresh `out/`
//! reuses a recorded answer the source's refresh policy still accepts,
//! and `replay-only` runs from disk with no network at all.

use std::path::{Path, PathBuf};
use std::time::Duration;

use callcache::fetch::Fetched;
use callcache::{CacheMode, FetchCache};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::config::{Refresh, SourceSpec};
use crate::error::{Error, Result};

/// Largest fetch body kept (64 MiB — PDFs can be big).
pub const MAX_BODY_BYTES: u64 = 64 * 1024 * 1024;

/// Default fetch timeout.
pub const FETCH_TIMEOUT: Duration = Duration::from_secs(60);

/// One fetched input, bytes in hand.
#[derive(Debug, Clone)]
pub struct FetchedSource {
    pub name: String,
    pub spec: SourceSpec,
    pub bytes: Vec<u8>,
    /// sha256 hex of `bytes` (cache key + ledger provenance).
    pub sha256: String,
    pub from_cache: bool,
    pub fetched_at: i64,
}

/// Cache validators for one source (`url`, `status` and `headers`
/// arrived with P391; older `meta.json` files read them as absent).
#[derive(Debug, Clone, Serialize, Deserialize)]
struct CacheMeta {
    sha256: String,
    fetched_at: i64,
    etag: Option<String>,
    last_modified: Option<String>,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    status: Option<u16>,
    #[serde(default)]
    headers: Vec<(String, String)>,
}

/// Fetch one source (URL or pack-relative file), recording live URL
/// fetches in `shared` when given (see the module docs).
pub fn fetch_source(
    spec: &SourceSpec,
    pack_dir: &Path,
    cache_dir: &Path,
    force_refresh: bool,
    now: i64,
    shared: Option<&FetchCache>,
) -> Result<FetchedSource> {
    if !spec.url.contains("://") {
        let path = pack_dir.join(&spec.url);
        let bytes = std::fs::read(&path).map_err(|e| {
            Error::Fetch(format!(
                "source '{}': read {}: {e}",
                spec.name,
                path.display()
            ))
        })?;
        let sha256 = sha_hex(&bytes);
        return Ok(FetchedSource {
            name: spec.name.clone(),
            spec: spec.clone(),
            bytes,
            sha256,
            from_cache: false,
            fetched_at: now,
        });
    }
    let dir = cache_dir.join(&spec.name);
    let meta = read_meta(&dir);
    let offline = shared.is_some_and(|c| c.mode() == CacheMode::ReplayOnly);
    let force_refresh =
        !offline && (force_refresh || shared.is_some_and(|c| c.mode() == CacheMode::Refresh));
    let stale = force_refresh
        || meta.as_ref().is_none_or(|m| {
            (!offline && is_stale(m.fetched_at, now, spec.refresh)) || !dir.join("bytes").is_file()
        });
    if !stale {
        let cached = meta.as_ref().expect("checked Some above");
        let bytes = std::fs::read(dir.join("bytes")).map_err(Error::Io)?;
        if sha_hex(&bytes) == cached.sha256 {
            return Ok(FetchedSource {
                name: spec.name.clone(),
                spec: spec.clone(),
                bytes,
                sha256: cached.sha256.clone(),
                from_cache: true,
                fetched_at: cached.fetched_at,
            });
        }
        // Hash mismatch: fall through and re-fetch.
    }
    if let Some(shared) = shared
        && !force_refresh
        && let Some(hit) = shared.latest(&spec.url).filter(|r| r.status == 200)
        && (offline || !is_stale(hit.fetched_at, now, spec.refresh))
    {
        return adopt(spec, &dir, hit);
    }
    if offline {
        return Err(Error::Fetch(format!(
            "source '{}': not in the fetch cache (replay-only)",
            spec.name
        )));
    }
    fetch_url(spec, &dir, meta.as_ref(), now, shared)
}

/// Serve a shared-cache answer and seed this run's own cache with it.
fn adopt(spec: &SourceSpec, dir: &Path, hit: callcache::FetchRecord) -> Result<FetchedSource> {
    write_cache(
        dir,
        &hit.body,
        &CacheMeta {
            sha256: hit.sha256.clone(),
            fetched_at: hit.fetched_at,
            etag: hit.header("etag").map(str::to_string),
            last_modified: hit.header("last-modified").map(str::to_string),
            url: Some(spec.url.clone()),
            status: Some(hit.status),
            headers: hit.headers.clone(),
        },
    )?;
    Ok(FetchedSource {
        name: spec.name.clone(),
        spec: spec.clone(),
        bytes: hit.body,
        sha256: hit.sha256,
        from_cache: true,
        fetched_at: hit.fetched_at,
    })
}

fn write_cache(dir: &Path, bytes: &[u8], meta: &CacheMeta) -> Result<()> {
    std::fs::create_dir_all(dir).map_err(Error::Io)?;
    std::fs::write(dir.join("bytes"), bytes).map_err(Error::Io)?;
    std::fs::write(
        dir.join("meta.json"),
        serde_json::to_string_pretty(meta).expect("meta serializes"),
    )
    .map_err(Error::Io)
}

fn is_stale(fetched_at: i64, now: i64, refresh: Refresh) -> bool {
    match refresh {
        Refresh::Always => true,
        Refresh::Never => false,
        Refresh::Daily => now.saturating_sub(fetched_at) >= 86_400,
        Refresh::Weekly => now.saturating_sub(fetched_at) >= 7 * 86_400,
    }
}

fn read_meta(dir: &Path) -> Option<CacheMeta> {
    let text = std::fs::read_to_string(dir.join("meta.json")).ok()?;
    serde_json::from_str(&text).ok()
}

fn fetch_url(
    spec: &SourceSpec,
    dir: &Path,
    meta: Option<&CacheMeta>,
    now: i64,
    shared: Option<&FetchCache>,
) -> Result<FetchedSource> {
    install_crypto_provider();
    let agent = ureq::Agent::new_with_config(
        ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_global(Some(FETCH_TIMEOUT))
            .build(),
    );
    let mut req = agent.get(&spec.url);
    if let Some(meta) = meta {
        if let Some(etag) = meta.etag.as_deref() {
            req = req.header("If-None-Match", etag);
        }
        if let Some(modified) = meta.last_modified.as_deref() {
            req = req.header("If-Modified-Since", modified);
        }
    }
    let mut response = req.call().map_err(|e| Error::Fetch(err_text(spec, &e)))?;
    let status = response.status().as_u16();
    let headers: Vec<(String, String)> = response
        .headers()
        .iter()
        .filter_map(|(k, v)| {
            Some((
                k.as_str().to_ascii_lowercase(),
                v.to_str().ok()?.to_string(),
            ))
        })
        .collect();
    let record = |body: &[u8]| {
        if let Some(shared) = shared {
            let fetched = Fetched {
                status,
                headers: headers.clone(),
                body: body.to_vec(),
                head: None,
                request: None,
            };
            // Recording is the archive, not the answer: a full disk
            // must not fail a fetch that already succeeded.
            let _ = shared.record(&spec.url, fetched);
        }
    };
    if status == 304 {
        record(b"");
        let meta = meta.expect("304 without a cached entry");
        let bytes = std::fs::read(dir.join("bytes")).map_err(Error::Io)?;
        return Ok(FetchedSource {
            name: spec.name.clone(),
            spec: spec.clone(),
            bytes,
            sha256: meta.sha256.clone(),
            from_cache: true,
            fetched_at: meta.fetched_at,
        });
    }
    if status != 200 {
        record(b"");
        return Err(Error::Fetch(format!(
            "source '{}': HTTP {}",
            spec.name,
            response.status()
        )));
    }
    let etag = response
        .headers()
        .get("ETag")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let last_modified = response
        .headers()
        .get("Last-Modified")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let mut bytes = Vec::new();
    {
        use std::io::Read as _;
        response
            .body_mut()
            .as_reader()
            .take(MAX_BODY_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(Error::Io)?;
    }
    if bytes.len() as u64 > MAX_BODY_BYTES {
        return Err(Error::Fetch(format!(
            "source '{}': body exceeds {MAX_BODY_BYTES} bytes",
            spec.name
        )));
    }
    let sha256 = sha_hex(&bytes);
    record(&bytes);
    write_cache(
        dir,
        &bytes,
        &CacheMeta {
            sha256: sha256.clone(),
            fetched_at: now,
            etag,
            last_modified,
            url: Some(spec.url.clone()),
            status: Some(status),
            headers,
        },
    )?;
    Ok(FetchedSource {
        name: spec.name.clone(),
        spec: spec.clone(),
        bytes,
        sha256,
        from_cache: false,
        fetched_at: now,
    })
}

fn err_text(spec: &SourceSpec, err: &ureq::Error) -> String {
    // ureq errors may echo the URL; keep the source name only.
    let _ = spec;
    match err {
        ureq::Error::Timeout(_) => format!("source '{}': timeout", spec.name),
        other => format!("source '{}': {other}", spec.name),
    }
}

/// sha256 hex of bytes.
pub fn sha_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

/// Cache directory for one pack run.
pub fn cache_dir(out: &Path) -> PathBuf {
    out.join("cache")
}

/// Install the workspace's single rustls provider (aws-lc-rs).
fn install_crypto_provider() {
    let provider = rustls::crypto::aws_lc_rs::default_provider();
    let _ = rustls::crypto::CryptoProvider::install_default(provider);
}

/// Minimal hex encoder (avoids a debug-only dependency).
mod hex {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";

    pub fn encode(bytes: impl AsRef<[u8]>) -> String {
        let bytes = bytes.as_ref();
        let mut out = String::with_capacity(bytes.len() * 2);
        for byte in bytes {
            out.push(DIGITS[(byte >> 4) as usize] as char);
            out.push(DIGITS[(byte & 0x0f) as usize] as char);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::SourceKind;

    fn spec(name: &str, url: &str) -> SourceSpec {
        SourceSpec {
            name: name.to_string(),
            url: url.to_string(),
            kind: SourceKind::Text,
            refresh: Refresh::Never,
        }
    }

    #[test]
    fn local_file_source_reads_directly() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("in.txt"), "hello").unwrap();
        let fetched = fetch_source(
            &spec("a", "in.txt"),
            tmp.path(),
            &tmp.path().join("cache"),
            false,
            1000,
            None,
        )
        .unwrap();
        assert_eq!(fetched.bytes, b"hello");
        assert!(!fetched.from_cache);
        assert_eq!(fetched.sha256.len(), 64);
    }

    /// Port-0 loopback stub answering one GET.
    fn serve_once(body: &'static str) -> String {
        use std::io::{Read as _, Write as _};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/convention.txt", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = [0u8; 4096];
            let _ = stream.read(&mut buf);
            let reply = format!(
                "HTTP/1.1 200 OK\r\nETag: \"e1\"\r\nContent-Length: {}\r\n\
                 Connection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(reply.as_bytes()).unwrap();
        });
        url
    }

    #[test]
    fn url_fetches_land_in_the_shared_cache_and_replay_offline() {
        let tmp = tempfile::tempdir().unwrap();
        let shared = FetchCache::new(tmp.path().join("fetch-cache"), CacheMode::Record);
        let url = serve_once("Article 3");
        let src = spec("conv", &url);
        let live = fetch_source(
            &src,
            tmp.path(),
            &tmp.path().join("a"),
            false,
            1000,
            Some(&shared),
        )
        .unwrap();
        assert!(!live.from_cache);
        let meta: CacheMeta = read_meta(&tmp.path().join("a").join("conv")).unwrap();
        assert_eq!(meta.url.as_deref(), Some(url.as_str()));
        assert_eq!(meta.status, Some(200));
        assert!(
            meta.headers
                .iter()
                .any(|(k, v)| k == "etag" && v == "\"e1\"")
        );
        // A fresh out/ dir reuses the recorded answer (the stub is gone).
        let again = fetch_source(
            &src,
            tmp.path(),
            &tmp.path().join("b"),
            false,
            1001,
            Some(&shared),
        )
        .unwrap();
        assert!(again.from_cache);
        assert_eq!(again.bytes, live.bytes);
        // Replay-only ignores staleness and never dials.
        let offline = FetchCache::new(tmp.path().join("fetch-cache"), CacheMode::ReplayOnly);
        let mut always = src.clone();
        always.refresh = Refresh::Always;
        let replayed = fetch_source(
            &always,
            tmp.path(),
            &tmp.path().join("c"),
            true,
            9_999,
            Some(&offline),
        )
        .unwrap();
        assert_eq!(replayed.bytes, b"Article 3");
        let missing = spec("gone", "http://127.0.0.1:9/none");
        assert!(
            fetch_source(
                &missing,
                tmp.path(),
                &tmp.path().join("c"),
                false,
                1,
                Some(&offline)
            )
            .is_err()
        );
    }

    #[test]
    fn staleness_follows_policy() {
        assert!(is_stale(0, 100, Refresh::Always));
        assert!(!is_stale(0, 100, Refresh::Never));
        assert!(is_stale(0, 86_401, Refresh::Daily));
        assert!(!is_stale(100, 101, Refresh::Daily));
        assert!(is_stale(0, 7 * 86_400 + 1, Refresh::Weekly));
        assert!(!is_stale(0, 7 * 86_400 - 1, Refresh::Weekly));
    }

    #[test]
    fn sha_hex_is_stable() {
        assert_eq!(
            sha_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
