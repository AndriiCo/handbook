//! Task runner: pack task → fetched → extracted → budgeted → run.
//!
//! One [`Runner::run`] call executes a task end to end: fetch +
//! cache, extract, transcribe voice notes, chunk + summarize text,
//! measure the budget (estimate, then live `input_tokens` when not
//! dry), extract structured JSON (fast chat, sync Responses for
//! native media, or a background batch per chunk), optionally
//! verify, then write `result.<task>.json` + debug dumps + ledger.
//! `--dry-run` walks the same path with [`DryPort`] (no network).

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::batch::{BatchJob, BatchRunner, job_dir};
use crate::chunk::{Chunk, chunk_text, estimate_tokens};
use crate::config::{AtlasConfig, Effort, ModelSpec, Protocol, TaskSpec, UseMode, Verify};
use crate::error::{Error, Result};
use crate::extract::{Document, ExtractConfig, NativePayload, PdfMode, extract};
use crate::ledger::{CallRecord, Ledger, estimate_cost};
use crate::model::{
    ChatRequest, Completion, INLINE_MEDIA_LIMIT, InputBlock, JsonSchema, ModelPort,
    ResponsesRequest,
};
use crate::render;
use crate::source::{FetchedSource, cache_dir, fetch_source, sha_hex};
use crate::summarize::{ReduceOutcome, Summarizer, SummaryCache, SummaryDraft, reduce};

/// CLI overrides for one run.
#[derive(Debug, Clone, Default)]
pub struct RunOptions {
    pub effort: Option<Effort>,
    pub model: Option<String>,
    pub protocol: Option<Protocol>,
    pub out: Option<PathBuf>,
    pub refresh: bool,
    pub limit: Option<usize>,
    pub dry_run: bool,
    pub pdf_tool: Option<Vec<String>>,
    /// Shared URL fetch cache (`paths.fetch_cache_dir`): every live
    /// source fetch is recorded; `replay-only` reruns from disk.
    pub fetch_cache: Option<std::sync::Arc<callcache::FetchCache>>,
}

/// Dry-run projection (no model calls made).
#[derive(Debug, Clone)]
pub struct TaskPlan {
    pub task: String,
    pub model: String,
    pub mode: UseMode,
    pub effort: Effort,
    pub protocol: Protocol,
    pub docs: usize,
    pub chunks: usize,
    pub est_input_tokens: u32,
    pub calls: usize,
    pub batch: bool,
    pub est_cost_usd: f64,
    pub skipped_cache: usize,
    /// Native attachments (PDF/image blocks) on the extract call.
    pub native_blocks: usize,
    /// Of those, how many need a Files upload (>50 MB inline cap).
    pub uploads: usize,
}

/// Outcome of [`Runner::run`].
#[derive(Debug)]
pub struct TaskOutcome {
    pub plan: TaskPlan,
    pub result: Value,
    pub out_files: Vec<PathBuf>,
}

/// Runs pack tasks through a [`ModelPort`].
pub struct Runner<'a, P: ModelPort> {
    port: &'a P,
    cfg: &'a AtlasConfig,
    pack_dir: &'a Path,
    out: PathBuf,
    run_id: String,
    now: i64,
}

impl<'a, P: ModelPort> Runner<'a, P> {
    pub fn new(
        port: &'a P,
        cfg: &'a AtlasConfig,
        pack_dir: &'a Path,
        out: PathBuf,
        run_id: &str,
        now: i64,
    ) -> Self {
        Self {
            port,
            cfg,
            pack_dir,
            out,
            run_id: run_id.to_string(),
            now,
        }
    }

