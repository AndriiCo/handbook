//! `cargo xtask a2-record --png-dir <dir> --out-dir <dir>`: record the
//! public photo-precheck demo verdicts (A2) from live Meta models.
//!
//! For each `syn-doc-*.png`, SAM 3.1 segments one bare noun phrase per
//! request ([`SAM_PROMPTS`]; lists return zero matches) on
//! `/responses`, then Muse Spark judges pass/retake from the pixels
//! plus the SAM boxes on chat (shapes pinned in
//! `docs/meta-api-shapes.md`). One `<sample>.json` per sample lands in
//! `--out-dir`; `xtask/ops/ops-a2-record.sh` installs them into
//! `<store>/demo_precheck/`, which the web tier serves verbatim
//! (`crates/web/src/demo_precheck.rs`): the public page never calls a
//! model.
//!
//! Every call goes through the Meta call cache
//! (`docs/ops/call-cache.md`; `--meta-cache`, `--cache-mode`,
//! `--daily-call-cap`, `--store`), so a rerun replays the recorded
//! answers instead of re-spending, and `--cache-mode replay-only` runs
//! offline with no key. Retries follow `xtask ask`: up to 4 attempts
//! honouring `Retry-After` on 429/5xx, none on terminal statuses.
//!
//! `--sam-backend open-weights` takes the boxes from the local SAM 3
//! weights on plant instead (`ops/open-weights/sam3_boxes.py`, run by
//! `--sam3-python`, e.g. `~/sam3-venv/bin/python`); `--mask "faces,
//! license plates"` blackens those boxes before Spark sees the pixels.
//! Spark still judges over the hosted API.
//!
//! Key: `$META_API_KEY` only (never a file, never a flag).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use atlas::cached::CachedTransport;
use atlas::model::{Transport, UreqTransport};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::flag;

/// Hosted segmenter (Meta only: model-gate).
pub const SAM_MODEL: &str = media::redact::SAM_MODEL_VERSION;
/// Open-weights segmenter on plant (`--sam-backend open-weights`).
pub const SAM3_OPEN_MODEL: &str = "facebook/sam3";
/// Judge.
pub const SPARK_MODEL: &str = "muse-spark-1.3";
/// One bare noun phrase per request; each concept gets its own call.
pub const SAM_PROMPTS: [&str; 4] = ["document page", "official seal", "embossed stamp", "face"];
/// Retry budget per POST (error-handling docs: 3-5 attempts).
const MAX_ATTEMPTS: u32 = 4;
/// Spark reasons inside this budget; `length` fails closed.
const MAX_COMPLETION_TOKENS: u32 = 4_000;
/// At most this many issues per verdict reach the public page.
const MAX_ISSUES: usize = 8;

/// Where the boxes come from.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Backend {
    Hosted,
    OpenWeights {
        python: String,
        script: PathBuf,
        mask: String,
    },
}

impl Backend {
    fn name(&self) -> &'static str {
        match self {
            Backend::Hosted => "hosted",
            Backend::OpenWeights { .. } => "open-weights",
        }
    }

    fn model(&self) -> &'static str {
        match self {
            Backend::Hosted => SAM_MODEL,
            Backend::OpenWeights { .. } => SAM3_OPEN_MODEL,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Options {
    png_dir: PathBuf,
    out_dir: PathBuf,
    base: String,
    backend: Backend,
}

impl Options {
    fn parse(args: &[String]) -> Result<Self, String> {
        let png_dir = flag(args, "png-dir").ok_or("--png-dir is required")?;
        let out_dir = flag(args, "out-dir").ok_or("--out-dir is required")?;
        let mask = flag(args, "mask").unwrap_or_default();
        let backend = match flag(args, "sam-backend").as_deref() {
            None | Some("hosted") => {
                if !mask.is_empty() {
                    return Err("--mask needs --sam-backend open-weights \
                                (masking runs inside sam3_boxes.py)"
                        .to_string());
                }
                Backend::Hosted
            }
            Some("open-weights") => Backend::OpenWeights {
                python: flag(args, "sam3-python").unwrap_or_else(|| "python3".to_string()),
                script: PathBuf::from(
                    flag(args, "sam3-script")
                        .unwrap_or_else(|| "ops/open-weights/sam3_boxes.py".to_string()),
                ),
                mask,
            },
            Some(other) => {
                return Err(format!(
                    "--sam-backend {other}: want hosted or open-weights"
                ));
            }
        };
        Ok(Self {
            png_dir: PathBuf::from(png_dir),
            out_dir: PathBuf::from(out_dir),
            base: flag(args, "base").unwrap_or_else(|| atlas::model::BASE_URL.to_string()),
            backend,
        })
    }
}

/// One box in the public record: concept label, SAM object id, box as
/// fractions of the image (4 decimals), confidence. `sam3_boxes.py`
/// writes the same shape, so both backends read alike downstream.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PrecheckBox {
    pub label: String,
    pub object_id: u32,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub score: f64,
}

