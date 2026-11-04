//! `precheck.glimmer`: the self-hosted Muse Glimmer document-photo
//! precheck on the plant GPU (P404).
//!
//! The worker holds `~/apostille-pipeline/gpu.lock` for the run (GPU
//! kind). Glimmer itself is served on plant by an OpenAI-compatible
//! server (`ops/open-weights/glimmer-serve.sh`: llama.cpp `llama-server`
//! over the weights in `/srv/scratch/apostille/models`); this handler
//! posts one vision chat per photo to it and returns the public
//! precheck's verdict shape (`evals/captures/demo_precheck/*.json`):
//! `{verdict, issues, boxes, models, recorded_at, seconds}`.
//!
//! Every request and answer goes through the call cache under
//! `<cache_dir>/meta-cache` (key includes the model id), so a rerun
//! replays instead of re-running the GPU. Local calls never count
//! toward the Meta daily cap.
//!
//! Worker env: `GLIMMER_URL` (default `http://127.0.0.1:18480/v1`),
//! `GLIMMER_MODEL` (the served model id, recorded on every verdict).
//! Without `GLIMMER_MODEL` the kind stays out of `hello`, so queued
//! jobs wait `pending` instead of failing.
//!
//! Payload: `{"sample": "<id>", "image_b64": "<png base64>"}` or
//! `{"sample": "<id>", "png": "<repo-relative path>"}`.
//!
//! W-3 fake (`HF_MODE=fake` in worker.env, the worker-side mirror of
//! the server's `hf.mode` row; `cargo xtask up` writes it): no weights
//! and no Glimmer server. The verdict is the recorded file
//! `evals/captures/demo_precheck/<sample>.json`, returned with
//! `judge_backend: "recorded"`; a sample without one fails the job.

use std::time::Instant;

use serde_json::{Value, json};

use super::{JobContext, JobError, JobHandler, JobOutput, WorkerEnv};

const DEFAULT_URL: &str = "http://127.0.0.1:18480/v1";
/// Recorded verdicts the fake answers from (repo-relative).
const RECORDED_DIR: &str = "evals/captures/demo_precheck";

fn fake(env: &WorkerEnv) -> bool {
    env.get("HF_MODE").is_some_and(|m| m.trim() == "fake")
}

/// The recorded verdict for `sample` (W-3 fake): no weights, no server.
fn recorded(ctx: &JobContext, sample: &str) -> Result<JobOutput, JobError> {
    if sample.is_empty()
        || !sample
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err(JobError::fatal("sample must be letters, digits, - or _"));
    }
    let path = ctx.repo.join(RECORDED_DIR).join(format!("{sample}.json"));
    let text = std::fs::read_to_string(&path)
        .map_err(|e| JobError::fatal(format!("no recorded verdict for {sample}: {e}")))?;
    let mut record: Value = serde_json::from_str(&text)
        .map_err(|e| JobError::fatal(format!("recorded verdict is not JSON: {e}")))?;
    record["judge_backend"] = json!("recorded");
    let summary = json!({ "sample": sample, "verdict": record["verdict"] });
    Ok(JobOutput::json(&record, summary))
}

pub struct PrecheckGlimmer;