    /// Execute one task (or project it with `options.dry_run`).
    pub fn run(&self, task_name: &str, options: &RunOptions) -> Result<TaskOutcome> {
        let task = self
            .cfg
            .tasks
            .iter()
            .find(|t| t.name == task_name)
            .ok_or_else(|| Error::Config(format!("unknown task '{task_name}'")))?;
        let model_name = options.model.as_deref().unwrap_or(&task.model);
        let model = self.cfg.model(model_name).ok_or_else(|| {
            Error::Config(format!("task '{task_name}': unknown model '{model_name}'"))
        })?;
        if model.mode == UseMode::Contributor && !task.allow_contributor {
            return Err(Error::Config(format!(
                "task '{task_name}': Contributor model '{model_name}' needs allow_contributor = true"
            )));
        }
        let effort = options
            .effort
            .or(task.effort)
            .unwrap_or(self.cfg.run.effort);
        let preset = effort.preset();
        let schema = load_schema(task)?;
        let base = options.out.clone().unwrap_or_else(|| self.out.clone());
        let out = base.join(&self.run_id);
        let debug = out.join("debug");
        std::fs::create_dir_all(&debug).map_err(Error::Io)?;
        let ledger = Ledger::new(out.join("ledger.jsonl"));

        // 1. Fetch + extract.
        let extract_cfg = ExtractConfig {
            pdf: match &options.pdf_tool {
                Some(argv) => PdfMode::External {
                    argv: argv.clone(),
                    timeout: Duration::from_secs(120),
                    max_output: 8 * 1024 * 1024,
                },
                None => PdfMode::ApiNative,
            },
        };
        let mut docs = Vec::new();
        let mut skipped_cache = 0;
        for spec in self.cfg.task_sources(task) {
            let fetched = fetch_source(
                spec,
                self.pack_dir,
                &cache_dir(&self.out),
                options.refresh,
                self.now,
                options.fetch_cache.as_deref(),
            )?;
            if fetched.from_cache {
                skipped_cache += 1;
            }
            let mut doc = extract(&fetched, &extract_cfg)?;
            // Voice notes become text via the transcribe endpoint
            // (skipped in dry-run: no network).
            if !options.dry_run
                && let Some(NativePayload::Audio {
                    bytes,
                    content_type,
                }) = &doc.native
            {
                let started = Instant::now();
                let transcript =
                    self.port
                        .transcribe(&format!("{}.audio", doc.id), content_type, bytes)?;
                ledger.append(&call_record(
                    &self.run_id,
                    self.now,
                    task_name,
                    model,
                    effort,
                    "transcribe",
                    &fetched.sha256,
                    0,
                    0,
                    started,
                    "debug",
                ))?;
                doc.text = transcript.to_text();
                doc.native = None;
            }
            docs.push((fetched, doc));
        }
        if let Some(limit) = options.limit.or(task_limit(task)) {
            docs.truncate(limit);
        }
        let _ = std::fs::write(
            debug.join("docs.json"),
            serde_json::to_string_pretty(
                &docs
                    .iter()
                    .map(|(fetched, doc)| {
                        serde_json::json!({
                            "id": doc.id, "kind": format!("{:?}", doc.kind),
                            "sha": fetched.sha256, "chars": doc.text.len(),
                            "native": doc.native.is_some(),
                        })
                    })
                    .collect::<Vec<_>>(),
            )
            .expect("docs dump serializes"),
        );

        // 2. Chunk text docs.
        let chunk_tokens = task.chunk_tokens.unwrap_or(preset.chunk_tokens);
        let mut chunks = Vec::new();
        for (_, doc) in &docs {
            if doc.text.is_empty() {
                continue;
            }
            chunks.extend(chunk_text(
                &doc.id,
                &doc.text,
                chunk_tokens,
                preset.overlap_tokens,
            ));
        }
        let _ = std::fs::write(
            debug.join("chunks.json"),
            serde_json::to_string_pretty(
                &chunks
                    .iter()
                    .map(|c| {
                        serde_json::json!({
                            "id": c.id, "tokens": c.tokens,
                            "span": [c.byte_start, c.byte_end],
                        })
                    })
                    .collect::<Vec<_>>(),
            )
            .expect("chunks dump serializes"),
        );

        // 3. Native attachments (PDF/image) force Responses; gather now.
        let natives =
            self.attachments(task_name, model, effort, &docs, &ledger, options.dry_run)?;
        let uploads = natives
            .iter()
            .filter(|b| matches!(b, InputBlock::FileId { .. }))
            .count();

        // 4. Summarize unless everything fits comfortably.
        let total_tokens: u32 = chunks.iter().map(|c| c.tokens).sum();
        let usable = model
            .context_window
            .saturating_sub(model.max_output_tokens + 4096);
        let summarizer = PortSummarizer {
            port: self.port,
            model,
            task: task_name,
            run_id: &self.run_id,
            effort,
            now: self.now,
            ledger: &ledger,
            dry: options.dry_run,
        };
        let cache = SummaryCache::new(self.out.join("summaries"));
        let brief = if chunks.len() <= 1 && total_tokens * 2 <= usable {
            chunks
                .iter()
                .map(|c| c.text.clone())
                .collect::<Vec<_>>()
                .join("\n\n")
        } else {
            let outcome: ReduceOutcome = reduce(
                &summarizer,
                &cache,
                &chunks,
                &summary_instructions(task),
                preset.summary_levels,
                usable,
            )?;
            let _ = std::fs::write(
                debug.join("summaries.json"),
                serde_json::to_string_pretty(
                    &outcome.all.iter().map(|s| &s.key).collect::<Vec<_>>(),
                )
                .expect("summaries dump serializes"),
            );
            outcome.brief
        };

        // 5. Budget: estimate, then measure live when not dry.
        let extract_input = format!("{}\n\n{}", task.instructions, brief);
        let est_tokens = estimate_tokens(&extract_input);
        if est_tokens > usable {
            return Err(Error::Budget(format!(
                "task '{task_name}': estimated {est_tokens} tokens exceed usable window {usable} — raise summary levels or narrow inputs"
            )));
        }
        if !options.dry_run {
            let measured = self.port.count_tokens(
                &model.id,
                &serde_json::json!([{"role": "user", "content": extract_input}]),
            )?;
            if measured > usable {
                return Err(Error::Budget(format!(
                    "task '{task_name}': measured {measured} tokens exceed usable window {usable}"
                )));
            }
        }

        // 6. Protocol: auto switches past the effort threshold.
        let est_calls = chunks.len().max(1) + 1; // +1 merge/extract
        let mut protocol = options.protocol.unwrap_or(task.protocol);
        if protocol == Protocol::Auto {
            protocol = if est_calls >= preset.batch_threshold || !natives.is_empty() {
                Protocol::Batch
            } else {
                Protocol::Fast
            };
        }
        // Fast chat is text-only: native attachments need Responses.
        let via_responses = !natives.is_empty();

        let plan = TaskPlan {
            task: task_name.to_string(),
            model: model_name.to_string(),
            mode: model.mode,
            effort,
            protocol,
            docs: docs.len(),
            chunks: chunks.len(),
            est_input_tokens: est_tokens,
            calls: est_calls,
            batch: protocol == Protocol::Batch,
            est_cost_usd: estimate_cost(model, est_tokens, model.max_output_tokens),
            skipped_cache,
            native_blocks: natives.len(),
            uploads,
        };
        if options.dry_run {
            return Ok(TaskOutcome {
                plan,
                result: Value::Null,
                out_files: vec![debug.join("docs.json"), debug.join("chunks.json")],
            });
        }

        // 7. Extract structured JSON.
        let started = Instant::now();
        let completion = match protocol {
            Protocol::Fast if !via_responses => self.port.complete_chat(&ChatRequest {
                model: model.id.clone(),
                instructions: task.instructions.clone(),
                user_text: brief.clone(),
                schema: schema.clone(),
                max_output_tokens: model.max_output_tokens,
                temperature: 0.1,
                reasoning_effort: None,
                cache_key: Some(format!(
                    "atlas-{}-{}",
                    task_name,
                    short_sha(&task.instructions)
                )),
            })?,
            _ => self.extract_via_responses(
                task,
                model,
                schema.as_ref(),
                &brief,
                &natives,
                &chunks,
                protocol,
                &out,
            )?,
        };
        ledger.append(&call_record(
            &self.run_id,
            self.now,
            task_name,
            model,
            effort,
            if via_responses { "responses" } else { "chat" },
            &sha_hex(extract_input.as_bytes()),
            completion.input_tokens,
            completion.output_tokens,
            started,
            "result",
        ))?;
        let _ = std::fs::write(debug.join("extract.txt"), &completion.text);
        let mut result = extract_json(&completion.text)?;

        // 8. Verify pass.
        let verify = task
            .verify
            .as_deref()
            .map(Verify::parse)
            .unwrap_or(preset.verify);
        if verify != Verify::None {
            let started = Instant::now();
            let verifier = match &verify {
                Verify::Model(name) => self.cfg.model(name).unwrap_or(model),
                _ => model,
            };
            let check = self.port.complete_chat(&ChatRequest {
                model: verifier.id.clone(),
                instructions: format!(
                    "{}\n\nVerify the JSON below against the brief: fix contradictions, keep the schema, return JSON only.",
                    task.instructions
                ),
                user_text: format!("BRIEF:\n{brief}\n\nJSON:\n{result}"),
                schema: schema.clone(),
                max_output_tokens: verifier.max_output_tokens,
                temperature: 0.1,
                reasoning_effort: None,
                cache_key: None,
            })?;
            ledger.append(&call_record(
                &self.run_id,
                self.now,
                task_name,
                verifier,
                effort,
                "verify",
                &sha_hex(completion.text.as_bytes()),
                check.input_tokens,
                check.output_tokens,
                started,
                "result",
            ))?;
            result = extract_json(&check.text)?;
        }

        // 9. Write result + optional table rendering.
        let mut out_files = vec![debug.join("docs.json"), debug.join("chunks.json")];
        let result_path = out.join(format!("result.{task_name}.json"));
        std::fs::write(
            &result_path,
            serde_json::to_string_pretty(&result).expect("result serializes"),
        )
        .map_err(Error::Io)?;
        out_files.push(result_path);
        if let Some((columns, rows)) = render::table_from_result(&result) {
            let html_path = out.join(format!("status.{task_name}.html"));
            let svg_path = out.join(format!("status.{task_name}.svg"));
            std::fs::write(&html_path, render::render_status_html(&columns, &rows))
                .map_err(Error::Io)?;
            std::fs::write(&svg_path, render::render_status_svg(&columns, &rows))
                .map_err(Error::Io)?;
            out_files.push(html_path);
            out_files.push(svg_path);
        }
        out_files.push(ledger.path().clone());
        Ok(TaskOutcome {
            plan,
            result,
            out_files,
        })
    }