impl PrecheckBox {
    /// A hosted SAM record tagged with its prompt; `None` when it falls
    /// outside the image once rounded.
    fn from_sam(label: &str, sam: &media::SamBox) -> Option<Self> {
        let b = Self {
            label: label.to_string(),
            object_id: sam.object_id,
            x: round_to(sam.x, 4),
            y: round_to(sam.y, 4),
            w: round_to(sam.w, 4),
            h: round_to(sam.h, 4),
            score: sam.score,
        };
        b.is_inside().then_some(b)
    }

    /// Origin inside the image and a positive area (the endpoint's
    /// contract for overlay boxes).
    fn is_inside(&self) -> bool {
        (0.0..=1.0).contains(&self.x)
            && (0.0..=1.0).contains(&self.y)
            && self.w > 0.0
            && self.h > 0.0
    }
}

fn round_to(value: f64, places: i32) -> f64 {
    let scale = 10f64.powi(places);
    (value * scale).round() / scale
}

/// Boxes for one sample plus what produced them.
#[derive(Debug, Default)]
struct Segmentation {
    boxes: Vec<PrecheckBox>,
    counts: BTreeMap<String, usize>,
    secs: f64,
    /// Masked copy of the PNG (open weights + `--mask`).
    masked_png: Option<PathBuf>,
}

/// Spark's judgment.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Verdict {
    verdict: String,
    issues: Vec<String>,
    prompt_tokens: u64,
    completion_tokens: u64,
}

/// `cargo xtask a2-record`.
pub(crate) fn run(args: &[String]) {
    let opts = Options::parse(args).unwrap_or_else(|e| {
        eprintln!("ABORT: {e}");
        std::process::exit(2);
    });
    let cache = crate::call_cache::meta_cache(args);
    let key = std::env::var("META_API_KEY").unwrap_or_default();
    if key.is_empty() && cache.mode() != callcache::CacheMode::ReplayOnly {
        eprintln!("ABORT: META_API_KEY is not set (export it in this shell only)");
        std::process::exit(1);
    }
    let transport = CachedTransport::new(UreqTransport::default(), Arc::clone(&cache));
    let result = record_all(&transport, &key, &opts);
    println!("{}", cache.summary());
    match result {
        Ok((count, tokens_in, tokens_out)) => println!(
            "DONE: {count} recording(s) tokens in={tokens_in} out={tokens_out} \
             — paste this whole output back to the agent"
        ),
        Err(e) => {
            eprintln!("ABORT: {e}");
            std::process::exit(1);
        }
    }
}

/// Record every sample; returns (recordings, prompt tokens, completion
/// tokens). Nothing is written for a sample whose calls fail.
fn record_all<T: Transport>(
    transport: &T,
    key: &str,
    opts: &Options,
) -> Result<(usize, u64, u64), String> {
    let pngs = sample_pngs(&opts.png_dir)?;
    if let Backend::OpenWeights { script, .. } = &opts.backend {
        if !script.is_file() {
            return Err(format!(
                "sam3_boxes.py not found at {} (--sam3-script to override)",
                script.display()
            ));
        }
        println!(
            "sam backend: open-weights ({SAM3_OPEN_MODEL} via {})",
            script.display()
        );
    }
    std::fs::create_dir_all(&opts.out_dir)
        .map_err(|e| format!("create {}: {e}", opts.out_dir.display()))?;
    let (mut tokens_in, mut tokens_out) = (0, 0);
    for png in &pngs {
        let record = record_one(transport, key, opts, png)?;
        tokens_in += record["usage"]["prompt_tokens"].as_u64().unwrap_or(0);
        tokens_out += record["usage"]["completion_tokens"].as_u64().unwrap_or(0);
        let sample = record_sample(png);
        let dest = opts.out_dir.join(format!("{sample}.json"));
        let text = serde_json::to_string_pretty(&record).map_err(|e| e.to_string())?;
        std::fs::write(&dest, text + "\n").map_err(|e| format!("write {}: {e}", dest.display()))?;
        println!("  wrote {}", dest.display());
    }
    Ok((pngs.len(), tokens_in, tokens_out))
}

