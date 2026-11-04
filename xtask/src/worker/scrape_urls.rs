//! `scrape.urls`: fetch public URLs with the repo's own fetch code
//! (`ingest::fetch`, robots gate included) and return the raw bytes.
//!
//! Payload `{"urls": ["https://…"], "politeness_ms": 2000}` (≤ 50 URLs).
//! Result `{"pages": [{url, status, sha256, bytes, body_b64} | {url,
//! error}], "truncated": bool}`; every body is also kept under the
//! job dir as `pages/<sha256>` for provenance. Bodies stop riding in
//! the result once [`MAX_INLINE_BYTES`] is reached (the entry keeps
//! its sha and size; `truncated` says so).

use std::collections::HashMap;
use std::time::{Duration, Instant};

use base64::Engine as _;
use serde_json::{Value, json};

use super::{JobContext, JobError, JobHandler, JobOutput};

const MAX_URLS: usize = 50;
/// Raw body budget inside one result (base64 grows it by a third, and
/// the server caps decoded results at 16 MiB).
const MAX_INLINE_BYTES: usize = 11 * 1024 * 1024;
const FETCH_TIMEOUT: Duration = Duration::from_secs(30);
const DEFAULT_POLITENESS_MS: u64 = 2000;

pub struct ScrapeUrls;

impl JobHandler for ScrapeUrls {
    fn kind(&self) -> &'static str {
        "scrape.urls"
    }

    fn run(&self, ctx: &JobContext, payload: &Value) -> Result<JobOutput, JobError> {
        let urls = parse_urls(payload)?;
        let politeness = Duration::from_millis(
            payload["politeness_ms"]
                .as_u64()
                .unwrap_or(DEFAULT_POLITENESS_MS)
                .min(60_000),
        );
        let pages_dir = ctx.dir.join("pages");
        std::fs::create_dir_all(&pages_dir).map_err(|e| JobError::retry(e.to_string()))?;
        let mut last_hit: HashMap<String, Instant> = HashMap::new();
        let mut robots: HashMap<String, bool> = HashMap::new();
        let (mut pages, mut inline, mut failed, mut truncated) = (Vec::new(), 0usize, 0, false);
        for url in &urls {
            let host = host_of(url);
            if let Some(at) = last_hit.get(&host) {
                let since = at.elapsed();
                if since < politeness {
                    std::thread::sleep(politeness - since);
                }
            }
            let allowed = *robots
                .entry(host.clone())
                .or_insert_with(|| ingest::fetch::check_robots(url, FETCH_TIMEOUT).allowed);
            last_hit.insert(host, Instant::now());
            if !allowed {
                failed += 1;
                pages.push(json!({"url": url, "error": "robots disallow"}));
                continue;
            }
            match ingest::fetch::fetch_url(url, FETCH_TIMEOUT) {
                Ok(fetched) => {
                    let sha = handbook_store::store::sha256_hex(&fetched.bytes);
                    let _ = std::fs::write(pages_dir.join(&sha), &fetched.bytes);
                    let mut page = json!({
                        "url": url, "status": fetched.status, "sha256": sha,
                        "bytes": fetched.bytes.len(),
                    });
                    if inline + fetched.bytes.len() <= MAX_INLINE_BYTES {
                        inline += fetched.bytes.len();
                        page["body_b64"] =
                            json!(base64::engine::general_purpose::STANDARD.encode(&fetched.bytes));
                    } else {
                        truncated = true;
                    }
                    pages.push(page);
                }
                Err(e) => {
                    failed += 1;
                    pages.push(json!({"url": url, "error": e.to_string()}));
                }
            }
        }
        let summary = json!({"pages": pages.len(), "failed": failed, "truncated": truncated});
        Ok(JobOutput::json(
            &json!({"pages": pages, "truncated": truncated}),
            summary,
        ))
    }
}

fn parse_urls(payload: &Value) -> Result<Vec<String>, JobError> {
    let Some(list) = payload["urls"].as_array() else {
        return Err(JobError::fatal(
            "payload.urls must be an array of http(s) URLs",
        ));
    };
    if list.len() > MAX_URLS {
        return Err(JobError::fatal(format!("at most {MAX_URLS} URLs per job")));
    }
    list.iter()
        .map(|v| match v.as_str() {
            Some(url) if url.starts_with("https://") || url.starts_with("http://") => {
                Ok(url.to_string())
            }
            _ => Err(JobError::fatal(format!("not an http(s) URL: {v}"))),
        })
        .collect()
}

fn host_of(url: &str) -> String {
    url.split("://")
        .nth(1)
        .and_then(|rest| rest.split(['/', '?', '#']).next())
        .unwrap_or("")
        .to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payload_is_validated() {
        assert!(parse_urls(&json!({})).is_err());
        assert!(parse_urls(&json!({"urls": ["ftp://x"]})).is_err());
        let many: Vec<String> = (0..51)
            .map(|i| format!("https://example.com/{i}"))
            .collect();
        assert!(parse_urls(&json!({"urls": many})).is_err());
        assert_eq!(
            parse_urls(&json!({"urls": ["https://example.com/a"]})).unwrap(),
            ["https://example.com/a"]
        );
        assert_eq!(host_of("https://Example.com:8443/a?b"), "example.com:8443");
    }

    #[test]
    fn fetch_failures_are_reported_not_fatal() {
        // A loopback port nothing listens on: the fetch fails fast.
        let probe = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://127.0.0.1:{}/x", probe.local_addr().unwrap().port());
        drop(probe);
        let dir = tempfile::tempdir().unwrap();
        let ctx = JobContext {
            id: 1,
            dir: dir.path().join("job"),
            repo: super::super::repo_root(),
            cache_dir: dir.path().to_path_buf(),
            settings: super::super::Settings::default(),
        };
        let out = ScrapeUrls
            .run(&ctx, &json!({"urls": [url], "politeness_ms": 0}))
            .unwrap();
        let result: Value = serde_json::from_slice(&out.bytes).unwrap();
        assert!(result["pages"][0]["error"].is_string());
        assert_eq!(out.summary["failed"], 1);
    }
}