    /// Upload oversized media, build Responses input blocks.
    /// In dry-run no upload happens: oversized files count as
    /// uploads with an explicit placeholder id (never sent).
    #[allow(clippy::too_many_arguments)]
    fn attachments(
        &self,
        task_name: &str,
        model: &ModelSpec,
        effort: Effort,
        docs: &[(FetchedSource, Document)],
        ledger: &Ledger,
        dry: bool,
    ) -> Result<Vec<InputBlock>> {
        let mut blocks = Vec::new();
        for (fetched, doc) in docs {
            let (bytes, filename, upload) = match &doc.native {
                Some(NativePayload::Pdf(bytes)) => (bytes.clone(), format!("{}.pdf", doc.id), true),
                Some(NativePayload::Image {
                    bytes,
                    content_type,
                }) => {
                    let ext = match content_type.as_str() {
                        "image/png" => "png",
                        "image/jpeg" => "jpg",
                        _ => "img",
                    };
                    (
                        bytes.clone(),
                        format!("{doc_id}.{ext}", doc_id = doc.id),
                        false,
                    )
                }
                _ => continue,
            };
            if upload && bytes.len() > INLINE_MEDIA_LIMIT {
                if dry {
                    blocks.push(InputBlock::FileId {
                        id: "dry-run-placeholder".to_string(),
                    });
                    continue;
                }
                let started = Instant::now();
                let id = self
                    .port
                    .upload_file(&filename, "application/pdf", &bytes)?;
                ledger.append(&call_record(
                    &self.run_id,
                    self.now,
                    task_name,
                    model,
                    effort,
                    "upload",
                    &fetched.sha256,
                    0,
                    0,
                    started,
                    &id,
                ))?;
                blocks.push(InputBlock::FileId { id });
            } else if matches!(doc.native, Some(NativePayload::Pdf(_))) {
                blocks.push(InputBlock::FileData {
                    filename,
                    data_url: format!("data:application/pdf;base64,{}", base64_encode(&bytes)),
                });
            } else {
                let mime = match &doc.native {
                    Some(NativePayload::Image { content_type, .. }) => content_type.clone(),
                    _ => "application/octet-stream".to_string(),
                };
                blocks.push(InputBlock::ImageUrl {
                    url: format!("data:{mime};base64,{}", base64_encode(&bytes)),
                });
            }
        }
        Ok(blocks)
    }