/// `syn-doc-*.png` in `dir`, sorted.
fn sample_pngs(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let entries = std::fs::read_dir(dir).map_err(|e| format!("read {}: {e}", dir.display()))?;
    let mut pngs: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|path| {
            path.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("syn-doc-") && n.ends_with(".png"))
        })
        .collect();
    pngs.sort();
    if pngs.is_empty() {
        return Err(format!("no syn-doc-*.png in {}", dir.display()));
    }
    Ok(pngs)
}

fn record_sample(png: &Path) -> String {
    png.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// SAM then Spark for one sample; returns the public record.
fn record_one<T: Transport>(
    transport: &T,
    key: &str,
    opts: &Options,
    png: &Path,
) -> Result<Value, String> {
    let sample = record_sample(png);
    println!("== {sample}");
    let image = std::fs::read(png).map_err(|e| format!("read {}: {e}", png.display()))?;
    if !image.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err(format!("{}: not a PNG", png.display()));
    }
    let seg = match &opts.backend {
        Backend::Hosted => sam_boxes(transport, key, &opts.base, &image)?,
        Backend::OpenWeights {
            python,
            script,
            mask,
        } => sam_boxes_via_script(
            python,
            script,
            mask,
            png,
            &opts.out_dir.join(".sam3-scratch"),
        )?,
    };
    println!(
        "  sam: {} box(es) {:?} in {:.1}s",
        seg.boxes.len(),
        seg.counts,
        seg.secs
    );
    if sample == "syn-doc-clean" && seg.boxes.is_empty() {
        return Err(format!(
            "SAM returned 0 boxes on the clean sample (counts={:?}) — the model or the \
             image changed; fix the prompt/image before recording",
            seg.counts
        ));
    }
    let mut masked = Vec::new();
    let judged = match &seg.masked_png {
        Some(path) => {
            for label in ["face", "license plate"] {
                if seg.counts.get(label).copied().unwrap_or(0) > 0 {
                    masked.push(label);
                }
            }
            std::fs::read(path).map_err(|e| format!("read {}: {e}", path.display()))?
        }
        None => image,
    };
    let label = sample.replace('-', " ");
    let started = Instant::now();
    let verdict = spark_verdict(
        transport, key, &opts.base, &sample, &label, &seg.boxes, &judged,
    )?;
    let spark_secs = started.elapsed().as_secs_f64();
    println!(
        "  spark: {} {:?} in {spark_secs:.1}s (in={} out={})",
        verdict.verdict, verdict.issues, verdict.prompt_tokens, verdict.completion_tokens
    );
    let sam_model = opts.backend.model();
    let mut seconds = serde_json::Map::new();
    seconds.insert(sam_model.to_string(), json!(round_to(seg.secs, 1)));
    seconds.insert(SPARK_MODEL.to_string(), json!(round_to(spark_secs, 1)));
    Ok(json!({
        "verdict": verdict.verdict,
        "issues": verdict.issues,
        "boxes": seg.boxes,
        "models": [sam_model, SPARK_MODEL],
        "seconds": seconds,
        "usage": {
            "prompt_tokens": verdict.prompt_tokens,
            "completion_tokens": verdict.completion_tokens,
        },
        "sam_counts": seg.counts,
        "sam_backend": opts.backend.name(),
        "masked": masked,
        "recorded_at": utc_stamp(callcache::unix_now()),
    }))
}

/// Hosted SAM 3.1: one request per [`SAM_PROMPTS`] concept, matches
/// merged across requests.
fn sam_boxes<T: Transport>(
    transport: &T,
    key: &str,
    base: &str,
    image: &[u8],
) -> Result<Segmentation, String> {
    let url = format!("{}{}", base.trim_end_matches('/'), media::redact::SAM_PATH);
    let started = Instant::now();
    let mut seg = Segmentation::default();
    for prompt in SAM_PROMPTS {
        let what = format!("SAM call for {prompt:?}");
        let answer = post(
            transport,
            &url,
            key,
            &media::sam_request_body(prompt, image),
            &what,
        )?;
        let found: Vec<PrecheckBox> = media::parse_sam_answer(&answer)
            .map_err(|e| format!("{what}: {e}"))?
            .iter()
            .filter_map(|b| PrecheckBox::from_sam(prompt, b))
            .collect();
        seg.counts.insert(prompt.to_string(), found.len());
        seg.boxes.extend(found);
    }
    seg.secs = started.elapsed().as_secs_f64();
    Ok(seg)
}