impl JobHandler for PrecheckGlimmer {
    fn kind(&self) -> &'static str {
        "precheck.glimmer"
    }

    fn gpu(&self) -> bool {
        true
    }

    fn available(&self, env: &WorkerEnv) -> bool {
        fake(env)
            || env
                .get("GLIMMER_MODEL")
                .is_some_and(|m| !m.trim().is_empty())
    }

    fn run(&self, ctx: &JobContext, payload: &Value) -> Result<JobOutput, JobError> {
        if fake(&ctx.env) {
            return recorded(ctx, payload["sample"].as_str().unwrap_or(""));
        }
        let Some(model) = ctx.env.get("GLIMMER_MODEL") else {
            return Err(JobError::fatal(
                "precheck.glimmer needs GLIMMER_MODEL in worker.env",
            ));
        };
        let base = ctx
            .env
            .get("GLIMMER_URL")
            .unwrap_or_else(|| DEFAULT_URL.to_string());
        let sample = payload["sample"].as_str().unwrap_or("sample").to_string();
        let image_b64 = image_of(ctx, payload)?;
        let body = request_body(&model, &sample, &image_b64);
        let cache = callcache::MetaCache::new(
            ctx.cache_dir.join("meta-cache"),
            ctx.env
                .get("META_CACHE_MODE")
                .and_then(|m| callcache::CacheMode::parse(&m))
                .unwrap_or_default(),
        );
        let url = format!("{}/chat/completions", base.trim_end_matches('/'));
        let started = Instant::now();
        let answer = post(&cache, &url, &body)?;
        let verdict = parse_verdict(&answer)?;
        let record = json!({
            "verdict": verdict.0,
            "issues": verdict.1,
            "boxes": [],
            "models": [model],
            "seconds": { model.as_str(): round1(started.elapsed().as_secs_f64()) },
            "judge_backend": "glimmer",
            "recorded_at": iso_now(),
        });
        let summary = json!({ "sample": sample, "verdict": record["verdict"] });
        Ok(JobOutput::json(&record, summary))
    }
}

fn image_of(ctx: &JobContext, payload: &Value) -> Result<String, JobError> {
    use base64::Engine as _;
    if let Some(b64) = payload["image_b64"].as_str() {
        return Ok(b64.to_string());
    }
    let Some(rel) = payload["png"].as_str() else {
        return Err(JobError::fatal("payload needs image_b64 or png"));
    };
    if rel.contains("..") || rel.starts_with('/') {
        return Err(JobError::fatal("png must be a repo-relative path"));
    }
    let bytes = std::fs::read(ctx.repo.join(rel))
        .map_err(|e| JobError::fatal(format!("read {rel}: {e}")))?;
    Ok(base64::engine::general_purpose::STANDARD.encode(bytes))
}

/// The judge request: same rule and schema as the hosted Spark judge
/// in `xtask/src/a2_record.rs`, so both backends grade alike.
pub(crate) fn request_body(model: &str, sample: &str, image_b64: &str) -> Value {
    json!({
        "model": model,
        "messages": [{
            "role": "user",
            "content": [
                {"type": "text", "text": format!(
                    "You judge document-photo prechecks. Sample {sample}. Pass only when a \
                     document page is fully visible with a seal-like mark present and no \
                     crop/darkness defect. Reply with JSON only: \
                     {{\"verdict\": \"pass\"|\"retake\", \"issues\": [short strings]}} \
                     (issues empty when pass)."
                )},
                {"type": "image_url",
                 "image_url": {"url": format!("data:image/png;base64,{image_b64}")}}
            ]
        }],
        "temperature": 0,
        "max_tokens": 400,
        "response_format": {"type": "json_object"},
        "stream": false
    })
}

fn post(cache: &callcache::MetaCache, url: &str, body: &Value) -> Result<Value, JobError> {
    let key = callcache::meta::json_key(url, body);
    if let Ok(Some(hit)) = cache.lookup(&key) {
        return serde_json::from_slice(&hit.body)
            .map_err(|e| JobError::fatal(format!("cached answer is not JSON: {e}")));
    }
    let text = body.to_string();
    let mut res = ureq::post(url)
        .header("content-type", "application/json")
        .send(text.as_bytes())
        .map_err(|e| JobError::retry(format!("glimmer server: {e}")))?;
    let status = res.status().as_u16();
    let bytes = res
        .body_mut()
        .read_to_vec()
        .map_err(|e| JobError::retry(format!("glimmer body: {e}")))?;
    if !(200..300).contains(&status) {
        return Err(JobError::retry(format!("glimmer HTTP {status}")));
    }
    let model = body["model"].as_str().unwrap_or("");
    let req = callcache::meta::Request {
        method: "POST",
        url,
        model,
    };
    let _ = cache.store(&key, &req, status, &bytes);
    serde_json::from_slice(&bytes).map_err(|e| JobError::fatal(format!("answer is not JSON: {e}")))
}

