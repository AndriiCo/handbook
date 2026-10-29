//! Source fetch with an on-disk cache (reruns skip the network).
//!
//! URL sources live under `<out>/cache/<name>/` (`bytes` +
//! `meta.json` with validators + sha). Pack-relative file paths are
//! read directly. Re-running with a fresh cache prints what it
//! skipped; `--refresh` forces a re-fetch.

use std::path::{Path, PathBuf};
use std::time::Duration;

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

/// Cache validators for one source.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct CacheMeta {
    sha256: String,
    fetched_at: i64,
    etag: Option<String>,
    last_modified: Option<String>,
}

/// Fetch one source (URL or pack-relative file).
pub fn fetch_source(
    spec: &SourceSpec,
    pack_dir: &Path,
    cache_dir: &Path,
    force_refresh: bool,
    now: i64,
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
    let stale = force_refresh
        || meta.as_ref().is_none_or(|m| {
            is_stale(m.fetched_at, now, spec.refresh) || !dir.join("bytes").is_file()
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
    fetch_url(spec, &dir, meta.as_ref(), now)
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
    if response.status() == 304 {
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
    if response.status() != 200 {
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
    std::fs::create_dir_all(dir).map_err(Error::Io)?;
    std::fs::write(dir.join("bytes"), &bytes).map_err(Error::Io)?;
    std::fs::write(
        dir.join("meta.json"),
        serde_json::to_string_pretty(&CacheMeta {
            sha256: sha256.clone(),
            fetched_at: now,
            etag,
            last_modified,
        })
        .expect("meta serializes"),
    )
    .map_err(Error::Io)?;
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
        )
        .unwrap();
        assert_eq!(fetched.bytes, b"hello");
        assert!(!fetched.from_cache);
        assert_eq!(fetched.sha256.len(), 64);
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