/// Open-weights SAM 3: one `sam3_boxes.py` run per sample (the model
/// loads once per run), boxes plus an optional masked PNG.
fn sam_boxes_via_script(
    python: &str,
    script: &Path,
    mask: &str,
    png: &Path,
    workdir: &Path,
) -> Result<Segmentation, String> {
    std::fs::create_dir_all(workdir).map_err(|e| format!("create {}: {e}", workdir.display()))?;
    let stem = record_sample(png);
    let boxes_path = workdir.join(format!("{stem}.boxes.json"));
    let mut cmd = std::process::Command::new(python);
    cmd.arg(script)
        .arg("--image")
        .arg(png)
        .arg("--out")
        .arg(&boxes_path);
    for prompt in SAM_PROMPTS {
        cmd.arg("--prompt").arg(prompt);
    }
    let masked_png = (!mask.is_empty()).then(|| workdir.join(format!("{stem}.masked.png")));
    if let Some(path) = &masked_png {
        cmd.arg("--mask").arg(mask).arg("--masked-out").arg(path);
    }
    let started = Instant::now();
    let output = cmd
        .output()
        .map_err(|e| format!("run {python} {}: {e}", script.display()))?;
    let secs = started.elapsed().as_secs_f64();
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let tail: String = stderr
            .chars()
            .rev()
            .take(2_000)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        return Err(format!("sam3_boxes.py failed ({}): {tail}", output.status));
    }
    let bytes = std::fs::read(&boxes_path)
        .map_err(|e| format!("sam3_boxes.py wrote no boxes file: {e}"))?;
    let boxes: Vec<PrecheckBox> = serde_json::from_slice(&bytes)
        .map_err(|e| format!("sam3_boxes.py wrote no boxes array: {e}"))?;
    let mut counts = BTreeMap::new();
    for b in &boxes {
        if !b.is_inside() {
            return Err(format!(
                "sam3_boxes.py wrote a box outside the image: {b:?}"
            ));
        }
        *counts.entry(b.label.clone()).or_insert(0) += 1;
    }
    Ok(Segmentation {
        boxes,
        counts,
        secs,
        masked_png,
    })
}

/// The judge request: the pixels decide (flat synthetic shapes ground
/// to zero boxes), the SAM boxes are evidence.
fn spark_body(sample: &str, label: &str, boxes: &[PrecheckBox], image: &[u8]) -> Value {
    use base64::Engine as _;
    let schema = json!({
        "type": "object",
        "properties": {
            "verdict": {"type": "string", "enum": ["pass", "retake"]},
            "issues": {"type": "array", "items": {"type": "string"}},
        },
        "required": ["verdict", "issues"],
        "additionalProperties": false,
    });
    let boxes = serde_json::to_string(boxes).unwrap_or_else(|_| "[]".to_string());
    let text = format!(
        "You judge document-photo prechecks from the image plus SAM box evidence. Reply \
         with the schema only. Sample {sample} ({label}). SAM 3.1 found these boxes \
         (fractions of image width/height): {boxes}. Judge: pass only when a document \
         page is fully visible with a seal-like mark present and no crop/darkness defect. \
         Reply with verdict + short issues[] (empty when pass)."
    );
    let image_url = format!(
        "data:image/png;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(image)
    );
    json!({
        "model": SPARK_MODEL,
        "messages": [{"role": "user", "content": [
            {"type": "text", "text": text},
            {"type": "image_url", "image_url": {"url": image_url}},
        ]}],
        "reasoning_effort": "low",
        "max_completion_tokens": MAX_COMPLETION_TOKENS,
        "response_format": {"type": "json_schema",
                            "json_schema": {"name": "PrecheckVerdict", "schema": schema}},
        "prompt_cache_key": "demo-precheck-v1",
        "stream": false,
    })
}

fn spark_verdict<T: Transport>(
    transport: &T,
    key: &str,
    base: &str,
    sample: &str,
    label: &str,
    boxes: &[PrecheckBox],
    image: &[u8],
) -> Result<Verdict, String> {
    let url = format!("{}/chat/completions", base.trim_end_matches('/'));
    let body = spark_body(sample, label, boxes, image);
    let answer = post(transport, &url, key, &body, "Spark call")?;
    parse_verdict(&answer)
}

