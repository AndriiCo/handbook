//! Record/replay wrapper over any [`Transport`] (operator decision
//! 2026-11-03: reruns never re-spend Meta budget).
//!
//! `POST` JSON (`/chat/completions`, `/responses`, token counts) and
//! multipart (`/files`, `/asr/transcribe`) go through the shared
//! [`MetaCache`]: a hit returns the stored status + raw bytes, so the
//! client's parsers run unchanged, and never counts toward the daily
//! cap. Each live attempt is admitted and counted per call, so the
//! client's own retries each count once. `GET /responses/{id}` is
//! stored only once the response is terminal (a poll never replays a
//! stale `in_progress`); cancels, deletes and `/models` pass through.
//! Uploaded file ids expire server-side, so a cached `/files` answer
//! is served for at most [`UPLOAD_EXPIRY_SECS`] minus an hour.

use std::sync::Arc;

use callcache::meta::{Request, TextPart, bytes_key, json_key, multipart_key, url_path};
use callcache::{CacheError, CacheMode, MetaCache};
use serde_json::Value;

use crate::error::{Error, Result};
use crate::model::{HttpResponse, MultipartField, MultipartFile, Transport, UPLOAD_EXPIRY_SECS};

/// [`Transport`] that consults `cache` before `inner`.
pub struct CachedTransport<T: Transport> {
    inner: T,
    cache: Arc<MetaCache>,
}

impl<T: Transport> CachedTransport<T> {
    pub fn new(inner: T, cache: Arc<MetaCache>) -> Self {
        Self { inner, cache }
    }

    pub fn cache(&self) -> &MetaCache {
        &self.cache
    }

    /// lookup → admit → live → count → store, for one attempt.
    fn through(
        &self,
        key: &str,
        req: &Request<'_>,
        max_age: Option<i64>,
        live: impl FnOnce() -> Result<HttpResponse>,
    ) -> Result<HttpResponse> {
        if let Some(hit) = self
            .cache
            .lookup_within(key, max_age)
            .map_err(cache_error)?
        {
            return Ok(HttpResponse {
                status: hit.status,
                body: hit.body,
                retry_after_secs: None,
            });
        }
        self.cache.admit(key).map_err(cache_error)?;
        let res = live();
        self.cache.spent(req, res.as_ref().map_or(0, |r| r.status));
        let res = res?;
        // A full disk must not discard an answer already paid for.
        let _ = self.cache.store(key, req, res.status, &res.body);
        Ok(res)
    }
}

fn cache_error(e: CacheError) -> Error {
    match e {
        CacheError::OverBudget { .. } => Error::Budget(e.to_string()),
        other => Error::Model(other.to_string()),
    }
}

/// Background states a later poll can no longer change.
fn terminal(body: &[u8]) -> bool {
    let json: Value = serde_json::from_slice(body).unwrap_or(Value::Null);
    matches!(
        json.get("status").and_then(Value::as_str),
        Some("completed" | "failed" | "cancelled" | "incomplete")
    )
}

impl<T: Transport> Transport for CachedTransport<T> {
    fn post_json(&self, url: &str, bearer: &str, body: &Value) -> Result<HttpResponse> {
        if url_path(url).ends_with("/cancel") {
            return self.inner.post_json(url, bearer, body);
        }
        let req = Request {
            method: "POST",
            url,
            model: body.get("model").and_then(Value::as_str).unwrap_or(""),
        };
        self.through(&json_key(url, body), &req, None, || {
            self.inner.post_json(url, bearer, body)
        })
    }

    fn post_multipart(
        &self,
        url: &str,
        bearer: &str,
        file: &MultipartFile,
        fields: &[MultipartField],
    ) -> Result<HttpResponse> {
        let parts: Vec<TextPart<'_>> = fields
            .iter()
            .map(|f| TextPart {
                name: f.name,
                value: f.value,
                content_type: f.content_type,
            })
            .collect();
        let key = multipart_key(url, file.field, file.content_type, file.bytes, &parts);
        let model = fields
            .iter()
            .find(|f| f.name == "request")
            .and_then(|f| serde_json::from_str::<Value>(f.value).ok())
            .and_then(|v| v.get("model").and_then(Value::as_str).map(str::to_string))
            .unwrap_or_default();
        let req = Request {
            method: "POST",
            url,
            model: &model,
        };
        let max_age = url_path(url)
            .ends_with("/files")
            .then(|| i64::from(UPLOAD_EXPIRY_SECS) - 3600);
        self.through(&key, &req, max_age, || {
            self.inner.post_multipart(url, bearer, file, fields)
        })
    }

    fn get(&self, url: &str, bearer: &str) -> Result<HttpResponse> {
        if !url_path(url).contains("/responses/") {
            return self.inner.get(url, bearer);
        }
        let key = bytes_key("GET", url, b"");
        if let Some(hit) = self.cache.lookup(&key).map_err(cache_error)? {
            return Ok(HttpResponse {
                status: hit.status,
                body: hit.body,
                retry_after_secs: None,
            });
        }
        let res = self.inner.get(url, bearer)?;
        if terminal(&res.body) {
            let req = Request {
                method: "GET",
                url,
                model: "",
            };
            let _ = self.cache.store(&key, &req, res.status, &res.body);
        }
        Ok(res)
    }

