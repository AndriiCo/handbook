//! Recorded-response replay for offline boots (`cargo xtask judge`).
//!
//! [`Recorder`] wraps a live client and appends every successful draft
//! call to a JSONL corpus; [`ReplayClient`] serves those responses back
//! for byte-identical requests, so a machine with no Meta key drafts
//! the exact records `muse-spark-1.3` drafted on the recording run.
//! Requests are keyed by a SHA-256 over everything that shapes the
//! model's input: the model id, the standing-rules template the prompt
//! version selects (hashed, so an edited template misses instead of
//! replaying stale drafts), and the request fields (slug, prompt
//! version, record cap, citation doc ids, input text); `capture_id` is
//! store-local and left out. The corpus is recorded from synthetic
//! evidence only (`evals/authorities`), never customer data.

use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Instant;

use contracts::model::{
    CitationRef, DraftRequest, DraftResponse, ModelClient, ModelError, ProposedRecord, Usage,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

/// The model every corpus line must name (the live drafting model).
pub const REPLAY_MODEL: &str = "muse-spark-1.3";

/// Stable request key for `model`: hex SHA-256 over NUL-separated
/// `replay-key/v2`, the model id, the template's sha256 and
/// [`fields_key`]. Composed, so a v1 corpus line (whose `key` is the
/// fields digest) re-keys from its own `model` and `prompt` columns.
pub fn request_key(req: &DraftRequest, model: &str) -> String {
    compose_key(model, req.prompt_version, &fields_key(req))
}

/// v2 key from a model id, a prompt version and a [`fields_key`].
pub fn compose_key(model: &str, prompt_version: &str, fields: &str) -> String {
    let template = callcache::sha256_hex(crate::model::template_for(prompt_version).as_bytes());
    nul_digest(&["replay-key/v2", model, &template, fields])
}

/// Digest of the request fields alone (the v1 corpus key).
pub fn fields_key(req: &DraftRequest) -> String {
    nul_digest(&[
        req.authority_slug.as_str(),
        req.prompt_version,
        &req.max_records.to_string(),
        &req.doc_ids.join("\u{1f}"),
        req.input_text.as_str(),
    ])
}

fn nul_digest(parts: &[&str]) -> String {
    let mut h = Sha256::new();
    for part in parts {
        h.update(part.as_bytes());
        h.update([0u8]);
    }
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

fn usage_json(u: &Usage) -> Value {
    json!({"input_tokens": u.input_tokens, "output_tokens": u.output_tokens,
           "cached_tokens": u.cached_tokens, "reasoning_tokens": u.reasoning_tokens})
}

fn entry_line(req: &DraftRequest, model: &str, latency_ms: u64, resp: &DraftResponse) -> String {
    let records: Vec<Value> = resp
        .records
        .iter()
        .map(|r| {
            json!({"topic": r.topic, "field": r.field, "value": r.value,
                   "doc_id": r.citation.doc_id, "byte_start": r.citation.byte_start,
                   "byte_end": r.citation.byte_end, "confidence": r.confidence})
        })
        .collect();
    json!({"key": request_key(req, model), "kv": 2, "slug": req.authority_slug,
           "prompt": req.prompt_version,
           "model": model, "latency_ms": latency_ms, "usage": usage_json(&resp.usage),
           "summary": resp.summary, "records": records})
    .to_string()
}

/// One recorded response (what [`ReplayClient`] serves for its key).
#[derive(Debug, Clone)]
struct Entry {
    response: DraftResponse,
}

fn u32_of(v: &Value) -> u32 {
    v.as_u64()
        .map(|n| u32::try_from(n).unwrap_or(u32::MAX))
        .unwrap_or(0)
}

fn parse_line(line: &str) -> Result<(String, Entry), String> {
    let v: Value = serde_json::from_str(line).map_err(|e| format!("bad JSON: {e}"))?;
    let key = v["key"].as_str().ok_or("missing key")?.to_string();
    if v["model"].as_str() != Some(REPLAY_MODEL) {
        return Err(format!("model must be {REPLAY_MODEL}"));
    }
    // v1 lines (no `kv`) keyed the fields alone: re-key them from
    // their own model and prompt columns.
    let key = if v["kv"].as_u64() == Some(2) {
        key
    } else {
        let prompt = v["prompt"].as_str().ok_or("missing prompt")?;
        compose_key(REPLAY_MODEL, prompt, &key)
    };
    let records = v["records"]
        .as_array()
        .ok_or("missing records")?
        .iter()
        .map(|r| {
            Ok(ProposedRecord {
                topic: r["topic"].as_str().ok_or("record topic")?.to_string(),
                field: r["field"].as_str().ok_or("record field")?.to_string(),
                value: r["value"].as_str().ok_or("record value")?.to_string(),
                citation: CitationRef {
                    doc_id: r["doc_id"].as_str().ok_or("record doc_id")?.to_string(),
                    byte_start: u32_of(&r["byte_start"]),
                    byte_end: u32_of(&r["byte_end"]),
                },
                confidence: r["confidence"].as_f64().unwrap_or(0.0) as f32,
            })
        })
        .collect::<Result<Vec<_>, &str>>()?;
    let u = &v["usage"];
    Ok((
        key,
        Entry {
            response: DraftResponse {
                records,
                usage: Usage {
                    input_tokens: u32_of(&u["input_tokens"]),
                    output_tokens: u32_of(&u["output_tokens"]),
                    cached_tokens: u32_of(&u["cached_tokens"]),
                    reasoning_tokens: u32_of(&u["reasoning_tokens"]),
                },
                summary: v["summary"].as_str().unwrap_or("").to_string(),
            },
        },
    ))
}

/// Serves recorded responses. A miss is an error when `strict` (a
/// staff "Draft from this capture" must say why nothing came back),
/// else an empty draft with a stderr note (a boot-time scan keeps going).
pub struct ReplayClient {
    entries: HashMap<String, Entry>,
    strict: bool,
}

impl ReplayClient {
    /// Load a corpus; later lines win for a repeated key.
    pub fn load(path: &Path, strict: bool) -> Result<Self, String> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("replay corpus {}: {e}", path.display()))?;
        let mut entries = HashMap::new();
        for (n, line) in text.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            let (key, entry) =
                parse_line(line).map_err(|e| format!("{}:{}: {e}", path.display(), n + 1))?;
            entries.insert(key, entry);
        }
        Ok(ReplayClient { entries, strict })
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl ModelClient for ReplayClient {
    fn model_id(&self) -> &'static str {
        REPLAY_MODEL
    }

    fn draft_records(&self, req: &DraftRequest) -> Result<DraftResponse, ModelError> {
        if let Some(entry) = self.entries.get(&request_key(req, REPLAY_MODEL)) {
            return Ok(entry.response.clone());
        }
        if self.strict {
            return Err(ModelError::Transport(
                "no recorded Muse Spark response for this input (offline replay); run with a META_API_KEY to draft it live".to_string(),
            ));
        }
        eprintln!(
            "replay: no recorded response for a {} request; drafted nothing",
            req.authority_slug
        );
        Ok(DraftResponse {
            records: Vec::new(),
            usage: Usage {
                input_tokens: 0,
                output_tokens: 0,
                cached_tokens: 0,
                reasoning_tokens: 0,
            },
            summary: String::new(),
        })
    }
}