/// Read Spark's verdict; truncation, refusals and any other finish
/// reason fail closed (never parse partial JSON).
fn parse_verdict(answer: &Value) -> Result<Verdict, String> {
    let choice = answer["choices"]
        .get(0)
        .ok_or("Spark call failed: no choice in the answer")?;
    let finish = choice["finish_reason"].as_str().unwrap_or("");
    if finish == "length" {
        return Err(
            "Spark call failed: truncated (finish_reason=length) — raise \
                    max_completion_tokens, never parse the partial JSON"
                .to_string(),
        );
    }
    if let Some(refusal) = choice["message"]["refusal"]
        .as_str()
        .filter(|r| !r.is_empty())
    {
        return Err(format!("Spark call failed: model refused: {refusal}"));
    }
    if finish != "stop" {
        return Err(format!("Spark call failed: finish_reason={finish}"));
    }
    let bad = |why: &str| format!("Spark call failed: bad verdict shape: {why}");
    let content = choice["message"]["content"]
        .as_str()
        .ok_or_else(|| bad("no content"))?;
    let parsed: Value = serde_json::from_str(content).map_err(|e| bad(&e.to_string()))?;
    let verdict = parsed["verdict"]
        .as_str()
        .filter(|v| matches!(*v, "pass" | "retake"))
        .ok_or_else(|| bad("verdict is not pass/retake"))?;
    let issues = parsed["issues"]
        .as_array()
        .ok_or_else(|| bad("issues is not a list"))?
        .iter()
        .take(MAX_ISSUES)
        .map(|i| i.as_str().map_or_else(|| i.to_string(), str::to_string))
        .collect();
    Ok(Verdict {
        verdict: verdict.to_string(),
        issues,
        prompt_tokens: answer["usage"]["prompt_tokens"].as_u64().unwrap_or(0),
        completion_tokens: answer["usage"]["completion_tokens"].as_u64().unwrap_or(0),
    })
}

/// One JSON POST with the `xtask ask` retry discipline; non-2xx after
/// the retries is an error carrying the first 500 bytes of the answer.
fn post<T: Transport>(
    transport: &T,
    url: &str,
    key: &str,
    body: &Value,
    what: &str,
) -> Result<Value, String> {
    let mut attempt_no = 0u32;
    loop {
        let response = transport
            .post_json(url, key, body)
            .map_err(|e| format!("{what} failed: {e}"))?;
        if crate::ask::retryable(response.status) && attempt_no < MAX_ATTEMPTS - 1 {
            attempt_no += 1;
            std::thread::sleep(crate::ask::retry_delay(
                attempt_no,
                response.retry_after_secs,
            ));
            continue;
        }
        if !(200..300).contains(&response.status) {
            let text = String::from_utf8_lossy(&response.body);
            let text: String = text.chars().take(500).collect();
            return Err(format!("{what} failed: HTTP {}: {text}", response.status));
        }
        return serde_json::from_slice(&response.body)
            .map_err(|e| format!("{what} failed: answer is not JSON: {e}"));
    }
}