/// `(verdict, issues)` from an OpenAI-shaped chat answer whose content
/// is the verdict JSON (fenced or bare).
pub(crate) fn parse_verdict(answer: &Value) -> Result<(String, Vec<String>), JobError> {
    let content = answer["choices"][0]["message"]["content"]
        .as_str()
        .ok_or_else(|| JobError::fatal("answer has no message content"))?;
    let start = content.find('{');
    let end = content.rfind('}');
    let (Some(start), Some(end)) = (start, end) else {
        return Err(JobError::fatal("verdict is not JSON"));
    };
    let verdict: Value = serde_json::from_str(&content[start..=end])
        .map_err(|e| JobError::fatal(format!("verdict is not JSON: {e}")))?;
    let label = verdict["verdict"].as_str().unwrap_or("");
    if !matches!(label, "pass" | "retake") {
        return Err(JobError::fatal(format!(
            "verdict must be pass|retake, got {label:?}"
        )));
    }
    let issues = verdict["issues"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|i| i.as_str().map(str::to_string))
                .take(8)
                .collect()
        })
        .unwrap_or_default();
    Ok((label.to_string(), issues))
}

fn round1(secs: f64) -> f64 {
    (secs * 10.0).round() / 10.0
}

fn iso_now() -> String {
    let secs = crate::unix_now();
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rem / 3_600,
        rem % 3_600 / 60,
        rem % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_fenced_and_bare_verdicts() {
        let fenced = json!({"choices": [{"message": {"content":
            "```json\n{\"verdict\": \"retake\", \"issues\": [\"glare top-right\"]}\n```"}}]});
        assert_eq!(
            parse_verdict(&fenced).unwrap(),
            ("retake".to_string(), vec!["glare top-right".to_string()])
        );
        let bare = json!({"choices": [{"message": {"content":
            "{\"verdict\":\"pass\",\"issues\":[]}"}}]});
        assert_eq!(parse_verdict(&bare).unwrap().0, "pass");
        let bad = json!({"choices": [{"message": {"content": "{\"verdict\":\"maybe\"}"}}]});
        assert!(parse_verdict(&bad).is_err());
    }

    #[test]
    fn request_names_the_model_and_carries_the_image() {
        let body = request_body("glimmer-test", "syn-doc-clean", "QUJD");
        assert_eq!(body["model"], "glimmer-test");
        let url = body["messages"][0]["content"][1]["image_url"]["url"]
            .as_str()
            .unwrap();
        assert_eq!(url, "data:image/png;base64,QUJD");
    }

    #[test]
    fn announced_when_worker_env_names_the_model() {
        let env = WorkerEnv::parse("GLIMMER_MODEL=glimmer-test\n");
        assert!(PrecheckGlimmer.available(&env));
        if std::env::var_os("GLIMMER_MODEL").is_none() && std::env::var_os("HF_MODE").is_none() {
            assert!(!PrecheckGlimmer.available(&WorkerEnv::default()));
        }
    }

    #[test]
    fn the_fake_answers_from_the_recorded_verdicts_without_a_server() {
        let env = WorkerEnv::parse("HF_MODE=fake\n");
        assert!(PrecheckGlimmer.available(&env), "no GLIMMER_MODEL needed");
        let tmp = tempfile::tempdir().unwrap();
        let ctx = JobContext {
            id: 1,
            dir: tmp.path().to_path_buf(),
            repo: std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(".."),
            cache_dir: tmp.path().to_path_buf(),
            env,
        };
        let out = PrecheckGlimmer
            .run(
                &ctx,
                &json!({"sample": "syn-doc-clean", "png": "nowhere.png"}),
            )
            .unwrap();
        let record: Value = serde_json::from_slice(&out.bytes).unwrap();
        assert_eq!(record["verdict"], "pass");
        assert_eq!(record["judge_backend"], "recorded");
        for bad in ["../secrets", "no-such-sample", ""] {
            assert!(
                PrecheckGlimmer.run(&ctx, &json!({"sample": bad})).is_err(),
                "{bad}"
            );
        }
    }

    #[test]
    fn iso_now_is_a_utc_timestamp() {
        let now = iso_now();
        assert_eq!(now.len(), 20);
        assert!(now.ends_with('Z') && now.starts_with("20"), "{now}");
    }
}