    /// Structured extract over Responses: one sync call, or a
    /// per-chunk background batch + merge.
    #[allow(clippy::too_many_arguments)]
    fn extract_via_responses(
        &self,
        task: &TaskSpec,
        model: &ModelSpec,
        schema: Option<&JsonSchema>,
        brief: &str,
        natives: &[InputBlock],
        chunks: &[Chunk],
        protocol: Protocol,
        out: &Path,
    ) -> Result<Completion> {
        if protocol != Protocol::Batch || chunks.len() < 2 {
            let mut input = vec![InputBlock::Text(format!(
                "{}\n\n{brief}",
                task.instructions
            ))];
            input.extend(natives.iter().cloned());
            let created = self.port.create_response(&ResponsesRequest {
                model: model.id.clone(),
                instructions: task.instructions.clone(),
                input,
                schema: schema.cloned(),
                max_output_tokens: Some(model.max_output_tokens),
                reasoning_effort: None,
                background: false,
                previous_response_id: None,
            })?;
            return created
                .completion
                .ok_or_else(|| Error::Model("sync response finished without output".to_string()));
        }
        // Batch: one background request per chunk, then merge.
        let job_id = format!("{}-extract", self.run_id);
        let dir = job_dir(out, &job_id);
        let requests = chunks
            .iter()
            .map(|chunk| {
                (
                    chunk.id.clone(),
                    ResponsesRequest {
                        model: model.id.clone(),
                        instructions: task.instructions.clone(),
                        input: vec![InputBlock::Text(format!(
                            "PART {} OF {} — extract per instructions; return JSON only.\n\n{}",
                            chunk.id,
                            chunks.len(),
                            chunk.text
                        ))],
                        schema: None, // parts are free-form; the merge enforces the schema
                        max_output_tokens: Some(model.max_output_tokens),
                        reasoning_effort: None,
                        background: true,
                        previous_response_id: None,
                    },
                )
            })
            .collect::<Vec<_>>();
        let mut job = BatchJob::create(&dir, &job_id, &model.id, self.now, requests)?;
        let runner = BatchRunner::new(self.port, model.rpm.max(1), Duration::from_secs(5));
        runner.submit_pending(&mut job, &dir)?;
        let (done, pending, failed) = runner.poll_until_done(&mut job, &dir, 120)?;
        if pending > 0 || failed > 0 {
            return Err(Error::Batch(format!(
                "job {job_id}: {done} done, {pending} pending, {failed} failed — resume with --batch-poll {job_id}"
            )));
        }
        let parts = job
            .collect()
            .into_iter()
            .map(|(key, text)| format!("[{key}]\n{text}"))
            .collect::<Vec<_>>()
            .join("\n\n");
        let mut input = vec![InputBlock::Text(format!(
            "{}\n\nMerge these per-part extractions into one result; return JSON only.\n\n{parts}",
            task.instructions
        ))];
        input.extend(natives.iter().cloned());
        let created = self.port.create_response(&ResponsesRequest {
            model: model.id.clone(),
            instructions: task.instructions.clone(),
            input,
            schema: schema.cloned(),
            max_output_tokens: Some(model.max_output_tokens),
            reasoning_effort: None,
            background: false,
            previous_response_id: None,
        })?;
        created
            .completion
            .ok_or_else(|| Error::Model("merge response finished without output".to_string()))
    }
}