/// `YYYY-MM-DDTHH:MM:SSZ` (UTC).
fn utc_stamp(unix: i64) -> String {
    let secs = unix.rem_euclid(86_400);
    format!(
        "{}T{:02}:{:02}:{:02}Z",
        callcache::utc_date(unix),
        secs / 3_600,
        (secs % 3_600) / 60,
        secs % 60
    )
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    use atlas::model::{HttpResponse, MultipartField, MultipartFile};
    use callcache::{CacheMode, MetaCache};

    use super::*;

    /// One `document page` box (pixels 10..110 x 20..220 of 400x800).
    const PAGE_TOKENS: &str = "<0f>0<|box;x1=10;y1=20;x2=110;y2=220;w=400;h=800;c=0.9|>\
        <|mask;x=0;y=0;data=27,60,~a,b|>";

    /// Fake Meta API: SAM answers [`PAGE_TOKENS`] for `document page`
    /// and nothing else; Spark passes. Counts every call; `fail_first`
    /// answers the first N calls with that status instead.
    #[derive(Clone, Default)]
    struct Fake {
        calls: Rc<Cell<u32>>,
        fail_first: Rc<RefCell<Vec<u16>>>,
        verdict: Rc<RefCell<Option<Value>>>,
    }

    fn ok(body: &Value) -> atlas::error::Result<HttpResponse> {
        Ok(HttpResponse {
            status: 200,
            body: serde_json::to_vec(body).unwrap(),
            retry_after_secs: None,
        })
    }

    fn spark_answer(content: &str, finish: &str) -> Value {
        json!({
            "choices": [{"finish_reason": finish,
                         "message": {"role": "assistant", "content": content}}],
            "usage": {"prompt_tokens": 120, "completion_tokens": 30},
        })
    }

    impl Transport for Fake {
        fn post_json(
            &self,
            url: &str,
            _: &str,
            body: &Value,
        ) -> atlas::error::Result<HttpResponse> {
            self.calls.set(self.calls.get() + 1);
            if let Some(status) = self.fail_first.borrow_mut().pop() {
                return Ok(HttpResponse {
                    status,
                    body: b"slow down".to_vec(),
                    retry_after_secs: Some(0),
                });
            }
            if url.ends_with("/responses") {
                let prompt = body["input"][0]["content"][0]["text"].as_str().unwrap();
                let text = if prompt == "document page" {
                    PAGE_TOKENS
                } else {
                    ""
                };
                return ok(&json!({
                    "status": "completed",
                    "output": [{"type": "message",
                                "content": [{"type": "output_text", "text": text}]}],
                }));
            }
            assert!(url.ends_with("/chat/completions"), "{url}");
            let answer = self
                .verdict
                .borrow()
                .clone()
                .unwrap_or_else(|| spark_answer(r#"{"verdict":"pass","issues":[]}"#, "stop"));
            ok(&answer)
        }

        fn post_multipart(
            &self,
            _: &str,
            _: &str,
            _: &MultipartFile,
            _: &[MultipartField],
        ) -> atlas::error::Result<HttpResponse> {
            panic!("a2-record never uploads files")
        }

        fn get(&self, _: &str, _: &str) -> atlas::error::Result<HttpResponse> {
            panic!("a2-record never GETs")
        }

        fn delete(&self, _: &str, _: &str) -> atlas::error::Result<HttpResponse> {
            panic!("a2-record never DELETEs")
        }
    }

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    /// Two synthetic samples (only the PNG magic is checked).
    fn fixture(dir: &Path) -> Options {
        let pngs = dir.join("pngs");
        std::fs::create_dir_all(&pngs).unwrap();
        for name in ["syn-doc-clean.png", "syn-doc-dark.png", "not-a-sample.png"] {
            let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
            bytes.extend_from_slice(name.as_bytes());
            std::fs::write(pngs.join(name), bytes).unwrap();
        }
        Options {
            png_dir: pngs,
            out_dir: dir.join("out"),
            base: atlas::model::BASE_URL.to_string(),
            backend: Backend::Hosted,
        }
    }

    fn read_record(opts: &Options, sample: &str) -> Value {
        let path = opts.out_dir.join(format!("{sample}.json"));
        serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
    }

    #[test]
    fn rerun_replays_from_the_cache_with_zero_transport_calls() {
        let tmp = tempfile::tempdir().unwrap();
        let opts = fixture(tmp.path());
        let cache_dir = tmp.path().join("meta-cache");
        let fake = Fake::default();
        let cache = Arc::new(MetaCache::new(&cache_dir, CacheMode::Record));
        let live = CachedTransport::new(fake.clone(), cache);
        assert_eq!(record_all(&live, "key-synthetic", &opts).unwrap().0, 2);
        // Two samples x (four SAM concepts + one Spark judgment).
        assert_eq!(fake.calls.get(), 10);
        let first = read_record(&opts, "syn-doc-clean");
        assert_eq!(first["verdict"], "pass");
        assert_eq!(first["models"], json!(["sam-3.1", "muse-spark-1.3"]));
        assert_eq!(first["sam_backend"], "hosted");
        assert_eq!(first["sam_counts"]["document page"], 1);
        assert_eq!(first["sam_counts"]["face"], 0);
        assert_eq!(first["usage"]["prompt_tokens"], 120);
        assert!(!opts.out_dir.join("not-a-sample.json").exists());

        let again = Fake::default();
        let cache = Arc::new(MetaCache::new(&cache_dir, CacheMode::Record));
        let replay = CachedTransport::new(again.clone(), Arc::clone(&cache));
        record_all(&replay, "key-synthetic", &opts).unwrap();
        assert_eq!(
            again.calls.get(),
            0,
            "second identical run must not call out"
        );
        assert_eq!(cache.counts().1, 0, "hits are not live calls");
        let second = read_record(&opts, "syn-doc-clean");
        assert_eq!(second["boxes"], first["boxes"]);
        assert_eq!(second["verdict"], first["verdict"]);

        // replay-only: offline, no key.
        let offline = Fake::default();
        let cache = Arc::new(MetaCache::new(&cache_dir, CacheMode::ReplayOnly));
        let replay = CachedTransport::new(offline.clone(), cache);
        record_all(&replay, "", &opts).unwrap();
        assert_eq!(offline.calls.get(), 0);
        let empty = Arc::new(MetaCache::new(
            tmp.path().join("empty"),
            CacheMode::ReplayOnly,
        ));
        let err = record_all(&CachedTransport::new(Fake::default(), empty), "", &opts).unwrap_err();
        assert!(err.contains("replay-only"), "{err}");
    }

    #[test]
    fn hosted_boxes_are_fractions_rounded_and_tagged_with_the_prompt() {
        // Same record `ops/open-weights/tests/test_sam3_boxes.py` pins
        // for the open-weights adapter: both backends must agree.
        let answer = json!({"status": "completed", "output": [{"content": [
            {"type": "output_text",
             "text": "0<|box;x1=10;y1=20;x2=110;y2=220;w=400;h=800;c=0.9|>"}]}]});
        let boxes: Vec<PrecheckBox> = media::parse_sam_answer(&answer)
            .unwrap()
            .iter()
            .filter_map(|b| PrecheckBox::from_sam("face", b))
            .collect();
        assert_eq!(
            serde_json::to_value(&boxes).unwrap(),
            json!([{"label": "face", "object_id": 0, "x": 0.025, "y": 0.025,
                    "w": 0.25, "h": 0.25, "score": 0.9}])
        );
        let sam = media::SamBox {
            object_id: 3,
            x: 1.2,
            y: 0.1,
            w: 0.1,
            h: 0.1,
            score: 1.0,
        };
        assert!(
            PrecheckBox::from_sam("face", &sam).is_none(),
            "outside the image"
        );
        assert!((round_to(1.0 / 3.0, 4) - 0.3333).abs() < 1e-12);
    }

    #[test]
    fn requests_keep_the_pinned_shapes() {
        let sam = media::sam_request_body("official seal", b"\x89PNG\r\n\x1a\nrest");
        assert_eq!(sam["model"], "sam-3.1");
        assert_eq!(sam["input"][0]["content"][0]["text"], "official seal");
        let url = sam["input"][0]["content"][1]["image_url"].as_str().unwrap();
        assert!(url.starts_with("data:image/png;base64,"), "{url}");
        let spark = spark_body("syn-doc-dark", "syn doc dark", &[], b"png");
        assert_eq!(spark["model"], SPARK_MODEL);
        assert_eq!(spark["reasoning_effort"], "low");
        assert_eq!(
            spark["response_format"]["json_schema"]["name"],
            "PrecheckVerdict"
        );
        assert!(
            spark.get("temperature").is_none(),
            "Spark keeps its sampling defaults"
        );
        let text = spark["messages"][0]["content"][0]["text"].as_str().unwrap();
        assert!(
            text.contains("Sample syn-doc-dark (syn doc dark)"),
            "{text}"
        );
    }

    #[test]
    fn verdicts_fail_closed() {
        let ok = parse_verdict(&spark_answer(
            r#"{"verdict":"retake","issues":["a","b","c","d","e","f","g","h","i",4]}"#,
            "stop",
        ))
        .unwrap();
        assert_eq!(ok.verdict, "retake");
        assert_eq!(ok.issues.len(), MAX_ISSUES);
        assert_eq!(ok.prompt_tokens, 120);
        for (answer, why) in [
            (spark_answer(r#"{"verdict":"pa"#, "length"), "truncated"),
            (
                spark_answer(r#"{"verdict":"pass","issues":[]}"#, "content_filter"),
                "finish",
            ),
            (
                spark_answer(r#"{"verdict":"maybe","issues":[]}"#, "stop"),
                "pass/retake",
            ),
            (spark_answer(r#"{"verdict":"pass"}"#, "stop"), "issues"),
            (json!({"choices": []}), "no choice"),
            (
                json!({"choices": [{"finish_reason": "stop",
                                    "message": {"refusal": "no", "content": null}}]}),
                "refused",
            ),
        ] {
            let err = parse_verdict(&answer).unwrap_err();
            assert!(err.contains(why), "{why}: {err}");
        }
    }

    #[test]
    fn clean_sample_with_no_boxes_aborts_and_writes_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let opts = fixture(tmp.path());
        let blind = Fake::default();
        // Every SAM answer empty: swap the fake's grammar for nothing.
        struct Blind(Fake);
        impl Transport for Blind {
            fn post_json(
                &self,
                url: &str,
                k: &str,
                body: &Value,
            ) -> atlas::error::Result<HttpResponse> {
                if url.ends_with("/responses") {
                    return ok(&json!({"status": "completed", "output": []}));
                }
                self.0.post_json(url, k, body)
            }
            fn post_multipart(
                &self,
                u: &str,
                k: &str,
                f: &MultipartFile,
                p: &[MultipartField],
            ) -> atlas::error::Result<HttpResponse> {
                self.0.post_multipart(u, k, f, p)
            }
            fn get(&self, u: &str, k: &str) -> atlas::error::Result<HttpResponse> {
                self.0.get(u, k)
            }
            fn delete(&self, u: &str, k: &str) -> atlas::error::Result<HttpResponse> {
                self.0.delete(u, k)
            }
        }
        let err = record_all(&Blind(blind.clone()), "key", &opts).unwrap_err();
        assert!(err.contains("0 boxes on the clean sample"), "{err}");
        assert_eq!(blind.calls.get(), 0, "Spark is never asked");
        assert!(!opts.out_dir.join("syn-doc-clean.json").exists());
    }

    #[test]
    fn retryable_statuses_retry_and_terminal_ones_do_not() {
        let fake = Fake::default();
        fake.fail_first.borrow_mut().extend([503, 429]);
        let url = format!("{}/chat/completions", atlas::model::BASE_URL);
        post(
            &fake,
            &url,
            "key",
            &json!({"model": SPARK_MODEL}),
            "Spark call",
        )
        .unwrap();
        assert_eq!(fake.calls.get(), 3);

        let fake = Fake::default();
        fake.fail_first.borrow_mut().push(401);
        let err = post(&fake, &url, "key", &json!({}), "Spark call").unwrap_err();
        assert!(err.contains("HTTP 401"), "{err}");
        assert_eq!(fake.calls.get(), 1);
    }

    #[test]
    fn flags_pick_the_backend_and_mask_needs_open_weights() {
        let base = ["--png-dir", "a", "--out-dir", "b"];
        let hosted = Options::parse(&args(&base)).unwrap();
        assert_eq!(hosted.backend, Backend::Hosted);
        assert_eq!(hosted.base, "https://api.meta.ai/v1");
        let mut masked = base.to_vec();
        masked.extend(["--mask", "faces,license plates"]);
        assert!(
            Options::parse(&args(&masked))
                .unwrap_err()
                .contains("open-weights")
        );
        masked.extend(["--sam-backend", "open-weights"]);
        let open = Options::parse(&args(&masked)).unwrap();
        assert_eq!(
            open.backend,
            Backend::OpenWeights {
                python: "python3".to_string(),
                script: PathBuf::from("ops/open-weights/sam3_boxes.py"),
                mask: "faces,license plates".to_string(),
            }
        );
        assert!(Options::parse(&args(&["--png-dir", "a"])).is_err());
        assert!(
            Options::parse(&args(&[
                "--png-dir",
                "a",
                "--out-dir",
                "b",
                "--sam-backend",
                "gpu"
            ]))
            .is_err()
        );
        // The default script path resolves from the repo root.
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        assert!(repo.join("ops/open-weights/sam3_boxes.py").is_file());
    }

    #[cfg(unix)]
    #[test]
    fn open_weights_backend_reads_the_script_boxes_and_judges_the_masked_copy() {
        let tmp = tempfile::tempdir().unwrap();
        let mut opts = fixture(tmp.path());
        // Stand-in for sam3_boxes.py, run by `sh` instead of the venv:
        // writes one face box to --out and a marker PNG to --masked-out.
        let script = tmp.path().join("fake_sam3.sh");
        std::fs::write(
            &script,
            r#"while [ $# -gt 0 ]; do
  case "$1" in
    --out) out="$2"; shift ;;
    --masked-out) masked="$2"; shift ;;
  esac
  shift
done
printf '[{"label":"face","object_id":1,"x":0.1,"y":0.1,"w":0.2,"h":0.2,"score":0.8}]' > "$out"
printf '\211PNGmasked' > "$masked"
"#,
        )
        .unwrap();
        opts.backend = Backend::OpenWeights {
            python: "sh".to_string(),
            script,
            mask: "faces".to_string(),
        };
        let fake = Fake::default();
        record_all(&fake, "key", &opts).unwrap();
        // Spark only: the boxes came from the local script.
        assert_eq!(fake.calls.get(), 2);
        let record = read_record(&opts, "syn-doc-clean");
        assert_eq!(record["sam_backend"], "open-weights");
        assert_eq!(record["models"][0], SAM3_OPEN_MODEL);
        assert_eq!(record["masked"], json!(["face"]));
        assert_eq!(record["boxes"][0]["label"], "face");
        assert_eq!(record["sam_counts"], json!({"face": 1}));
    }

    #[test]
    fn stamps_are_utc_iso() {
        assert_eq!(utc_stamp(1_793_664_000 + 3_661), "2026-11-03T01:01:01Z");
    }
}