/// Wraps a live client and appends each successful call to `path`.
pub struct Recorder<C> {
    inner: C,
    path: PathBuf,
}

impl<C: ModelClient> Recorder<C> {
    pub fn new(inner: C, path: PathBuf) -> Self {
        Recorder { inner, path }
    }
}

impl<C: ModelClient> ModelClient for Recorder<C> {
    fn model_id(&self) -> &'static str {
        self.inner.model_id()
    }

    fn draft_records(&self, req: &DraftRequest) -> Result<DraftResponse, ModelError> {
        let started = Instant::now();
        let resp = self.inner.draft_records(req)?;
        let latency_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        let line = entry_line(req, self.inner.model_id(), latency_ms, &resp);
        let written = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .and_then(|mut f| writeln!(f, "{line}"));
        if let Err(e) = written {
            return Err(ModelError::Transport(format!("record replay corpus: {e}")));
        }
        Ok(resp)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixed;
    impl ModelClient for Fixed {
        fn model_id(&self) -> &'static str {
            REPLAY_MODEL
        }
        fn draft_records(&self, req: &DraftRequest) -> Result<DraftResponse, ModelError> {
            Ok(DraftResponse {
                records: vec![ProposedRecord {
                    topic: "fees".to_string(),
                    field: "fee".to_string(),
                    value: req.input_text.chars().take(4).collect(),
                    citation: CitationRef {
                        doc_id: "fixture".to_string(),
                        byte_start: 0,
                        byte_end: 4,
                    },
                    confidence: 0.9,
                }],
                usage: Usage {
                    input_tokens: 120,
                    output_tokens: 30,
                    cached_tokens: 100,
                    reasoning_tokens: 7,
                },
                summary: "one fee".to_string(),
            })
        }
    }

    fn req(text: &str, capture_id: i64) -> DraftRequest {
        DraftRequest {
            authority_slug: "usa-wa-sos".to_string(),
            prompt_version: "aa-prompt-v1",
            input_text: text.to_string(),
            max_records: 8,
            capture_id,
            doc_ids: vec!["fixture".to_string()],
        }
    }

    #[test]
    fn recorded_calls_replay_byte_identical_and_ignore_capture_ids() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("corpus.jsonl");
        let recorder = Recorder::new(Fixed, path.clone());
        let live = recorder.draft_records(&req("$15 per document", 3)).unwrap();
        recorder.draft_records(&req("Olympia office", 4)).unwrap();

        let replay = ReplayClient::load(&path, true).unwrap();
        assert_eq!(replay.len(), 2);
        assert_eq!(replay.model_id(), REPLAY_MODEL);
        // Same input on another store (different capture id) hits.
        assert_eq!(
            replay.draft_records(&req("$15 per document", 99)).unwrap(),
            live
        );
        // Any change to the input misses: strict errors, lenient drafts nothing.
        assert!(replay.draft_records(&req("$16 per document", 3)).is_err());
        let lenient = ReplayClient::load(&path, false).unwrap();
        assert!(
            lenient
                .draft_records(&req("$16 per document", 3))
                .unwrap()
                .records
                .is_empty()
        );
    }

    #[test]
    fn key_covers_every_input_field() {
        let base = req("text", 1);
        let mut other = base.clone();
        other.max_records = 9;
        assert_ne!(
            request_key(&base, REPLAY_MODEL),
            request_key(&other, REPLAY_MODEL)
        );
        let mut other = base.clone();
        other.authority_slug = "vietnam".to_string();
        assert_ne!(
            request_key(&base, REPLAY_MODEL),
            request_key(&other, REPLAY_MODEL)
        );
        let mut other = base.clone();
        other.doc_ids.push("b".to_string());
        assert_ne!(
            request_key(&base, REPLAY_MODEL),
            request_key(&other, REPLAY_MODEL)
        );
        let mut other = base.clone();
        other.prompt_version = "aa-prompt-v2";
        assert_ne!(
            request_key(&base, REPLAY_MODEL),
            request_key(&other, REPLAY_MODEL)
        );
    }

    #[test]
    fn key_covers_the_model_and_the_template() {
        let base = req("text", 1);
        assert_ne!(
            request_key(&base, REPLAY_MODEL),
            request_key(&base, "muse-wonder-1.0")
        );
        // Same fields, other standing rules: a different key.
        let fields = fields_key(&base);
        assert_ne!(
            compose_key(REPLAY_MODEL, "aa-prompt-v1", &fields),
            compose_key(REPLAY_MODEL, crate::model::PROMPT_VERSION_V2, &fields)
        );
    }

    #[test]
    fn v1_corpus_lines_rekey_on_load() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("v1.jsonl");
        let base = req("$15 per document", 3);
        let line = json!({"key": fields_key(&base), "model": REPLAY_MODEL,
                          "prompt": base.prompt_version, "records": [], "summary": "s"});
        std::fs::write(&path, format!("{line}\n")).unwrap();
        let replay = ReplayClient::load(&path, true).unwrap();
        assert_eq!(replay.draft_records(&base).unwrap().summary, "s");
    }

    #[test]
    fn committed_corpus_still_loads() {
        let path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../evals/replay/drafts-spark.jsonl");
        assert!(ReplayClient::load(&path, true).unwrap().len() >= 60);
    }

    #[test]
    fn corpus_refuses_other_models_and_broken_lines() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("c.jsonl");
        std::fs::write(&path, r#"{"key":"k","model":"stub","records":[]}"#).unwrap();
        assert!(ReplayClient::load(&path, true).is_err());
        std::fs::write(&path, "not json\n").unwrap();
        assert!(ReplayClient::load(&path, true).is_err());
    }
}