/// [`Summarizer`] over a [`ModelPort`] (sync chat, no schema).
struct PortSummarizer<'a, P: ModelPort> {
    port: &'a P,
    model: &'a ModelSpec,
    task: &'a str,
    run_id: &'a str,
    effort: Effort,
    now: i64,
    ledger: &'a Ledger,
    dry: bool,
}

impl<P: ModelPort> Summarizer for PortSummarizer<'_, P> {
    fn summarize(&self, instructions: &str, input: &str) -> Result<SummaryDraft> {
        if self.dry {
            return Ok(SummaryDraft {
                text: String::new(),
                input_tokens: estimate_tokens(input),
                output_tokens: 0,
            });
        }
        let started = Instant::now();
        let out = self.port.complete_chat(&ChatRequest {
            model: self.model.id.clone(),
            instructions: instructions.to_string(),
            user_text: input.to_string(),
            schema: None,
            max_output_tokens: 2048,
            temperature: 0.1,
            reasoning_effort: None,
            cache_key: Some(format!("atlas-sum-{}", short_sha(instructions))),
        })?;
        self.ledger.append(&CallRecord {
            run_id: self.run_id.to_string(),
            at: self.now,
            task: self.task.to_string(),
            model: self.model.name.clone(),
            mode: format!("{:?}", self.model.mode).to_lowercase(),
            effort: format!("{:?}", self.effort).to_lowercase(),
            protocol: "fast".to_string(),
            kind: "summarize".to_string(),
            input_sha: sha_hex(input.as_bytes()),
            input_tokens: out.input_tokens,
            output_tokens: out.output_tokens,
            cached_tokens: out.cached_tokens,
            cost_usd: estimate_cost(self.model, out.input_tokens, out.output_tokens),
            elapsed_ms: started.elapsed().as_millis() as u64,
            output: "debug".to_string(),
        })?;
        Ok(SummaryDraft {
            text: out.text,
            input_tokens: out.input_tokens,
            output_tokens: out.output_tokens,
        })
    }
}