    fn delete(&self, url: &str, bearer: &str) -> Result<HttpResponse> {
        if self.cache.mode() == CacheMode::ReplayOnly {
            return Err(Error::Model(
                "replay-only cache: delete needs the network".to_string(),
            ));
        }
        self.inner.delete(url, bearer)
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;
    use crate::model::{BASE_URL, ChatRequest, MetaClient, ModelPort};

    /// Counts calls; answers every POST with one chat completion.
    #[derive(Default)]
    struct Counting {
        posts: Cell<u32>,
        gets: Cell<u32>,
    }

    const COMPLETION: &str = r#"{"choices":[{"message":{"content":"ok"},"finish_reason":"stop"}],
        "usage":{"prompt_tokens":3,"completion_tokens":1}}"#;

    impl Transport for &Counting {
        fn post_json(&self, _url: &str, _bearer: &str, _body: &Value) -> Result<HttpResponse> {
            self.posts.set(self.posts.get() + 1);
            Ok(HttpResponse {
                status: 200,
                body: COMPLETION.as_bytes().to_vec(),
                retry_after_secs: None,
            })
        }
        fn post_multipart(
            &self,
            _url: &str,
            _bearer: &str,
            _file: &MultipartFile,
            _fields: &[MultipartField],
        ) -> Result<HttpResponse> {
            self.posts.set(self.posts.get() + 1);
            Ok(HttpResponse {
                status: 200,
                body: br#"{"id":"file-1"}"#.to_vec(),
                retry_after_secs: None,
            })
        }
        fn get(&self, _url: &str, _bearer: &str) -> Result<HttpResponse> {
            self.gets.set(self.gets.get() + 1);
            let status = if self.gets.get() == 1 {
                "in_progress"
            } else {
                "completed"
            };
            Ok(HttpResponse {
                status: 200,
                body: format!(r#"{{"id":"r1","status":"{status}"}}"#).into_bytes(),
                retry_after_secs: None,
            })
        }
        fn delete(&self, _url: &str, _bearer: &str) -> Result<HttpResponse> {
            Ok(HttpResponse {
                status: 200,
                body: b"{}".to_vec(),
                retry_after_secs: None,
            })
        }
    }

    fn chat() -> ChatRequest {
        ChatRequest {
            model: "muse-spark-1.3".to_string(),
            instructions: "Answer.".to_string(),
            user_text: "fees?".to_string(),
            schema: None,
            max_output_tokens: 64,
            temperature: 0.2,
            reasoning_effort: Some("low".to_string()),
            cache_key: None,
        }
    }

    #[test]
    fn second_identical_chat_makes_zero_transport_calls() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Arc::new(MetaCache::new(dir.path(), CacheMode::Record).with_budget(9));
        let counting = Counting::default();
        let client = MetaClient::new(
            CachedTransport::new(&counting, cache.clone()),
            BASE_URL,
            "k".to_string(),
        );
        let first = client.complete_chat(&chat()).unwrap();
        let second = client.complete_chat(&chat()).unwrap();
        assert_eq!(first.text, second.text);
        assert_eq!(counting.posts.get(), 1);
        assert_eq!(cache.budget().unwrap().used(callcache::unix_now()), 1);

        // Offline rerun: the stored answer, no transport at all.
        let offline = Arc::new(MetaCache::new(dir.path(), CacheMode::ReplayOnly));
        let idle = Counting::default();
        let client = MetaClient::new(CachedTransport::new(&idle, offline), BASE_URL, "k".into());
        assert_eq!(client.complete_chat(&chat()).unwrap().text, first.text);
        let mut other = chat();
        other.user_text.push_str(" (revised)");
        assert!(client.complete_chat(&other).is_err());
        assert_eq!(idle.posts.get(), 0);
    }

    #[test]
    fn polls_cache_only_terminal_answers_and_cap_blocks_posts() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Arc::new(MetaCache::new(dir.path(), CacheMode::Record).with_budget(0));
        let counting = Counting::default();
        let transport = CachedTransport::new(&counting, cache);
        let url = format!("{BASE_URL}/responses/r1");
        assert!(
            String::from_utf8_lossy(&transport.get(&url, "k").unwrap().body)
                .contains("in_progress")
        );
        assert!(
            String::from_utf8_lossy(&transport.get(&url, "k").unwrap().body).contains("completed")
        );
        assert!(
            String::from_utf8_lossy(&transport.get(&url, "k").unwrap().body).contains("completed")
        );
        assert_eq!(counting.gets.get(), 2);
        // Cap 0: no POST leaves the process.
        let body = serde_json::json!({"model": "muse-spark-1.3"});
        let err = transport
            .post_json(&format!("{BASE_URL}/chat/completions"), "k", &body)
            .unwrap_err();
        assert!(matches!(err, Error::Budget(_)));
        assert_eq!(counting.posts.get(), 0);
    }
}