fn task_limit(task: &TaskSpec) -> Option<usize> {
    (task.limit > 0).then_some(task.limit)
}

fn summary_instructions(task: &TaskSpec) -> String {
    format!(
        "Summarize for later structured extraction. Keep every name, date, number, \
         and footnote marker verbatim; keep chunk ids cited as [id]. Task context: {}",
        task.instructions.lines().next().unwrap_or("")
    )
}

fn load_schema(task: &TaskSpec) -> Result<Option<JsonSchema>> {
    let Some(path) = task.schema.as_deref() else {
        return Ok(None);
    };
    let text = std::fs::read_to_string(path).map_err(Error::Io)?;
    let schema: Value =
        serde_json::from_str(&text).map_err(|e| Error::Config(format!("schema {path}: {e}")))?;
    Ok(Some(JsonSchema {
        name: sanitize_schema_name(&task.name),
        schema,
        strict: false,
    }))
}

fn sanitize_schema_name(task: &str) -> String {
    let clean: String = task
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .take(64)
        .collect();
    if clean.is_empty() {
        "Result".to_string()
    } else {
        clean
    }
}

fn short_sha(text: &str) -> String {
    sha_hex(text.as_bytes()).chars().take(12).collect()
}

fn base64_encode(bytes: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// Parse model JSON: whole body, else the first `{…}` span.
fn extract_json(text: &str) -> Result<Value> {
    if let Ok(value) = serde_json::from_str(text) {
        return Ok(value);
    }
    let (Some(start), Some(end)) = (text.find('{'), text.rfind('}')) else {
        return Err(Error::Model("model output is not JSON".to_string()));
    };
    if start >= end {
        return Err(Error::Model("model output is not JSON".to_string()));
    }
    serde_json::from_str(&text[start..=end])
        .map_err(|_| Error::Model("model output is not JSON".to_string()))
}

#[allow(clippy::too_many_arguments)]
fn call_record(
    run_id: &str,
    now: i64,
    task: &str,
    model: &ModelSpec,
    effort: Effort,
    kind: &str,
    input_sha: &str,
    input_tokens: u32,
    output_tokens: u32,
    started: Instant,
    output: &str,
) -> CallRecord {
    CallRecord {
        run_id: run_id.to_string(),
        at: now,
        task: task.to_string(),
        model: model.name.clone(),
        mode: format!("{:?}", model.mode).to_lowercase(),
        effort: format!("{effort:?}").to_lowercase(),
        protocol: "fast".to_string(),
        kind: kind.to_string(),
        input_sha: input_sha.to_string(),
        input_tokens,
        output_tokens,
        cached_tokens: 0,
        cost_usd: estimate_cost(model, input_tokens, output_tokens),
        elapsed_ms: started.elapsed().as_millis() as u64,
        output: output.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AtlasConfig;
    use crate::model::StubPort;

    const PACK: &str = r#"
[run]
out = "out-test"
effort = "low"

[[models]]
name = "spark"
id = "muse-spark-1.3"
context_window = 1048576
input_per_1m = 2.0
output_per_1m = 8.0

[[sources]]
name = "note"
url = "note.txt"
kind = "text"

[[tasks]]
name = "scan"
model = "spark"
instructions = "Extract."
inputs = ["note"]
"#;

    fn fixture(pack_dir: &Path) {
        std::fs::write(
            pack_dir.join("note.txt"),
            "Viet Nam: member since 2026-09-11.",
        )
        .unwrap();
    }

    #[test]
    fn dry_run_projects_without_calls() {
        let tmp = tempfile::tempdir().unwrap();
        fixture(tmp.path());
        let cfg = AtlasConfig::from_toml_str(PACK).unwrap();
        let port = StubPort::new("{}", "{}");
        let out = tmp.path().join("out");
        let runner = Runner::new(&port, &cfg, tmp.path(), out, "run-1", 1000);
        let outcome = runner
            .run(
                "scan",
                &RunOptions {
                    dry_run: true,
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(outcome.plan.task, "scan");
        assert_eq!(outcome.plan.docs, 1);
        assert_eq!(outcome.plan.chunks, 1);
        assert!(!outcome.plan.batch);
        assert!(outcome.result.is_null());
    }

    #[test]
    fn fast_run_writes_result_and_ledger() {
        let tmp = tempfile::tempdir().unwrap();
        fixture(tmp.path());
        let cfg = AtlasConfig::from_toml_str(PACK).unwrap();
        let port = StubPort::new(r#"{"columns":["S"],"rows":[["x"]]}"#, "{}");
        let out = tmp.path().join("out");
        let runner = Runner::new(&port, &cfg, tmp.path(), out.clone(), "run-2", 1000);
        let outcome = runner.run("scan", &RunOptions::default()).unwrap();
        assert_eq!(outcome.result["columns"][0], "S");
        assert!(out.join("run-2").join("result.scan.json").is_file());
        assert!(out.join("run-2").join("status.scan.html").is_file());
        assert!(out.join("run-2").join("status.scan.svg").is_file());
        let ledger = std::fs::read_to_string(out.join("run-2").join("ledger.jsonl")).unwrap();
        assert!(ledger.contains("\"kind\":\"chat\""));
    }

    #[test]
    fn json_spans_extract() {
        assert_eq!(extract_json(r#"{"a":1}"#).unwrap()["a"], 1);
        assert_eq!(extract_json("prefix {\"a\":2} suffix").unwrap()["a"], 2);
        assert!(extract_json("no json here").is_err());
    }
}
